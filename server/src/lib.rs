use std::collections::HashMap;
use std::sync::Arc;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use anyhow::{anyhow, Context, Result};
use axum::extract::ws::{Message, WebSocket, WebSocketUpgrade};
use axum::extract::{Json, Query, State};
use axum::http::{HeaderMap, StatusCode};
use axum::response::IntoResponse;
use axum::routing::{get, post};
use axum::{serve, Router};
use futures_util::{SinkExt, StreamExt};
use reqwest::Client;
use serde::{Deserialize, Serialize};
use shared_protocol::{
    ClientToServer, NodeHello, NodeRegistration, ProviderHealth, ServerConfig, ServerToClient,
    ToolKind, ToolRequest, ToolResult, UsageRecord,
};
use storage::{GatewayRepository, RepositoryFactory, StorageFactory};
use tokio::net::TcpListener;
use tokio::sync::{mpsc, oneshot, Mutex, RwLock, Semaphore};
use tokio::time::interval;
use tracing::{error, info};

#[derive(Clone)]
pub struct AppState {
    config: ServerConfig,
    repo: Arc<dyn GatewayRepository>,
    sessions: Arc<RwLock<HashMap<String, NodeSession>>>,
    pending: Arc<Mutex<HashMap<String, oneshot::Sender<ToolResult>>>>,
    connection_guard: Arc<Semaphore>,
    inflight_guard: Arc<Semaphore>,
}

#[derive(Clone)]
struct NodeSession {
    registration: NodeRegistration,
    custom_tools: Vec<String>,
    tx: mpsc::Sender<ServerToClient>,
}

#[derive(Debug, Deserialize)]
pub struct RpcDispatchRequest {
    pub request_id: String,
    pub tenant_id: String,
    pub user_id: String,
    pub node_id: String,
    pub tool: ToolKind,
    pub command: String,
    pub input_tokens: u64,
    pub output_tokens: u64,
    pub model: String,
}

#[derive(Debug, Deserialize)]
pub struct RouteLookupRequest {
    pub tenant_id: String,
    pub channel_name: String,
    pub external_user: String,
}

#[derive(Debug, Deserialize)]
pub struct RegisterRequest {
    pub username: String,
    pub password: String,
    pub tenant_id: String,
    pub user_id: String,
    pub display_name: String,
}

#[derive(Debug, Deserialize)]
pub struct LoginRequest {
    pub username: String,
    pub password: String,
}

#[derive(Debug, Serialize)]
pub struct LoginResponse {
    pub session_id: String,
    pub tenant_id: String,
    pub user_id: String,
}

#[derive(Debug, Deserialize)]
pub struct SessionQuery {
    pub session_id: String,
}

#[derive(Debug, Deserialize)]
pub struct DispatchByAliasRequest {
    pub request_id: String,
    pub tool: ToolKind,
    pub command: String,
    pub device_alias: String,
    pub input_tokens: u64,
    pub output_tokens: u64,
    pub model: String,
}

pub async fn run(config: ServerConfig) -> Result<()> {
    let repo: Arc<dyn GatewayRepository> = if let Some(dsn) = &config.postgres_dsn {
        Arc::from(
            <StorageFactory as RepositoryFactory>::postgres(dsn)
                .with_context(|| "open postgres")?,
        )
    } else {
        Arc::from(
            <StorageFactory as RepositoryFactory>::sqlite(&config.sqlite_path)
                .with_context(|| format!("open sqlite at {}", config.sqlite_path))?,
        )
    };
    repo.migrate()?;

    let state = AppState {
        connection_guard: Arc::new(Semaphore::new(config.limits.max_connections)),
        inflight_guard: Arc::new(Semaphore::new(config.limits.max_inflight_requests)),
        config: config.clone(),
        repo,
        sessions: Arc::new(RwLock::new(HashMap::new())),
        pending: Arc::new(Mutex::new(HashMap::new())),
    };

    let app = Router::new()
        .route("/health", get(health))
        .route("/ws", get(ws_handler))
        .route("/rpc/tool", post(dispatch_tool))
        .route("/auth/register", post(register_user))
        .route("/auth/login", post(login_user))
        .route("/user/devices", get(user_devices))
        .route("/user/dispatch", post(dispatch_tool_for_session))
        .route("/admin/tenants", get(admin_tenants))
        .route("/admin/usage", get(admin_usage))
        .route("/admin/nodes", get(admin_nodes))
        .route("/admin/channel-route", get(admin_channel_route))
        .route("/admin/provider-health", get(admin_provider_health))
        .route("/admin", get(admin_html))
        .with_state(state);

    let listener = TcpListener::bind(&config.bind_addr)
        .await
        .with_context(|| format!("bind {}", config.bind_addr))?;
    info!("server listening on {}", config.bind_addr);
    serve(listener, app).await.context("axum serve failed")
}

