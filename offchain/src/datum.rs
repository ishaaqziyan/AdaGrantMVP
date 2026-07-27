//! Mirrors `onchain/lib/milestone_escrow/types.ak` -- keep in sync.
//! On-chain `review_deadline` is a mandatory `Int`, always 8 fields; `None` here encodes as `NO_DEADLINE_SENTINEL`, not a dropped field.

use anyhow::{anyhow, bail, Context, Result};
use pallas_codec::minicbor;
use pallas_codec::utils::MaybeIndefArray;
use pallas_primitives::conway::{BigInt, BoundedBytes, Constr, PlutusData};
use serde::{Deserialize, Serialize};

/// Stands in for "no deadline" in the mandatory on-chain `Int` field -- far
/// enough out that `ClaimExpired`'s `valid_after` check can never pass.
const NO_DEADLINE_SENTINEL: i64 = i64::MAX;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Datum {
    pub reviewer: [u8; 28],
    pub proposer: [u8; 28],
    pub total_locked: i64,
    pub tranche_bps: Vec<i64>,
    pub approved: Vec<bool>,
    pub released_count: i64,
    pub receipt_policy_id: [u8; 28],
    pub review_deadline: Option<i64>,
    /// `true` iff this decoded from a genuinely 7-field constructor (no
    /// `review_deadline` at all) -- on-chain `Datum` now has 8 mandatory
    /// fields, so any grant with this set is permanently unspendable
    /// against the current script: `handlers::get_grants` must exclude it.
    /// Never set by anything this codebase encodes going forward (see
    /// `NO_DEADLINE_SENTINEL`), so this only ever fires on pre-existing,
    /// already-broken on-chain data.
    #[serde(default)]
    pub unspendable_legacy_shape: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Redeemer {
    ApproveMilestone(i64),
    ReleaseTranche(i64),
    ClaimExpired(i64),
}

fn pd_int(i: i64) -> PlutusData {
    PlutusData::BigInt(BigInt::Int(i.into()))
}

fn pd_bytes(bs: &[u8]) -> PlutusData {
    PlutusData::BoundedBytes(BoundedBytes::from(bs.to_vec()))
}

fn pd_bool(b: bool) -> PlutusData {
    pd_constr(if b { 122 } else { 121 }, vec![])
}

fn pd_array(xs: Vec<PlutusData>) -> PlutusData {
    PlutusData::Array(MaybeIndefArray::Def(xs))
}

fn pd_constr(tag: u64, fields: Vec<PlutusData>) -> PlutusData {
    PlutusData::Constr(Constr {
        tag,
        any_constructor: None,
        fields: MaybeIndefArray::Def(fields),
    })
}

fn encode(data: &PlutusData) -> Result<Vec<u8>> {
    pallas_codec::minicbor::to_vec(data).context("failed to CBOR-encode PlutusData")
}

fn as_constr(data: &PlutusData) -> Result<&Constr<PlutusData>> {
    match data {
        PlutusData::Constr(c) => Ok(c),
        other => bail!("expected a Constr, got {other:?}"),
    }
}

fn expect_constr_tag<'a>(data: &'a PlutusData, tag: u64, what: &str) -> Result<&'a Constr<PlutusData>> {
    let c = as_constr(data)?;
    if c.tag != tag {
        bail!("expected {what} (tag {tag}), got tag {}", c.tag);
    }
    Ok(c)
}

fn as_bytes28(data: &PlutusData) -> Result<[u8; 28]> {
    match data {
        PlutusData::BoundedBytes(b) => {
            let v: Vec<u8> = b.clone().into();
            v.try_into()
                .map_err(|v: Vec<u8>| anyhow!("expected 28 bytes, got {}", v.len()))
        }
        other => bail!("expected bytes, got {other:?}"),
    }
}

fn as_i64(data: &PlutusData) -> Result<i64> {
    match data {
        PlutusData::BigInt(BigInt::Int(i)) => {
            let v: i128 = (*i).into();
            i64::try_from(v).context("integer out of i64 range")
        }
        other => bail!("expected an integer, got {other:?}"),
    }
}

fn as_bool(data: &PlutusData) -> Result<bool> {
    let c = as_constr(data)?;
    match c.tag {
        121 => Ok(false),
        122 => Ok(true),
        other => bail!("expected a Bool (tag 121/122), got tag {other}"),
    }
}

