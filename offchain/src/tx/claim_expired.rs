//! Mirrors `release.rs` almost exactly -- same payout/sequence/state-transition
//! rules (`payout_valid` on-chain is shared by both redeemers). The two
//! differences: no `approved[index]` check, and the tx's validity range lower
//! bound must land strictly after `datum.review_deadline` so the on-chain
//! `valid_after` check passes. See `ESCROW-UPGRADE.md` for the full context.

use anyhow::{ensure, Context, Result};
use pallas_txbuilder::{BuildConway, BuiltTransaction, Output, ScriptKind, StagingTransaction};

use crate::address::{parse_address, payment_key_hash};
use crate::blockfrost_client::{BlockfrostClient, EscrowUtxo};
use crate::config::Config;
use crate::datum::{Datum, Redeemer};
use crate::fees::{linear_fee, min_utxo_lovelace, script_execution_fee, VKEY_WITNESS_CBOR_BYTES};
use crate::tx::UtxoRef;

#[derive(Debug, serde::Deserialize)]
pub struct ClaimExpiredRequest {
    pub milestone_index: u8,
    pub submitter_address: String,
    pub proposer_address: String,
    pub tx_hash: String,
    pub output_index: u64,
    pub fee_input: UtxoRef,
    pub fee_input_lovelace: u64,
    pub collateral: UtxoRef,
}

pub async fn build(req: &ClaimExpiredRequest, config: &Config, client: &BlockfrostClient, escrow: &EscrowUtxo) -> Result<Vec<u8>> {
    let index = req.milestone_index as usize;

    ensure!(
        index < escrow.datum.tranche_bps.len(),
        "milestone_index {index} out of range (0..{})",
        escrow.datum.tranche_bps.len()
    );
    ensure!(
        index as i64 == escrow.datum.released_count,
        "milestone {index} is not next in sequence (released_count is {})",
        escrow.datum.released_count
    );

    let deadline = escrow.datum.review_deadline.context(
        "this escrow has no review_deadline -- it was created on a deploy that predates \
         ClaimExpired, so there's no expiry to claim against",
    )?;

    let latest = client.latest_block().await?;
    ensure!(
        latest.time_ms > deadline,
        "review_deadline ({deadline}) has not passed yet -- chain time is currently {}",
        latest.time_ms
    );

    let proposer_hash = payment_key_hash(&req.proposer_address)?;
    ensure!(
        proposer_hash == escrow.datum.proposer,
        "proposer_address does not match this escrow's proposer"
    );

    let amount = (escrow.datum.total_locked * escrow.datum.tranche_bps[index] / 10_000) as u64;
    let is_final = index + 1 == escrow.datum.tranche_bps.len();
    let payout = if is_final { escrow.lovelace } else { amount };

    let redeemer_cbor = Redeemer::ClaimExpired(index as i64).to_cbor()?;

    let escrow_input = UtxoRef {
        tx_hash: escrow.tx_hash.clone(),
        output_index: escrow.output_index,
    }
    .to_input()?;
    let fee_input = req.fee_input.to_input()?;
    let collateral_input = req.collateral.to_input()?;

    let escrow_addr = parse_address(&config.escrow_address)?;
    let submitter_addr = parse_address(&req.submitter_address)?;
    let proposer_addr = parse_address(&req.proposer_address)?;

    let escrow_script_bytes = hex::decode(&config.escrow_compiled_code).context("escrow_compiled_code is not valid hex")?;

    let params = client.protocol_params().await?;

    let change_floor = min_utxo_lovelace(|lovelace| Output::new(submitter_addr.clone(), lovelace), params.coins_per_utxo_byte)?;

    let new_datum_cbor = if is_final {
        None
    } else {
        let new_datum = Datum {
            released_count: escrow.datum.released_count + 1,
            ..escrow.datum.clone()
        };
        Some(new_datum.to_cbor()?)
    };

    let placeholder_ex_units = pallas_txbuilder::ExUnits {
        mem: 14_000_000,
        steps: 10_000_000_000,
    };

    let assemble = |fee: u64, spend_ex_units: pallas_txbuilder::ExUnits| -> Result<BuiltTransaction> {
        ensure!(
            req.fee_input_lovelace > fee,
            "fee_input ({} lovelace) doesn't cover fee ({})",
            req.fee_input_lovelace,
            fee
        );
        let raw_change = req.fee_input_lovelace - fee;
        let (change, actual_fee) = if raw_change > 0 && raw_change < change_floor {
            (0, fee + raw_change)
        } else {
            (raw_change, fee)
        };

        let mut tx = StagingTransaction::new()
            .input(escrow_input.clone())
            .input(fee_input.clone())
            .collateral_input(collateral_input.clone())
            .valid_from_slot(latest.slot)
            .output(Output::new(proposer_addr.clone(), payout))
            .script(ScriptKind::PlutusV3, escrow_script_bytes.clone())
            .add_spend_redeemer(escrow_input.clone(), redeemer_cbor.clone(), Some(spend_ex_units))
            .add_language(ScriptKind::PlutusV3, params.plutus_v3_cost_model.clone())
            .fee(actual_fee);

        if let Some(ref datum_cbor) = new_datum_cbor {
            tx = tx.output(Output::new(escrow_addr.clone(), escrow.lovelace - payout).set_inline_datum(datum_cbor.clone()));
        }

        if change > 0 {
            tx = tx.output(Output::new(submitter_addr.clone(), change));
        }

        tx.build_conway_raw().context("failed to build claim-expired transaction")
    };

    let draft = assemble(0, placeholder_ex_units.clone())?;
    let budgets = client.evaluate(&draft.tx_bytes.0).await.context("evaluate() failed")?;

    let spend_budget = budgets
        .iter()
        .find(|b| b.purpose == "spend")
        .context("evaluate() returned no budget for the spend redeemer")?;
    let spend_ex_units = pallas_txbuilder::ExUnits {
        mem: spend_budget.mem,
        steps: spend_budget.steps,
    };

    let script_fee = script_execution_fee(&[(spend_ex_units.mem, spend_ex_units.steps)], params.price_mem, params.price_step);

    let built = assemble(script_fee, spend_ex_units.clone())?;
    let fee1 = linear_fee(built.tx_bytes.0.len() + VKEY_WITNESS_CBOR_BYTES as usize, params.min_fee_a, params.min_fee_b) + script_fee;
    let built = assemble(fee1, spend_ex_units.clone())?;
    let fee2 = linear_fee(built.tx_bytes.0.len() + VKEY_WITNESS_CBOR_BYTES as usize, params.min_fee_a, params.min_fee_b) + script_fee;
    let final_built = if fee2 == fee1 { built } else { assemble(fee2, spend_ex_units)? };

    Ok(final_built.tx_bytes.0)
}