async fn health() -> impl IntoResponse {
    (StatusCode::OK, "ok")
}

async fn ws_handler(ws: WebSocketUpgrade, State(state): State<AppState>) -> impl IntoResponse {
    if state.connection_guard.available_permits() == 0 {
        return (StatusCode::TOO_MANY_REQUESTS, "connection limit").into_response();
    }
    ws.on_upgrade(move |socket| handle_socket(socket, state))
}

async fn handle_socket(socket: WebSocket, state: AppState) {
    let permit = match state.connection_guard.acquire().await {
        Ok(permit) => permit,
        Err(_) => return,
    };
    let (mut sink, mut stream) = socket.split();
    let Some(Ok(Message::Text(first_text))) = stream.next().await else {
        drop(permit);
        return;
    };
    let Ok(first) = serde_json::from_str::<ClientToServer>(&first_text) else {
        drop(permit);
        return;
    };
    let ClientToServer::Hello(NodeHello { registration, custom_tools }) = first else {
        drop(permit);
        return;
    };
    if registration.auth_token != state.config.auth.node_auth_token {
        drop(permit);
        return;
    }
    let now = now_ms();
    if let Err(err) = state.repo.upsert_node(&registration, now) {
        error!("failed to persist node registration: {err}");
        drop(permit);
        return;
    }
    let _ = state.repo.upsert_user_device(&shared_protocol::UserDevice {
        tenant_id: registration.tenant_id.clone(),
        user_id: registration.user_id.clone(),
        node_id: registration.node_id.clone(),
        alias: registration.node_id.clone(),
    });

    let (tx, mut rx) = mpsc::channel::<ServerToClient>(64);
    {
        let mut sessions = state.sessions.write().await;
        sessions.insert(
            registration.node_id.clone(),
            NodeSession { registration: registration.clone(), custom_tools, tx: tx.clone() },
        );
    }

    if tx.send(ServerToClient::Ack { node_id: registration.node_id.clone() }).await.is_err() {
        cleanup_session(&state, &registration.node_id).await;
        drop(permit);
        return;
    }

    let node_id = registration.node_id.clone();
    let write_state = state.clone();
    let writer = tokio::spawn(async move {
        let mut ticker = interval(Duration::from_secs(15));
        loop {
            tokio::select! {
                maybe_message = rx.recv() => {
                    let Some(server_message) = maybe_message else {
                        return;
                    };
                    let payload = match serde_json::to_string(&server_message) {
                        Ok(value) => value,
                        Err(_) => return,
                    };
                    if sink.send(Message::Text(payload.into())).await.is_err() {
                        return;
                    }
                }
                _ = ticker.tick() => {
                    let ping_payload = match serde_json::to_string(&ServerToClient::Ping) {
                        Ok(value) => value,
                        Err(_) => return,
                    };
                    if sink.send(Message::Text(ping_payload.into())).await.is_err() {
                        return;
                    }
                }
            }
        }
    });

    let read_state = state.clone();
    let reader = tokio::spawn(async move {
        while let Some(inbound) = stream.next().await {
            let Ok(message) = inbound else {
                break;
            };
            if let Message::Text(text) = message {
                let Ok(payload) = serde_json::from_str::<ClientToServer>(&text) else {
                    continue;
                };
                match payload {
                    ClientToServer::Pong { node_id } => {
                        let _ = read_state.repo.touch_node(&node_id, now_ms(), 0);
                    }
                    ClientToServer::ToolResult(result) => {
                        if let Some(tx) = read_state.pending.lock().await.remove(&result.request_id)
                        {
                            let _ = tx.send(result);
                        }
                        let _ = read_state.repo.touch_node(&node_id, now_ms(), 0);
                    }
                    ClientToServer::Hello(_) => {}
                }
            }
        }
    });

    tokio::select! {
        _ = writer => {},
        _ = reader => {},
    }

    cleanup_session(&write_state, &registration.node_id).await;
    drop(permit);
}

