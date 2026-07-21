mod address;
mod blockfrost_client;
mod config;
mod datum;
mod error;
mod fees;
mod grants_meta;
mod handlers;
mod tx;

use std::sync::Arc;

use blockfrost_client::BlockfrostClient;
use config::Config;
use grants_meta::GrantMetaStore;
use handlers::AppState;

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    tracing_subscriber::fmt::init();

    let config = Config::load()?;
    let bind_addr = config.bind_addr.clone();
    let client = BlockfrostClient::new(config.blockfrost_project_id.clone());
    let grants_meta = GrantMetaStore::load(config.grants_meta_path.clone())?;

    let state = AppState {
        config: Arc::new(config),
        client: Arc::new(client),
        grants_meta: Arc::new(grants_meta),
    };

    let app = handlers::router(state);
    let listener = tokio::net::TcpListener::bind(&bind_addr).await?;
    tracing::info!("listening on {bind_addr}");
    axum::serve(listener, app).await?;

    Ok(())
}
