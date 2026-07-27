//! Loads runtime config from environment variables plus the
//! deploy-specific `params.json`/`plutus.applied.json` under `DEPLOY_DIR`
//! (default `onchain/deploy/testnet-v4`) -- see `onchain/deploy/README.md`.

use std::collections::HashSet;
use std::path::PathBuf;

use anyhow::{Context, Result};

pub struct Config {
    pub blockfrost_project_id: String,
    pub bind_addr: String,
    pub frontend_origin: String,

    pub escrow_address: String,
    pub receipt_policy_id: String,

    pub escrow_compiled_code: String,
    pub receipt_compiled_code: String,

    pub grants_meta_path: PathBuf,

    pub ignored_grant_ids: HashSet<String>,
}

impl Config {
    pub fn load() -> Result<Self> {
        dotenvy::dotenv().ok();

        let blockfrost_project_id = std::env::var("BLOCKFROST_PROJECT_ID")
            .context("BLOCKFROST_PROJECT_ID must be set (see .env.example)")?;
        let bind_addr =
            std::env::var("BIND_ADDR").unwrap_or_else(|_| "127.0.0.1:3000".to_string());
        let frontend_origin = std::env::var("FRONTEND_ORIGIN")
            .unwrap_or_else(|_| "http://localhost:4321".to_string());

        let deploy_dir = std::env::var("DEPLOY_DIR")
            .unwrap_or_else(|_| "../onchain/deploy/testnet-v4".to_string());

        let deploy_dir = PathBuf::from(deploy_dir);
        let params_path = deploy_dir.join("params.json");
        let applied_blueprint_path = deploy_dir.join("plutus.applied.json");

        let params: serde_json::Value = read_json(&params_path)?;
        let escrow_address = require_str(&params, "escrow_address", &params_path)?;
        let receipt_policy_id = require_str(&params, "receipt_policy_id", &params_path)?;

        let applied_blueprint: serde_json::Value = read_json(&applied_blueprint_path)?;
        let escrow_compiled_code = compiled_code_for(
            &applied_blueprint,
            "milestone_escrow.milestone_escrow.spend",
            &applied_blueprint_path,
        )?;
        let receipt_compiled_code = compiled_code_for(
            &applied_blueprint,
            "milestone_receipt.milestone_receipt.mint",
            &applied_blueprint_path,
        )?;

        let grants_meta_path = PathBuf::from(
            std::env::var("GRANTS_META_PATH").unwrap_or_else(|_| "grants_meta.json".to_string()),
        );

        let ignored_grant_ids = std::env::var("IGNORED_GRANT_IDS")
            .unwrap_or_default()
            .split(',')
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .map(str::to_string)
            .collect();

        Ok(Self {
            blockfrost_project_id,
            bind_addr,
            frontend_origin,
            escrow_address,
            receipt_policy_id,
            escrow_compiled_code,
            receipt_compiled_code,
            grants_meta_path,
            ignored_grant_ids,
        })
    }
}

fn read_json(path: impl AsRef<std::path::Path>) -> Result<serde_json::Value> {
    let path = path.as_ref();
    let bytes = std::fs::read(path)
        .with_context(|| format!("failed to read {}", path.display()))?;
    serde_json::from_slice(&bytes)
        .with_context(|| format!("failed to parse {} as JSON", path.display()))
}

fn require_str(value: &serde_json::Value, key: &str, source: impl AsRef<std::path::Path>) -> Result<String> {
    value
        .get(key)
        .and_then(|v| v.as_str())
        .map(str::to_string)
        .with_context(|| format!("missing \"{key}\" in {}", source.as_ref().display()))
}

fn compiled_code_for(
    blueprint: &serde_json::Value,
    title: &str,
    source: impl AsRef<std::path::Path>,
) -> Result<String> {
    blueprint
        .get("validators")
        .and_then(|v| v.as_array())
        .and_then(|validators| validators.iter().find(|v| v.get("title").and_then(|t| t.as_str()) == Some(title)))
        .and_then(|v| v.get("compiledCode"))
        .and_then(|c| c.as_str())
        .map(str::to_string)
        .with_context(|| format!("validator \"{title}\" not found in {}", source.as_ref().display()))
}
