//! Discord REST API helpers: send messages, typing indicator, message splitting.

use std::sync::LazyLock;
use reqwest::Client;
use tracing::warn;

const DISCORD_API_BASE: &str = "https://discord.com/api/v10";
const MAX_MESSAGE_LENGTH: usize = 2000;
const MAX_RETRIES: usize = 3;

static HTTP_CLIENT: LazyLock<Client> = LazyLock::new(|| {
    Client::builder()
        .timeout(std::time::Duration::from_secs(30))
        .build()
        .expect("failed to create Discord HTTP client")
});

pub async fn send_message(
    bot_token: &str,
    channel_id: &str,
    content: &str,
) -> Result<(), String> {
    let chunks = split_message(content, MAX_MESSAGE_LENGTH);

    for chunk in &chunks {
        send_single_message(bot_token, channel_id, chunk).await?;
    }

    Ok(())
}

async fn send_single_message(
    bot_token: &str,
    channel_id: &str,
    content: &str,
) -> Result<(), String> {
    let url = format!("{}/channels/{}/messages", DISCORD_API_BASE, channel_id);

    for attempt in 0..MAX_RETRIES {
        let response = HTTP_CLIENT
            .post(&url)
            .header("Authorization", format!("Bot {}", bot_token))
            .json(&serde_json::json!({ "content": content }))
            .send()
            .await
            .map_err(|e| format!("Discord send error: {}", e))?;

        let status = response.status().as_u16();

        if status == 200 || status == 201 {
            return Ok(());
        }

        if status == 429 {
            let body: serde_json::Value = response
                .json()
                .await
                .unwrap_or_else(|_| serde_json::json!({"retry_after": 1.0}));
            let retry_after = body
                .get("retry_after")
                .and_then(|v| v.as_f64())
                .unwrap_or(1.0);
            warn!(
                "Discord 429 rate limited (attempt {}/{}), retrying after {:.1}s",
                attempt + 1,
                MAX_RETRIES,
                retry_after
            );
            tokio::time::sleep(std::time::Duration::from_secs_f64(retry_after)).await;
            continue;
        }

        let body = response.text().await.unwrap_or_default();
        return Err(format!("Discord API error {}: {}", status, body));
    }

    Err("Discord send: max retries exceeded on 429".to_string())
}

/// Download a Discord attachment to a temp directory.
/// Returns the absolute path to the saved file.
pub async fn download_attachment(url: &str, filename: &str) -> Result<String, String> {
    let response = HTTP_CLIENT
        .get(url)
        .send()
        .await
        .map_err(|e| format!("download error: {}", e))?;

    if !response.status().is_success() {
        return Err(format!("download HTTP {}", response.status()));
    }

    let bytes = response
        .bytes()
        .await
        .map_err(|e| format!("download read error: {}", e))?;

    // Save to temp directory
    let dir = std::path::Path::new("/tmp/nexus-media");
    tokio::fs::create_dir_all(dir).await.map_err(|e| format!("mkdir error: {}", e))?;

    let safe_name = filename.replace(['/', '\\', '\0'], "_");
    let uuid = uuid::Uuid::new_v4();
    let path = dir.join(format!("{}_{}", uuid, safe_name));

    tokio::fs::write(&path, &bytes).await.map_err(|e| format!("write error: {}", e))?;

    Ok(path.to_string_lossy().to_string())
}

/// Send a Discord message with file attachments via multipart form.
pub async fn send_message_with_files(
    bot_token: &str,
    channel_id: &str,
    content: &str,
    file_paths: &[String],
) -> Result<(), String> {
    let url = format!("{}/channels/{}/messages", DISCORD_API_BASE, channel_id);

    let mut form = build_multipart_form(content, file_paths).await?;

    for _attempt in 0..MAX_RETRIES {
        let response = HTTP_CLIENT
            .post(&url)
            .header("Authorization", format!("Bot {}", bot_token))
            .multipart(form)
            .send()
            .await
            .map_err(|e| format!("Discord send error: {}", e))?;

        let status = response.status().as_u16();
        if status == 200 || status == 201 {
            return Ok(());
        }

        if status == 429 {
            let body: serde_json::Value = response
                .json()
                .await
                .unwrap_or_else(|_| serde_json::json!({"retry_after": 1.0}));
            let retry_after = body
                .get("retry_after")
                .and_then(|v| v.as_f64())
                .unwrap_or(1.0);
            warn!(
                "Discord 429 (file upload), retrying after {:.1}s",
                retry_after
            );
            tokio::time::sleep(std::time::Duration::from_secs_f64(retry_after)).await;
            // Rebuild form for retry (multipart::Form is consumed by send)
            form = build_multipart_form(content, file_paths).await?;
            continue;
        }

        let body = response.text().await.unwrap_or_default();
        return Err(format!("Discord API error {}: {}", status, body));
    }

    Err("Discord file send: max retries exceeded".to_string())
}

