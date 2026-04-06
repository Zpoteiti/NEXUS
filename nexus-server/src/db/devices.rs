use serde::Serialize;
use sqlx::PgPool;

#[derive(Debug, sqlx::FromRow)]
pub struct DeviceTokenVerification {
    pub user_id: String,
    pub device_name: String,
}

#[derive(Debug, Clone, sqlx::FromRow, Serialize)]
pub struct DeviceTokenInfo {
    pub token: String,
    pub device_name: Option<String>,
    pub revoked: bool,
    pub created_at: Option<chrono::DateTime<chrono::Utc>>,
}

pub async fn create_device_token(
    db: &PgPool,
    token: &str,
    user_id: &str,
    device_name: &str,
) -> Result<(), sqlx::Error> {
    sqlx::query(
        "INSERT INTO device_tokens (token, user_id, device_name) VALUES ($1, $2, $3)",
    )
    .bind(token)
    .bind(user_id)
    .bind(device_name)
    .execute(db)
    .await?;
    Ok(())
}

/// Returns user_id and device_name if token is valid and not revoked.
pub async fn verify_device_token(
    pool: &PgPool,
    token: &str,
) -> Result<Option<DeviceTokenVerification>, sqlx::Error> {
    sqlx::query_as::<_, DeviceTokenVerification>(
        r#"
        SELECT user_id, COALESCE(device_name, 'unnamed') AS device_name
        FROM device_tokens
        WHERE token = $1
          AND revoked = FALSE
        "#,
    )
    .bind(token)
    .fetch_optional(pool)
    .await
}

pub async fn list_device_tokens(
    db: &PgPool,
    user_id: &str,
) -> Result<Vec<DeviceTokenInfo>, sqlx::Error> {
    sqlx::query_as::<_, DeviceTokenInfo>(
        "SELECT token, device_name, revoked, created_at FROM device_tokens WHERE user_id = $1 ORDER BY created_at DESC",
    )
    .bind(user_id)
    .fetch_all(db)
    .await
}

pub async fn revoke_device_token(
    db: &PgPool,
    token: &str,
    user_id: &str,
) -> Result<bool, sqlx::Error> {
    let result = sqlx::query(
        "UPDATE device_tokens SET revoked = TRUE WHERE token = $1 AND user_id = $2 AND revoked = FALSE",
    )
    .bind(token)
    .bind(user_id)
    .execute(db)
    .await?;
    Ok(result.rows_affected() > 0)
}

// ============================================================================
// Device Policy
// ============================================================================

pub async fn get_device_policy(
    db: &PgPool,
    user_id: &str,
    device_name: &str,
) -> Result<nexus_common::protocol::FsPolicy, sqlx::Error> {
    let row: (serde_json::Value,) = sqlx::query_as(
        "SELECT COALESCE(fs_policy, '{\"mode\":\"sandbox\"}'::jsonb) FROM device_tokens WHERE user_id = $1 AND device_name = $2 AND revoked = FALSE"
    )
    .bind(user_id)
    .bind(device_name)
    .fetch_one(db)
    .await?;

    serde_json::from_value(row.0)
        .map_err(|e| sqlx::Error::Protocol(format!("invalid fs_policy JSON: {e}")))
}

pub async fn update_device_policy(
    db: &PgPool,
    user_id: &str,
    device_name: &str,
    policy: &nexus_common::protocol::FsPolicy,
) -> Result<bool, sqlx::Error> {
    let json = serde_json::to_value(policy)
        .map_err(|e| sqlx::Error::Protocol(format!("failed to serialize policy: {e}")))?;

    let result = sqlx::query(
        "UPDATE device_tokens SET fs_policy = $1 WHERE user_id = $2 AND device_name = $3 AND revoked = FALSE"
    )
    .bind(json)
    .bind(user_id)
    .bind(device_name)
    .execute(db)
    .await?;

    Ok(result.rows_affected() > 0)
}

// ============================================================================
// Device MCP Config
// ============================================================================

pub async fn get_device_mcp_config(
    db: &PgPool,
    user_id: &str,
    device_name: &str,
) -> Result<Vec<nexus_common::protocol::McpServerEntry>, sqlx::Error> {
    let row: (serde_json::Value,) = sqlx::query_as(
        "SELECT COALESCE(mcp_config, '[]'::jsonb) FROM device_tokens WHERE user_id = $1 AND device_name = $2 AND revoked = FALSE"
    )
    .bind(user_id)
    .bind(device_name)
    .fetch_one(db)
    .await?;

    serde_json::from_value(row.0)
        .map_err(|e| sqlx::Error::Protocol(format!("invalid mcp_config JSON: {e}")))
}

pub async fn update_device_mcp_config(
    db: &PgPool,
    user_id: &str,
    device_name: &str,
    config: &[nexus_common::protocol::McpServerEntry],
) -> Result<bool, sqlx::Error> {
    let json = serde_json::to_value(config)
        .map_err(|e| sqlx::Error::Protocol(format!("failed to serialize mcp_config: {e}")))?;

    let result = sqlx::query(
        "UPDATE device_tokens SET mcp_config = $1 WHERE user_id = $2 AND device_name = $3 AND revoked = FALSE"
    )
    .bind(json)
    .bind(user_id)
    .bind(device_name)
    .execute(db)
    .await?;

    Ok(result.rows_affected() > 0)
}
