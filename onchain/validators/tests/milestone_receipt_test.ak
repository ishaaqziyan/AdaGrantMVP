use aiken/collection/list
use cardano/address.{Address}
use cardano/assets.{PolicyId, from_lovelace}
use cardano/transaction.{Input, NoDatum, Output, Transaction}
use milestone_escrow/utils.{milestone_asset_name}
use milestone_receipt
use mocktail.{complete, mint, mocktail_tx, tx_in}
use mocktail/virgin_address.{mock_pub_key_address, mock_script_address}
use mocktail/virgin_key_hash.{mock_policy_id, mock_script_hash}
use mocktail/virgin_output_reference.{mock_tx_hash, mock_utxo_ref}

fn escrow_script_hash() -> ByteArray {
  mock_script_hash(0)
}

fn escrow_addr() -> Address {
  mock_script_address(0, None)
}

fn policy_id() -> PolicyId {
  mock_policy_id(0)
}

fn asset_name() -> ByteArray {
  milestone_asset_name(0)
}

test mint_succeeds_alongside_escrow_input() {
  let tx =
    mocktail_tx()
      |> tx_in(True, mock_tx_hash(0), 0, from_lovelace(2_000_000), escrow_addr())
      |> mint(True, 1, policy_id(), asset_name())
      |> complete()

  milestone_receipt.milestone_receipt.mint(escrow_script_hash(), Void, policy_id(), tx)
}

test mint_fails_without_escrow_input() fail {
  let tx =
    mocktail_tx()
      |> tx_in(True, mock_tx_hash(0), 0, from_lovelace(2_000_000), mock_pub_key_address(1, None))
      |> mint(True, 1, policy_id(), asset_name())
      |> complete()

  milestone_receipt.milestone_receipt.mint(escrow_script_hash(), Void, policy_id(), tx)
}

test mint_fails_with_two_escrow_inputs() fail {
  let tx =
    mocktail_tx()
      |> tx_in(True, mock_tx_hash(0), 0, from_lovelace(2_000_000), escrow_addr())
      |> mint(True, 1, policy_id(), asset_name())
      |> complete()
  let tx =
    Transaction {
      ..tx,
      inputs: tx.inputs
        |> list.concat(
            [
              Input {
                output_reference: mock_utxo_ref(1, 0),
                output: Output {
                  address: escrow_addr(),
                  value: from_lovelace(1_000_000),
                  datum: NoDatum,
                  reference_script: None,
                },
              },
            ],
          ),
    }

  milestone_receipt.milestone_receipt.mint(escrow_script_hash(), Void, policy_id(), tx)
}

test mint_fails_with_quantity_two() {
  let tx =
    mocktail_tx()
      |> tx_in(True, mock_tx_hash(0), 0, from_lovelace(2_000_000), escrow_addr())
      |> mint(True, 2, policy_id(), asset_name())
      |> complete()

  !milestone_receipt.milestone_receipt.mint(escrow_script_hash(), Void, policy_id(), tx)
}

test mint_fails_on_burn() {
  let tx =
    mocktail_tx()
      |> tx_in(True, mock_tx_hash(0), 0, from_lovelace(2_000_000), escrow_addr())
      |> mint(True, -1, policy_id(), asset_name())
      |> complete()

  !milestone_receipt.milestone_receipt.mint(escrow_script_hash(), Void, policy_id(), tx)
}

test mint_fails_with_two_distinct_assets() fail {
  let tx =
    mocktail_tx()
      |> tx_in(True, mock_tx_hash(0), 0, from_lovelace(2_000_000), escrow_addr())
      |> mint(True, 1, policy_id(), milestone_asset_name(0))
      |> mint(True, 1, policy_id(), milestone_asset_name(1))
      |> complete()

  milestone_receipt.milestone_receipt.mint(escrow_script_hash(), Void, policy_id(), tx)
}
