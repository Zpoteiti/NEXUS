use async_trait::async_trait;
use nexus_common::error::{ErrorCode, NexusError};
use serde_json::{json, Value};
use std::sync::Arc;

use super::{ServerTool, ServerToolResult};
use crate::state::AppState;

pub struct ReadSkillTool;

#[async_trait]
impl ServerTool for ReadSkillTool {
    fn name(&self) -> &str { "read_skill" }

    fn schema(&self) -> Value {
        json!({
            "type": "function",
            "function": {
                "name": "read_skill",
                "description": "Read the full instructions for a skill. Use when you need detailed guidance on how to perform a specific task.",
                "parameters": {
                    "type": "object",
                    "properties": {
                        "name": {
                            "type": "string",
                            "description": "The skill name"
                        }
                    },
                    "required": ["name"]
                }
            }
        })
    }

    async fn execute(
        &self,
        state: &Arc<AppState>,
        user_id: &str,
        _session_id: &str,
        arguments: Value,
        _event_channel: &str,
        _event_chat_id: &str,
    ) -> Result<ServerToolResult, NexusError> {
        let name = arguments.get("name")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_string();
        if name.is_empty() {
            return Err(NexusError::new(ErrorCode::ToolInvalidParams, "read_skill: name is required"));
        }

        let skill = crate::db::get_skill(&state.db, user_id, &name)
            .await
            .map_err(|e| NexusError::new(ErrorCode::ExecutionFailed, format!("failed to look up skill: {}", e)))?
            .ok_or_else(|| NexusError::new(ErrorCode::ToolNotFound, format!("skill '{}' not found", name)))?;

        // Read SKILL.md from the filesystem
        let skill_md_path = std::path::Path::new(&skill.skill_path).join("SKILL.md");
        let content = tokio::fs::read_to_string(&skill_md_path)
            .await
            .map_err(|e| NexusError::new(ErrorCode::ExecutionFailed, format!("failed to read SKILL.md for '{}': {}", name, e)))?;

        // Strip frontmatter — return only the body
        let body = strip_frontmatter(&content);

        Ok(ServerToolResult {
            output: body,
            media: vec![],
        })
    }
}

/// Strip YAML frontmatter (between `---` markers) from content, returning only the body.
pub fn strip_frontmatter(content: &str) -> String {
    let trimmed = content.trim_start();
    if !trimmed.starts_with("---") {
        return content.to_string();
    }
    // Find the closing ---
    if let Some(end) = trimmed[3..].find("\n---") {
        let after = &trimmed[3 + end + 4..]; // skip past "\n---"
        after.trim_start_matches('\n').to_string()
    } else {
        content.to_string()
    }
}

/// Parse YAML frontmatter from SKILL.md content.
/// Returns (name, description, always_on).
pub fn parse_frontmatter(content: &str) -> (Option<String>, String, bool) {
    let trimmed = content.trim_start();
    if !trimmed.starts_with("---") {
        return (None, String::new(), false);
    }

    let rest = &trimmed[3..];
    let end = match rest.find("\n---") {
        Some(pos) => pos,
        None => return (None, String::new(), false),
    };

    let frontmatter = &rest[..end];

    let mut name: Option<String> = None;
    let mut description = String::new();
    let mut always = false;

    for line in frontmatter.lines() {
        let line = line.trim();
        if let Some(val) = line.strip_prefix("name:") {
            name = Some(val.trim().trim_matches('"').trim_matches('\'').to_string());
        } else if let Some(val) = line.strip_prefix("description:") {
            description = val.trim().trim_matches('"').trim_matches('\'').to_string();
        } else if let Some(val) = line.strip_prefix("always:") {
            let val = val.trim().to_lowercase();
            always = val == "true" || val == "yes";
        }
    }

    (name, description, always)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_frontmatter_basic() {
        let content = "---\nname: weather\ndescription: Get weather forecasts\nalways: true\n---\n\n# Weather\nSome content";
        let (name, desc, always) = parse_frontmatter(content);
        assert_eq!(name, Some("weather".to_string()));
        assert_eq!(desc, "Get weather forecasts");
        assert!(always);
    }

    #[test]
    fn test_parse_frontmatter_quoted() {
        let content = "---\nname: \"my-skill\"\ndescription: 'A cool skill'\nalways: false\n---\nBody";
        let (name, desc, always) = parse_frontmatter(content);
        assert_eq!(name, Some("my-skill".to_string()));
        assert_eq!(desc, "A cool skill");
        assert!(!always);
    }

    #[test]
    fn test_parse_frontmatter_missing() {
        let content = "# No frontmatter\nJust content";
        let (name, desc, always) = parse_frontmatter(content);
        assert!(name.is_none());
        assert_eq!(desc, "");
        assert!(!always);
    }

    #[test]
    fn test_strip_frontmatter() {
        let content = "---\nname: test\n---\n\n# Body\nHello";
        let body = strip_frontmatter(content);
        assert_eq!(body, "# Body\nHello");
    }

    #[test]
    fn test_strip_frontmatter_no_frontmatter() {
        let content = "# Just content";
        let body = strip_frontmatter(content);
        assert_eq!(body, "# Just content");
    }
}
