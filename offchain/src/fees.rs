use anyhow::{bail, Context, Result};
use pallas_txbuilder::Output;

pub fn linear_fee(tx_size_bytes: usize, min_fee_a: u64, min_fee_b: u64) -> u64 {
    min_fee_a * tx_size_bytes as u64 + min_fee_b
}

pub const VKEY_WITNESS_CBOR_BYTES: u64 = 108;

pub fn script_execution_fee(ex_units: &[(u64, u64)], price_mem: f64, price_step: f64) -> u64 {
    let total: f64 = ex_units
        .iter()
        .map(|(mem, steps)| *mem as f64 * price_mem + *steps as f64 * price_step)
        .sum();
    total.ceil() as u64
}

pub fn min_utxo_lovelace(build_output: impl Fn(u64) -> Output, coins_per_utxo_byte: u64) -> Result<u64> {
    let mut lovelace = 1_000_000u64;
    for _ in 0..5 {
        let output = build_output(lovelace);
        let raw = output
            .build_babbage_raw()
            .context("failed to serialize candidate output for min-UTxO calc")?;
        let bytes = pallas_codec::minicbor::to_vec(&raw).context("failed to encode candidate output")?;
        let required = coins_per_utxo_byte * (160 + bytes.len() as u64);
        if required <= lovelace {
            return Ok(lovelace);
        }
        lovelace = required;
    }
    bail!("min-UTxO calculation did not converge")
}
