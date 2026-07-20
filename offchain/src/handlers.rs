//! Axum router and HTTP handlers for grants, roles, transactions, and tx-building endpoints.

use std::collections::HashMap;
use std::sync::Arc;

use axum::{extract::{Query, State}, http::{HeaderValue, Method}, routing::{get, post}, Json, Router};
use serde::Deserialize;
use serde_json::json;
use tower_http::cors::CorsLayer;

use crate::address::payment_key_hash;
use crate::blockfrost_client::{find_by_outref, BlockfrostClient};
use crate::config::Config;
use crate::datum::Datum;
use crate::error::{AppError, AppResult};
use crate::grants_meta::{grant_id, GrantMetaStore, GrantSnapshot, MilestoneMeta};
use crate::tx::{approve, create_escrow, release};

#[derive(Clone)]
pub struct AppState {
    pub config: Arc<Config>,
    pub client: Arc<BlockfrostClient>,
    pub grants_meta: Arc<GrantMetaStore>,
}

pub fn router(state: AppState) -> Router {
    let frontend_origin: HeaderValue = state
        .config
        .frontend_origin
        .parse()
        .expect("FRONTEND_ORIGIN must be a valid header value (e.g. http://localhost:4321)");

    let cors = CorsLayer::new()
        .allow_origin(frontend_origin)
        .allow_methods([Method::GET, Method::POST])
        .allow_headers([axum::http::header::CONTENT_TYPE]);

    Router::new()
        .route("/grants", get(get_grants).post(post_grant_meta))
        .route("/grants/role", get(get_grant_role))
        .route("/transactions", get(get_transactions))
        .route("/tx/create-escrow", post(post_create_escrow))
        .route("/tx/approve-milestone", post(post_approve_milestone))
        .route("/tx/release-tranche", post(post_release_tranche))
        .layer(cors)
        .with_state(state)
}

async fn get_grants(State(state): State<AppState>) -> AppResult<Json<serde_json::Value>> {
    let grants = state.client.list_grants(&state.config.escrow_address).await?;

    let ids: Vec<String> = grants
        .iter()
        .map(|g| grant_id(&g.datum.reviewer, &g.datum.proposer, g.datum.total_locked, &g.datum.tranche_bps))
        .collect();
    let mut id_counts: HashMap<&str, usize> = HashMap::new();
    for id in &ids {
        *id_counts.entry(id.as_str()).or_insert(0) += 1;
    }

    let mut out: Vec<serde_json::Value> = grants
        .iter()
        .zip(ids.iter())
        .filter(|(_, id)| !state.config.ignored_grant_ids.contains(id.as_str()))
        .map(|(g, id)| {
            let mut warnings: Vec<String> = Vec::new();

            let receipt_policy_verified = hex::encode(g.datum.receipt_policy_id) == state.config.receipt_policy_id;
            if !receipt_policy_verified {
                warnings.push(
                    "datum's receipt_policy_id does not match this deployment's real receipt policy -- likely spoofed"
                        .to_string(),
                );
            }

            let id_collision = id_counts.get(id.as_str()).copied().unwrap_or(0) > 1;
            if id_collision {
                warnings.push(
                    "grant_id shared with another live UTxO at this address -- cannot tell which is genuine"
                        .to_string(),
                );
            }

            let trusted = receipt_policy_verified && !id_collision;
            let meta = if trusted { state.grants_meta.get(id) } else { None };

            if trusted {
                let _ = state.grants_meta.record_snapshot(
                    id,
                    GrantSnapshot {
                        tx_hash: g.tx_hash.clone(),
                        output_index: g.output_index,
                        lovelace: g.lovelace,
                        datum: g.datum.clone(),
                    },
                );
            }

            json!({
                "grant_id": id,
                "tx_hash": g.tx_hash,
                "output_index": g.output_index,
                "lovelace": g.lovelace,
                "datum": g.datum.to_json(),
                "name": meta.as_ref().map(|m| m.name.clone()).unwrap_or_else(|| format!("Unnamed grant ({})", &id[..8])),
                "milestones": meta.map(|m| m.milestones),
                "trusted": trusted,
                "warnings": warnings,
                "completed": false,
            })
        })
        .collect();

    let live_ids: std::collections::HashSet<&str> = ids.iter().map(|s| s.as_str()).collect();
    for (id, meta) in state.grants_meta.completed() {
        if live_ids.contains(id.as_str()) || state.config.ignored_grant_ids.contains(&id) {
            continue;
        }
        let Some(snapshot) = meta.snapshot else { continue };
        out.push(json!({
            "grant_id": id,
            "tx_hash": snapshot.tx_hash,
            "output_index": snapshot.output_index,
            "lovelace": 0,
            "datum": snapshot.datum.to_json(),
            "name": meta.name,
            "milestones": meta.milestones,
            "trusted": true,
            "warnings": [],
            "completed": true,
        }));
    }

    Ok(Json(json!(out)))
}

#[derive(Debug, Deserialize)]
struct CreateGrantMetaRequest {
    proposer_address: String,
    reviewer_address: String,
    tranche_bps: Vec<i64>,
    total_locked: u64,
    name: String,
    milestones: Vec<MilestoneMeta>,
}

