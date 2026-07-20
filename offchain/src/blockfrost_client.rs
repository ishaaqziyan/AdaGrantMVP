//! Blockfrost client: UTxO/datum/transaction reads plus tx evaluation.

use anyhow::{bail, Context, Result};
use blockfrost::{BlockFrostSettings, BlockfrostAPI, Pagination, CARDANO_MAINNET_URL, CARDANO_PREPROD_URL, CARDANO_PREVIEW_URL};
use serde::Deserialize;

use crate::datum::Datum;
use crate::grants_meta::grant_id;

pub struct EscrowUtxo {
    pub tx_hash: String,
    pub output_index: u64,
    pub lovelace: u64,
    pub datum: Datum,
}

pub struct TxSummary {
    pub tx_hash: String,
    pub block_time: u64,
    pub block_height: u64,
}

pub fn find_by_outref<'a>(grants: &'a [EscrowUtxo], tx_hash: &str, output_index: u64) -> Option<&'a EscrowUtxo> {
    grants.iter().find(|g| g.tx_hash == tx_hash && g.output_index == output_index)
}

pub struct ProtocolParams {
    pub min_fee_a: u64,
    pub min_fee_b: u64,
    pub price_mem: f64,
    pub price_step: f64,
    pub plutus_v3_cost_model: Vec<i64>,
    pub coins_per_utxo_byte: u64,
}

pub struct RedeemerBudget {
    pub purpose: String,
    pub mem: u64,
    pub steps: u64,
}

pub struct BlockfrostClient {
    api: BlockfrostAPI,
    http: reqwest::Client,
    base_url: String,
    project_id: String,
}

impl BlockfrostClient {
    pub fn new(project_id: String) -> Self {
        let base_url = base_url_for_project_id(&project_id).to_string();
        let api = BlockfrostAPI::new(&project_id, BlockFrostSettings::new());
        Self {
            api,
            http: reqwest::Client::new(),
            base_url,
            project_id,
        }
    }

    pub async fn list_grants(&self, address: &str) -> Result<Vec<EscrowUtxo>> {
        let utxos = match self
            .api
            .addresses_utxos(address, Pagination::new(blockfrost::Order::Asc, 1, 100))
            .await
        {
            Ok(utxos) => utxos,
            Err(blockfrost::BlockfrostError::Response { reason, .. }) if reason.status_code == 404 => Vec::new(),
            Err(e) => return Err(e).with_context(|| format!("failed to fetch UTxOs at {address}")),
        };

        utxos
            .into_iter()
            .filter(|u| u.inline_datum.is_some())
            .map(|utxo| {
                let inline_datum = utxo
                    .inline_datum
                    .as_ref()
                    .expect("filtered on inline_datum.is_some() above");
                let datum = Datum::from_inline_datum_hex(inline_datum)
                    .with_context(|| format!("failed to decode inline datum at {address}"))?;

                let lovelace: u64 = utxo
                    .amount
                    .iter()
                    .find(|a| a.unit == "lovelace")
                    .context("UTxO has no lovelace amount")?
                    .quantity
                    .parse()
                    .context("lovelace quantity is not a valid number")?;

                Ok(EscrowUtxo {
                    tx_hash: utxo.tx_hash,
                    output_index: utxo.output_index as u64,
                    lovelace,
                    datum,
                })
            })
            .collect()
    }

    pub async fn recent_transactions(&self, address: &str, count: usize) -> Result<Vec<TxSummary>> {
        let txs = match self
            .api
            .addresses_transactions(address, Pagination::new(blockfrost::Order::Desc, 1, count))
            .await
        {
            Ok(txs) => txs,
            Err(blockfrost::BlockfrostError::Response { reason, .. }) if reason.status_code == 404 => Vec::new(),
            Err(e) => return Err(e).with_context(|| format!("failed to fetch transactions at {address}")),
        };

        Ok(txs
            .into_iter()
            .map(|t| TxSummary {
                tx_hash: t.tx_hash,
                block_time: t.block_time as u64,
                block_height: t.block_height as u64,
            })
            .collect())
    }

