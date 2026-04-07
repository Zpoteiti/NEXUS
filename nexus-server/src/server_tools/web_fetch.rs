use async_trait::async_trait;
use nexus_common::error::{ErrorCode, NexusError};
use serde_json::{json, Value};
use std::sync::Arc;
use tokio::sync::Semaphore;
use tracing::info;

use super::{ServerTool, ServerToolResult};
use crate::state::AppState;

/// Maximum concurrent fetches across all sessions.
const MAX_CONCURRENT_FETCHES: usize = 50;

/// Maximum response body size (1 MB).
const MAX_BODY_BYTES: usize = 1_024 * 1_024;

/// Maximum characters returned to the LLM.
const MAX_OUTPUT_CHARS: usize = 50_000;

/// HTTP request timeout.
const FETCH_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(15);

static FETCH_SEMAPHORE: std::sync::LazyLock<Semaphore> =
    std::sync::LazyLock::new(|| Semaphore::new(MAX_CONCURRENT_FETCHES));

static HTTP_CLIENT: std::sync::LazyLock<reqwest::Client> = std::sync::LazyLock::new(|| {
    reqwest::Client::builder()
        .timeout(FETCH_TIMEOUT)
        .connect_timeout(std::time::Duration::from_secs(10))
        .redirect(reqwest::redirect::Policy::limited(5))
        .user_agent("Mozilla/5.0 (X11; Linux x86_64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/131.0.0.0 Safari/537.36")
        .build()
        .expect("failed to build HTTP client")
});

pub struct WebFetchTool;

#[async_trait]
impl ServerTool for WebFetchTool {
    fn name(&self) -> &str {
        "web_fetch"
    }

    fn schema(&self) -> Value {
        json!({
            "type": "function",
            "function": {
                "name": "web_fetch",
                "description": "Fetch a URL and extract readable content (HTML → markdown). Use this to read web pages, documentation, API responses, etc.",
                "parameters": {
                    "type": "object",
                    "properties": {
                        "url": {
                            "type": "string",
                            "description": "The URL to fetch (http or https)."
                        },
                        "max_chars": {
                            "type": "integer",
                            "description": "Maximum characters to return. Default 50000.",
                            "default": 50000,
                            "minimum": 100,
                            "maximum": 50000
                        }
                    },
                    "required": ["url"]
                }
            }
        })
    }

    async fn execute(
        &self,
        _state: &Arc<AppState>,
        _user_id: &str,
        _session_id: &str,
        arguments: Value,
        _event_channel: &str,
        _event_chat_id: &str,
    ) -> Result<ServerToolResult, NexusError> {
        let url = arguments
            .get("url")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_string();
        let max_chars = arguments
            .get("max_chars")
            .and_then(|v| v.as_u64())
            .unwrap_or(MAX_OUTPUT_CHARS as u64)
            .min(MAX_OUTPUT_CHARS as u64) as usize;

        if url.is_empty() {
            return Err(NexusError::new(
                ErrorCode::ToolInvalidParams,
                "web_fetch: url is required",
            ));
        }

        // Validate URL scheme
        if !url.starts_with("http://") && !url.starts_with("https://") {
            return Err(NexusError::new(
                ErrorCode::ToolInvalidParams,
                "web_fetch: only http and https URLs are supported",
            ));
        }

        // SSRF protection: block private/internal IPs
        if let Err(reason) = validate_url_safe(&url) {
            return Err(NexusError::new(
                ErrorCode::ValidationFailed,
                format!("web_fetch: blocked — {}", reason),
            ));
        }

        // Acquire semaphore permit (limits concurrent fetches)
        let _permit = FETCH_SEMAPHORE.acquire().await.map_err(|_| {
            NexusError::new(ErrorCode::ExecutionFailed, "web_fetch: semaphore closed")
        })?;

        info!("web_fetch: fetching {}", url);

        let response = HTTP_CLIENT.get(&url).send().await.map_err(|e| {
            NexusError::new(
                ErrorCode::ExecutionFailed,
                format!("web_fetch: request failed — {}", e),
            )
        })?;

        let status = response.status().as_u16();
        let content_type = response
            .headers()
            .get("content-type")
            .and_then(|v| v.to_str().ok())
            .unwrap_or("")
            .to_string();

        if !response.status().is_success() {
            return Ok(ServerToolResult {
                output: format!("HTTP {} for {}", status, url),
                media: vec![],
            });
        }

        // Read body with size limit
        let body_bytes = read_body_limited(response, MAX_BODY_BYTES).await?;

        let text = if content_type.contains("application/json") {
            // Pretty-print JSON
            match serde_json::from_slice::<Value>(&body_bytes) {
                Ok(v) => serde_json::to_string_pretty(&v).unwrap_or_else(|_| {
                    String::from_utf8_lossy(&body_bytes).to_string()
                }),
                Err(_) => String::from_utf8_lossy(&body_bytes).to_string(),
            }
        } else if content_type.contains("text/html") || content_type.contains("application/xhtml") {
            // Extract readable text from HTML
            html_to_text(&String::from_utf8_lossy(&body_bytes))
        } else {
            // Plain text, XML, etc — return as-is
            String::from_utf8_lossy(&body_bytes).to_string()
        };

        // Truncate to max_chars
        let (output, truncated) = if text.len() > max_chars {
            (
                format!(
                    "{}\n\n[Truncated: showing {}/{} chars]",
                    &text[..max_chars],
                    max_chars,
                    text.len()
                ),
                true,
            )
        } else {
            (text.clone(), false)
        };

        let result = format!(
            "URL: {}\nStatus: {}\nContent-Type: {}\nLength: {} chars{}\n\n{}",
            url,
            status,
            content_type,
            text.len(),
            if truncated { " (truncated)" } else { "" },
            output,
        );

        Ok(ServerToolResult {
            output: result,
            media: vec![],
        })
    }
}