async fn post_grant_meta(
    State(state): State<AppState>,
    Json(req): Json<CreateGrantMetaRequest>,
) -> AppResult<Json<serde_json::Value>> {
    let reviewer = payment_key_hash(&req.reviewer_address).map_err(|e| AppError::BadRequest(e.to_string()))?;
    let proposer = payment_key_hash(&req.proposer_address).map_err(|e| AppError::BadRequest(e.to_string()))?;

    let id = grant_id(&reviewer, &proposer, req.total_locked as i64, &req.tranche_bps);
    state.grants_meta.set_meta(id.clone(), req.name, req.milestones)?;

    Ok(Json(json!({ "grant_id": id })))
}

#[derive(Debug, Deserialize)]
struct GrantRoleQuery {
    address: String,
    tx_hash: String,
    output_index: u64,
}

async fn get_grant_role(State(state): State<AppState>, Query(q): Query<GrantRoleQuery>) -> AppResult<Json<serde_json::Value>> {
    let hash = payment_key_hash(&q.address).map_err(|e| AppError::BadRequest(e.to_string()))?;
    let grants = state.client.list_grants(&state.config.escrow_address).await?;

    let datum: Datum = match find_by_outref(&grants, &q.tx_hash, q.output_index) {
        Some(grant) => grant.datum.clone(),
        None => state
            .grants_meta
            .snapshot_by_outref(&q.tx_hash, q.output_index)
            .map(|s| s.datum)
            .ok_or_else(|| {
                AppError::NotFound(
                    "grant not found at that UTxO -- it may have already been spent, refresh and retry".to_string(),
                )
            })?,
    };

    let role = if hash == datum.reviewer {
        "funder"
    } else if hash == datum.proposer {
        "grantee"
    } else {
        "none"
    };

    Ok(Json(json!({ "role": role })))
}

#[derive(Debug, Deserialize)]
struct GrantTransactionsQuery {
    tx_hash: String,
    output_index: u64,
}

async fn get_transactions(
    State(state): State<AppState>,
    Query(q): Query<GrantTransactionsQuery>,
) -> AppResult<Json<serde_json::Value>> {
    let grants = state.client.list_grants(&state.config.escrow_address).await?;

    let datum: Datum = match find_by_outref(&grants, &q.tx_hash, q.output_index) {
        Some(grant) => grant.datum.clone(),
        None => state
            .grants_meta
            .snapshot_by_outref(&q.tx_hash, q.output_index)
            .map(|s| s.datum)
            .ok_or_else(|| {
                AppError::NotFound(
                    "grant not found at that UTxO -- it may have already been spent, refresh and retry".to_string(),
                )
            })?,
    };

    let txs = state
        .client
        .transactions_for_grant(&state.config.escrow_address, &datum, 10, 100)
        .await?;
    let out: Vec<serde_json::Value> = txs
        .into_iter()
        .map(|t| json!({ "tx_hash": t.tx_hash, "block_time": t.block_time, "block_height": t.block_height }))
        .collect();
    Ok(Json(json!(out)))
}

async fn post_create_escrow(
    State(state): State<AppState>,
    Json(req): Json<create_escrow::CreateEscrowRequest>,
) -> AppResult<Json<serde_json::Value>> {
    let params = state.client.protocol_params().await?;
    let receipt_policy_id: [u8; 28] = hex::decode(&state.config.receipt_policy_id)
        .map_err(|e| anyhow::anyhow!("invalid receipt_policy_id in config: {e}"))?
        .try_into()
        .map_err(|_| anyhow::anyhow!("receipt_policy_id in config is not 28 bytes"))?;

    let cbor = create_escrow::build(
        &req,
        &state.config.escrow_address,
        &receipt_policy_id,
        params.min_fee_a,
        params.min_fee_b,
    )?;

    Ok(Json(json!({ "unsigned_tx_cbor": hex::encode(cbor) })))
}

async fn post_approve_milestone(
    State(state): State<AppState>,
    Json(req): Json<approve::ApproveMilestoneRequest>,
) -> AppResult<Json<serde_json::Value>> {
    let grants = state.client.list_grants(&state.config.escrow_address).await?;
    let escrow = find_by_outref(&grants, &req.tx_hash, req.output_index).ok_or_else(|| {
        AppError::NotFound("grant not found at that UTxO -- it may have already been spent, refresh and retry".to_string())
    })?;

    let cbor = approve::build(&req, &state.config, &state.client, escrow).await?;
    Ok(Json(json!({ "unsigned_tx_cbor": hex::encode(cbor) })))
}

async fn post_release_tranche(
    State(state): State<AppState>,
    Json(req): Json<release::ReleaseTrancheRequest>,
) -> AppResult<Json<serde_json::Value>> {
    let grants = state.client.list_grants(&state.config.escrow_address).await?;
    let escrow = find_by_outref(&grants, &req.tx_hash, req.output_index).ok_or_else(|| {
        AppError::NotFound("grant not found at that UTxO -- it may have already been spent, refresh and retry".to_string())
    })?;

    let cbor = release::build(&req, &state.config, &state.client, escrow).await?;
    Ok(Json(json!({ "unsigned_tx_cbor": hex::encode(cbor) })))
}
