use axum::{
    body::Body,
    extract::{Request, State},
    http::{HeaderMap, HeaderName, HeaderValue, StatusCode},
    response::{IntoResponse, Response},
};
use tracing::warn;

use crate::browser::verify_jwt;
use crate::state::SharedState;

/// Maximum request body size: 25 MB.
const MAX_BODY_SIZE: usize = 25 * 1024 * 1024;

/// Public auth endpoints that skip JWT validation.
fn is_public_endpoint(path: &str) -> bool {
    let p = path.trim_start_matches('/');
    p == "api/auth/login" || p == "api/auth/register"
}

pub async fn api_proxy(
    State(state): State<SharedState>,
    req: Request<Body>,
) -> Response {
    let path = req.uri().path().to_string();
    let query = req.uri().query().map(|q| format!("?{q}")).unwrap_or_default();

    // --- Path traversal check ---
    if path.contains("..") {
        return (StatusCode::BAD_REQUEST, "Invalid path").into_response();
    }

    // --- JWT check (skip for public endpoints) ---
    if !is_public_endpoint(&path) {
        let auth_header = req
            .headers()
            .get("Authorization")
            .and_then(|v| v.to_str().ok())
            .and_then(|v| v.strip_prefix("Bearer "));

        match auth_header {
            Some(token) => {
                if let Err(_) = verify_jwt(token, &state.jwt_secret) {
                    return (StatusCode::UNAUTHORIZED, "Invalid token").into_response();
                }
            }
            None => {
                return (StatusCode::UNAUTHORIZED, "Missing Authorization header").into_response();
            }
        }
    }

    // --- Build upstream URL ---
    let upstream_url = format!(
        "{}{}{}",
        state.server_api_url.trim_end_matches('/'),
        path,
        query,
    );

    // --- Collect request parts ---
    let method = req.method().clone();
    let headers = req.headers().clone();

    // Read body with size limit
    let body_bytes = match axum::body::to_bytes(req.into_body(), MAX_BODY_SIZE).await {
        Ok(b) => b,
        Err(_) => {
            return (StatusCode::PAYLOAD_TOO_LARGE, "Request body too large").into_response();
        }
    };

    // --- Forward to nexus-server ---
    let client = &state.http_client;

    // Convert axum headers to reqwest headers
    let mut reqwest_headers = reqwest::header::HeaderMap::new();
    for (name, value) in headers.iter() {
        // Skip hop-by-hop headers
        let name_str = name.as_str();
        if matches!(
            name_str,
            "host" | "connection" | "transfer-encoding" | "upgrade"
        ) {
            continue;
        }
        if let Ok(rname) = reqwest::header::HeaderName::from_bytes(name.as_ref()) {
            if let Ok(rval) = reqwest::header::HeaderValue::from_bytes(value.as_bytes()) {
                reqwest_headers.insert(rname, rval);
            }
        }
    }

    let upstream_resp = match client
        .request(method, &upstream_url)
        .headers(reqwest_headers)
        .body(body_bytes)
        .send()
        .await
    {
        Ok(r) => r,
        Err(e) => {
            warn!("proxy upstream error: {e}");
            return (StatusCode::BAD_GATEWAY, "Upstream server unreachable").into_response();
        }
    };

    // --- Convert response back to axum ---
    let status = StatusCode::from_u16(upstream_resp.status().as_u16()).unwrap_or(StatusCode::BAD_GATEWAY);

    let mut response_headers = HeaderMap::new();
    for (name, value) in upstream_resp.headers().iter() {
        if let Ok(hname) = HeaderName::from_bytes(name.as_ref()) {
            if let Ok(hval) = HeaderValue::from_bytes(value.as_bytes()) {
                response_headers.insert(hname, hval);
            }
        }
    }

    let body = match upstream_resp.bytes().await {
        Ok(b) => b,
        Err(e) => {
            warn!("proxy body read error: {e}");
            return (StatusCode::BAD_GATEWAY, "Failed to read upstream response").into_response();
        }
    };

    if body.len() > MAX_BODY_SIZE {
        return (StatusCode::BAD_GATEWAY, "Response too large").into_response();
    }

    let mut resp = Response::new(Body::from(body));
    *resp.status_mut() = status;
    *resp.headers_mut() = response_headers;
    resp
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn public_endpoints_detected() {
        assert!(is_public_endpoint("/api/auth/login"));
        assert!(is_public_endpoint("/api/auth/register"));
        assert!(!is_public_endpoint("/api/sessions"));
        assert!(!is_public_endpoint("/api/auth/logout"));
    }
}
