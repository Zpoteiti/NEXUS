use anyhow::Result;
use shared_protocol::ServerConfig;
use tracing::info;

#[tokio::main]
async fn main() -> Result<()> {
    tracing_subscriber::fmt::init();
    let config = load_config()?;
    let payload = serde_json::to_string(&config)?;
    info!(config = %payload, "starting rustnano server");
    server::run(config).await
}

fn load_config() -> Result<ServerConfig> {
    let path = std::path::Path::new("config/default.toml");
    if !path.exists() {
        return Ok(ServerConfig::default());
    }
    let raw = std::fs::read_to_string(path)?;
    Ok(toml::from_str::<ServerConfig>(&raw)?)
}
