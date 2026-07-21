// Thin client for the offchain API -- only builds unsigned tx CBOR; signing/submitting happen in the browser via the connected wallet (wallet.ts).

const BASE_URL = import.meta.env.PUBLIC_API_BASE_URL ?? "http://localhost:3000";

export interface EscrowDatum {
  reviewer: string; // hex, 28-byte payment key hash
  proposer: string;
  total_locked: number;
  tranche_bps: number[];
  approved: boolean[];
  released_count: number;
  receipt_policy_id: string;
}

export interface MilestoneMeta {
  name: string;
  description: string;
}

/** `grant_id` is stable for the grant's lifetime; `tx_hash`/`output_index` are the *current* live UTxO and change on every approve/release -- read fresh, never cache across a submit. */
export interface GrantSummary {
  grant_id: string;
  tx_hash: string;
  output_index: number;
  lovelace: number;
  datum: EscrowDatum;
  name: string;
  milestones: MilestoneMeta[] | null;
  trusted: boolean;
  warnings: string[];
  completed: boolean;
}

export interface TxSummary {
  tx_hash: string;
  block_time: number;
  block_height: number;
}

export interface UtxoRefInput {
  tx_hash: string;
  output_index: number;
}

class ApiError extends Error {
  constructor(
    message: string,
    public status: number,
  ) {
    super(message);
  }
}

async function request<T>(path: string, init?: RequestInit): Promise<T> {
  const res = await fetch(`${BASE_URL}${path}`, {
    ...init,
    headers: { "Content-Type": "application/json", ...init?.headers },
  });
  const body = await res.json().catch(() => null);
  if (!res.ok) {
    const message = body?.error ?? `request to ${path} failed with ${res.status}`;
    throw new ApiError(message, res.status);
  }
  return body as T;
}

export async function getGrants(): Promise<GrantSummary[]> {
  return request<GrantSummary[]>("/grants");
}

/** Role `address` matches for this grant's current UTxO -- lets the UI give a "wrong wallet" message before building a tx that'd be rejected server-side. */
export async function getGrantRole(address: string, grant: UtxoRefInput): Promise<"funder" | "grantee" | "none"> {
  const params = new URLSearchParams({
    address,
    tx_hash: grant.tx_hash,
    output_index: String(grant.output_index),
  });
  const res = await request<{ role: "funder" | "grantee" | "none" }>(`/grants/role?${params}`);
  return res.role;
}

export interface GrantMetaRequest {
  proposer_address: string;
  reviewer_address: string;
  tranche_bps: number[];
  total_locked: number;
  name: string;
  milestones: MilestoneMeta[];
}

/** Stores grant name/milestone metadata ahead of `postCreateEscrow`; same key fields let the backend re-derive the same `grant_id` once the escrow is on-chain. */
export async function postGrantMeta(req: GrantMetaRequest): Promise<string> {
  const res = await request<{ grant_id: string }>("/grants", {
    method: "POST",
    body: JSON.stringify(req),
  });
  return res.grant_id;
}

/** Transactions for one grant's own lineage only, never every grant at the shared escrow address. */
export async function getTransactions(grant: UtxoRefInput): Promise<TxSummary[]> {
  const params = new URLSearchParams({
    tx_hash: grant.tx_hash,
    output_index: String(grant.output_index),
  });
  return request<TxSummary[]>(`/transactions?${params}`);
}

export interface CreateEscrowRequest {
  proposer_address: string;
  reviewer_address: string;
  tranche_bps: number[];
  total_locked: number;
  fee_input: UtxoRefInput;
  fee_input_lovelace: number;
}

export interface ApproveMilestoneRequest {
  milestone_index: number;
  reviewer_address: string;
  tx_hash: string;
  output_index: number;
  fee_input: UtxoRefInput;
  fee_input_lovelace: number;
  collateral: UtxoRefInput;
}

export interface ReleaseTrancheRequest {
  milestone_index: number;
  submitter_address: string;
  proposer_address: string;
  tx_hash: string;
  output_index: number;
  fee_input: UtxoRefInput;
  fee_input_lovelace: number;
  collateral: UtxoRefInput;
}

interface UnsignedTxResponse {
  unsigned_tx_cbor: string;
}

export async function postCreateEscrow(req: CreateEscrowRequest): Promise<string> {
  const res = await request<UnsignedTxResponse>("/tx/create-escrow", {
    method: "POST",
    body: JSON.stringify(req),
  });
  return res.unsigned_tx_cbor;
}

export async function postApproveMilestone(req: ApproveMilestoneRequest): Promise<string> {
  const res = await request<UnsignedTxResponse>("/tx/approve-milestone", {
    method: "POST",
    body: JSON.stringify(req),
  });
  return res.unsigned_tx_cbor;
}

export async function postReleaseTranche(req: ReleaseTrancheRequest): Promise<string> {
  const res = await request<UnsignedTxResponse>("/tx/release-tranche", {
    method: "POST",
    body: JSON.stringify(req),
  });
  return res.unsigned_tx_cbor;
}
