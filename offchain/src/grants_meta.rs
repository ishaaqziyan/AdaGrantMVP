//! Off-chain metadata (name, milestone descriptions) for grants, keyed by
//! a content-derived `grant_id` since that's stable across a grant's
//! lifetime while its live UTxO isn't. Also caches the last-seen on-chain
//! snapshot so a grant stays displayable after its UTxO is spent (e.g.
//! once fully released).

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Mutex;

use anyhow::{Context, Result};
use pallas_crypto::hash::Hasher;
use serde::{Deserialize, Serialize};

use crate::datum::Datum;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MilestoneMeta {
    pub name: String,
    pub description: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GrantSnapshot {
    pub tx_hash: String,
    pub output_index: u64,
    pub lovelace: u64,
    pub datum: Datum,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GrantMeta {
    pub name: String,
    pub milestones: Vec<MilestoneMeta>,
    #[serde(default)]
    pub snapshot: Option<GrantSnapshot>,
}

pub fn grant_id(reviewer: &[u8; 28], proposer: &[u8; 28], total_locked: i64, tranche_bps: &[i64]) -> String {
    let mut hasher = Hasher::<256>::new();
    hasher.input(reviewer);
    hasher.input(proposer);
    hasher.input(&total_locked.to_be_bytes());
    for bps in tranche_bps {
        hasher.input(&bps.to_be_bytes());
    }
    hasher.finalize().to_string()
}

pub struct GrantMetaStore {
    path: PathBuf,
    entries: Mutex<HashMap<String, GrantMeta>>,
}

impl GrantMetaStore {
    pub fn load(path: PathBuf) -> Result<Self> {
        let entries = if path.exists() {
            let bytes = std::fs::read(&path).with_context(|| format!("failed to read {}", path.display()))?;
            serde_json::from_slice(&bytes).with_context(|| format!("failed to parse {} as JSON", path.display()))?
        } else {
            HashMap::new()
        };

        Ok(Self {
            path,
            entries: Mutex::new(entries),
        })
    }

    pub fn get(&self, grant_id: &str) -> Option<GrantMeta> {
        self.entries.lock().unwrap().get(grant_id).cloned()
    }

    /// Locked once `snapshot` is attached (grant confirmed live on-chain) -- this endpoint has no
    /// auth, so that's the point past which an overwrite would mean spoofing a real grant's
    /// metadata. Before confirmation, retries with the same inputs may still overwrite.
    pub fn set_meta(&self, grant_id: String, name: String, milestones: Vec<MilestoneMeta>) -> Result<bool> {
        let mut entries = self.entries.lock().unwrap();
        if entries.get(&grant_id).is_some_and(|m| m.snapshot.is_some()) {
            return Ok(false);
        }
        entries.insert(grant_id, GrantMeta { name, milestones, snapshot: None });
        self.flush(&entries)?;
        Ok(true)
    }

    pub fn record_snapshot(&self, grant_id: &str, snapshot: GrantSnapshot) -> Result<()> {
        let mut entries = self.entries.lock().unwrap();
        let Some(meta) = entries.get_mut(grant_id) else {
            return Ok(());
        };
        meta.snapshot = Some(snapshot);
        self.flush(&entries)
    }

    pub fn snapshot_by_outref(&self, tx_hash: &str, output_index: u64) -> Option<GrantSnapshot> {
        self.entries
            .lock()
            .unwrap()
            .values()
            .find_map(|meta| match &meta.snapshot {
                Some(s) if s.tx_hash == tx_hash && s.output_index == output_index => Some(s.clone()),
                _ => None,
            })
    }

    pub fn completed(&self) -> Vec<(String, GrantMeta)> {
        self.entries
            .lock()
            .unwrap()
            .iter()
            .filter(|(_, meta)| {
                meta.snapshot
                    .as_ref()
                    .is_some_and(|s| s.datum.released_count as usize == s.datum.tranche_bps.len())
            })
            .map(|(id, meta)| (id.clone(), meta.clone()))
            .collect()
    }

    fn flush(&self, entries: &HashMap<String, GrantMeta>) -> Result<()> {
        let json = serde_json::to_vec_pretty(entries).context("failed to serialize grants metadata")?;
        std::fs::write(&self.path, json).with_context(|| format!("failed to write {}", self.path.display()))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn grant_id_is_deterministic() {
        let reviewer = [0x11; 28];
        let proposer = [0x22; 28];
        let id1 = grant_id(&reviewer, &proposer, 100_000_000, &[4000, 3000, 3000]);
        let id2 = grant_id(&reviewer, &proposer, 100_000_000, &[4000, 3000, 3000]);
        assert_eq!(id1, id2);
    }

    #[test]
    fn grant_id_differs_on_tranche_split() {
        let reviewer = [0x11; 28];
        let proposer = [0x22; 28];
        let id1 = grant_id(&reviewer, &proposer, 100_000_000, &[4000, 3000, 3000]);
        let id2 = grant_id(&reviewer, &proposer, 100_000_000, &[5000, 2500, 2500]);
        assert_ne!(id1, id2);
    }

    #[test]
    fn store_round_trips_through_disk() {
        let dir = std::env::temp_dir().join(format!("grants_meta_test_{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("grants_meta.json");

        let store = GrantMetaStore::load(path.clone()).unwrap();
        let milestones = vec![
            MilestoneMeta { name: "M1".to_string(), description: "d1".to_string() },
            MilestoneMeta { name: "M2".to_string(), description: "d2".to_string() },
            MilestoneMeta { name: "M3".to_string(), description: "d3".to_string() },
        ];
        store.set_meta("abc123".to_string(), "Test Grant".to_string(), milestones).unwrap();

        let reloaded = GrantMetaStore::load(path).unwrap();
        let got = reloaded.get("abc123").expect("entry should have persisted");
        assert_eq!(got.name, "Test Grant");
        assert_eq!(got.milestones.len(), 3);

        std::fs::remove_dir_all(&dir).ok();
    }

    fn sample_datum(released_count: i64) -> Datum {
        Datum {
            reviewer: [0x11; 28],
            proposer: [0x22; 28],
            total_locked: 100_000_000,
            tranche_bps: vec![4000, 3000, 3000],
            approved: vec![true, true, true],
            released_count,
            receipt_policy_id: [0x33; 28],
            review_deadline: None,
        }
    }

    #[test]
    fn record_snapshot_is_noop_without_existing_meta() {
        let dir = std::env::temp_dir().join(format!("grants_meta_test_noop_{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let store = GrantMetaStore::load(dir.join("grants_meta.json")).unwrap();

        store
            .record_snapshot(
                "no-meta",
                GrantSnapshot { tx_hash: "tx1".to_string(), output_index: 0, lovelace: 0, datum: sample_datum(1) },
            )
            .unwrap();

        assert!(store.get("no-meta").is_none());
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn completed_grant_survives_utxo_being_spent() {
        let dir = std::env::temp_dir().join(format!("grants_meta_test_completed_{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let store = GrantMetaStore::load(dir.join("grants_meta.json")).unwrap();

        store.set_meta("g1".to_string(), "Grant One".to_string(), vec![]).unwrap();
        store
            .record_snapshot(
                "g1",
                GrantSnapshot { tx_hash: "tx-mid".to_string(), output_index: 0, lovelace: 40_000_000, datum: sample_datum(2) },
            )
            .unwrap();
        assert!(store.completed().is_empty(), "not fully released yet, should not be reported completed");

        store
            .record_snapshot(
                "g1",
                GrantSnapshot { tx_hash: "tx-final".to_string(), output_index: 0, lovelace: 0, datum: sample_datum(3) },
            )
            .unwrap();

        let completed = store.completed();
        assert_eq!(completed.len(), 1);
        assert_eq!(completed[0].0, "g1");

        let by_outref = store.snapshot_by_outref("tx-final", 0).expect("snapshot should be findable by outref");
        assert_eq!(by_outref.datum.released_count, 3);

        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn set_meta_allows_overwrite_before_grant_is_confirmed_on_chain() {
        let dir = std::env::temp_dir().join(format!("grants_meta_test_unconfirmed_{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let store = GrantMetaStore::load(dir.join("grants_meta.json")).unwrap();

        let created = store.set_meta("g1".to_string(), "Original".to_string(), vec![]).unwrap();
        assert!(created, "first write should be accepted");

        let overwritten = store.set_meta("g1".to_string(), "Retry After Failed Signing".to_string(), vec![]).unwrap();
        assert!(overwritten, "no on-chain snapshot yet -- retry must be allowed to overwrite");
        assert_eq!(store.get("g1").unwrap().name, "Retry After Failed Signing");

        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn set_meta_rejects_overwrite_once_grant_is_confirmed_on_chain() {
        let dir = std::env::temp_dir().join(format!("grants_meta_test_confirmed_{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let store = GrantMetaStore::load(dir.join("grants_meta.json")).unwrap();

        store.set_meta("g1".to_string(), "Real Grant".to_string(), vec![]).unwrap();
        store
            .record_snapshot(
                "g1",
                GrantSnapshot { tx_hash: "tx1".to_string(), output_index: 0, lovelace: 100_000_000, datum: sample_datum(0) },
            )
            .unwrap();

        let spoofed = store.set_meta("g1".to_string(), "Spoofed Name".to_string(), vec![]).unwrap();
        assert!(!spoofed, "grant is confirmed live on-chain -- an unauthenticated caller must not be able to overwrite its metadata");
        assert_eq!(store.get("g1").unwrap().name, "Real Grant", "original metadata must survive the rejected overwrite attempt");

        std::fs::remove_dir_all(&dir).ok();
    }
}
