use chrono::{DateTime, Utc};
use sqlx::PgPool;

#[derive(Debug, Clone, sqlx::FromRow, serde::Serialize)]
pub struct DeviceToken {
    pub token: String,
    pub user_id: String,
    pub device_name: String,
    pub fs_policy: serde_json::Value,
    pub mcp_config: serde_json::Value,
    pub workspace_path: String,
    pub shell_timeout: i64,
    pub ssrf_whitelist: serde_json::Value,
    pub created_at: DateTime<Utc>,
}

pub async fn create_token(
    pool: &PgPool,
    token: &str,
    user_id: &str,
    device_name: &str,
) -> Result<(), sqlx::Error> {
    sqlx::query("INSERT INTO device_tokens (token, user_id, device_name) VALUES ($1, $2, $3)")
        .bind(token)
        .bind(user_id)
        .bind(device_name)
        .execute(pool)
        .await?;
    Ok(())
}

pub async fn find_by_token(pool: &PgPool, token: &str) -> Result<Option<DeviceToken>, sqlx::Error> {
    sqlx::query_as::<_, DeviceToken>("SELECT * FROM device_tokens WHERE token = $1")
        .bind(token)
        .fetch_optional(pool)
        .await
}

pub async fn list_by_user(pool: &PgPool, user_id: &str) -> Result<Vec<DeviceToken>, sqlx::Error> {
    sqlx::query_as::<_, DeviceToken>(
        "SELECT * FROM device_tokens WHERE user_id = $1 ORDER BY created_at",
    )
    .bind(user_id)
    .fetch_all(pool)
    .await
}

pub async fn delete_token(pool: &PgPool, token: &str) -> Result<bool, sqlx::Error> {
    let result = sqlx::query("DELETE FROM device_tokens WHERE token = $1")
        .bind(token)
        .execute(pool)
        .await?;
    Ok(result.rows_affected() > 0)
}

pub async fn update_fs_policy(
    pool: &PgPool,
    user_id: &str,
    device_name: &str,
    fs_policy: &serde_json::Value,
) -> Result<bool, sqlx::Error> {
    let result = sqlx::query(
        "UPDATE device_tokens SET fs_policy = $1 WHERE user_id = $2 AND device_name = $3",
    )
    .bind(fs_policy)
    .bind(user_id)
    .bind(device_name)
    .execute(pool)
    .await?;
    Ok(result.rows_affected() > 0)
}

pub async fn update_mcp_config(
    pool: &PgPool,
    user_id: &str,
    device_name: &str,
    mcp_config: &serde_json::Value,
) -> Result<bool, sqlx::Error> {
    let result = sqlx::query(
        "UPDATE device_tokens SET mcp_config = $1 WHERE user_id = $2 AND device_name = $3",
    )
    .bind(mcp_config)
    .bind(user_id)
    .bind(device_name)
    .execute(pool)
    .await?;
    Ok(result.rows_affected() > 0)
}

pub async fn find_by_user_and_device(
    pool: &PgPool,
    user_id: &str,
    device_name: &str,
) -> Result<Option<DeviceToken>, sqlx::Error> {
    sqlx::query_as::<_, DeviceToken>(
        "SELECT * FROM device_tokens WHERE user_id = $1 AND device_name = $2",
    )
    .bind(user_id)
    .bind(device_name)
    .fetch_optional(pool)
    .await
}
