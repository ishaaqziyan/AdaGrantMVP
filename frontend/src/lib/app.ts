import "./buffer-shim";
import { BrowserWallet } from "@meshsdk/wallet";
import {
  getGrantRole,
  getGrants,
  getTransactions,
  postApproveMilestone,
  postCreateEscrow,
  postGrantMeta,
  postReleaseTranche,
  type GrantSummary,
  type MilestoneMeta,
  type TxSummary,
} from "./api";
import { currentAddress, pickCollateral, pickFeeInput } from "./wallet";

const EXPLORER_TX_BASE_URL =
  import.meta.env.PUBLIC_EXPLORER_TX_BASE_URL ?? "https://preview.cardanoscan.io/transaction";

/** Grant name and milestone name/description are funder-supplied free text
 * (`postGrantMeta`, unauthenticated) rendered into `innerHTML` -- escape
 * before interpolating so a malicious grant/milestone name can't inject
 * markup into a wallet-connected page. */
function escapeHtml(s: string): string {
  return s
    .replace(/&/g, "&amp;")
    .replace(/</g, "&lt;")
    .replace(/>/g, "&gt;")
    .replace(/"/g, "&quot;")
    .replace(/'/g, "&#39;");
}

// On-chain, "reviewer" is the only field the validator ever checks a
// signature for (must sign ApproveMilestone) -- that's the Funder here.
// "proposer" is a pure payout destination the validator never checks a
// signature for (ReleaseTranche can be submitted by anyone) -- that's the
// Grantee. No contract change needed for this split, only which typed vs.
// connected address feeds which request field.
type Role = "funder" | "grantee";

type Status = { kind: "idle" | "busy" | "error" | "success"; message: string };

/** A just-submitted tx we haven't seen confirmed yet. `grantId` is stable
 * across spends (see `GrantSummary` docs in api.ts); `txHashBefore` is that
 * grant's live UTxO tx_hash at submit time (or null if the grant didn't
 * exist yet). Every redeemer spend and the create-escrow tx both produce a
 * *new* UTxO with a new tx_hash for the same grant_id, so comparing
 * against this on refresh is how we detect confirmation without guessing
 * at Blockfrost's indexing lag. */
type PendingTx = { hash: string; action: string; grantId: string; txHashBefore: string | null };

let role: Role | null = null;
let wallet: BrowserWallet | null = null;
let address: string | null = null;
let grants: GrantSummary[] | undefined = undefined; // undefined = not loaded yet
let grantsError: string | null = null; // set on fetch failure, so render() doesn't refire forever
let selectedGrantId: string | null = null;
let walletRole: "funder" | "grantee" | "none" | undefined = undefined; // undefined = not checked yet
let walletRoleError: string | null = null;
let transactions: TxSummary[] | undefined = undefined;
let status: Status = { kind: "idle", message: "" };
let pendingTx: PendingTx | null = null;

// In-flight guards: `render()` can re-run (setStatus, poll ticks, sibling
// fetches resolving) before an async fetch triggered by an earlier render
// finishes. Without these, every re-render while the corresponding state
// is still undefined refires the same request -- cheap for most endpoints,
// but /transactions can cost up to ~100 Blockfrost calls per hit, so a
// duplicate refire compounds fast.
let grantsLoading = false;
let walletRoleLoading = false;
let transactionsLoading = false;

const appEl = document.getElementById("app") as HTMLElement;

function setStatus(kind: Status["kind"], message = "") {
  status = { kind, message };
  render();
}

function switchRole(newRole: Role | null) {
  stopAccountPolling();
  stopPendingPolling();
  role = newRole;
  wallet = null;
  address = null;
  grants = undefined;
  grantsError = null;
  selectedGrantId = null;
  walletRole = undefined;
  walletRoleError = null;
  transactions = undefined;
  pendingTx = null;
  status = { kind: "idle", message: "" };
  grantsLoading = false;
  walletRoleLoading = false;
  transactionsLoading = false;
  render();
}

// CIP-30 has no push-based "account changed" event (unlike EIP-1193's
// accountsChanged) -- the only way to detect a user switching accounts
// inside the wallet extension itself is to keep re-checking.
const ACCOUNT_POLL_INTERVAL_MS = 2000;
let accountPollTimer: ReturnType<typeof setInterval> | null = null;

function startAccountPolling() {
  stopAccountPolling();
  accountPollTimer = setInterval(async () => {
    if (!wallet) return;
    try {
      const current = await currentAddress(wallet);
      if (current !== address) {
        address = current;
        grants = undefined; // re-check grants under the new address
        grantsError = null;
        selectedGrantId = null;
        walletRole = undefined;
        walletRoleError = null;
        pendingTx = null;
        stopPendingPolling();
        setStatus("idle", "wallet account changed");
      }
    } catch {
      // Wallet may be mid-switch or briefly locked; next poll retries.
    }
  }, ACCOUNT_POLL_INTERVAL_MS);
}

function stopAccountPolling() {
  if (accountPollTimer !== null) {
    clearInterval(accountPollTimer);
    accountPollTimer = null;
  }
}

const PENDING_POLL_INTERVAL_MS = 30_000;
let pendingPollTimer: ReturnType<typeof setInterval> | null = null;

function startPendingPolling() {
  stopPendingPolling();
  pendingPollTimer = setInterval(() => {
    refreshPending();
  }, PENDING_POLL_INTERVAL_MS);
}

function stopPendingPolling() {
  if (pendingPollTimer !== null) {
    clearInterval(pendingPollTimer);
    pendingPollTimer = null;
  }
}

async function connect(walletId: string) {
  setStatus("busy", `connecting ${walletId}...`);
  try {
    wallet = await BrowserWallet.enable(walletId);
    address = await currentAddress(wallet);
    setStatus("idle");
    startAccountPolling();
  } catch (e) {
    setStatus("error", e instanceof Error ? e.message : String(e));
  }
}

function disconnect() {
  stopAccountPolling();
  stopPendingPolling();
  wallet = null;
  address = null;
  grants = undefined;
  grantsError = null;
  selectedGrantId = null;
  walletRole = undefined;
  walletRoleError = null;
  pendingTx = null;
  grantsLoading = false;
  walletRoleLoading = false;
  transactionsLoading = false;
  setStatus("idle");
}

async function signAndSubmit(cbor: string): Promise<string> {
  if (!wallet) throw new Error("not connected");
  setStatus("busy", "waiting for wallet signature...");
  const signed = await wallet.signTx(cbor, true);
  setStatus("busy", "submitting...");
  return wallet.submitTx(signed);
}

async function withWallet(actionLabel: string, grantId: string, txHashBefore: string | null, action: () => Promise<string>) {
  if (!wallet || !address) {
    setStatus("error", "connect a wallet first");
    return;
  }
  if (pendingTx) {
    setStatus("error", "a transaction is already processing -- refresh to check its status first");
    return;
  }
  setStatus("busy", "building transaction...");
  try {
    const hash = await action();
    pendingTx = { hash, action: actionLabel, grantId, txHashBefore };
    startPendingPolling();
    setStatus("success", `submitted: ${hash}`);
  } catch (e) {
    setStatus("error", e instanceof Error ? e.message : String(e));
  }
}

function selectGrant(grantId: string) {
  selectedGrantId = grantId;
  walletRole = undefined;
  walletRoleError = null;
  transactions = undefined;
  walletRoleLoading = false;
  transactionsLoading = false;
  render();
}

function approveMilestone(index: number) {
  const grant = grants?.find((g) => g.grant_id === selectedGrantId);
  if (!grant) return;
  return withWallet(`approve milestone ${index + 1}`, grant.grant_id, grant.tx_hash, async () => {
    const feeInput = await pickFeeInput(wallet!);
    const collateral = await pickCollateral(wallet!, feeInput);
    const cbor = await postApproveMilestone({
      milestone_index: index,
      reviewer_address: address!, // funder
      tx_hash: grant.tx_hash,
      output_index: grant.output_index,
      fee_input: { tx_hash: feeInput.tx_hash, output_index: feeInput.output_index },
      fee_input_lovelace: feeInput.lovelace,
      collateral,
    });
    return signAndSubmit(cbor);
  });
}

function releaseTranche(index: number) {
  const grant = grants?.find((g) => g.grant_id === selectedGrantId);
  if (!grant) return;
  return withWallet(`release tranche ${index + 1}`, grant.grant_id, grant.tx_hash, async () => {
    const feeInput = await pickFeeInput(wallet!);
    const collateral = await pickCollateral(wallet!, feeInput);
    const cbor = await postReleaseTranche({
      milestone_index: index,
      submitter_address: address!, // grantee (anyone can actually submit this)
      proposer_address: address!, // payout target -- the frontend only shows Release once this wallet is confirmed as grantee
      tx_hash: grant.tx_hash,
      output_index: grant.output_index,
      fee_input: { tx_hash: feeInput.tx_hash, output_index: feeInput.output_index },
      fee_input_lovelace: feeInput.lovelace,
      collateral,
    });
    return signAndSubmit(cbor);
  });
}

async function createGrant(
  granteeAddress: string,
  trancheBps: number[],
  totalLocked: number,
  name: string,
  milestones: MilestoneMeta[],
) {
  if (!wallet || !address) {
    setStatus("error", "connect a wallet first");
    return;
  }
  if (pendingTx) {
    setStatus("error", "a transaction is already processing -- refresh to check its status first");
    return;
  }

  setStatus("busy", "saving grant details...");
  let grantId: string;
  try {
    grantId = await postGrantMeta({
      proposer_address: granteeAddress,
      reviewer_address: address, // funder: connected wallet, must approve later
      tranche_bps: trancheBps,
      total_locked: totalLocked,
      name,
      milestones,
    });
  } catch (e) {
    setStatus("error", e instanceof Error ? e.message : String(e));
    return;
  }

  return withWallet("create grant", grantId, null, async () => {
    const feeInput = await pickFeeInput(wallet!);
    const cbor = await postCreateEscrow({
      proposer_address: granteeAddress, // grantee: typed in, pure payout destination
      reviewer_address: address!,
      tranche_bps: trancheBps,
      total_locked: totalLocked,
      fee_input: { tx_hash: feeInput.tx_hash, output_index: feeInput.output_index },
      fee_input_lovelace: feeInput.lovelace,
    });
    return signAndSubmit(cbor);
  });
}

/** Re-checks whether a pending tx has landed by comparing the target
 * grant's tx_hash against what it was before submission (see `PendingTx`).
 * Manual, not polled on a fixed schedule alone -- Blockfrost's indexing lag
 * is variable enough that a fixed poll interval would either spam it or
 * feel sluggish; a refresh button puts the user in control instead. */
async function refreshPending() {
  if (!pendingTx) return;
  setStatus("busy", "checking...");
  let newGrants: GrantSummary[];
  try {
    newGrants = await getGrants();
  } catch (e) {
    setStatus("error", e instanceof Error ? e.message : String(e));
    return;
  }
  const match = newGrants.find((g) => g.grant_id === pendingTx!.grantId);
  const newTxHash = match?.tx_hash ?? null;
  if (newTxHash !== pendingTx.txHashBefore) {
    grants = newGrants;
    selectedGrantId = pendingTx.grantId;
    walletRole = undefined; // re-check role against the now-current grant
    transactions = undefined; // refetch so the new tx shows up
    walletRoleLoading = false;
    transactionsLoading = false;
    pendingTx = null;
    stopPendingPolling();
    setStatus("success", "confirmed on-chain");
  } else {
    setStatus("idle", "still processing -- Blockfrost can lag behind submission, try again shortly");
  }
}

// --- rendering ---

/** render() rebuilds the whole #app subtree from scratch on every call,
 * including calls triggered by unrelated status changes (poll ticks,
 * setStatus during an in-flight submit) -- without this, an open
 * create-grant <details> and any text the funder had already typed into it
 * gets wiped mid-edit. Snapshot it before the wipe, reapply after. */
function captureCreateFormState(): { open: boolean; values: Record<string, string> } | null {
  const form = appEl.querySelector<HTMLFormElement>("#create-form");
  if (!form) return null;
  const details = form.closest("details");
  const values: Record<string, string> = {};
  for (const [k, v] of new FormData(form).entries()) values[k] = String(v);
  return { open: details?.open ?? false, values };
}

function restoreCreateFormState(saved: { open: boolean; values: Record<string, string> } | null) {
  if (!saved) return;
  const form = appEl.querySelector<HTMLFormElement>("#create-form");
  if (!form) return;
  const details = form.closest("details");
  if (details) details.open = saved.open;
  for (const [name, value] of Object.entries(saved.values)) {
    const el = form.elements.namedItem(name);
    if (el instanceof HTMLInputElement) el.value = value;
  }
}

function render() {
  const savedCreateFormState = captureCreateFormState();
  appEl.innerHTML = "";

  if (!role) {
    appEl.appendChild(renderRoleSelect());
    return;
  }

  const back = document.createElement("button");
  back.textContent = "← switch role";
  back.addEventListener("click", () => switchRole(null));
  appEl.appendChild(back);

  const roleLabel = document.createElement("p");
  roleLabel.innerHTML = `role: <strong>${role === "funder" ? "Funder" : "Grantee"}</strong>`;
  appEl.appendChild(roleLabel);

  appEl.appendChild(renderWalletBar());
  appEl.appendChild(renderStatus());

  if (!address) return;

  if (pendingTx) {
    appEl.appendChild(renderPendingTx());
    return;
  }

  if (grants === undefined) {
    if (grantsError) {
      appEl.appendChild(renderFetchError(grantsError, () => {
        grantsError = null;
        render();
      }));
      return;
    }
    const p = document.createElement("p");
    p.textContent = "loading grants...";
    appEl.appendChild(p);
    if (!grantsLoading) {
      grantsLoading = true;
      getGrants()
        .then((g) => {
          grantsLoading = false;
          grants = g;
          if (!selectedGrantId && g.length > 0) selectedGrantId = g[0].grant_id;
          render();
        })
        .catch((e) => {
          grantsLoading = false;
          grantsError = e instanceof Error ? e.message : String(e);
          render();
        });
    }
    return;
  }

  if (role === "funder") {
    appEl.appendChild(renderCreateGrantForm());
    restoreCreateFormState(savedCreateFormState);
  }

  appEl.appendChild(renderGrantSwitcher());

  const selectedGrant = grants.find((g) => g.grant_id === selectedGrantId) ?? null;
  if (!selectedGrant) {
    if (grants.length === 0 && role === "grantee") {
      appEl.appendChild(renderNoGrantYet());
    }
    return;
  }

  if (role === "funder") {
    appEl.appendChild(renderTransactionsTable(selectedGrant));
  }

  if (walletRole === undefined) {
    if (walletRoleError) {
      appEl.appendChild(renderFetchError(walletRoleError, () => {
        walletRoleError = null;
        render();
      }));
      return;
    }
    const p = document.createElement("p");
    p.textContent = "checking wallet against this grant...";
    appEl.appendChild(p);
    if (!walletRoleLoading) {
      walletRoleLoading = true;
      getGrantRole(address, selectedGrant)
        .then((r) => {
          walletRoleLoading = false;
          walletRole = r;
          render();
        })
        .catch((e) => {
          walletRoleLoading = false;
          walletRoleError = e instanceof Error ? e.message : String(e);
          render();
        });
    }
    return;
  }

  if (walletRole !== role) {
    const warning = document.createElement("p");
    warning.style.color = "#b91c1c";
    warning.textContent =
      walletRole === "none"
        ? `this wallet isn't the ${role} for this grant.`
        : `this wallet is the ${walletRole} for this grant, not the ${role} -- switch role above, or connect a different wallet.`;
    appEl.appendChild(warning);
  }

  appEl.appendChild(renderMilestones(selectedGrant, walletRole === role));
}

function renderRoleSelect(): HTMLElement {
  const div = document.createElement("div");
  div.innerHTML = "<p>who are you?</p>";
  const funderBtn = document.createElement("button");
  funderBtn.textContent = "I'm a Funder";
  funderBtn.addEventListener("click", () => switchRole("funder"));
  const granteeBtn = document.createElement("button");
  granteeBtn.textContent = "I'm a Grantee";
  granteeBtn.addEventListener("click", () => switchRole("grantee"));
  div.appendChild(funderBtn);
  div.appendChild(granteeBtn);
  return div;
}

function renderStatus(): HTMLElement {
  const p = document.createElement("p");
  p.textContent = status.message;
  p.style.color = status.kind === "error" ? "#b91c1c" : status.kind === "success" ? "#15803d" : "#555";
  return p;
}

function renderWalletBar(): HTMLElement {
  const container = document.createElement("section");
  if (address) {
    const p = document.createElement("p");
    p.innerHTML = `connected: <code>${address}</code> `;
    const disconnectBtn = document.createElement("button");
    disconnectBtn.textContent = "disconnect";
    disconnectBtn.addEventListener("click", () => {
      disconnect();
    });
    p.appendChild(disconnectBtn);
    container.appendChild(p);
    return container;
  }
  const installed = BrowserWallet.getInstalledWallets().filter((w) => !w.name.toLowerCase().includes("brave"));
  if (installed.length === 0) {
    container.textContent = "no CIP-30 wallet extension detected";
    return container;
  }
  for (const w of installed) {
    const btn = document.createElement("button");
    btn.textContent = w.name.toLowerCase() === "lace" ? "Connect Wallet" : `connect ${w.name}`;
    btn.addEventListener("click", () => connect(w.id));
    container.appendChild(btn);
  }
  return container;
}

function renderPendingTx(): HTMLElement {
  const explorerUrl = `${EXPLORER_TX_BASE_URL}/${pendingTx!.hash}`;
  const div = document.createElement("div");
  div.innerHTML = `
    <table style="border-collapse:collapse;margin:0.5rem 0;">
      <tbody>
        <tr><th style="text-align:left;padding-right:1rem;">action</th><td>${pendingTx!.action}</td></tr>
        <tr><th style="text-align:left;padding-right:1rem;">tx hash</th><td><a href="${explorerUrl}" target="_blank" rel="noopener noreferrer"><code>${pendingTx!.hash}</code></a></td></tr>
        <tr><th style="text-align:left;padding-right:1rem;">status</th><td><span class="pending-dot"></span> processing on-chain -- checking every ${PENDING_POLL_INTERVAL_MS / 1000}s</td></tr>
      </tbody>
    </table>
    <p style="font-size:0.875rem;color:#555;">Blockfrost indexing can lag behind submission.</p>
    <button id="refresh-pending">refresh now</button>
  `;
  div.querySelector("#refresh-pending")!.addEventListener("click", () => refreshPending());
  return div;
}

function renderFetchError(message: string, onRetry: () => void): HTMLElement {
  const div = document.createElement("div");
  const p = document.createElement("p");
  p.style.color = "#b91c1c";
  p.textContent = message;
  div.appendChild(p);
  const retryBtn = document.createElement("button");
  retryBtn.textContent = "retry";
  retryBtn.addEventListener("click", onRetry);
  div.appendChild(retryBtn);
  return div;
}

function renderNoGrantYet(): HTMLElement {
  const p = document.createElement("p");
  p.textContent = "no grant exists yet -- ask your funder to create one.";
  return p;
}

function renderGrantSwitcher(): HTMLElement {
  const container = document.createElement("div");
  if (!grants || grants.length === 0) return container;

  const label = document.createElement("label");
  label.textContent = "grant: ";
  const select = document.createElement("select");
  for (const g of grants) {
    const opt = document.createElement("option");
    opt.value = g.grant_id;
    opt.textContent = g.name;
    opt.selected = g.grant_id === selectedGrantId;
    select.appendChild(opt);
  }
  select.addEventListener("change", () => selectGrant(select.value));
  label.appendChild(select);
  container.appendChild(label);
  return container;
}

function renderTransactionsTable(grant: GrantSummary): HTMLElement {
  const container = document.createElement("div");
  container.style.margin = "1rem 0";

  if (transactions === undefined) {
    container.textContent = "loading recent transactions...";
    if (!transactionsLoading) {
      transactionsLoading = true;
      getTransactions(grant)
        .then((t) => {
          transactionsLoading = false;
          transactions = t;
          render();
        })
        .catch(() => {
          transactionsLoading = false;
          transactions = [];
          render();
        });
    }
    return container;
  }

  if (transactions.length === 0) {
    container.innerHTML = "<h3>transactions for this grant</h3><p>none yet.</p>";
    return container;
  }

  const rows = transactions
    .map((t) => {
      const url = `${EXPLORER_TX_BASE_URL}/${t.tx_hash}`;
      const when = new Date(t.block_time * 1000).toLocaleString();
      return `
        <tr>
          <td><a href="${url}" target="_blank" rel="noopener noreferrer"><code>${t.tx_hash.slice(0, 12)}…</code></a></td>
          <td>${when}</td>
          <td>${t.block_height}</td>
        </tr>
      `;
    })
    .join("");

  container.innerHTML = `
    <h3>transactions for this grant</h3>
    <table style="border-collapse:collapse;width:100%;font-size:0.875rem;">
      <thead>
        <tr>
          <th style="text-align:left;padding-right:1rem;">tx hash</th>
          <th style="text-align:left;padding-right:1rem;">time</th>
          <th style="text-align:left;">block</th>
        </tr>
      </thead>
      <tbody>${rows}</tbody>
    </table>
  `;
  return container;
}

function renderProgressBar(trancheBps: number[], releasedCount: number): string {
  const releasedBps = trancheBps.slice(0, releasedCount).reduce((a, b) => a + b, 0);
  const percent = releasedBps / 100;
  return `
    <div style="width:100%;height:1.5rem;background:#e5e5e5;border-radius:0.5rem;overflow:hidden;">
      <div style="width:${percent}%;height:100%;background:#2563eb;transition:width 0.3s ease;"></div>
    </div>
    <p style="margin-top:0.25rem;font-size:0.875rem;color:#555;">${percent.toFixed(0)}% released (${releasedCount}/${trancheBps.length} milestones)</p>
  `;
}

function renderMilestones(grant: GrantSummary, walletMatchesRole: boolean): HTMLElement {
  const { datum, milestones } = grant;
  const container = document.createElement("div");

  const items = datum.tranche_bps
    .map((bps, i) => {
      const approved = datum.approved[i];
      const released = i < datum.released_count;
      const isNextRelease = i === datum.released_count && approved;
      const label = released ? " (released)" : approved ? " (approved)" : " (pending)";
      const meta = milestones?.[i];
      const title = meta?.name ? escapeHtml(meta.name) : `milestone ${i + 1}`;
      const description = meta?.description
        ? `<br /><span style="font-size:0.875rem;color:#555;">${escapeHtml(meta.description)}</span>`
        : "";
      const approveBtn =
        walletMatchesRole && role === "funder" && !approved ? `<button data-approve="${i}">approve</button>` : "";
      const releaseBtn =
        walletMatchesRole && role === "grantee" && isNextRelease ? `<button data-release="${i}">release</button>` : "";
      return `<li><strong>${title}</strong> -- ${(bps / 100).toFixed(0)}%${label} ${approveBtn} ${releaseBtn}${description}</li>`;
    })
    .join("");

  container.innerHTML = `
    <h3>${escapeHtml(grant.name)}</h3>
    ${renderProgressBar(datum.tranche_bps, datum.released_count)}
    <p>locked: ${(grant.lovelace / 1_000_000).toFixed(2)} ADA (total_locked: ${(datum.total_locked / 1_000_000).toFixed(2)} ADA)</p>
    <ul>${items}</ul>
  `;

  container.querySelectorAll<HTMLButtonElement>("[data-approve]").forEach((btn) => {
    btn.addEventListener("click", () => approveMilestone(Number(btn.dataset.approve)));
  });
  container.querySelectorAll<HTMLButtonElement>("[data-release]").forEach((btn) => {
    btn.addEventListener("click", () => releaseTranche(Number(btn.dataset.release)));
  });

  return container;
}

function renderCreateGrantForm(): HTMLElement {
  const container = document.createElement("div");
  const milestoneFields = [0, 1, 2]
    .map(
      (i) => `
        <div>
          <label>milestone ${i + 1} name <input name="m${i}Name" placeholder="milestone ${i + 1} name" required /></label>
          <label>description <input name="m${i}Desc" placeholder="what proves this milestone is done" required /></label>
        </div>
      `,
    )
    .join("");

  container.innerHTML = `
    <details>
      <summary>+ create new grant</summary>
      <form id="create-form">
        <div><label>grant name <input name="grantName" placeholder="e.g. Q3 Docs Overhaul" required /></label></div>
        <div><label>grantee address <input name="granteeAddress" placeholder="addr_test1..." required /></label></div>
        <div><label>tranche basis points (must sum to 10000) <input name="trancheBps" value="4000,3000,3000" required /></label></div>
        <div><label>total locked (ADA) <input name="totalLocked" type="number" value="100" required /></label></div>
        <fieldset>
          <legend>milestones</legend>
          ${milestoneFields}
        </fieldset>
        <button type="submit">create grant</button>
      </form>
    </details>
  `;
  const form = container.querySelector("#create-form") as HTMLFormElement;
  form.addEventListener("submit", (e) => {
    e.preventDefault();
    const data = new FormData(form);
    const granteeAddress = String(data.get("granteeAddress"));
    const trancheBps = String(data.get("trancheBps"))
      .split(",")
      .map((s) => Number(s.trim()));
    const totalLocked = Math.round(Number(data.get("totalLocked")) * 1_000_000);
    const name = String(data.get("grantName"));
    const milestones: MilestoneMeta[] = [0, 1, 2].map((i) => ({
      name: String(data.get(`m${i}Name`)),
      description: String(data.get(`m${i}Desc`)),
    }));
    createGrant(granteeAddress, trancheBps, totalLocked, name, milestones);
  });
  return container;
}

render();
