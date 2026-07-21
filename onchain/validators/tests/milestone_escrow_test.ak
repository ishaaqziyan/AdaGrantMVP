use aiken/collection/list
use cardano/address.{Address}
use cardano/assets.{PolicyId, from_lovelace}
use cardano/transaction.{Input, NoDatum, Output, OutputReference, Transaction}
use milestone_escrow
use milestone_escrow/types.{ApproveMilestone, Datum, ReleaseTranche}
use milestone_escrow/utils.{milestone_asset_name}
use mocktail.{
  complete, mint, mocktail_tx, required_signer_hash, tx_in, tx_in_inline_datum,
  tx_out, tx_out_inline_datum,
}
use mocktail/virgin_address.{mock_pub_key_address, mock_script_address}
use mocktail/virgin_key_hash.{mock_policy_id, mock_pub_key_hash}
use mocktail/virgin_output_reference.{mock_tx_hash, mock_utxo_ref}

const total_locked = 100_000_000

const tranche_bps = [4000, 3000, 3000]

fn reviewer() -> ByteArray {
  mock_pub_key_hash(0)
}

fn proposer() -> ByteArray {
  mock_pub_key_hash(1)
}

fn receipt_policy_id() -> PolicyId {
  mock_policy_id(0)
}

fn script_address() -> Address {
  mock_script_address(0, None)
}

fn deployment_tag() -> ByteArray {
  #""
}

fn base_datum() -> Datum {
  Datum {
    reviewer: reviewer(),
    proposer: proposer(),
    total_locked,
    tranche_bps,
    approved: [False, False, False],
    released_count: 0,
    receipt_policy_id: receipt_policy_id(),
  }
}

fn oref() -> OutputReference {
  mock_utxo_ref(0, 0)
}

// --- ApproveMilestone -------------------------------------------------

type ApproveTestCase {
  is_signed: Bool,
  already_approved: Bool,
  datum_updated: Bool,
  value_preserved: Bool,
  receipt_minted: Bool,
}

fn all_valid_approve() -> ApproveTestCase {
  ApproveTestCase {
    is_signed: True,
    already_approved: False,
    datum_updated: True,
    value_preserved: True,
    receipt_minted: True,
  }
}

fn approve_input_datum(test_case: ApproveTestCase) -> Datum {
  if test_case.already_approved {
    Datum { ..base_datum(), approved: [True, False, False] }
  } else {
    base_datum()
  }
}

fn get_approve_tx(test_case: ApproveTestCase, index: Int) -> Transaction {
  let ApproveTestCase {
    is_signed,
    datum_updated,
    value_preserved,
    receipt_minted,
    ..
  } = test_case

  let input_datum = approve_input_datum(test_case)

  let correct_new_datum =
    Datum { ..base_datum(), approved: [True, False, False] }
  let wrong_new_datum = Datum { ..base_datum(), approved: [False, False, False] }

  let out_value =
    if value_preserved {
      from_lovelace(total_locked)
    } else {
      from_lovelace(total_locked - 1)
    }

  mocktail_tx()
    |> required_signer_hash(is_signed, reviewer())
    |> tx_in(True, mock_tx_hash(0), 0, from_lovelace(total_locked), script_address())
    |> tx_in_inline_datum(True, input_datum)
    |> tx_out(True, script_address(), out_value)
    |> tx_out_inline_datum(datum_updated, correct_new_datum)
    |> tx_out_inline_datum(!datum_updated, wrong_new_datum)
    |> mint(receipt_minted, 1, receipt_policy_id(), milestone_asset_name(index))
    |> complete()
}

test approve_milestone_succeeds() {
  let tx = get_approve_tx(all_valid_approve(), 0)
  milestone_escrow.milestone_escrow.spend(
    deployment_tag(),
    Some(approve_input_datum(all_valid_approve())),
    ApproveMilestone(0),
    oref(),
    tx,
  )
}

test approve_milestone_wrong_signer_fails() {
  let test_case = ApproveTestCase { ..all_valid_approve(), is_signed: False }
  let tx = get_approve_tx(test_case, 0)
  !milestone_escrow.milestone_escrow.spend(
    deployment_tag(),
    Some(approve_input_datum(test_case)),
    ApproveMilestone(0),
    oref(),
    tx,
  )
}

test approve_milestone_already_approved_fails() fail {
  let test_case = ApproveTestCase { ..all_valid_approve(), already_approved: True }
  let tx = get_approve_tx(test_case, 0)
  milestone_escrow.milestone_escrow.spend(
    deployment_tag(),
    Some(approve_input_datum(test_case)),
    ApproveMilestone(0),
    oref(),
    tx,
  )
}

test approve_milestone_datum_not_updated_fails() {
  let test_case = ApproveTestCase { ..all_valid_approve(), datum_updated: False }
  let tx = get_approve_tx(test_case, 0)
  !milestone_escrow.milestone_escrow.spend(
    deployment_tag(),
    Some(approve_input_datum(test_case)),
    ApproveMilestone(0),
    oref(),
    tx,
  )
}