    pub async fn transactions_for_grant(
        &self,
        address: &str,
        target: &Datum,
        count: usize,
        scan_limit: usize,
    ) -> Result<Vec<TxSummary>> {
        let target_id = grant_id(&target.reviewer, &target.proposer, target.total_locked, &target.tranche_bps);
        let candidates = self.recent_transactions(address, scan_limit).await?;

        let mut matched = Vec::new();
        for candidate in candidates {
            if matched.len() >= count {
                break;
            }

            let utxos = self
                .api
                .transactions_utxos(&candidate.tx_hash)
                .await
                .with_context(|| format!("failed to fetch utxos for tx {}", candidate.tx_hash))?;

            let sides = utxos
                .inputs
                .iter()
                .map(|i| (i.address.as_str(), &i.inline_datum))
                .chain(utxos.outputs.iter().map(|o| (o.address.as_str(), &o.inline_datum)));

            let belongs_to_grant = sides.filter(|(addr, _)| *addr == address).any(|(_, datum_hex)| {
                datum_hex
                    .as_deref()
                    .and_then(|hex| Datum::from_inline_datum_hex(hex).ok())
                    .map(|d| grant_id(&d.reviewer, &d.proposer, d.total_locked, &d.tranche_bps) == target_id)
                    .unwrap_or(false)
            });

            if belongs_to_grant {
                matched.push(candidate);
            }
        }

        Ok(matched)
    }

    pub async fn protocol_params(&self) -> Result<ProtocolParams> {
        let params = self
            .api
            .epochs_latest_parameters()
            .await
            .context("failed to fetch latest epoch parameters")?;

        let cost_models_raw = params
            .cost_models_raw
            .flatten()
            .context("epoch parameters missing cost_models_raw")?;
        let plutus_v3 = cost_models_raw
            .get("PlutusV3")
            .context("cost_models_raw missing PlutusV3")?;
        let plutus_v3_cost_model: Vec<i64> = serde_json::from_value(plutus_v3.clone())
            .context("cost_models_raw.PlutusV3 is not an array of integers")?;

        let price_mem = params
            .price_mem
            .context("epoch parameters missing price_mem")?;
        let price_step = params
            .price_step
            .context("epoch parameters missing price_step")?;
        let coins_per_utxo_byte: u64 = params
            .coins_per_utxo_size
            .context("epoch parameters missing coins_per_utxo_size")?
            .parse()
            .context("coins_per_utxo_size is not a valid number")?;

        Ok(ProtocolParams {
            min_fee_a: params.min_fee_a as u64,
            min_fee_b: params.min_fee_b as u64,
            price_mem,
            price_step,
            plutus_v3_cost_model,
            coins_per_utxo_byte,
        })
    }

    pub async fn evaluate(&self, tx_cbor: &[u8]) -> Result<Vec<RedeemerBudget>> {
        let url = format!("{}/utils/txs/evaluate?version=6", self.base_url);

        let response = self
            .http
            .post(&url)
            .header("project_id", &self.project_id)
            .header("Content-Type", "application/cbor")
            .body(tx_cbor.to_vec())
            .send()
            .await
            .context("evaluate request failed")?;

        let status = response.status();
        let text = response.text().await.context("failed to read evaluate response body")?;

        if !status.is_success() {
            bail!("evaluate returned HTTP {status}: {text}");
        }

        let parsed: EvaluateResponse =
            serde_json::from_str(&text).with_context(|| format!("unexpected evaluate response shape: {text}"))?;

        if let Some(error) = parsed.error {
            bail!("evaluateTransaction returned an error: {error}");
        }

        let items = parsed
            .result
            .context("evaluateTransaction response has neither result nor error")?;

        items
            .into_iter()
            .map(|item| {
                Ok(RedeemerBudget {
                    purpose: item.validator.purpose,
                    mem: item.budget.memory,
                    steps: item.budget.cpu,
                })
            })
            .collect()
    }
}

fn base_url_for_project_id(project_id: &str) -> &'static str {
    if project_id.starts_with("mainnet") {
        CARDANO_MAINNET_URL
    } else if project_id.starts_with("preview") {
        CARDANO_PREVIEW_URL
    } else if project_id.starts_with("preprod") {
        CARDANO_PREPROD_URL
    } else {
        CARDANO_MAINNET_URL
    }
}

#[derive(Debug, Deserialize)]
struct EvaluateResponse {
    result: Option<Vec<EvaluateResultItem>>,
    error: Option<serde_json::Value>,
}

#[derive(Debug, Deserialize)]
struct EvaluateResultItem {
    validator: EvaluateValidator,
    budget: EvaluateBudget,
}

#[derive(Debug, Deserialize)]
struct EvaluateValidator {
    purpose: String,
}

#[derive(Debug, Deserialize)]
struct EvaluateBudget {
    memory: u64,
    cpu: u64,
}
