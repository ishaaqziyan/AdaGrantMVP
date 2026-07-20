# Deploy artifacts

`milestone_escrow` is parameterized by `_deployment_tag: ByteArray` — a
value the spend logic never inspects, that exists purely so each fresh
deploy gets a distinct script hash/address from any prior one. (Without
it, the whole system supports exactly one escrow ever, globally, since the
address is otherwise fixed by the compiled script alone — see the
validator's doc comment in `validators/milestone_escrow.ak`.) Both
`milestone_escrow` and `milestone_receipt` (parameterized by
`escrow_script_hash`, which now varies per deploy too) need to be *applied*
before they have real addresses/policy IDs — a one-time, deploy-time step,
not something the off-chain service should redo per-request.

Regenerate everything for a given target (`testnet-v2/`, `preprod/`,
`mainnet/`, ...) with:

```sh
cd onchain

# 1. Apply a deployment tag to milestone_escrow (any distinct bytes --
#    e.g. ascii "v2" as a CBOR bytestring: 0x42 <2 ascii bytes>).
aiken blueprint apply -v milestone_escrow -o deploy/<target>/plutus.applied.json "427632"

ESCROW_HASH=$(aiken blueprint hash -v milestone_escrow -i deploy/<target>/plutus.applied.json)

# 2. Apply escrow_script_hash to milestone_receipt, onto the SAME file
#    (chained -- both validators end up applied in one blueprint).
#    Parameter is CBOR-encoded Plutus Data: 0x58 0x1c <28-byte hash>.
aiken blueprint apply -v milestone_receipt -i deploy/<target>/plutus.applied.json -o deploy/<target>/plutus.applied.json "581c${ESCROW_HASH}"

POLICY=$(aiken blueprint policy -v milestone_receipt -i deploy/<target>/plutus.applied.json)
ADDR=$(aiken blueprint address -v milestone_escrow -i deploy/<target>/plutus.applied.json)   # add --mainnet for mainnet

cat > deploy/<target>/params.json <<EOF
{
  "network": "<target>",
  "deployment_tag": "v2",
  "escrow_script_hash": "$ESCROW_HASH",
  "escrow_address": "$ADDR",
  "receipt_policy_id": "$POLICY"
}
EOF
```

`offchain/` reads both `compiledCode`s from the single
`deploy/<target>/plutus.applied.json` (both validators are parameterized,
so neither has a directly-usable `compiledCode` in the project's plain
`plutus.json` anymore) plus `deploy/<target>/params.json` for the
hashes/address, so it doesn't need to recompute them. See
`offchain/.env.example` (`DEPLOY_DIR`).

`receipt_policy_id` from `params.json` also has to be embedded in the
escrow's `Datum.receipt_policy_id` field when the escrow UTxO is first
created/funded — that initial "lock" transaction isn't built by this API
(out of scope: it's a one-time setup step for the proposer, not part of the
approve/release flow), but whatever builds it needs this same value.

## Prior deployments

- `testnet/` — v1 (unparameterized `milestone_escrow`). Abandoned: has one
  stuck test escrow whose reviewer key isn't controlled by anyone who can
  approve/release it, and the whole target only ever supports one escrow.
- `testnet-v2/` — abandoned: escrow was created with `reviewer`/`proposer`
  set to the wrong wallets (funder wallet ended up matching `proposer`, not
  `reviewer`), so the intended funder wallet couldn't approve milestones.
  Left in place for reference, not read by `offchain/` anymore (default
  `DEPLOY_DIR` now points at `testnet-v3/`).
