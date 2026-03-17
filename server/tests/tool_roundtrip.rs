use std::env;
use std::net::TcpListener;
use std::time::Duration;

use futures_util::{SinkExt, StreamExt};
use reqwest::StatusCode;
use serde_json::json;
use shared_protocol::{
    AuthConfig, ClientToServer, NodeHello, NodeRegistration, RuntimeLimits, ServerConfig,
    ServerToClient, ToolResult,
};

#[tokio::test]
async fn tool_roundtrip_returns_client_result() {
    let Some(dsn) = env::var("NEXUS_TEST_POSTGRES_DSN").ok() else {
        return;
    };
    let port = next_port();
    let address = format!("127.0.0.1:{port}");
    let endpoint = format!("ws://{address}/ws");
    let base = format!("http://{address}");
    let config = ServerConfig {
        bind_addr: address.clone(),
        postgres_dsn: dsn,
        vlm_endpoint: "http://127.0.0.1:1/health".to_owned(),
        limits: RuntimeLimits {
            max_connections: 600,
            request_timeout_ms: 5_000,
            max_inflight_requests: 3_000,
        },
        auth: AuthConfig {
            token_issuer: "nexus".to_owned(),
            audience: "nexus-nodes".to_owned(),
            node_auth_token: "dev-token".to_owned(),
            admin_username: "admin".to_owned(),
            admin_password: "admin".to_owned(),
        },
    };

    let server_task = tokio::spawn(async move {
        let _ = server::run(config).await;
    });

    tokio::time::sleep(Duration::from_millis(200)).await;

    let node_task = tokio::spawn(async move {
        let (ws, _) = tokio_tungstenite::connect_async(endpoint).await.expect("connect");
        let (mut write, mut read) = ws.split();
        let hello = ClientToServer::Hello(NodeHello {
            registration: NodeRegistration::new("node-a", "tenant-a", "user-a", "dev-token"),
            custom_tools: vec!["weather.lookup".to_owned()],
        });
        write
            .send(tokio_tungstenite::tungstenite::Message::Text(
                serde_json::to_string(&hello).expect("serialize").into(),
            ))
            .await
            .expect("send hello");

        while let Some(message) = read.next().await {
            let message = message.expect("message");
            if let tokio_tungstenite::tungstenite::Message::Text(text) = message {
                let payload: ServerToClient = serde_json::from_str(&text).expect("decode");
                match payload {
                    ServerToClient::Ack { .. } => {}
                    ServerToClient::Ping => {
                        let pong = ClientToServer::Pong { node_id: "node-a".to_owned() };
                        write
                            .send(tokio_tungstenite::tungstenite::Message::Text(
                                serde_json::to_string(&pong).expect("serialize").into(),
                            ))
                            .await
                            .expect("pong");
                    }
                    ServerToClient::ToolRequest(request) => {
                        let result = ClientToServer::ToolResult(ToolResult {
                            request_id: request.request_id,
                            ok: true,
                            output: "tool-ok".to_owned(),
                        });
                        write
                            .send(tokio_tungstenite::tungstenite::Message::Text(
                                serde_json::to_string(&result).expect("serialize").into(),
                            ))
                            .await
                            .expect("result");
                        break;
                    }
                }
            }
        }
    });

    tokio::time::sleep(Duration::from_millis(200)).await;
    let client = reqwest::Client::new();
    let response = client
        .post(format!("{base}/rpc/tool"))
        .header("authorization", "Basic admin:admin")
        .json(&json!({
            "request_id": "req-1",
            "tenant_id": "tenant-a",
            "user_id": "user-a",
            "node_id": "node-a",
            "tool": "Shell",
            "command": "echo hello",
            "input_tokens": 4,
            "output_tokens": 2,
            "model": "test-model"
        }))
        .send()
        .await
        .expect("rpc request");
    assert_eq!(response.status(), StatusCode::OK);
    let body = response.text().await.expect("body");
    assert!(body.contains("tool-ok"));

    node_task.await.expect("node task");
    server_task.abort();
}

fn next_port() -> u16 {
    let listener = TcpListener::bind("127.0.0.1:0").expect("bind local port");
    listener.local_addr().expect("addr").port()
}