test approve_milestone_value_moved_fails() {
  let test_case = ApproveTestCase { ..all_valid_approve(), value_preserved: False }
  let tx = get_approve_tx(test_case, 0)
  !milestone_escrow.milestone_escrow.spend(
    deployment_tag(),
    Some(approve_input_datum(test_case)),
    ApproveMilestone(0),
    oref(),
    tx,
  )
}

test approve_milestone_no_receipt_minted_fails() {
  let test_case = ApproveTestCase { ..all_valid_approve(), receipt_minted: False }
  let tx = get_approve_tx(test_case, 0)
  !milestone_escrow.milestone_escrow.spend(
    deployment_tag(),
    Some(approve_input_datum(test_case)),
    ApproveMilestone(0),
    oref(),
    tx,
  )
}

test approve_milestone_wrong_receipt_index_fails() {
  // mints a receipt for milestone 1's asset name while approving milestone 0
  let tx = get_approve_tx(all_valid_approve(), 1)
  !milestone_escrow.milestone_escrow.spend(
    deployment_tag(),
    Some(approve_input_datum(all_valid_approve())),
    ApproveMilestone(0),
    oref(),
    tx,
  )
}

test approve_milestone_malformed_tranche_bps_fails() {
  // tranche_bps sums to 11000, not 10000 -- otherwise a fully valid approve.
  let bad_datum = Datum { ..base_datum(), tranche_bps: [5000, 6000] }
  let new_datum = Datum { ..bad_datum, approved: [True, False, False] }

  let tx =
    mocktail_tx()
      |> required_signer_hash(True, reviewer())
      |> tx_in(True, mock_tx_hash(0), 0, from_lovelace(total_locked), script_address())
      |> tx_in_inline_datum(True, bad_datum)
      |> tx_out(True, script_address(), from_lovelace(total_locked))
      |> tx_out_inline_datum(True, new_datum)
      |> mint(True, 1, receipt_policy_id(), milestone_asset_name(0))
      |> complete()

  !milestone_escrow.milestone_escrow.spend(
    deployment_tag(),
    Some(bad_datum),
    ApproveMilestone(0),
    oref(),
    tx,
  )
}

// --- ReleaseTranche -----------------------------------------------------

type ReleaseTestCase {
  is_next_in_sequence: Bool,
  is_approved: Bool,
  is_paid: Bool,
  state_conserved: Bool,
  receipt_minted: Bool,
}

fn all_valid_release() -> ReleaseTestCase {
  ReleaseTestCase {
    is_next_in_sequence: True,
    is_approved: True,
    is_paid: True,
    state_conserved: True,
    receipt_minted: False,
  }
}

fn release_input_datum(test_case: ReleaseTestCase, index: Int) -> Datum {
  Datum {
    ..base_datum(),
    approved: if test_case.is_approved {
      [True, True, True]
    } else {
      [False, False, False]
    },
    released_count: if test_case.is_next_in_sequence {
      index
    } else {
      index + 1
    },
  }
}

fn get_release_tx(test_case: ReleaseTestCase, index: Int) -> Transaction {
  let ReleaseTestCase { is_paid, state_conserved, receipt_minted, .. } =
    test_case

  let input_datum = release_input_datum(test_case, index)

  // milestone 0 pays 40% of total_locked == 40_000_000
  let amount = 40_000_000
  let paid_amount = if is_paid {
    amount
  } else {
    amount - 1
  }

  let correct_continuing_datum =
    Datum { ..input_datum, released_count: index + 1 }
  let wrong_continuing_datum = input_datum

  let continuing_value =
    if state_conserved {
      from_lovelace(total_locked - amount)
    } else {
      from_lovelace(total_locked)
    }

  mocktail_tx()
    |> tx_in(True, mock_tx_hash(0), 0, from_lovelace(total_locked), script_address())
    |> tx_in_inline_datum(True, input_datum)
    |> tx_out(True, mock_pub_key_address(1, None), from_lovelace(paid_amount))
    |> tx_out(True, script_address(), continuing_value)
    |> tx_out_inline_datum(state_conserved, correct_continuing_datum)
    |> tx_out_inline_datum(!state_conserved, wrong_continuing_datum)
    |> mint(receipt_minted, 1, receipt_policy_id(), milestone_asset_name(index))
    |> complete()
}

test release_tranche_succeeds() {
  let test_case = all_valid_release()
  let tx = get_release_tx(test_case, 0)
  milestone_escrow.milestone_escrow.spend(
    deployment_tag(),
    Some(release_input_datum(test_case, 0)),
    ReleaseTranche(0),
    oref(),
    tx,
  )
}

test release_tranche_out_of_sequence_fails() fail {
  let test_case = ReleaseTestCase { ..all_valid_release(), is_next_in_sequence: False }
  let tx = get_release_tx(test_case, 0)
  milestone_escrow.milestone_escrow.spend(
    deployment_tag(),
    Some(release_input_datum(test_case, 0)),
    ReleaseTranche(0),
    oref(),
    tx,
  )
}

