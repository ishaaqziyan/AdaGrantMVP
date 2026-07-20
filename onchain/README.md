# onchain

On-chain validators for the milestone-based grant escrow dApp, written in
[Aiken](https://aiken-lang.org) (Plutus v3). See `../instructions.md` at the
repo root for the full system spec (off-chain and frontend layers included).

## Project layout

```
onchain/
├── aiken.toml                          # package manifest, deps, plutus version
├── validators/
│   ├── milestone_escrow.ak             # combined spend validator: milestone
│   │                                    # registry + treasury escrow
│   └── milestone_receipt.ak            # minting policy for the soulbound
│                                        # Milestone Receipt NFT
│   └── tests/
│       └── milestone_escrow_test.ak    # test suite (boolean-toggle style)
├── lib/
│   └── milestone_escrow/
│       ├── types.ak                    # shared Datum / Redeemer types
│       └── utils.ak                    # asset-name / tranche-math helpers
├── env/                                # optional per-network config modules
└── plutus.json                         # generated blueprint (after `aiken build`)
```

Aiken forbids importing a validator module from a module under `lib/` (this
fails `aiken check`/`aiken build` — and in this environment, silently: no
error text at all, just exit 1, because the progress-bar output swallows it
under a non-interactive TTY; run through `script -qec "aiken check" /dev/null`
to see the real error if this happens again). Test modules that call a
validator's handler directly must live under `validators/` instead — hence
`validators/tests/`.

## The contracts

### `milestone_escrow` (spend validator)

Parameterized by `receipt_policy_id: PolicyId` — the policy ID of the
companion `milestone_receipt` minting script — so it can verify the exact
receipt NFT minted alongside an approval.

**Datum** (`milestone_escrow/types.{Datum}`):

| field           | type         | meaning                                                          |
|-----------------|--------------|-------------------------------------------------------------------|
| `reviewer`      | `VerificationKeyHash` | key that must sign every `ApproveMilestone`             |
| `proposer`      | `VerificationKeyHash` | key that receives every tranche payout                  |
| `total_locked`  | `Int`        | ADA (lovelace) originally deposited; fixed base for every tranche % |
| `tranche_bps`   | `List<Int>`  | payout share per milestone, in basis points (must sum to 10000)  |
| `approved`      | `List<Bool>` | approval flag per milestone, same length as `tranche_bps`         |
| `released_count`| `Int`        | how many tranches have been paid out so far                       |

**Redeemer** (`milestone_escrow/types.{Redeemer}`):

- `ApproveMilestone(index)` — requires `reviewer`'s signature; marks
  milestone `index` approved; mints exactly one Milestone Receipt NFT for
  that index; funds and every other datum field stay untouched.
- `ReleaseTranche(index)` — requires `index == released_count` (strictly
  sequential) and milestone `index` already approved; pays
  `total_locked * tranche_bps[index] / 10000` lovelace to `proposer`;
  increments `released_count`. The final tranche drains the script UTxO
  entirely (no continuing output required); every other tranche leaves a
  continuing output with the balance reduced by the payout.

A transaction may only spend **one** UTxO from this script — enforced
explicitly to block double-satisfaction attacks.

### `milestone_receipt` (minting policy)

Parameterized by `escrow_script_hash: ScriptHash` — the payment credential
hash of `milestone_escrow`. Minting requires an input from that script
address to be present in the same transaction (i.e. minting can only ever
piggyback on a `milestone_escrow` spend). The heavy lifting — pinning the
exact asset name to the approved index, and forbidding minting during
`ReleaseTranche` — is done on the spend side; this policy is a second,
independent check on top of that.

Asset name: `"Milestone" <> index_as_1_byte` (see
`milestone_escrow/utils.{milestone_asset_name}`), unique per milestone, so
each approval can only ever mint its own receipt once.

## Building

```sh
aiken build
```

Generates/updates `plutus.json` — the CIP-57 blueprint with compiled
scripts, used by the off-chain (Rust/Blockfrost) layer to build
transactions and derive script addresses/policy IDs.

## Type-checking & running tests

```sh
aiken check
```

Run a subset by name:

```sh
aiken check -m milestone_escrow      # everything in that module
aiken check -m approve_milestone     # only tests matching this substring
```

Tests live in `validators/tests/milestone_escrow_test.ak` and follow
the **boolean-toggle methodology**: one success test with every condition
valid, then one failing test per guard, each flipping exactly one
condition. Transactions are built with `mocktail` (from the `sidan-lab/vodka`
dependency) — a fluent builder over `cardano/transaction.Transaction` where
every step takes an `include: Bool` first argument, so a single
`get_..._tx(test_case)` helper can produce both the valid and every invalid
variant of a transaction shape.

Calling a parameterized validator's handler directly in a test:

```aiken
use milestone_escrow

milestone_escrow.milestone_escrow.spend(
  receipt_policy_id,   // validator parameter, comes first
  Some(datum),
  ApproveMilestone(0),
  own_ref,
  tx,
)
```

(`<module>.<validator_name>.<handler>(<params...>, <handler_args...>)` —
the module path and the validator name coincide here because
`validators/milestone_escrow.ak` declares `validator milestone_escrow(...)`.)

## Configuring

**`aiken.toml`**
```toml
[config.default]
network_id = 41
```

Network-specific overrides can go in per-environment modules under `env/`
(e.g. `env/preview.ak`, `env/mainnet.ak`) — see the Aiken manual's
[Modules](https://aiken-lang.org/language-tour/modules) page for the
`aiken env` pattern.

## Generating docs

```sh
aiken docs
```

Emits HTML API docs for everything under `lib/`.

## Resources

- [Aiken language tour](https://aiken-lang.org/language-tour)
- [Aiken standard library](https://aiken-lang.github.io/stdlib)
- [sidan-lab/vodka](https://github.com/sidan-lab/vodka) — `cocktail`
  (in-validator helpers) and `mocktail` (test transaction builder)
- `../instructions.md` — full system architecture (this repo)
