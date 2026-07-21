use aiken/collection/list
use cardano/address.{Address, VerificationKey}
use cardano/assets.{has_any_nft, has_nft_strict, lovelace_of}
use cardano/transaction.{
  InlineDatum, Output, OutputReference, Transaction, find_input,
}
use milestone_escrow/types.{ApproveMilestone, Datum, ReleaseTranche, Redeemer}
use milestone_escrow/utils.{
  milestone_asset_name, set_at, tranche_amount, valid_tranche_bps,
}

/// Single combined validator for the milestone-based grant escrow: holds
/// the locked funds and the milestone approval state together, so
/// `ReleaseTranche` can trust `ApproveMilestone`'s result without a
/// cross-validator reference.
///
/// Takes one validator parameter, `deployment_tag`, which the spend logic
/// never inspects -- it exists purely so a fresh deploy gets a distinct
/// script hash/address from any prior one, without touching the escrow's
/// own state model. (The companion Milestone Receipt minting policy,
/// `milestone_receipt.ak`, is parameterized by *this* script's hash, so
/// parameterizing this script by the receipt policy ID in turn would make
/// the two hashes depend on each other -- that's why `receipt_policy_id`
/// is carried in the datum instead, set once when the escrow UTxO is
/// created; see `types.ak`.)
validator milestone_escrow(_deployment_tag: ByteArray) {
  spend(
    datum: Option<Datum>,
    redeemer: Redeemer,
    own_ref: OutputReference,
    tx: Transaction,
  ) {
    expect Some(datum) = datum
    expect Some(own_input) = find_input(tx.inputs, own_ref)
    let own_address = own_input.output.address

    // Double-satisfaction guard: only one UTxO from this script may be
    // spent per transaction.
    expect [_] = tx.inputs |> list.filter(fn(i) { i.output.address == own_address })

    when redeemer is {
      ApproveMilestone(index) ->
        approve_milestone(datum, index, own_address, own_input.output, tx)
      ReleaseTranche(index) ->
        release_tranche(datum, index, own_address, own_input.output, tx)
    }
  }

  else(_) {
    fail
  }
}

fn approve_milestone(
  datum: Datum,
  index: Int,
  own_address: Address,
  own_output: Output,
  tx: Transaction,
) -> Bool {
  let datum_well_formed =
    valid_tranche_bps(datum.tranche_bps, datum.approved, datum.total_locked)

  let signed_by_reviewer = list.has(tx.extra_signatories, datum.reviewer)

  expect Some(currently_approved) = list.at(datum.approved, index)
  let not_yet_approved = !currently_approved

  let new_datum =
    Datum { ..datum, approved: set_at(datum.approved, index, True) }

  let state_preserved =
    when list.find(tx.outputs, fn(o) { o.address == own_address }) is {
      Some(out) -> {
        expect InlineDatum(out_datum_data) = out.datum
        expect out_parsed: Datum = out_datum_data
        out_parsed == new_datum && lovelace_of(out.value) == lovelace_of(
          own_output.value,
        )
      }
      None -> False
    }

  let receipt_minted =
    has_nft_strict(tx.mint, datum.receipt_policy_id, milestone_asset_name(index))

  datum_well_formed && signed_by_reviewer && not_yet_approved && state_preserved && receipt_minted
}

fn release_tranche(
  datum: Datum,
  index: Int,
  own_address: Address,
  own_output: Output,
  tx: Transaction,
) -> Bool {
  let datum_well_formed =
    valid_tranche_bps(datum.tranche_bps, datum.approved, datum.total_locked)

  let is_next_in_sequence = index == datum.released_count

  expect Some(is_approved) = list.at(datum.approved, index)

  let amount = tranche_amount(datum.total_locked, datum.tranche_bps, index)
  let is_final = index + 1 == list.length(datum.tranche_bps)

  let paid_to_proposer =
    tx.outputs
      |> list.any(
          fn(o) {
            o.address.payment_credential == VerificationKey(datum.proposer) && lovelace_of(
              o.value,
            ) >= amount
          },
        )

  let state_conserved =
    if is_final {
      // Final tranche drains the script UTxO entirely; no continuing
      // output is required.
      True
    } else {
      when list.find(tx.outputs, fn(o) { o.address == own_address }) is {
        Some(out) -> {
          expect InlineDatum(out_datum_data) = out.datum
          expect out_parsed: Datum = out_datum_data
          let expected_datum = Datum { ..datum, released_count: index + 1 }
          out_parsed == expected_datum && lovelace_of(out.value) == lovelace_of(
            own_output.value,
          ) - amount
        }
        None -> False
      }
    }

  // Receipt minting only ever accompanies an approval, never a release.
  let no_receipt_minted = !has_any_nft(tx.mint, datum.receipt_policy_id)

  datum_well_formed && is_next_in_sequence && is_approved && paid_to_proposer && state_conserved && no_receipt_minted
}
