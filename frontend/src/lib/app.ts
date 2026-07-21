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

/** Grant/milestone text is funder-supplied and rendered into innerHTML -- escape to block markup injection on a wallet-connected page. */
function escapeHtml(s: string): string {
  return s
    .replace(/&/g, "&amp;")
    .replace(/</g, "&lt;")
    .replace(/>/g, "&gt;")
    .replace(/"/g, "&quot;")
    .replace(/'/g, "&#39;");
}

// "reviewer" (must sign ApproveMilestone) = Funder; "proposer" (pure payout dest, unchecked) = Grantee.
type Role = "funder" | "grantee";

type Status = { kind: "idle" | "busy" | "error" | "success"; message: string };

/** `txHashBefore` is the grant's live UTxO tx_hash at submit time; comparing against it on refresh detects confirmation without guessing at Blockfrost's indexing lag. */
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

// Guards against refiring the same fetch on every re-render while its state is still undefined -- /transactions can cost ~100 Blockfrost calls per hit.
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

// CIP-30 has no push-based "account changed" event -- must poll to detect a wallet-side account switch.
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

/** Checks whether a pending tx has landed by comparing the grant's tx_hash to what it was before submission. */
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

/** render() rebuilds #app from scratch on every call, so an open create-grant form would get wiped mid-edit -- snapshot it before the wipe, reapply after. */
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

function prefersReducedMotion(): boolean {
  return window.matchMedia("(prefers-reduced-motion: reduce)").matches;
}

/** View Transitions API cross-fades old vs. new DOM for free on render(), instead of snapping instantly between views. */
function render() {
  if (!prefersReducedMotion() && "startViewTransition" in document) {
    (document as Document & { startViewTransition: (cb: () => void) => void }).startViewTransition(renderInner);
  } else {
    renderInner();
  }
}

function renderInner() {
  const savedCreateFormState = captureCreateFormState();
  appEl.innerHTML = "";

  if (!role) {
    appEl.appendChild(renderRoleSelect());
    return;
  }

  const mainCol = document.createElement("div");
  mainCol.className = "main-column";
  const sideCol = document.createElement("div");
  sideCol.className = "side-column";
  appEl.appendChild(mainCol);
  appEl.appendChild(sideCol);

  mainCol.appendChild(renderWalletBar());

  const roleRow = document.createElement("div");
  roleRow.className = "row";
  const roleLabel = document.createElement("p");
  roleLabel.innerHTML = `ROLE: <strong>${role === "funder" ? "Funder" : "Grantee"}</strong>`;
  roleRow.appendChild(roleLabel);
  const back = document.createElement("button");
  back.className = "btn-sm";
  back.textContent = "← switch role";
  back.addEventListener("click", () => switchRole(null));
  roleRow.appendChild(back);
  mainCol.appendChild(roleRow);

  mainCol.appendChild(renderStatus());

  if (!address) return;

  if (pendingTx) {
    mainCol.appendChild(renderPendingTx());
    return;
  }

  if (grants === undefined) {
    if (grantsError) {
      mainCol.appendChild(renderFetchError(grantsError, () => {
        grantsError = null;
        render();
      }));
      return;
    }
    mainCol.appendChild(renderSkeleton(3));
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
    mainCol.appendChild(renderCreateGrantForm());
    restoreCreateFormState(savedCreateFormState);
  }

  mainCol.appendChild(renderGrantsTable(role, grants));

  const selectedGrant = grants.find((g) => g.grant_id === selectedGrantId) ?? null;
  if (!selectedGrant) {
    if (grants.length === 0 && role === "grantee") {
      mainCol.appendChild(renderNoGrantYet());
    }
    return;
  }

  if (role === "funder") {
    sideCol.appendChild(renderTransactionsTable(selectedGrant));
  }

  if (walletRole === undefined) {
    if (walletRoleError) {
      mainCol.appendChild(renderFetchError(walletRoleError, () => {
        walletRoleError = null;
        render();
      }));
      return;
    }
    mainCol.appendChild(renderSkeleton(2));
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
    warning.style.color = "var(--color-error)";
    warning.textContent =
      walletRole === "none"
        ? `this wallet isn't the ${role} for this grant.`
        : `this wallet is the ${walletRole} for this grant, not the ${role} -- switch role above, or connect a different wallet.`;
    mainCol.appendChild(warning);
  }

  mainCol.appendChild(renderMilestones(selectedGrant, walletRole === role));
}

function renderRoleSelect(): HTMLElement {
  const div = document.createElement("div");
  div.className = "panel role-select";
  div.innerHTML = "<p>who are you?</p>";
  const funderBtn = document.createElement("button");
  funderBtn.className = "btn-primary";
  funderBtn.textContent = "I'm a Funder";
  funderBtn.addEventListener("click", () => switchRole("funder"));
  const granteeBtn = document.createElement("button");
  granteeBtn.className = "btn-primary";
  granteeBtn.textContent = "I'm a Grantee";
  granteeBtn.addEventListener("click", () => switchRole("grantee"));
  div.appendChild(funderBtn);
  div.appendChild(granteeBtn);
  return div;
}

function renderStatus(): HTMLElement {
  const bar = document.createElement("div");
  bar.className = `status-bar status-${status.kind}`;
  if (!status.message) {
    bar.style.display = "none";
    return bar;
  }
  if (status.kind === "busy") {
    const spinner = document.createElement("span");
    spinner.className = "status-icon status-spinner";
    spinner.setAttribute("aria-hidden", "true");
    bar.appendChild(spinner);
  } else if (status.kind === "success" || status.kind === "error") {
    const icon = document.createElement("span");
    icon.className = "status-icon";
    icon.setAttribute("aria-hidden", "true");
    icon.textContent = status.kind === "success" ? "✓" : "✕";
    bar.appendChild(icon);
  }
  const text = document.createElement("span");
  text.textContent = status.message;
  bar.appendChild(text);
  return bar;
}

function renderWalletBar(): HTMLElement {
  const container = document.createElement("section");
  container.className = "panel";
  if (address) {
    const p = document.createElement("p");
    p.className = "row";
    p.innerHTML = `connected: <code title="${address}">${truncatedAddress(address)}</code>`;
    const disconnectBtn = document.createElement("button");
    disconnectBtn.className = "push-right";
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
    btn.className = "wallet-btn btn-primary";
    if (w.icon) {
      const icon = document.createElement("img");
      icon.src = w.icon;
      icon.alt = "";
      icon.width = 20;
      icon.height = 20;
      btn.appendChild(icon);
    }
    const label = document.createElement("span");
    label.textContent = w.name.toLowerCase() === "lace" ? "Connect Wallet" : `connect ${w.name}`;
    btn.appendChild(label);
    btn.addEventListener("click", () => connect(w.id));
    container.appendChild(btn);
  }
  return container;
}

function renderPendingTx(): HTMLElement {
  const explorerUrl = `${EXPLORER_TX_BASE_URL}/${pendingTx!.hash}`;
  const div = document.createElement("div");
  div.className = "panel";
  div.innerHTML = `
    <table class="kv-table">
      <tbody>
        <tr><th>action</th><td>${pendingTx!.action}</td></tr>
        <tr><th>tx hash</th><td><a href="${explorerUrl}" target="_blank" rel="noopener noreferrer"><code>${pendingTx!.hash}</code></a></td></tr>
        <tr><th>status</th><td><span class="pending-dot"></span> processing on-chain -- checking every ${PENDING_POLL_INTERVAL_MS / 1000}s</td></tr>
      </tbody>
    </table>
    <p style="font-size:0.875rem;color:var(--text-muted);">Blockfrost indexing can lag behind submission.</p>
    <button id="refresh-pending">refresh now</button>
  `;
  div.querySelector("#refresh-pending")!.addEventListener("click", () => refreshPending());
  return div;
}

function renderSkeleton(lines: number): HTMLElement {
  const div = document.createElement("div");
  div.className = "stack";
  for (let i = 0; i < lines; i++) {
    const bar = document.createElement("div");
    bar.className = "skeleton skeleton-line";
    bar.style.width = i === lines - 1 ? "60%" : "100%";
    div.appendChild(bar);
  }
  return div;
}

function renderFetchError(message: string, onRetry: () => void): HTMLElement {
  const div = document.createElement("div");
  div.className = "stack";
  const p = document.createElement("p");
  p.style.color = "var(--color-error)";
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

/** "pending approval" -> next milestone unapproved; "awaiting release" -> approved, release available; "completed" -> all tranches released. */
function grantStatus(g: GrantSummary): { label: string; className: string } {
  const total = g.datum.tranche_bps.length;
  if (g.completed || g.datum.released_count >= total) {
    return { label: "completed", className: "completed" };
  }
  const nextIndex = g.datum.released_count;
  if (g.datum.approved[nextIndex]) {
    return { label: "awaiting release", className: "awaiting-release" };
  }
  return { label: "pending approval", className: "pending" };
}

function truncatedHash(hex: string): string {
  return `${hex.slice(0, 8)}…`;
}

/** Bech32 addresses run ~100+ chars -- too long for the wallet bar's fixed-width layout. Keep enough of both ends to stay recognizable/comparable, full value stays in `title` for hover/copy. */
function truncatedAddress(addr: string): string {
  return addr.length <= 24 ? addr : `${addr.slice(0, 14)}…${addr.slice(-8)}`;
}

function renderGrantsTable(role: Role, grants: GrantSummary[]): HTMLElement {
  const wrap = document.createElement("div");
  wrap.className = "data-table-wrap panel";
  if (grants.length === 0) return document.createElement("div");

  const counterpartyHeader = role === "funder" ? "grantee" : "funder";

  const table = document.createElement("table");
  table.className = "data-table grants-table";
  table.innerHTML = `
    <thead>
      <tr>
        <th>name</th>
        <th>${counterpartyHeader}</th>
        <th>locked</th>
        <th>progress</th>
        <th>status</th>
        ${role === "funder" ? "<th>trust</th>" : ""}
      </tr>
    </thead>
    <tbody></tbody>
  `;

  const tbody = table.querySelector("tbody")!;
  for (const g of grants) {
    const counterparty = role === "funder" ? g.datum.proposer : g.datum.reviewer;
    const status = grantStatus(g);
    const total = g.datum.tranche_bps.length;

    const tr = document.createElement("tr");
    if (g.grant_id === selectedGrantId) tr.classList.add("selected");
    tr.innerHTML = `
      <td>${escapeHtml(g.name)}</td>
      <td><code>${truncatedHash(counterparty)}</code></td>
      <td>${(g.lovelace / 1_000_000).toFixed(2)} ADA</td>
      <td>${g.datum.released_count}/${total} milestones</td>
      <td><span class="status-pill ${status.className}">${status.label}</span></td>
      ${
        role === "funder"
          ? `<td>${g.trusted ? "" : `<span class="trust-warning" title="${escapeHtml(g.warnings.join("; "))}">⚠ unverified</span>`}</td>`
          : ""
      }
    `;
    tr.addEventListener("click", () => selectGrant(g.grant_id));
    tbody.appendChild(tr);
  }

  wrap.appendChild(table);
  return wrap;
}

function renderTransactionsTable(grant: GrantSummary): HTMLElement {
  const container = document.createElement("div");
  container.className = "panel";

  if (transactions === undefined) {
    container.appendChild(renderSkeleton(3));
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
    <div class="data-table-wrap">
      <table class="data-table">
        <thead>
          <tr>
            <th>tx hash</th>
            <th>time</th>
            <th>block</th>
          </tr>
        </thead>
        <tbody>${rows}</tbody>
      </table>
    </div>
  `;
  return container;
}

function renderProgressBar(trancheBps: number[], releasedCount: number): string {
  const releasedBps = trancheBps.slice(0, releasedCount).reduce((a, b) => a + b, 0);
  const percent = releasedBps / 100;
  return `
    <div style="width:100%;height:1.5rem;background:var(--track-bg);border-radius:0.5rem;overflow:hidden;">
      <div style="width:${percent}%;height:100%;background:var(--color-accent);transition:width 0.3s ease;"></div>
    </div>
    <p style="font-size:0.875rem;color:var(--text-muted);">${percent.toFixed(0)}% released (${releasedCount}/${trancheBps.length} milestones)</p>
  `;
}

function renderMilestones(grant: GrantSummary, walletMatchesRole: boolean): HTMLElement {
  const { datum, milestones } = grant;
  const container = document.createElement("div");
  container.className = "panel";

  const rows = datum.tranche_bps
    .map((bps, i) => {
      const approved = datum.approved[i];
      const released = i < datum.released_count;
      const isNextRelease = i === datum.released_count && approved;
      const state = released ? "released" : approved ? "approved" : "pending";
      const meta = milestones?.[i];
      const title = meta?.name ? escapeHtml(meta.name) : `milestone ${i + 1}`;
      const description = meta?.description ? `<div class="milestone-desc">${escapeHtml(meta.description)}</div>` : "";
      const approveBtn =
        walletMatchesRole && role === "funder" && !approved
          ? `<button class="btn-primary btn-sm" data-approve="${i}">approve</button>`
          : "";
      const releaseBtn =
        walletMatchesRole && role === "grantee" && isNextRelease
          ? `<button class="btn-primary btn-sm" data-release="${i}">release</button>`
          : "";
      return `
        <tr>
          <td>${i + 1}</td>
          <td><strong>${title}</strong>${description}</td>
          <td>${(bps / 100).toFixed(0)}%</td>
          <td><span class="status-pill ${state === "released" ? "completed" : state === "approved" ? "awaiting-release" : ""}">${state}</span></td>
          <td>${approveBtn}${releaseBtn}</td>
        </tr>
      `;
    })
    .join("");

  container.innerHTML = `
    <h3>${escapeHtml(grant.name)}</h3>
    ${renderProgressBar(datum.tranche_bps, datum.released_count)}
    <p>locked: ${(grant.lovelace / 1_000_000).toFixed(2)} ADA (total_locked: ${(datum.total_locked / 1_000_000).toFixed(2)} ADA)</p>
    <div class="data-table-wrap">
      <table class="data-table">
        <thead>
          <tr>
            <th>#</th>
            <th>milestone</th>
            <th>share</th>
            <th>status</th>
            <th>action</th>
          </tr>
        </thead>
        <tbody>${rows}</tbody>
      </table>
    </div>
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
  container.className = "panel";
  const milestoneFields = [0, 1, 2]
    .map(
      (i) => `
        <div class="milestone-field">
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
        <button type="submit" class="btn-primary">create grant</button>
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
