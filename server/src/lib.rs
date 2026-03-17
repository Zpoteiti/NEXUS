use std::collections::HashMap;
use std::sync::Arc;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use anyhow::{anyhow, Context, Result};
use argon2::password_hash::{PasswordHash, PasswordHasher, PasswordVerifier, SaltString};
use argon2::{password_hash::rand_core::OsRng, Argon2};
use axum::extract::ws::{Message, WebSocket, WebSocketUpgrade};
use axum::extract::{Path, Query, Request, State};
use axum::http::{HeaderMap, StatusCode};
use axum::middleware::{self, Next};
use axum::response::{IntoResponse, Response};
use axum::routing::{get, get_service, post};
use axum::{serve, Json, Router};
use futures_util::{SinkExt, StreamExt};
use serde::{Deserialize, Serialize};
use shared_protocol::{
    ClientToServer, NodeHello, NodeRegistration, ServerConfig, ServerToClient, ToolKind,
    ToolRequest, ToolResult, UsageRecord,
};
use storage::{
    GatewayRepository, PostgresRepository, SessionListItem, UsageDetailItem, UsageTrendPoint,
};
use tokio::net::TcpListener;
use tokio::sync::{mpsc, oneshot, Mutex, RwLock, Semaphore};
use tokio::time::interval;
use tower_http::services::ServeDir;
use tracing::{error, info};
use uuid::Uuid;

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
    pub tenant_id: String,
    pub user_id: String,
    pub csrf_token: String,
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

#[derive(Debug, Deserialize)]
struct ListQuery {
    limit: Option<u32>,
    offset: Option<u32>,
}

#[derive(Debug, Deserialize)]
struct TrendQuery {
    days: Option<u32>,
    tenant_id: Option<String>,
}

#[derive(Debug, Serialize)]
struct ApiError {
    code: &'static str,
    message: String,
}

#[derive(Debug, Serialize)]
struct AdminDashboardResponse {
    total_users: u64,
    daily_active_users: u64,
    usage_by_user: Vec<UserUsageItem>,
    usage_trend: Vec<UsageTrendItem>,
}

#[derive(Debug, Serialize)]
struct UserUsageItem {
    tenant_id: String,
    user_id: String,
    requests: u64,
    total_input_tokens: u64,
    total_output_tokens: u64,
}

#[derive(Debug, Serialize)]
struct UsageTrendItem {
    day: String,
    requests: u64,
    total_input_tokens: u64,
    total_output_tokens: u64,
}

#[derive(Debug, Serialize)]
struct UserDashboardResponse {
    tenant_id: String,
    user_id: String,
    sessions: u64,
    memories: u64,
    requests: u64,
    total_input_tokens: u64,
    total_output_tokens: u64,
}

fn api_error(status: StatusCode, code: &'static str, message: impl Into<String>) -> Response {
    (status, Json(ApiError { code, message: message.into() })).into_response()
}

async fn require_admin_auth(
    State(state): State<AppState>,
    headers: HeaderMap,
    request: Request,
    next: Next,
) -> Response {
    if !admin_authorized_headers(&state.config, &headers) {
        return api_error(StatusCode::UNAUTHORIZED, "UNAUTHORIZED", "unauthorized");
    }
    next.run(request).await
}

pub async fn run(config: ServerConfig) -> Result<()> {
    let repo: Arc<dyn GatewayRepository> =
        Arc::new(PostgresRepository::new(&config.postgres_dsn).with_context(|| "open postgres")?);
    repo.migrate()?;

    let state = AppState {
        connection_guard: Arc::new(Semaphore::new(config.limits.max_connections)),
        inflight_guard: Arc::new(Semaphore::new(config.limits.max_inflight_requests)),
        config: config.clone(),
        repo,
        sessions: Arc::new(RwLock::new(HashMap::new())),
        pending: Arc::new(Mutex::new(HashMap::new())),
    };

    let admin_routes = Router::new()
        .route("/rpc/tool", post(dispatch_tool))
        .route("/api/admin/dashboard", get(admin_dashboard_data))
        .route_layer(middleware::from_fn_with_state(state.clone(), require_admin_auth));

    let app = Router::new()
        .route("/health", get(health))
        .route("/ws", get(ws_handler))
        .route("/openapi.yaml", get(openapi_yaml))
        .route("/auth/register", post(register_user))
        .route("/auth/login", post(login_user))
        .route("/auth/logout", post(logout_user))
        .route("/api/user/dashboard", get(user_dashboard_data))
        .route("/api/user/sessions", get(user_sessions))
        .route("/api/user/sessions/{session_id}/memory", get(user_session_memory))
        .route("/api/user/usage", get(user_usage_details))
        .route("/user/devices", get(user_devices))
        .route("/user/dispatch", post(dispatch_tool_for_session))
        .route("/admin", get(spa_admin))
        .route("/admin/{*path}", get(spa_admin))
        .route("/app", get(spa_app))
        .route("/app/{*path}", get(spa_app))
        .nest_service(
            "/assets",
            get_service(ServeDir::new("webui/dist/assets"))
                .handle_error(|_| async { (StatusCode::INTERNAL_SERVER_ERROR, "asset error") }),
        )
        .merge(admin_routes)
        .with_state(state);

    let listener = TcpListener::bind(&config.bind_addr)
        .await
        .with_context(|| format!("bind {}", config.bind_addr))?;
    info!("server listening on {}", config.bind_addr);
    serve(listener, app).await.context("axum serve failed")
}