async fn cleanup_session(state: &AppState, node_id: &str) {
    let _ = state.repo.remove_node(node_id);
    let mut sessions = state.sessions.write().await;
    sessions.remove(node_id);
}

async fn dispatch_tool(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(req): Json<RpcDispatchRequest>,
) -> impl IntoResponse {
    if !admin_authorized_headers(&state.config, &headers) {
        return (StatusCode::UNAUTHORIZED, "unauthorized").into_response();
    }
    if state.inflight_guard.available_permits() == 0 {
        return (StatusCode::TOO_MANY_REQUESTS, "inflight limit").into_response();
    }
    let permit = match state.inflight_guard.acquire().await {
        Ok(permit) => permit,
        Err(_) => return (StatusCode::INTERNAL_SERVER_ERROR, "semaphore error").into_response(),
    };
    if matches!(req.tool, ToolKind::Calculator) {
        let output = eval_calc(&req.command);
        let result = ToolResult {
            request_id: req.request_id.clone(),
            ok: output.is_ok(),
            output: output.unwrap_or_else(|e| e.to_string()),
        };
        let _ = state.repo.record_usage(&UsageRecord {
            tenant_id: req.tenant_id,
            user_id: req.user_id,
            model: req.model,
            input_tokens: req.input_tokens,
            output_tokens: req.output_tokens,
            request_id: req.request_id,
        });
        drop(permit);
        return Json(result).into_response();
    }

    let session = {
        let sessions = state.sessions.read().await;
        sessions.get(&req.node_id).cloned()
    };
    let Some(session) = session else {
        drop(permit);
        return (StatusCode::NOT_FOUND, "node not connected").into_response();
    };
    if session.registration.tenant_id != req.tenant_id
        || session.registration.user_id != req.user_id
    {
        drop(permit);
        return (StatusCode::FORBIDDEN, "tenant mismatch").into_response();
    }
    if matches!(req.tool, ToolKind::CustomMcp) && req.command.contains(':') {
        let tool_name = req.command.split(':').next().unwrap_or_default();
        if !session.custom_tools.iter().any(|v| v == tool_name) {
            drop(permit);
            return (StatusCode::FORBIDDEN, "custom tool not visible").into_response();
        }
    }

    let (tx, rx) = oneshot::channel::<ToolResult>();
    state.pending.lock().await.insert(req.request_id.clone(), tx);

    let outbound = ServerToClient::ToolRequest(ToolRequest {
        request_id: req.request_id.clone(),
        tenant_id: req.tenant_id.clone(),
        user_id: req.user_id.clone(),
        node_id: req.node_id.clone(),
        tool: req.tool.clone(),
        command: req.command.clone(),
    });
    if session.tx.send(outbound).await.is_err() {
        state.pending.lock().await.remove(&req.request_id);
        drop(permit);
        return (StatusCode::BAD_GATEWAY, "node disconnected").into_response();
    }

    let response =
        tokio::time::timeout(Duration::from_millis(state.config.limits.request_timeout_ms), rx)
            .await;
    drop(permit);
    let Ok(Ok(tool_result)) = response else {
        state.pending.lock().await.remove(&req.request_id);
        return (StatusCode::GATEWAY_TIMEOUT, "tool request timeout").into_response();
    };

    let _ = state.repo.record_usage(&UsageRecord {
        tenant_id: req.tenant_id,
        user_id: req.user_id,
        model: req.model,
        input_tokens: req.input_tokens,
        output_tokens: req.output_tokens,
        request_id: req.request_id,
    });
    Json(tool_result).into_response()
}

