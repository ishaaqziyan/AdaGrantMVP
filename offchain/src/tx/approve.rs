//! Builds an `ApproveMilestone` tx: reviewer signs, mints the milestone's
//! receipt NFT, flips `approved[index]`.

use anyhow::{ensure, Context, Result};
use pallas_crypto::hash::Hash;
use pallas_txbuilder::{BuildConway, BuiltTransaction, Output, ScriptKind, StagingTransaction};

use crate::address::{parse_address, payment_key_hash};
use crate::blockfrost_client::{BlockfrostClient, EscrowUtxo};
use crate::config::Config;
use crate::datum::{milestone_asset_name, void_redeemer_cbor, Datum, Redeemer};
use crate::fees::{linear_fee, min_utxo_lovelace, script_execution_fee, VKEY_WITNESS_CBOR_BYTES};
use crate::tx::UtxoRef;

#[derive(Debug, serde::Deserialize)]
pub struct ApproveMilestoneRequest {
    pub milestone_index: u8,
    pub reviewer_address: String,
    pub tx_hash: String,
    pub output_index: u64,
    pub fee_input: UtxoRef,
    pub fee_input_lovelace: u64,
    pub collateral: UtxoRef,
}

pub async fn build(req: &ApproveMilestoneRequest, config: &Config, client: &BlockfrostClient, escrow: &EscrowUtxo) -> Result<Vec<u8>> {
    let index = req.milestone_index as usize;

    ensure!(
        index < escrow.datum.tranche_bps.len(),
        "milestone_index {index} out of range (0..{})",
        escrow.datum.tranche_bps.len()
    );
    ensure!(!escrow.datum.approved[index], "milestone {index} is already approved");

    let reviewer_hash = payment_key_hash(&req.reviewer_address)?;
    ensure!(
        reviewer_hash == escrow.datum.reviewer,
        "reviewer_address does not match this escrow's reviewer"
    );

    let mut new_approved = escrow.datum.approved.clone();
    new_approved[index] = true;
    let new_datum = Datum {
        approved: new_approved,
        ..escrow.datum.clone()
    };
    let new_datum_cbor = new_datum.to_cbor()?;

    let redeemer_cbor = Redeemer::ApproveMilestone(index as i64).to_cbor()?;
    let mint_redeemer_cbor = void_redeemer_cbor()?;
    let asset_name = milestone_asset_name(req.milestone_index);

    let receipt_policy_hash: Hash<28> = config
        .receipt_policy_id
        .parse()
        .context("invalid receipt_policy_id in config")?;

    let escrow_input = UtxoRef {
        tx_hash: escrow.tx_hash.clone(),
        output_index: escrow.output_index,
    }
    .to_input()?;
    let fee_input = req.fee_input.to_input()?;
    let collateral_input = req.collateral.to_input()?;

    let escrow_addr = parse_address(&config.escrow_address)?;
    let reviewer_addr = parse_address(&req.reviewer_address)?;

    let escrow_script_bytes = hex::decode(&config.escrow_compiled_code).context("escrow_compiled_code is not valid hex")?;
    let receipt_script_bytes =
        hex::decode(&config.receipt_compiled_code).context("receipt_compiled_code is not valid hex")?;

    let params = client.protocol_params().await?;

    let receipt_lovelace = min_utxo_lovelace(
        |lovelace| {
            Output::new(reviewer_addr.clone(), lovelace)
                .add_asset(receipt_policy_hash, asset_name.clone(), 1)
                .expect("asset name is <= 32 bytes")
        },
        params.coins_per_utxo_byte,
    )?;

    ensure!(
        req.fee_input_lovelace > receipt_lovelace,
        "fee_input ({} lovelace) doesn't cover the receipt output's min-UTxO ({})",
        req.fee_input_lovelace,
        receipt_lovelace
    );

    let change_floor = min_utxo_lovelace(|lovelace| Output::new(reviewer_addr.clone(), lovelace), params.coins_per_utxo_byte)?;

    let placeholder_ex_units = pallas_txbuilder::ExUnits {
        mem: 14_000_000,
        steps: 10_000_000_000,
    };

    let assemble = |fee: u64,
                     spend_ex_units: pallas_txbuilder::ExUnits,
                     mint_ex_units: pallas_txbuilder::ExUnits|
     -> Result<BuiltTransaction> {
        ensure!(
            req.fee_input_lovelace > receipt_lovelace + fee,
            "fee_input ({} lovelace) doesn't cover receipt min-UTxO ({}) + fee ({})",
            req.fee_input_lovelace,
            receipt_lovelace,
            fee
        );
        let raw_change = req.fee_input_lovelace - receipt_lovelace - fee;
        let (change, actual_fee) = if raw_change > 0 && raw_change < change_floor {
            (0, fee + raw_change)
        } else {
            (raw_change, fee)
        };

        let mut tx = StagingTransaction::new()
            .input(escrow_input.clone())
            .input(fee_input.clone())
            .collateral_input(collateral_input.clone())
            .disclosed_signer(Hash::new(reviewer_hash))
            .output(Output::new(escrow_addr.clone(), escrow.lovelace).set_inline_datum(new_datum_cbor.clone()))
            .output(
                Output::new(reviewer_addr.clone(), receipt_lovelace)
                    .add_asset(receipt_policy_hash, asset_name.clone(), 1)
                    .expect("asset name is <= 32 bytes"),
            )
            .mint_asset(receipt_policy_hash, asset_name.clone(), 1)
            .expect("asset name is <= 32 bytes")
            .script(ScriptKind::PlutusV3, escrow_script_bytes.clone())
            .script(ScriptKind::PlutusV3, receipt_script_bytes.clone())
            .add_spend_redeemer(escrow_input.clone(), redeemer_cbor.clone(), Some(spend_ex_units))
            .add_mint_redeemer(receipt_policy_hash, mint_redeemer_cbor.clone(), Some(mint_ex_units))
            .add_language(ScriptKind::PlutusV3, params.plutus_v3_cost_model.clone())
            .fee(actual_fee);

        if change > 0 {
            tx = tx.output(Output::new(reviewer_addr.clone(), change));
        }

        tx.build_conway_raw().context("failed to build approve-milestone transaction")
    };

    let draft = assemble(0, placeholder_ex_units.clone(), placeholder_ex_units.clone())?;
    let budgets = client.evaluate(&draft.tx_bytes.0).await.context("evaluate() failed")?;

    let spend_budget = budgets
        .iter()
        .find(|b| b.purpose == "spend")
        .context("evaluate() returned no budget for the spend redeemer")?;
    let mint_budget = budgets
        .iter()
        .find(|b| b.purpose == "mint")
        .context("evaluate() returned no budget for the mint redeemer")?;

    let spend_ex_units = pallas_txbuilder::ExUnits {
        mem: spend_budget.mem,
        steps: spend_budget.steps,
    };
    let mint_ex_units = pallas_txbuilder::ExUnits {
        mem: mint_budget.mem,
        steps: mint_budget.steps,
    };

    let script_fee = script_execution_fee(
        &[
            (spend_ex_units.mem, spend_ex_units.steps),
            (mint_ex_units.mem, mint_ex_units.steps),
        ],
        params.price_mem,
        params.price_step,
    );

    let built = assemble(script_fee, spend_ex_units.clone(), mint_ex_units.clone())?;
    let fee1 = linear_fee(built.tx_bytes.0.len() + VKEY_WITNESS_CBOR_BYTES as usize, params.min_fee_a, params.min_fee_b) + script_fee;
    let built = assemble(fee1, spend_ex_units.clone(), mint_ex_units.clone())?;
    let fee2 = linear_fee(built.tx_bytes.0.len() + VKEY_WITNESS_CBOR_BYTES as usize, params.min_fee_a, params.min_fee_b) + script_fee;
    let final_built = if fee2 == fee1 {
        built
    } else {
        assemble(fee2, spend_ex_units, mint_ex_units)?
    };

    Ok(final_built.tx_bytes.0)
}
