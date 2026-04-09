//! read_skill and install_skill server tools.

use crate::state::AppState;
use serde_json::Value;
use std::sync::Arc;

pub async fn read_skill(state: &Arc<AppState>, user_id: &str, args: &Value) -> (i32, String) {
    let skill_name = match args.get("skill_name").and_then(Value::as_str) {
        Some(n) => n,
        None => return (1, "Missing required parameter: skill_name".into()),
    };

    let skill_path = format!("{}/{user_id}/{skill_name}", state.config.skills_dir);
    let md_path = format!("{skill_path}/SKILL.md");

    let content = match tokio::fs::read_to_string(&md_path).await {
        Ok(c) => c,
        Err(_) => return (1, format!("Skill '{skill_name}' not found.")),
    };

    // Check if skill directory has additional files
    let mut has_extras = false;
    if let Ok(mut entries) = tokio::fs::read_dir(&skill_path).await {
        while let Ok(Some(entry)) = entries.next_entry().await {
            let name = entry.file_name().to_string_lossy().to_string();
            if name != "SKILL.md" {
                has_extras = true;
                break;
            }
        }
    }

    let output = if has_extras {
        format!(
            "{content}\n\n[This skill has additional files at {skill_path}. \
             To use scripts or resources, use file_transfer(from_device='server', \
             file_path='{skill_path}/filename') to copy them to your target device.]"
        )
    } else {
        content
    };

    (0, output)
}

pub async fn install_skill(state: &Arc<AppState>, user_id: &str, args: &Value) -> (i32, String) {
    let repo = match args.get("repo").and_then(Value::as_str) {
        Some(r) => r,
        None => return (1, "Missing required parameter: repo (owner/repo)".into()),
    };
    let branch = args.get("branch").and_then(Value::as_str).unwrap_or("main");

    let url = format!("https://raw.githubusercontent.com/{repo}/{branch}/SKILL.md");

    // Fetch SKILL.md from GitHub
    let resp = match state.http_client.get(&url).send().await {
        Ok(r) => r,
        Err(e) => return (1, format!("Failed to fetch {url}: {e}")),
    };

    if !resp.status().is_success() {
        return (1, format!("HTTP {}: {url}", resp.status()));
    }

    let content = match resp.text().await {
        Ok(t) => t,
        Err(e) => return (1, format!("Read response: {e}")),
    };

    // Parse YAML frontmatter
    let (name, description, always_on) = match parse_skill_frontmatter(&content) {
        Ok(meta) => meta,
        Err(e) => return (1, format!("Invalid SKILL.md: {e}")),
    };

    // Write to disk
    let skill_dir = format!("{}/{user_id}/{name}", state.config.skills_dir);
    if let Err(e) = tokio::fs::create_dir_all(&skill_dir).await {
        return (1, format!("Create skill dir: {e}"));
    }
    if let Err(e) = tokio::fs::write(format!("{skill_dir}/SKILL.md"), &content).await {
        return (1, format!("Write SKILL.md: {e}"));
    }

    // Upsert in DB
    let skill_id = uuid::Uuid::new_v4().to_string();
    match crate::db::skills::upsert_skill(
        &state.db,
        &skill_id,
        user_id,
        &name,
        &description,
        always_on,
        &skill_dir,
    )
    .await
    {
        Ok(()) => (
            0,
            format!(
                "Skill '{name}' installed from {repo}.\nDescription: {description}\nAlways-on: {always_on}"
            ),
        ),
        Err(e) => (1, format!("DB upsert: {e}")),
    }
}

/// Parse YAML frontmatter from SKILL.md content.
/// Expected format:
/// ---
/// name: My Skill
/// description: Does stuff
/// always_on: false
/// ---
fn parse_skill_frontmatter(content: &str) -> Result<(String, String, bool), String> {
    let content = content.trim();
    if !content.starts_with("---") {
        return Err("Missing YAML frontmatter (must start with ---)".into());
    }

    let rest = &content[3..];
    let end = rest
        .find("---")
        .ok_or("Missing closing --- in frontmatter")?;
    let frontmatter = &rest[..end];

    let mut name = String::new();
    let mut description = String::new();
    let mut always_on = false;

    for line in frontmatter.lines() {
        let line = line.trim();
        if let Some(val) = line.strip_prefix("name:") {
            name = val.trim().to_string();
        } else if let Some(val) = line.strip_prefix("description:") {
            description = val.trim().to_string();
        } else if let Some(val) = line.strip_prefix("always_on:") {
            always_on = val.trim() == "true";
        }
    }

    if name.is_empty() {
        return Err("Missing 'name' in frontmatter".into());
    }

    Ok((name, description, always_on))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_frontmatter() {
        let content = "---\nname: Test Skill\ndescription: Does things\nalways_on: true\n---\n\nInstructions here";
        let (name, desc, ao) = parse_skill_frontmatter(content).unwrap();
        assert_eq!(name, "Test Skill");
        assert_eq!(desc, "Does things");
        assert!(ao);
    }

    #[test]
    fn test_parse_frontmatter_minimal() {
        let content = "---\nname: Minimal\n---\nContent";
        let (name, desc, ao) = parse_skill_frontmatter(content).unwrap();
        assert_eq!(name, "Minimal");
        assert_eq!(desc, "");
        assert!(!ao);
    }

    #[test]
    fn test_parse_frontmatter_missing_name() {
        let content = "---\ndescription: no name\n---\n";
        assert!(parse_skill_frontmatter(content).is_err());
    }
}
