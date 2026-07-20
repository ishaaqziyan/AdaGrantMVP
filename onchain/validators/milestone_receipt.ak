use aiken/collection/dict
use aiken/collection/list
use aiken/crypto.{ScriptHash}
use cardano/address.{Script}
use cardano/assets.{PolicyId}
use cardano/transaction.{Transaction}

/// Soulbound Milestone Receipt NFT: mints exactly one token per approved
/// milestone. Parameterized by the escrow validator's script hash so
/// minting can only happen alongside a spend of the escrow UTxO -- the
/// escrow validator's `ApproveMilestone` branch is the only place that
/// pins down the correct asset name and quantity (see
/// `milestone_escrow.ak`), and its `ReleaseTranche` branch explicitly
/// forbids minting, closing the gap this policy alone can't check.
validator milestone_receipt(escrow_script_hash: ScriptHash) {
  mint(_redeemer: Data, policy_id: PolicyId, tx: Transaction) {
    let escrow_inputs =
      tx.inputs
        |> list.filter(
            fn(i) { i.output.address.payment_credential == Script(escrow_script_hash) },
          )

    expect [_escrow_input] = escrow_inputs

    expect [Pair(_asset_name, quantity)] =
      tx.mint |> assets.tokens(policy_id) |> dict.to_pairs()

    quantity == 1
  }

  else(_) {
    fail
  }
}