test release_tranche_not_approved_fails() fail {
  let test_case = ReleaseTestCase { ..all_valid_release(), is_approved: False }
  let tx = get_release_tx(test_case, 0)
  milestone_escrow.milestone_escrow.spend(
    deployment_tag(),
    Some(release_input_datum(test_case, 0)),
    ReleaseTranche(0),
    oref(),
    tx,
  )
}

test release_tranche_1_fails_before_milestone_1_approved() fail {
  // Generalizes release_tranche_not_approved_fails (index 0 only) to a
  // later milestone: milestone 0 is already approved and released
  // (released_count == 1, so index 1 is next in sequence), but milestone 1
  // itself is not yet approved -- ReleaseTranche(1) must still fail.
  let input_datum =
    Datum { ..base_datum(), approved: [True, False, False], released_count: 1 }
  let amount = 30_000_000
  let continuing_datum = Datum { ..input_datum, released_count: 2 }

  let tx =
    mocktail_tx()
      |> tx_in(True, mock_tx_hash(0), 0, from_lovelace(60_000_000), script_address())
      |> tx_in_inline_datum(True, input_datum)
      |> tx_out(True, mock_pub_key_address(1, None), from_lovelace(amount))
      |> tx_out(True, script_address(), from_lovelace(30_000_000))
      |> tx_out_inline_datum(True, continuing_datum)
      |> complete()

  milestone_escrow.milestone_escrow.spend(
    deployment_tag(),
    Some(input_datum),
    ReleaseTranche(1),
    oref(),
    tx,
  )
}

test release_tranche_underpaid_fails() {
  let test_case = ReleaseTestCase { ..all_valid_release(), is_paid: False }
  let tx = get_release_tx(test_case, 0)
  !milestone_escrow.milestone_escrow.spend(
    deployment_tag(),
    Some(release_input_datum(test_case, 0)),
    ReleaseTranche(0),
    oref(),
    tx,
  )
}

test release_tranche_state_not_conserved_fails() {
  let test_case = ReleaseTestCase { ..all_valid_release(), state_conserved: False }
  let tx = get_release_tx(test_case, 0)
  !milestone_escrow.milestone_escrow.spend(
    deployment_tag(),
    Some(release_input_datum(test_case, 0)),
    ReleaseTranche(0),
    oref(),
    tx,
  )
}

test release_tranche_sneaks_receipt_mint_fails() {
  let test_case = ReleaseTestCase { ..all_valid_release(), receipt_minted: True }
  let tx = get_release_tx(test_case, 0)
  !milestone_escrow.milestone_escrow.spend(
    deployment_tag(),
    Some(release_input_datum(test_case, 0)),
    ReleaseTranche(0),
    oref(),
    tx,
  )
}

test release_final_tranche_needs_no_continuing_output() {
  // index 2 is the final milestone (tranche_bps has 3 entries): the script
  // UTxO drains entirely, so no continuing output is required.
  let input_datum =
    Datum { ..base_datum(), approved: [True, True, True], released_count: 2 }
  let amount = 30_000_000

  let tx =
    mocktail_tx()
      |> tx_in(True, mock_tx_hash(0), 0, from_lovelace(30_000_000), script_address())
      |> tx_in_inline_datum(True, input_datum)
      |> tx_out(True, mock_pub_key_address(1, None), from_lovelace(amount))
      |> complete()

  milestone_escrow.milestone_escrow.spend(
    deployment_tag(),
    Some(input_datum),
    ReleaseTranche(2),
    oref(),
    tx,
  )
}

test release_tranche_malformed_tranche_bps_fails() {
  // tranche_bps sums to 11000, not 10000 -- otherwise a fully valid release.
  let bad_datum =
    Datum { ..base_datum(), tranche_bps: [5000, 6000], approved: [True, True] }
  let amount = 50_000_000
  let continuing_datum = Datum { ..bad_datum, released_count: 1 }

  let tx =
    mocktail_tx()
      |> tx_in(True, mock_tx_hash(0), 0, from_lovelace(total_locked), script_address())
      |> tx_in_inline_datum(True, bad_datum)
      |> tx_out(True, mock_pub_key_address(1, None), from_lovelace(amount))
      |> tx_out(True, script_address(), from_lovelace(total_locked - amount))
      |> tx_out_inline_datum(True, continuing_datum)
      |> complete()

  !milestone_escrow.milestone_escrow.spend(
    deployment_tag(),
    Some(bad_datum),
    ReleaseTranche(0),
    oref(),
    tx,
  )
}

test double_satisfaction_two_script_inputs_fails() fail {
  let test_case = all_valid_release()
  let tx =
    get_release_tx(test_case, 0)
      |> fn(t) {
          Transaction {
            ..t,
            inputs: list.concat(
              t.inputs,
              [
                Input {
                  output_reference: mock_utxo_ref(1, 0),
                  output: Output {
                    address: script_address(),
                    value: from_lovelace(1_000_000),
                    datum: NoDatum,
                    reference_script: None,
                  },
                },
              ],
            ),
          }
        }

  milestone_escrow.milestone_escrow.spend(
    deployment_tag(),
    Some(release_input_datum(test_case, 0)),
    ReleaseTranche(0),
    oref(),
    tx,
  )
}