fn as_list<T>(data: &PlutusData, decode_one: impl Fn(&PlutusData) -> Result<T>) -> Result<Vec<T>> {
    match data {
        PlutusData::Array(arr) => {
            let items: Vec<PlutusData> = arr.clone().into();
            items.iter().map(decode_one).collect()
        }
        other => bail!("expected a list, got {other:?}"),
    }
}

impl Datum {
    pub fn to_plutus_data(&self) -> PlutusData {
        let fields = vec![
            pd_bytes(&self.reviewer),
            pd_bytes(&self.proposer),
            pd_int(self.total_locked),
            pd_array(self.tranche_bps.iter().map(|&i| pd_int(i)).collect()),
            pd_array(self.approved.iter().map(|&b| pd_bool(b)).collect()),
            pd_int(self.released_count),
            pd_bytes(&self.receipt_policy_id),
            pd_int(self.review_deadline.unwrap_or(NO_DEADLINE_SENTINEL)),
        ];
        pd_constr(121, fields)
    }

    pub fn to_cbor(&self) -> Result<Vec<u8>> {
        encode(&self.to_plutus_data())
    }

    pub fn from_plutus_data(data: &PlutusData) -> Result<Self> {
        let c = expect_constr_tag(data, 121, "Datum")?;
        let mut fields: Vec<PlutusData> = c.fields.clone().into();

        let (review_deadline, unspendable_legacy_shape) = match fields.len() {
            7 => (None, true),
            8 => {
                let v = as_i64(&fields.pop().unwrap()).context("Datum.review_deadline")?;
                (if v == NO_DEADLINE_SENTINEL { None } else { Some(v) }, false)
            }
            n => bail!("Datum: expected 7 or 8 fields, got {n}"),
        };
        let [reviewer, proposer, total_locked, tranche_bps, approved, released_count, receipt_policy_id] =
            <[PlutusData; 7]>::try_from(fields)
                .map_err(|v: Vec<PlutusData>| anyhow!("Datum: expected 7 fields, got {}", v.len()))?;

        Ok(Datum {
            reviewer: as_bytes28(&reviewer).context("Datum.reviewer")?,
            proposer: as_bytes28(&proposer).context("Datum.proposer")?,
            total_locked: as_i64(&total_locked).context("Datum.total_locked")?,
            tranche_bps: as_list(&tranche_bps, as_i64).context("Datum.tranche_bps")?,
            approved: as_list(&approved, as_bool).context("Datum.approved")?,
            released_count: as_i64(&released_count).context("Datum.released_count")?,
            receipt_policy_id: as_bytes28(&receipt_policy_id).context("Datum.receipt_policy_id")?,
            review_deadline,
            unspendable_legacy_shape,
        })
    }

    pub fn from_inline_datum_hex(hex_str: &str) -> Result<Self> {
        let hex_str = hex_str.strip_prefix("0x").unwrap_or(hex_str);
        let bytes = hex::decode(hex_str).context("inline_datum is not valid hex")?;
        let data: PlutusData = minicbor::decode(&bytes).context("inline_datum is not valid PlutusData CBOR")?;
        Self::from_plutus_data(&data)
    }

    pub fn to_json(&self) -> serde_json::Value {
        serde_json::json!({
            "reviewer": hex::encode(self.reviewer),
            "proposer": hex::encode(self.proposer),
            "total_locked": self.total_locked,
            "tranche_bps": self.tranche_bps,
            "approved": self.approved,
            "released_count": self.released_count,
            "receipt_policy_id": hex::encode(self.receipt_policy_id),
            "review_deadline": self.review_deadline,
        })
    }
}

impl Redeemer {
    pub fn to_plutus_data(&self) -> PlutusData {
        match self {
            Redeemer::ApproveMilestone(i) => pd_constr(121, vec![pd_int(*i)]),
            Redeemer::ReleaseTranche(i) => pd_constr(122, vec![pd_int(*i)]),
            Redeemer::ClaimExpired(i) => pd_constr(123, vec![pd_int(*i)]),
        }
    }

    pub fn to_cbor(&self) -> Result<Vec<u8>> {
        encode(&self.to_plutus_data())
    }
}

pub fn void_redeemer_cbor() -> Result<Vec<u8>> {
    encode(&pd_constr(121, vec![]))
}