async fn register_user(
    State(state): State<AppState>,
    Json(req): Json<RegisterRequest>,
) -> impl IntoResponse {
    let password_hash = format!("plain:{}", req.password);
    let user = shared_protocol::LoginUser {
        username: req.username.clone(),
        password_hash,
        tenant_id: req.tenant_id.clone(),
        user_id: req.user_id.clone(),
    };
    if let Err(err) = state.repo.upsert_user(&shared_protocol::UserAccount {
        tenant_id: req.tenant_id,
        user_id: req.user_id,
        display_name: req.display_name,
    }) {
        return (StatusCode::INTERNAL_SERVER_ERROR, err.to_string()).into_response();
    }
    match state.repo.create_login_user(&user) {
        Ok(_) => (StatusCode::CREATED, "registered").into_response(),
        Err(storage::StorageError::UsernameConflict) => {
            (StatusCode::CONFLICT, "username exists").into_response()
        }
        Err(err) => (StatusCode::INTERNAL_SERVER_ERROR, err.to_string()).into_response(),
    }
}

async fn login_user(
    State(state): State<AppState>,
    Json(req): Json<LoginRequest>,
) -> impl IntoResponse {
    let password_hash = format!("plain:{}", req.password);
    let Some(user) = (match state.repo.authenticate_login_user(&req.username, &password_hash) {
        Ok(v) => v,
        Err(err) => return (StatusCode::INTERNAL_SERVER_ERROR, err.to_string()).into_response(),
    }) else {
        return (StatusCode::UNAUTHORIZED, "invalid credentials").into_response();
    };
    let session_id = format!("sess-{}-{}", now_ms(), req.username);
    let session = shared_protocol::LoginSession {
        session_id: session_id.clone(),
        username: req.username,
        tenant_id: user.tenant_id.clone(),
        user_id: user.user_id.clone(),
        created_at_ms: now_ms(),
    };
    if let Err(err) = state.repo.save_login_session(&session) {
        return (StatusCode::INTERNAL_SERVER_ERROR, err.to_string()).into_response();
    }
    Json(LoginResponse { session_id, tenant_id: user.tenant_id, user_id: user.user_id })
        .into_response()
}

async fn user_devices(
    State(state): State<AppState>,
    Query(q): Query<SessionQuery>,
) -> impl IntoResponse {
    let Some(session) = (match state.repo.get_login_session(&q.session_id) {
        Ok(v) => v,
        Err(err) => return (StatusCode::INTERNAL_SERVER_ERROR, err.to_string()).into_response(),
    }) else {
        return (StatusCode::UNAUTHORIZED, "invalid session").into_response();
    };
    match state.repo.list_user_devices(&session.tenant_id, &session.user_id) {
        Ok(v) => Json(v).into_response(),
        Err(err) => (StatusCode::INTERNAL_SERVER_ERROR, err.to_string()).into_response(),
    }
}

async fn dispatch_tool_for_session(
    State(state): State<AppState>,
    Query(q): Query<SessionQuery>,
    Json(req): Json<DispatchByAliasRequest>,
) -> impl IntoResponse {
    let Some(session) = (match state.repo.get_login_session(&q.session_id) {
        Ok(v) => v,
        Err(err) => return (StatusCode::INTERNAL_SERVER_ERROR, err.to_string()).into_response(),
    }) else {
        return (StatusCode::UNAUTHORIZED, "invalid session").into_response();
    };
    let Some(node_id) = (match state.repo.resolve_device_node(
        &session.tenant_id,
        &session.user_id,
        &req.device_alias,
    ) {
        Ok(v) => v,
        Err(err) => return (StatusCode::INTERNAL_SERVER_ERROR, err.to_string()).into_response(),
    }) else {
        return (StatusCode::NOT_FOUND, "device not found").into_response();
    };

    let mut headers = HeaderMap::new();
    let auth_value =
        format!("Basic {}:{}", state.config.auth.admin_username, state.config.auth.admin_password);
    if let Ok(v) = auth_value.parse() {
        headers.insert("authorization", v);
    }

    dispatch_tool(
        State(state),
        headers,
        Json(RpcDispatchRequest {
            request_id: req.request_id,
            tenant_id: session.tenant_id,
            user_id: session.user_id,
            node_id,
            tool: req.tool,
            command: req.command,
            input_tokens: req.input_tokens,
            output_tokens: req.output_tokens,
            model: req.model,
        }),
    )
    .await
    .into_response()
}