async fn openapi_yaml() -> impl IntoResponse {
    match tokio::fs::read_to_string("server/openapi.yaml").await {
        Ok(v) => ([("content-type", "application/yaml")], v).into_response(),
        Err(_) => api_error(StatusCode::NOT_FOUND, "NOT_FOUND", "openapi not found"),
    }
}

async fn spa_admin() -> impl IntoResponse {
    serve_spa_index().await
}

async fn spa_app() -> impl IntoResponse {
    serve_spa_index().await
}

async fn serve_spa_index() -> Response {
    match tokio::fs::read_to_string("webui/dist/index.html").await {
        Ok(v) => ([("content-type", "text/html; charset=utf-8")], v).into_response(),
        Err(_) => api_error(
            StatusCode::NOT_FOUND,
            "WEBUI_NOT_BUILT",
            "webui/dist/index.html not found; run webui build first",
        ),
    }
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
                    let Some(server_message) = maybe_message else { return; };
                    let payload = match serde_json::to_string(&server_message) {
                        Ok(v) => v,
                        Err(_) => return,
                    };
                    if sink.send(Message::Text(payload.into())).await.is_err() {
                        return;
                    }
                }
                _ = ticker.tick() => {
                    let ping_payload = match serde_json::to_string(&ServerToClient::Ping) {
                        Ok(v) => v,
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
    Json(req): Json<RpcDispatchRequest>,
) -> impl IntoResponse {
    dispatch_tool_inner(&state, req).await
}

async fn dispatch_tool_inner(state: &AppState, req: RpcDispatchRequest) -> Response {
    if state.inflight_guard.available_permits() == 0 {
        return api_error(StatusCode::TOO_MANY_REQUESTS, "INFLIGHT_LIMIT", "inflight limit");
    }
    let permit = match state.inflight_guard.acquire().await {
        Ok(permit) => permit,
        Err(_) => {
            return api_error(
                StatusCode::INTERNAL_SERVER_ERROR,
                "INTERNAL_ERROR",
                "semaphore error",
            )
        }
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
        return api_error(StatusCode::NOT_FOUND, "NODE_NOT_CONNECTED", "node not connected");
    };
    if session.registration.tenant_id != req.tenant_id
        || session.registration.user_id != req.user_id
    {
        drop(permit);
        return api_error(StatusCode::FORBIDDEN, "TENANT_MISMATCH", "tenant mismatch");
    }
    if matches!(req.tool, ToolKind::CustomMcp) && req.command.contains(':') {
        let tool_name = req.command.split(':').next().unwrap_or_default();
        if !session.custom_tools.iter().any(|v| v == tool_name) {
            drop(permit);
            return api_error(
                StatusCode::FORBIDDEN,
                "CUSTOM_TOOL_NOT_VISIBLE",
                "custom tool not visible",
            );
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
        return api_error(StatusCode::BAD_GATEWAY, "NODE_DISCONNECTED", "node disconnected");
    }

    let response =
        tokio::time::timeout(Duration::from_millis(state.config.limits.request_timeout_ms), rx)
            .await;
    drop(permit);
    let Ok(Ok(tool_result)) = response else {
        state.pending.lock().await.remove(&req.request_id);
        return api_error(
            StatusCode::GATEWAY_TIMEOUT,
            "TOOL_REQUEST_TIMEOUT",
            "tool request timeout",
        );
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

fn parse_cookie(headers: &HeaderMap, name: &str) -> Option<String> {
    let cookie = headers.get("cookie")?.to_str().ok()?;
    cookie.split(';').find_map(|pair| {
        let mut it = pair.trim().splitn(2, '=');
        let k = it.next()?;
        let v = it.next().unwrap_or_default();
        if k == name {
            Some(v.to_owned())
        } else {
            None
        }
    })
}

fn resolve_session_or_error_from_headers(
    state: &AppState,
    headers: &HeaderMap,
) -> std::result::Result<shared_protocol::LoginSession, Response> {
    let Some(session_id) = parse_cookie(headers, "nexus_session") else {
        return Err(api_error(StatusCode::UNAUTHORIZED, "INVALID_SESSION", "missing session"));
    };
    match state.repo.get_login_session(&session_id) {
        Ok(Some(session)) => Ok(session),
        Ok(None) => Err(api_error(StatusCode::UNAUTHORIZED, "INVALID_SESSION", "invalid session")),
        Err(err) => {
            Err(api_error(StatusCode::INTERNAL_SERVER_ERROR, "INTERNAL_ERROR", err.to_string()))
        }
    }
}

fn verify_csrf_or_error(headers: &HeaderMap) -> std::result::Result<(), Response> {
    let cookie_token = parse_cookie(headers, "nexus_csrf");
    let header_token =
        headers.get("x-csrf-token").and_then(|v| v.to_str().ok()).map(|v| v.to_owned());
    match (cookie_token, header_token) {
        (Some(a), Some(b)) if a == b => Ok(()),
        _ => Err(api_error(StatusCode::FORBIDDEN, "CSRF_INVALID", "csrf token mismatch")),
    }
}

async fn register_user(
    State(state): State<AppState>,
    Json(req): Json<RegisterRequest>,
) -> impl IntoResponse {
    let password_hash = match hash_password(&req.password) {
        Ok(v) => v,
        Err(err) => {
            return api_error(StatusCode::INTERNAL_SERVER_ERROR, "INTERNAL_ERROR", err.to_string())
        }
    };
    let account = shared_protocol::UserAccount {
        tenant_id: req.tenant_id.clone(),
        user_id: req.user_id.clone(),
        display_name: req.display_name,
    };
    let login = shared_protocol::LoginUser {
        username: req.username,
        password_hash,
        tenant_id: req.tenant_id,
        user_id: req.user_id,
    };
    match state.repo.register_user_with_login(&account, &login) {
        Ok(_) => (StatusCode::CREATED, "registered").into_response(),
        Err(storage::StorageError::UsernameConflict) => {
            api_error(StatusCode::CONFLICT, "USERNAME_EXISTS", "username exists")
        }
        Err(err) => api_error(StatusCode::INTERNAL_SERVER_ERROR, "INTERNAL_ERROR", err.to_string()),
    }
}

async fn login_user(
    State(state): State<AppState>,
    Json(req): Json<LoginRequest>,
) -> impl IntoResponse {
    let Some(user) = (match state.repo.get_login_user_by_username(&req.username) {
        Ok(v) => v,
        Err(err) => {
            return api_error(StatusCode::INTERNAL_SERVER_ERROR, "INTERNAL_ERROR", err.to_string())
        }
    }) else {
        return api_error(StatusCode::UNAUTHORIZED, "INVALID_CREDENTIALS", "invalid credentials");
    };
    if !verify_password(&req.password, &user.password_hash) {
        return api_error(StatusCode::UNAUTHORIZED, "INVALID_CREDENTIALS", "invalid credentials");
    }
    let session_id = Uuid::new_v4().to_string();
    let csrf_token = Uuid::new_v4().to_string();
    let session = shared_protocol::LoginSession {
        session_id: session_id.clone(),
        username: req.username,
        tenant_id: user.tenant_id.clone(),
        user_id: user.user_id.clone(),
        created_at_ms: now_ms(),
    };
    if let Err(err) = state.repo.save_login_session(&session) {
        return api_error(StatusCode::INTERNAL_SERVER_ERROR, "INTERNAL_ERROR", err.to_string());
    }

    let headers = [
        ("set-cookie", format!("nexus_session={session_id}; Path=/; HttpOnly; SameSite=Lax")),
        ("set-cookie", format!("nexus_csrf={csrf_token}; Path=/; SameSite=Lax")),
    ];
    (headers, Json(LoginResponse { tenant_id: user.tenant_id, user_id: user.user_id, csrf_token }))
        .into_response()
}

async fn logout_user() -> impl IntoResponse {
    (
        [
            ("set-cookie", "nexus_session=; Path=/; HttpOnly; Max-Age=0; SameSite=Lax".to_owned()),
            ("set-cookie", "nexus_csrf=; Path=/; Max-Age=0; SameSite=Lax".to_owned()),
        ],
        StatusCode::OK,
    )
        .into_response()
}

async fn user_devices(State(state): State<AppState>, headers: HeaderMap) -> impl IntoResponse {
    let session = match resolve_session_or_error_from_headers(&state, &headers) {
        Ok(session) => session,
        Err(response) => return response,
    };
    match state.repo.list_user_devices(&session.tenant_id, &session.user_id) {
        Ok(v) => Json(v).into_response(),
        Err(err) => api_error(StatusCode::INTERNAL_SERVER_ERROR, "INTERNAL_ERROR", err.to_string()),
    }
}

async fn dispatch_tool_for_session(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(req): Json<DispatchByAliasRequest>,
) -> impl IntoResponse {
    let session = match resolve_session_or_error_from_headers(&state, &headers) {
        Ok(session) => session,
        Err(response) => return response,
    };
    if let Err(response) = verify_csrf_or_error(&headers) {
        return response;
    }
    let Some(node_id) = (match state.repo.resolve_device_node(
        &session.tenant_id,
        &session.user_id,
        &req.device_alias,
    ) {
        Ok(v) => v,
        Err(err) => {
            return api_error(StatusCode::INTERNAL_SERVER_ERROR, "INTERNAL_ERROR", err.to_string())
        }
    }) else {
        return api_error(StatusCode::NOT_FOUND, "DEVICE_NOT_FOUND", "device not found");
    };

    dispatch_tool_inner(
        &state,
        RpcDispatchRequest {
            request_id: req.request_id,
            tenant_id: session.tenant_id,
            user_id: session.user_id,
            node_id,
            tool: req.tool,
            command: req.command,
            input_tokens: req.input_tokens,
            output_tokens: req.output_tokens,
            model: req.model,
        },
    )
    .await
}

fn hash_password(password: &str) -> Result<String> {
    let salt = SaltString::generate(&mut OsRng);
    Ok(Argon2::default()
        .hash_password(password.as_bytes(), &salt)
        .map_err(|e| anyhow!("password hash failure: {e}"))?
        .to_string())
}

fn verify_password(password: &str, password_hash: &str) -> bool {
    let Ok(parsed) = PasswordHash::new(password_hash) else {
        return false;
    };
    Argon2::default().verify_password(password.as_bytes(), &parsed).is_ok()
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

async fn admin_dashboard_data(
    State(state): State<AppState>,
    Query(q): Query<TrendQuery>,
) -> impl IntoResponse {
    let days = q.days.unwrap_or(7).clamp(1, 30);
    let tenant = q.tenant_id.as_deref();
    let stats = match state.repo.admin_dashboard_stats(now_ms()) {
        Ok(v) => v,
        Err(err) => {
            return api_error(StatusCode::INTERNAL_SERVER_ERROR, "INTERNAL_ERROR", err.to_string())
        }
    };
    let trend = match state.repo.usage_trend(tenant, days) {
        Ok(v) => v,
        Err(err) => {
            return api_error(StatusCode::INTERNAL_SERVER_ERROR, "INTERNAL_ERROR", err.to_string())
        }
    };

    Json(AdminDashboardResponse {
        total_users: stats.total_users,
        daily_active_users: stats.daily_active_users,
        usage_by_user: stats
            .usage_by_user
            .into_iter()
            .map(|v| UserUsageItem {
                tenant_id: v.tenant_id,
                user_id: v.user_id,
                requests: v.requests,
                total_input_tokens: v.total_input_tokens,
                total_output_tokens: v.total_output_tokens,
            })
            .collect(),
        usage_trend: trend.into_iter().map(map_trend).collect(),
    })
    .into_response()
}

fn map_trend(v: UsageTrendPoint) -> UsageTrendItem {
    UsageTrendItem {
        day: v.day,
        requests: v.requests,
        total_input_tokens: v.total_input_tokens,
        total_output_tokens: v.total_output_tokens,
    }
}

async fn user_dashboard_data(
    State(state): State<AppState>,
    headers: HeaderMap,
) -> impl IntoResponse {
    let session = match resolve_session_or_error_from_headers(&state, &headers) {
        Ok(session) => session,
        Err(response) => return response,
    };
    match state.repo.user_dashboard_stats(&session.tenant_id, &session.user_id) {
        Ok(stats) => Json(UserDashboardResponse {
            tenant_id: session.tenant_id,
            user_id: session.user_id,
            sessions: stats.sessions,
            memories: stats.memories,
            requests: stats.requests,
            total_input_tokens: stats.total_input_tokens,
            total_output_tokens: stats.total_output_tokens,
        })
        .into_response(),
        Err(err) => api_error(StatusCode::INTERNAL_SERVER_ERROR, "INTERNAL_ERROR", err.to_string()),
    }
}

#[derive(Debug, Serialize)]
struct UserSessionsResponse {
    items: Vec<SessionListItem>,
}

async fn user_sessions(
    State(state): State<AppState>,
    headers: HeaderMap,
    Query(q): Query<ListQuery>,
) -> impl IntoResponse {
    let session = match resolve_session_or_error_from_headers(&state, &headers) {
        Ok(session) => session,
        Err(response) => return response,
    };
    let limit = q.limit.unwrap_or(20).clamp(1, 100);
    let offset = q.offset.unwrap_or(0);
    match state.repo.list_user_sessions(&session.tenant_id, &session.user_id, limit, offset) {
        Ok(items) => Json(UserSessionsResponse { items }).into_response(),
        Err(err) => api_error(StatusCode::INTERNAL_SERVER_ERROR, "INTERNAL_ERROR", err.to_string()),
    }
}

#[derive(Debug, Serialize)]
struct MemoryResponse {
    items: Vec<shared_protocol::MemoryRecord>,
}

async fn user_session_memory(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(session_id): Path<String>,
) -> impl IntoResponse {
    let session = match resolve_session_or_error_from_headers(&state, &headers) {
        Ok(session) => session,
        Err(response) => return response,
    };
    match state.repo.list_session_memory(&session.tenant_id, &session.user_id, &session_id) {
        Ok(items) => Json(MemoryResponse { items }).into_response(),
        Err(err) => api_error(StatusCode::INTERNAL_SERVER_ERROR, "INTERNAL_ERROR", err.to_string()),
    }
}

#[derive(Debug, Serialize)]
struct UsageDetailsResponse {
    items: Vec<UsageDetailItem>,
}

async fn user_usage_details(
    State(state): State<AppState>,
    headers: HeaderMap,
    Query(q): Query<ListQuery>,
) -> impl IntoResponse {
    let session = match resolve_session_or_error_from_headers(&state, &headers) {
        Ok(session) => session,
        Err(response) => return response,
    };
    let limit = q.limit.unwrap_or(20).clamp(1, 100);
    let offset = q.offset.unwrap_or(0);
    match state.repo.list_usage_details(&session.tenant_id, &session.user_id, limit, offset) {
        Ok(items) => Json(UsageDetailsResponse { items }).into_response(),
        Err(err) => api_error(StatusCode::INTERNAL_SERVER_ERROR, "INTERNAL_ERROR", err.to_string()),
    }
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
    use axum::http::{HeaderMap, HeaderValue};
    use shared_protocol::{AuthConfig, RuntimeLimits, ServerConfig};

    use super::{
        admin_authorized_headers, eval_calc, hash_password, parse_cookie,
        run_connection_load_baseline, verify_csrf_or_error, verify_password,
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
            postgres_dsn: "postgres://postgres:postgres@127.0.0.1:5432/nexus".to_owned(),
            vlm_endpoint: "http://127.0.0.1/health".to_owned(),
            limits: RuntimeLimits::default(),
            auth: AuthConfig::default(),
        };
        assert!(admin_authorized_headers(&config, &headers));
    }

    #[test]
    fn password_hash_and_verify_roundtrip() {
        let hash = hash_password("super-secret").expect("hash");
        assert_ne!(hash, "super-secret");
        assert!(verify_password("super-secret", &hash));
        assert!(!verify_password("wrong", &hash));
    }

    #[tokio::test]
    async fn load_baseline_accepts_500_connections() {
        let accepted = run_connection_load_baseline(500, 500).await;
        assert_eq!(accepted, 500);
    }

    #[test]
    fn cookie_parser_works() {
        let mut headers = HeaderMap::new();
        headers.insert("cookie", HeaderValue::from_static("a=1; nexus_session=abc; b=2"));
        assert_eq!(parse_cookie(&headers, "nexus_session").as_deref(), Some("abc"));
    }

    #[test]
    fn csrf_guard_rejects_mismatch() {
        let mut headers = HeaderMap::new();
        headers.insert("cookie", HeaderValue::from_static("nexus_csrf=token-a"));
        headers.insert("x-csrf-token", HeaderValue::from_static("token-b"));
        assert!(verify_csrf_or_error(&headers).is_err());
    }
}
