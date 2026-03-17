use anyhow::Result;
use shared_protocol::{load_toml_config_or_default, ServerConfig};
use tracing::info;

#[tokio::main]
async fn main() -> Result<()> {
    tracing_subscriber::fmt::init();
    let config = load_config()?;
    let payload = serde_json::to_string(&config)?;
    info!(config = %payload, "starting nexus server");
    server::run(config).await
}

fn load_config() -> Result<ServerConfig> {
    Ok(load_toml_config_or_default("config/default.toml")?)
}