pub fn milestone_asset_name(index: u8) -> Vec<u8> {
    let mut name = b"Milestone".to_vec();
    name.push(index);
    name
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample() -> Datum {
        Datum {
            reviewer: [0x11; 28],
            proposer: [0x22; 28],
            total_locked: 100_000_000,
            tranche_bps: vec![4000, 3000, 3000],
            approved: vec![true, false, false],
            released_count: 0,
            receipt_policy_id: [0x33; 28],
            review_deadline: None,
            unspendable_legacy_shape: false,
        }
    }

    #[test]
    fn datum_round_trips_through_cbor() {
        let datum = sample();
        let bytes = datum.to_cbor().unwrap();
        let decoded = Datum::from_inline_datum_hex(&hex::encode(&bytes)).unwrap();
        assert_eq!(datum, decoded);
    }

    /// Legacy testnet-v4 escrows have no 8th field at all -- not a `null`,
    /// the constructor genuinely has 7 fields. Must decode as `None`, not
    /// error out, or every existing v4 grant breaks on the next `/grants` call.
    #[test]
    fn datum_decodes_legacy_seven_field_shape() {
        let legacy = pd_constr(
            121,
            vec![
                pd_bytes(&[0x11; 28]),
                pd_bytes(&[0x22; 28]),
                pd_int(100_000_000),
                pd_array(vec![pd_int(4000), pd_int(3000), pd_int(3000)]),
                pd_array(vec![pd_bool(true), pd_bool(false), pd_bool(false)]),
                pd_int(0),
                pd_bytes(&[0x33; 28]),
            ],
        );
        let bytes = encode(&legacy).unwrap();
        let decoded = Datum::from_inline_datum_hex(&hex::encode(&bytes)).unwrap();
        assert_eq!(decoded.review_deadline, None);
        assert!(decoded.unspendable_legacy_shape, "a genuinely 7-field datum must be flagged unspendable");
        assert_eq!(decoded, Datum { unspendable_legacy_shape: true, ..sample() });
    }

    /// On-chain `review_deadline` is a mandatory `Int` (never `Option<Int>`)
    /// on any deploy with `ClaimExpired` -- encoding `None` as a dropped
    /// 7th... 8th field (the pre-fix bug) makes the validator's very first
    /// `expect Some(datum) = datum` unparseable, stranding the grant on
    /// every redeemer. Must always emit 8 fields, using the sentinel.
    #[test]
    fn datum_encodes_none_deadline_as_eight_fields_with_sentinel() {
        let datum = sample();
        assert_eq!(datum.review_deadline, None);

        let PlutusData::Constr(c) = datum.to_plutus_data() else { panic!("not a Constr") };
        let fields: Vec<PlutusData> = c.fields.into();
        assert_eq!(fields.len(), 8, "must always encode all 8 fields, never drop review_deadline");

        let PlutusData::BigInt(BigInt::Int(last)) = fields.into_iter().next_back().unwrap() else {
            panic!("8th field not an Int")
        };
        assert_eq!(i128::from(last), NO_DEADLINE_SENTINEL as i128);
    }

    #[test]
    fn datum_round_trips_with_review_deadline() {
        let datum = Datum { review_deadline: Some(1_800_000_000_000), ..sample() };
        let bytes = datum.to_cbor().unwrap();
        let decoded = Datum::from_inline_datum_hex(&hex::encode(&bytes)).unwrap();
        assert_eq!(datum, decoded);
        assert_eq!(decoded.review_deadline, Some(1_800_000_000_000));
    }

    #[test]
    fn redeemer_tags_match_aiken() {
        let approve = Redeemer::ApproveMilestone(0).to_plutus_data();
        let PlutusData::Constr(c) = &approve else { panic!("not a Constr") };
        assert_eq!(c.tag, 121);

        let claim_expired = Redeemer::ClaimExpired(1).to_plutus_data();
        let PlutusData::Constr(c) = &claim_expired else { panic!("not a Constr") };
        assert_eq!(c.tag, 123);

        let release = Redeemer::ReleaseTranche(2).to_plutus_data();
        let PlutusData::Constr(c) = &release else { panic!("not a Constr") };
        assert_eq!(c.tag, 122);
    }

    #[test]
    fn milestone_asset_name_matches_aiken() {
        assert_eq!(hex::encode(milestone_asset_name(0)), "4d696c6573746f6e6500");
        assert_eq!(hex::encode(milestone_asset_name(2)), "4d696c6573746f6e6502");
    }
}
