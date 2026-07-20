//! Shared tx-builder types: submodule declarations plus `UtxoRef`.

pub mod approve;
pub mod create_escrow;
pub mod release;

use anyhow::{Context, Result};
use pallas_crypto::hash::Hash;
use pallas_txbuilder::Input;

#[derive(Debug, Clone, serde::Deserialize)]
pub struct UtxoRef {
    pub tx_hash: String,
    pub output_index: u64,
}

impl UtxoRef {
    pub fn to_input(&self) -> Result<Input> {
        let hash: Hash<32> = self
            .tx_hash
            .parse()
            .with_context(|| format!("invalid tx_hash: {}", self.tx_hash))?;
        Ok(Input::new(hash, self.output_index))
    }
}
