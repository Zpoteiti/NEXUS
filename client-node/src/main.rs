use anyhow::Result;
use futures_util::{SinkExt, StreamExt};
use shared_protocol::{
    load_toml_config_or_default, ClientNodeConfig, ClientToServer, NodeHello, NodeRegistration,
    ServerToClient, ToolKind, ToolResult,
};
use tokio::process::Command;
use tokio_tungstenite::connect_async;
use tracing::info;

#[tokio::main]
async fn main() -> Result<()> {
    tracing_subscriber::fmt::init();
    let config = load_config()?;
    let registration = NodeRegistration::new(
        config.node_id.clone(),
        config.tenant_id.clone(),
        config.user_id.clone(),
        config.auth_token.clone(),
    );
    let hello = ClientToServer::Hello(NodeHello {
        registration,
        custom_tools: vec!["weather.lookup".to_owned()],
    });
    let (stream, _) = connect_async(config.server_endpoint.clone()).await?;
    info!(endpoint = %config.server_endpoint, "client connected");
    let (mut writer, mut reader) = stream.split();
    writer
        .send(tokio_tungstenite::tungstenite::Message::Text(serde_json::to_string(&hello)?.into()))
        .await?;
    while let Some(message) = reader.next().await {
        let message = message?;
        if let tokio_tungstenite::tungstenite::Message::Text(text) = message {
            let payload: ServerToClient = serde_json::from_str(&text)?;
            match payload {
                ServerToClient::Ack { node_id } => {
                    info!(node_id = %node_id, "node acked");
                }
                ServerToClient::Ping => {
                    let pong = ClientToServer::Pong { node_id: config.node_id.clone() };
                    writer
                        .send(tokio_tungstenite::tungstenite::Message::Text(
                            serde_json::to_string(&pong)?.into(),
                        ))
                        .await?;
                }
                ServerToClient::ToolRequest(request) => {
                    let output = execute_tool(request.tool.clone(), &request.command).await;
                    let result = ClientToServer::ToolResult(ToolResult {
                        request_id: request.request_id,
                        ok: output.is_ok(),
                        output: output.unwrap_or_else(|e| e.to_string()),
                    });
                    writer
                        .send(tokio_tungstenite::tungstenite::Message::Text(
                            serde_json::to_string(&result)?.into(),
                        ))
                        .await?;
                }
            }
        }
    }
    Ok(())
}

fn load_config() -> Result<ClientNodeConfig> {
    Ok(load_toml_config_or_default("config/default.toml")?)
}

async fn execute_tool(tool: ToolKind, command: &str) -> Result<String> {
    match tool {
        ToolKind::Shell => {
            let output = Command::new("powershell")
                .arg("-NoProfile")
                .arg("-Command")
                .arg(command)
                .output()
                .await?;
            Ok(String::from_utf8_lossy(&output.stdout).trim().to_owned())
        }
        ToolKind::Filesystem => {
            if let Some(path) = command.strip_prefix("read:") {
                let text = tokio::fs::read_to_string(path).await?;
                return Ok(text);
            }
            if let Some(path) = command.strip_prefix("list:") {
                let mut rd = tokio::fs::read_dir(path).await?;
                let mut entries = Vec::new();
                while let Some(entry) = rd.next_entry().await? {
                    entries.push(entry.file_name().to_string_lossy().to_string());
                }
                entries.sort();
                return Ok(entries.join("\n"));
            }
            Ok("filesystem command unsupported".to_owned())
        }
        ToolKind::Browser => Ok(format!("browser dispatch completed: {}", command)),
        ToolKind::Calculator => eval_calc(command),
        ToolKind::CustomMcp => Ok(format!("custom mcp result: {}", command)),
    }
}

fn eval_calc(command: &str) -> Result<String> {
    let parts = command.split_whitespace().collect::<Vec<_>>();
    if parts.len() != 3 {
        anyhow::bail!("calculator format is `<number> <op> <number>`");
    }
    let left = parts[0].parse::<f64>()?;
    let right = parts[2].parse::<f64>()?;
    let value = match parts[1] {
        "+" => left + right,
        "-" => left - right,
        "*" => left * right,
        "/" => left / right,
        _ => anyhow::bail!("unsupported operator"),
    };
    Ok(value.to_string())
}