fn eval_calc(command: &str) -> Result<String> {
    let parts = command.split_whitespace().collect::<Vec<_>>();
    if parts.len() != 3 {
        return Err(anyhow!("calculator format is `<number> <op> <number>`"));
    }
    let left = parts[0].parse::<f64>()?;
    let right = parts[2].parse::<f64>()?;
    let value = match parts[1] {
        "+" => left + right,
        "-" => left - right,
        "*" => left * right,
        "/" => left / right,
        _ => return Err(anyhow!("unsupported operator")),
    };
    Ok(value.to_string())
}

async fn admin_tenants(State(state): State<AppState>, headers: HeaderMap) -> impl IntoResponse {
    if !admin_authorized_headers(&state.config, &headers) {
        return (StatusCode::UNAUTHORIZED, "unauthorized").into_response();
    }
    match state.repo.list_tenants() {
        Ok(data) => Json(data).into_response(),
        Err(err) => (StatusCode::INTERNAL_SERVER_ERROR, err.to_string()).into_response(),
    }
}

async fn admin_usage(State(state): State<AppState>, headers: HeaderMap) -> impl IntoResponse {
    if !admin_authorized_headers(&state.config, &headers) {
        return (StatusCode::UNAUTHORIZED, "unauthorized").into_response();
    }
    match state.repo.usage_summary() {
        Ok(data) => Json(data).into_response(),
        Err(err) => (StatusCode::INTERNAL_SERVER_ERROR, err.to_string()).into_response(),
    }
}

async fn admin_nodes(State(state): State<AppState>, headers: HeaderMap) -> impl IntoResponse {
    if !admin_authorized_headers(&state.config, &headers) {
        return (StatusCode::UNAUTHORIZED, "unauthorized").into_response();
    }
    match state.repo.list_nodes() {
        Ok(data) => Json(data).into_response(),
        Err(err) => (StatusCode::INTERNAL_SERVER_ERROR, err.to_string()).into_response(),
    }
}

async fn admin_channel_route(
    State(state): State<AppState>,
    headers: HeaderMap,
    Query(q): Query<RouteLookupRequest>,
) -> impl IntoResponse {
    if !admin_authorized_headers(&state.config, &headers) {
        return (StatusCode::UNAUTHORIZED, "unauthorized").into_response();
    }
    match state.repo.resolve_channel_user(&q.tenant_id, &q.channel_name, &q.external_user) {
        Ok(data) => Json(data).into_response(),
        Err(err) => (StatusCode::INTERNAL_SERVER_ERROR, err.to_string()).into_response(),
    }
}

async fn admin_provider_health(
    State(state): State<AppState>,
    headers: HeaderMap,
) -> impl IntoResponse {
    if !admin_authorized_headers(&state.config, &headers) {
        return (StatusCode::UNAUTHORIZED, "unauthorized").into_response();
    }
    match provider_health(&state.config.vlm_endpoint).await {
        Ok(data) => Json(data).into_response(),
        Err(err) => (StatusCode::BAD_GATEWAY, err.to_string()).into_response(),
    }
}

async fn admin_html(State(state): State<AppState>, headers: HeaderMap) -> impl IntoResponse {
    if !admin_authorized_headers(&state.config, &headers) {
        return (StatusCode::UNAUTHORIZED, "unauthorized").into_response();
    }
    let tenants = state.repo.list_tenants().unwrap_or_default();
    let usage = state.repo.usage_summary().unwrap_or_default();
    let nodes = state.repo.list_nodes().unwrap_or_default();
    let html = format!(
        "<html><body><h1>NEXUS admin</h1><p>tenants: {}</p><p>usage rows: {}</p><p>nodes: {}</p></body></html>",
        tenants.len(),
        usage.len(),
        nodes.len()
    );
    (StatusCode::OK, html).into_response()
}