/// Read response body up to `limit` bytes.
async fn read_body_limited(
    response: reqwest::Response,
    limit: usize,
) -> Result<Vec<u8>, NexusError> {
    // Check Content-Length header first for early rejection
    if let Some(len) = response.content_length() {
        if len as usize > limit {
            return Err(NexusError::new(
                ErrorCode::ValidationFailed,
                format!(
                    "web_fetch: response too large ({} bytes, max {})",
                    len, limit
                ),
            ));
        }
    }

    let bytes = response.bytes().await.map_err(|e| {
        NexusError::new(
            ErrorCode::ExecutionFailed,
            format!("web_fetch: failed to read body — {}", e),
        )
    })?;

    if bytes.len() > limit {
        return Err(NexusError::new(
            ErrorCode::ValidationFailed,
            format!(
                "web_fetch: response too large ({} bytes, max {})",
                bytes.len(),
                limit
            ),
        ));
    }

    Ok(bytes.to_vec())
}

/// Validate that a URL does not point to a private/internal address (SSRF protection).
fn validate_url_safe(url: &str) -> Result<(), String> {
    let parsed = url::Url::parse(url).map_err(|e| format!("invalid URL: {}", e))?;

    let host = parsed.host_str().ok_or("missing host")?;

    // Check for common private hostnames
    if host == "localhost" || host == "0.0.0.0" || host.ends_with(".local") {
        return Err(format!("blocked host: {}", host));
    }

    // Try to parse as IP and check for private ranges
    if let Ok(ip) = host.parse::<std::net::IpAddr>() {
        if is_private_ip(ip) {
            return Err(format!("blocked private IP: {}", ip));
        }
    }

    // Resolve hostname to check for DNS rebinding to private IPs
    // (sync DNS resolution is acceptable here — it's behind the semaphore)
    if host.parse::<std::net::IpAddr>().is_err() {
        if let Ok(addrs) = std::net::ToSocketAddrs::to_socket_addrs(&(host, 80)) {
            for addr in addrs {
                if is_private_ip(addr.ip()) {
                    return Err(format!("hostname {} resolves to private IP {}", host, addr.ip()));
                }
            }
        }
    }

    Ok(())
}

