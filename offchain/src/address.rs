//! Bech32 address helpers: extracting a payment key hash and parsing a full address.

use anyhow::{bail, Context, Result};
use pallas_addresses::{Address, ShelleyPaymentPart};

pub fn payment_key_hash(bech32_addr: &str) -> Result<[u8; 28]> {
    let address = Address::from_bech32(bech32_addr)
        .with_context(|| format!("invalid bech32 address: {bech32_addr}"))?;

    let Address::Shelley(shelley) = address else {
        bail!("{bech32_addr} is not a Shelley payment address");
    };

    let ShelleyPaymentPart::Key(hash) = shelley.payment() else {
        bail!("{bech32_addr} is a script address, not a key address");
    };

    hash.as_ref()
        .try_into()
        .context("payment key hash was not 28 bytes")
}

pub fn parse_address(bech32_addr: &str) -> Result<Address> {
    Address::from_bech32(bech32_addr).with_context(|| format!("invalid bech32 address: {bech32_addr}"))
}
