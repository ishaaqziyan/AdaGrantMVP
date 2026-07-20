# offchain

Thin Rust/Blockfrost API for the milestone-based grant escrow dApp. Builds
unsigned transaction CBOR; never signs or submits — the frontend gets a
CIP-30 wallet to sign, and submits directly.

## Setup

```sh
cp .env.example .env   # fill in BLOCKFROST_PROJECT_ID
cargo run
```

Requires `../onchain/deploy/<target>/{params.json,plutus.applied.json}` to
exist (see `../onchain/deploy/README.md` for how they're generated) —
`Config::load()` reads them at startup and fails fast if they're missing.
`DEPLOY_DIR` defaults to `../onchain/deploy/testnet-v2`; override it in
`.env` to point at a different deploy target.

## Endpoints

- `GET /grants` — every live grant at the escrow address (multi-grant: many
  can live at the same address simultaneously), with decoded datum and any
  saved name/milestone metadata.
- `POST /grants` — saves a grant's off-chain name/milestone metadata ahead
  of `create-escrow`, keyed by a `grant_id` the caller can re-derive once
  the escrow exists on-chain.
- `GET /grants/role?address&tx_hash&output_index` — which role (`funder`,
  `grantee`, or `none`) a wallet address holds for one specific grant.
- `GET /transactions?tx_hash&output_index` — transaction history scoped to
  one grant's own lineage, not every grant at the shared address.
- `POST /tx/create-escrow` — the one-time "lock funds" transaction.
- `POST /tx/approve-milestone` — `ApproveMilestone(index)`.
- `POST /tx/release-tranche` — `ReleaseTranche(index)`.

All three `POST /tx/*` endpoints return `{"unsigned_tx_cbor": "<hex>"}`. The
caller (frontend) is expected to already know its own selected UTxOs (fee
input, collateral) from the wallet's own `getUtxos()` — this server doesn't
do coin selection.

## Source layout

- `main.rs` — entry point.
- `config.rs` — deploy-time config.
- `handlers.rs` — Axum router and HTTP handlers.
- `blockfrost_client.rs` — Blockfrost reads.
- `datum.rs` — `Datum`/`Redeemer` PlutusData encoding.
- `grants_meta.rs` — off-chain grant/milestone metadata store.
- `address.rs` — bech32 address helpers.
- `fees.rs` — fee math.
- `error.rs` — API error type.
- `tx/create_escrow.rs`, `tx/approve.rs`, `tx/release.rs` — the three
  tx builders.

## Known issues

- `DEPLOY_DIR` still defaults to `testnet-v2`, which `../onchain/deploy/README.md`
  documents as abandoned (in favor of `testnet-v3`). Not yet switched over —
  any grant already living on `testnet-v2` would drop out of the app's view
  if the default changes.
- Error handling collapses most failures to a generic message; detail is
  logged server-side via `tracing`, not returned to the caller.
