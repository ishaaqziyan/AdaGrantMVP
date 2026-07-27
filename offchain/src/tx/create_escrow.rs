//! Builds the initial escrow-locking tx: funds sent to the script address
//! with the starting `Datum`. Not a redeemer -- the validator never runs
//! at creation, only on the first spend.

use anyhow::{ensure, Context, Result};
use pallas_txbuilder::{BuildConway, BuiltTransaction, Output, StagingTransaction};

use crate::address::{parse_address, payment_key_hash};
use crate::datum::Datum;
use crate::fees::{linear_fee, VKEY_WITNESS_CBOR_BYTES};
use crate::tx::UtxoRef;

#[derive(Debug, serde::Deserialize)]
pub struct CreateEscrowRequest {
    pub proposer_address: String,
    pub reviewer_address: String,
    pub tranche_bps: Vec<i64>,
    pub total_locked: u64,
    pub fee_input: UtxoRef,
    pub fee_input_lovelace: u64,
    /// POSIX ms after which `ClaimExpired` bypasses the reviewer's
    /// signature -- only meaningful (and only usable on-chain) against a
    /// deploy whose validator actually has that redeemer, which the
    /// current `testnet-v4` deploy does. `None` for no deadline.
    pub review_deadline: Option<i64>,
}

pub fn build(
    req: &CreateEscrowRequest,
    escrow_address: &str,
    receipt_policy_id: &[u8; 28],
    min_fee_a: u64,
    min_fee_b: u64,
) -> Result<Vec<u8>> {
    ensure!(!req.tranche_bps.is_empty(), "tranche_bps must not be empty");
    ensure!(
        req.tranche_bps.iter().sum::<i64>() == 10_000,
        "tranche_bps must sum to 10000 (basis points)"
    );
    ensure!(req.total_locked > 0, "total_locked must be positive");

    let reviewer = payment_key_hash(&req.reviewer_address)?;
    let proposer = payment_key_hash(&req.proposer_address)?;

    let datum = Datum {
        reviewer,
        proposer,
        total_locked: req.total_locked as i64,
        tranche_bps: req.tranche_bps.clone(),
        approved: vec![false; req.tranche_bps.len()],
        released_count: 0,
        receipt_policy_id: *receipt_policy_id,
        review_deadline: req.review_deadline,
    };
    let datum_cbor = datum.to_cbor()?;

    let escrow_addr = parse_address(escrow_address)?;
    let reviewer_addr = parse_address(&req.reviewer_address)?;
    let fee_input = req.fee_input.to_input()?;

    let build_with_fee = |fee: u64| -> Result<BuiltTransaction> {
        ensure!(
            req.fee_input_lovelace > req.total_locked + fee,
            "fee_input ({} lovelace) doesn't cover total_locked ({}) + fee ({})",
            req.fee_input_lovelace,
            req.total_locked,
            fee
        );
        let change = req.fee_input_lovelace - req.total_locked - fee;

        let mut tx = StagingTransaction::new()
            .input(fee_input.clone())
            .output(Output::new(escrow_addr.clone(), req.total_locked).set_inline_datum(datum_cbor.clone()))
            .fee(fee);

        if change > 0 {
            tx = tx.output(Output::new(reviewer_addr.clone(), change));
        }

        tx.build_conway_raw().context("failed to build create-escrow transaction")
    };

    let draft = build_with_fee(0)?;
    let fee1 = linear_fee(draft.tx_bytes.0.len() + VKEY_WITNESS_CBOR_BYTES as usize, min_fee_a, min_fee_b);
    let built = build_with_fee(fee1)?;
    let fee2 = linear_fee(built.tx_bytes.0.len() + VKEY_WITNESS_CBOR_BYTES as usize, min_fee_a, min_fee_b);
    let final_built = if fee2 == fee1 { built } else { build_with_fee(fee2)? };

    Ok(final_built.tx_bytes.0)
}

#[cfg(test)]
mod live_tests {
    use super::*;
    use crate::blockfrost_client::BlockfrostClient;
    use crate::config::Config;
    use pallas_addresses::{Network, ShelleyAddress, ShelleyDelegationPart, ShelleyPaymentPart};
    use pallas_crypto::hash::Hash;

    fn fake_testnet_address(fill: u8) -> String {
        let hash: Hash<28> = Hash::new([fill; 28]);
        ShelleyAddress::new(Network::Testnet, ShelleyPaymentPart::Key(hash), ShelleyDelegationPart::Null)
            .to_bech32()
            .unwrap()
    }

    #[tokio::test]
    #[ignore]
    async fn live_evaluate_rejects_on_unknown_utxo_not_bad_cbor() {
        let config = Config::load().expect("load onchain/deploy/testnet config");
        let client = BlockfrostClient::new(config.blockfrost_project_id.clone());

        let receipt_policy_id: [u8; 28] = hex::decode(&config.receipt_policy_id)
            .unwrap()
            .try_into()
            .unwrap();

        let req = CreateEscrowRequest {
            proposer_address: fake_testnet_address(0xaa),
            reviewer_address: fake_testnet_address(0xbb),
            tranche_bps: vec![4000, 3000, 3000],
            total_locked: 100_000_000,
            fee_input: UtxoRef {
                tx_hash: "00".repeat(32),
                output_index: 0,
            },
            fee_input_lovelace: 1_000_000_000_000,
            review_deadline: None,
        };

        let params = client.protocol_params().await.expect("fetch protocol params");
        let cbor = build(&req, &config.escrow_address, &receipt_policy_id, params.min_fee_a, params.min_fee_b)
            .expect("build create-escrow tx");

        eprintln!("built {} bytes: {}", cbor.len(), hex::encode(&cbor));

        match client.evaluate(&cbor).await {
            Ok(redeemers) => {
                assert!(redeemers.is_empty());
            }
            Err(err) => {
                let msg = err.to_string();
                eprintln!("evaluate error: {msg}");
                assert!(
                    !msg.to_lowercase().contains("cbor") && !msg.to_lowercase().contains("deserial"),
                    "expected a ledger-state error (e.g. unknown UTxO), got what looks like a CBOR decode error: {msg}"
                );
            }
        }
    }
}