/// Check if an IP address is in a private/internal range.
fn is_private_ip(ip: std::net::IpAddr) -> bool {
    match ip {
        std::net::IpAddr::V4(v4) => {
            v4.is_loopback()                                // 127.0.0.0/8
                || v4.is_private()                          // 10.0.0.0/8, 172.16.0.0/12, 192.168.0.0/16
                || v4.is_link_local()                       // 169.254.0.0/16 (cloud metadata!)
                || v4.is_broadcast()                        // 255.255.255.255
                || v4.is_unspecified()                      // 0.0.0.0
                || v4.octets()[0] == 100 && v4.octets()[1] >= 64 && v4.octets()[1] <= 127  // 100.64.0.0/10 (CGNAT)
        }
        std::net::IpAddr::V6(v6) => {
            v6.is_loopback()        // ::1
                || v6.is_unspecified()  // ::
                // fc00::/7 (unique local) — check first byte
                || (v6.segments()[0] & 0xfe00) == 0xfc00
                // fe80::/10 (link-local)
                || (v6.segments()[0] & 0xffc0) == 0xfe80
        }
    }
}

/// Simple HTML to readable text extraction.
/// Strips tags, converts headings/links/lists to markdown-ish format.
fn html_to_text(html: &str) -> String {
    let mut text = html.to_string();

    // Remove script and style blocks
    let script_re = regex::Regex::new(r"(?is)<script[^>]*>.*?</script>").unwrap();
    text = script_re.replace_all(&text, "").to_string();
    let style_re = regex::Regex::new(r"(?is)<style[^>]*>.*?</style>").unwrap();
    text = style_re.replace_all(&text, "").to_string();
    // Remove HTML comments
    let comment_re = regex::Regex::new(r"(?s)<!--.*?-->").unwrap();
    text = comment_re.replace_all(&text, "").to_string();

    // Convert headings
    for level in 1..=6 {
        let hashes = "#".repeat(level);
        let re = regex::Regex::new(&format!(r"(?is)<h{}[^>]*>(.*?)</h{}>", level, level)).unwrap();
        text = re
            .replace_all(&text, |caps: &regex::Captures| {
                format!("\n{} {}\n", hashes, caps[1].trim())
            })
            .to_string();
    }

    // Convert links: <a href="url">text</a> -> [text](url)
    let link_re = regex::Regex::new(r#"(?is)<a[^>]+href="([^"]*)"[^>]*>(.*?)</a>"#).unwrap();
    text = link_re
        .replace_all(&text, |caps: &regex::Captures| {
            let href = &caps[1];
            let link_text = caps[2].trim();
            if link_text.is_empty() || link_text == href {
                href.to_string()
            } else {
                format!("[{}]({})", link_text, href)
            }
        })
        .to_string();

    // Convert list items
    let li_re = regex::Regex::new(r"(?is)<li[^>]*>(.*?)</li>").unwrap();
    text = li_re
        .replace_all(&text, |caps: &regex::Captures| {
            format!("- {}\n", caps[1].trim())
        })
        .to_string();

    // Convert paragraphs and line breaks
    let p_re = regex::Regex::new(r"(?is)<p[^>]*>(.*?)</p>").unwrap();
    text = p_re
        .replace_all(&text, |caps: &regex::Captures| {
            format!("\n{}\n", caps[1].trim())
        })
        .to_string();
    let br_re = regex::Regex::new(r"(?i)<br\s*/?>").unwrap();
    text = br_re.replace_all(&text, "\n").to_string();

    // Strip remaining HTML tags
    let tag_re = regex::Regex::new(r"<[^>]+>").unwrap();
    text = tag_re.replace_all(&text, "").to_string();

    // Decode common HTML entities
    text = text
        .replace("&amp;", "&")
        .replace("&lt;", "<")
        .replace("&gt;", ">")
        .replace("&quot;", "\"")
        .replace("&#39;", "'")
        .replace("&apos;", "'")
        .replace("&nbsp;", " ");

    // Collapse excessive whitespace
    let multi_newline_re = regex::Regex::new(r"\n{3,}").unwrap();
    text = multi_newline_re.replace_all(&text, "\n\n").to_string();
    let multi_space_re = regex::Regex::new(r"[ \t]{2,}").unwrap();
    text = multi_space_re.replace_all(&text, " ").to_string();

    text.trim().to_string()
}