async fn build_multipart_form(
    content: &str,
    file_paths: &[String],
) -> Result<reqwest::multipart::Form, String> {
    use reqwest::multipart;

    let mut form = multipart::Form::new().text(
        "payload_json",
        serde_json::json!({"content": content}).to_string(),
    );

    for (i, path) in file_paths.iter().enumerate() {
        let file_path = std::path::Path::new(path);
        let file_name = file_path
            .file_name()
            .map(|n| n.to_string_lossy().to_string())
            .unwrap_or_else(|| format!("file_{}", i));

        let bytes =
            tokio::fs::read(path).await.map_err(|e| format!("Failed to read file {}: {}", path, e))?;

        // Check 25MB per-file limit
        if bytes.len() > 25 * 1024 * 1024 {
            warn!("Skipping file {} — exceeds 25MB Discord limit", path);
            continue;
        }

        let part = multipart::Part::bytes(bytes)
            .file_name(file_name)
            .mime_str("application/octet-stream")
            .map_err(|e| format!("mime error: {}", e))?;

        form = form.part(format!("files[{}]", i), part);
    }

    Ok(form)
}

pub fn start_typing(
    bot_token: String,
    channel_id: String,
    cancel: tokio_util::sync::CancellationToken,
) -> tokio::task::JoinHandle<()> {
    tokio::spawn(async move {
        let url = format!("{}/channels/{}/typing", DISCORD_API_BASE, channel_id);
        loop {
            let _ = HTTP_CLIENT
                .post(&url)
                .header("Authorization", format!("Bot {}", bot_token))
                .send()
                .await;

            tokio::select! {
                _ = tokio::time::sleep(std::time::Duration::from_secs(8)) => {}
                _ = cancel.cancelled() => break,
            }
        }
    })
}

pub fn split_message(content: &str, max_len: usize) -> Vec<String> {
    if content.len() <= max_len {
        return vec![content.to_string()];
    }

    let mut chunks = Vec::new();
    let mut remaining = content;

    while !remaining.is_empty() {
        if remaining.len() <= max_len {
            chunks.push(remaining.to_string());
            break;
        }

        // Find a safe byte boundary (UTF-8 safe)
        let safe_max = remaining.floor_char_boundary(max_len);
        let search_region = &remaining[..safe_max];
        let split_at = search_region
            .rfind('\n')
            .map(|pos| pos + 1)
            .unwrap_or(safe_max);

        chunks.push(remaining[..split_at].to_string());
        remaining = &remaining[split_at..];
    }

    chunks
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_split_message_short() {
        let chunks = split_message("hello", 2000);
        assert_eq!(chunks, vec!["hello"]);
    }

    #[test]
    fn test_split_message_exact() {
        let msg = "a".repeat(2000);
        let chunks = split_message(&msg, 2000);
        assert_eq!(chunks.len(), 1);
        assert_eq!(chunks[0].len(), 2000);
    }

    #[test]
    fn test_split_message_at_newline() {
        let msg = format!("{}\n{}", "a".repeat(1500), "b".repeat(1000));
        let chunks = split_message(&msg, 2000);
        assert_eq!(chunks.len(), 2);
        assert_eq!(chunks[0], format!("{}\n", "a".repeat(1500)));
        assert_eq!(chunks[1], "b".repeat(1000));
    }

    #[test]
    fn test_split_message_hard_split() {
        let msg = "a".repeat(5000);
        let chunks = split_message(&msg, 2000);
        assert_eq!(chunks.len(), 3);
        assert_eq!(chunks[0].len(), 2000);
        assert_eq!(chunks[1].len(), 2000);
        assert_eq!(chunks[2].len(), 1000);
    }
}