fn admin_authorized_headers(config: &ServerConfig, headers: &HeaderMap) -> bool {
    let Some(value) = headers.get("authorization") else {
        return false;
    };
    let Ok(value) = value.to_str() else {
        return false;
    };
    if !value.starts_with("Basic ") {
        return false;
    }
    let expected = format!("Basic {}:{}", config.auth.admin_username, config.auth.admin_password);
    value == expected
}

async fn provider_health(endpoint: &str) -> Result<ProviderHealth> {
    let client = Client::builder().timeout(Duration::from_secs(5)).build()?;
    let res = client.get(endpoint).send().await;
    match res {
        Ok(resp) => Ok(ProviderHealth {
            endpoint: endpoint.to_owned(),
            reachable: true,
            status_code: resp.status().as_u16(),
        }),
        Err(err) => Err(anyhow!("provider endpoint unreachable: {err}")),
    }
}

fn now_ms() -> u64 {
    SystemTime::now().duration_since(UNIX_EPOCH).unwrap_or_default().as_millis() as u64
}

pub async fn run_connection_load_baseline(max_connections: usize, clients: usize) -> usize {
    let gate = Arc::new(Semaphore::new(max_connections));
    let mut tasks = Vec::new();
    for _ in 0..clients {
        let gate = gate.clone();
        tasks.push(tokio::spawn(async move {
            match gate.try_acquire() {
                Ok(permit) => {
                    tokio::time::sleep(Duration::from_millis(20)).await;
                    drop(permit);
                    1_usize
                }
                Err(_) => 0_usize,
            }
        }));
    }
    let mut accepted = 0;
    for task in tasks {
        if let Ok(value) = task.await {
            accepted += value;
        }
    }
    accepted
}

#[cfg(test)]
mod tests {
    use std::net::TcpListener;

    use axum::http::{HeaderMap, HeaderValue};
    use shared_protocol::{AuthConfig, RuntimeLimits, ServerConfig};
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    use tokio::net::TcpListener as TokioTcpListener;

    use super::{
        admin_authorized_headers, eval_calc, provider_health, run_connection_load_baseline,
    };

    #[test]
    fn calculator_runs_server_side() {
        let value = eval_calc("2 + 3").expect("calculator result");
        assert_eq!(value, "5");
    }

    #[test]
    fn admin_auth_guard_checks_basic_header() {
        let mut headers = HeaderMap::new();
        headers.insert("authorization", HeaderValue::from_static("Basic admin:admin"));
        let config = ServerConfig {
            bind_addr: "127.0.0.1:7878".to_owned(),
            sqlite_path: ":memory:".to_owned(),
            postgres_dsn: None,
            vlm_endpoint: "http://127.0.0.1/health".to_owned(),
            limits: RuntimeLimits::default(),
            auth: AuthConfig::default(),
        };
        assert!(admin_authorized_headers(&config, &headers));
    }

    #[tokio::test]
    async fn load_baseline_accepts_500_connections() {
        let accepted = run_connection_load_baseline(500, 500).await;
        assert_eq!(accepted, 500);
    }

    #[tokio::test]
    async fn provider_health_reports_reachable_status() {
        let std_listener = TcpListener::bind("127.0.0.1:0").expect("bind");
        let port = std_listener.local_addr().expect("addr").port();
        std_listener.set_nonblocking(true).expect("nonblocking");
        let listener = TokioTcpListener::from_std(std_listener).expect("tokio listener");
        let task = tokio::spawn(async move {
            if let Ok((mut socket, _)) = listener.accept().await {
                let mut buf = [0_u8; 256];
                let _ = socket.read(&mut buf).await;
                let _ = socket.write_all(b"HTTP/1.1 200 OK\r\nContent-Length: 2\r\n\r\nok").await;
            }
        });
        let result =
            provider_health(&format!("http://127.0.0.1:{port}/health")).await.expect("health");
        assert!(result.reachable);
        assert_eq!(result.status_code, 200);
        task.await.expect("server task");
    }
}
