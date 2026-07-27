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
  Left in place for reference, not read by `offchain/` anymore.
- `testnet-v3/` — superseded: predates the on-chain `tranche_bps`
  sum/sign invariant check added to `milestone_escrow`'s spend logic.
  Address was never funded (checked before cutover), so nothing was
  orphaned by moving on. Not read by `offchain/` anymore (default
  `DEPLOY_DIR` now points at `testnet-v4/`).
- `testnet-v4/` (original) — superseded in place: redeployed with the same
  deployment tag (`"v4"`) once `ClaimExpired` was added, since nobody
  still holds the keys needed to act on the one grant that was live there
  (see `ESCROW-UPGRADE.md`). New script code -> new hash -> new address
  regardless of the tag being reused, so `params.json`/`plutus.applied.json`
  under `testnet-v4/` now describe the *new* deploy; the original address
  and its stuck grant are orphaned deliberately, not tracked in a live
  file anymore:
  - address: `addr_test1wpt3j5g8gzldt4sks9fyfgcxkw0mlm6yk74nxda7gu4p2ncjlvhqh`
  - script hash: `5719510740bed5d616815244a306b39fbfef44b7ab3337be472a154f`
  - stuck UTxO: `43f4ff898ca054923bf3816c46c07b6cd8b9e2711cc025e88922503c2ffe93af#1`,
    90 ADA, milestone 3 of 3, unapprovable (old script has no
    `ClaimExpired` arm to rescue it either way).
