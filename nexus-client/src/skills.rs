/// 职责边界：
/// 1. 负责在本地扫描 Skill 目录、解析 SKILL.md frontmatter、生成 SkillSummary。
/// 2. 支持热加载检测（mtime 缓存）。
/// 3. 提供 Skill 原文读取（供 Agent 用 read_file 自行读取）。
///
/// Skill 不是工具，不通过 RegisterTools.schemas 注册。
/// Skill 通过 RegisterTools.skill_summaries 发送给 Server（name + description + always）。
///
/// 目录结构：workspace/skills/{name}/SKILL.md
///
/// 参考 nanobot：
/// - `nanobot/nanobot/agent/skills.py` 的 `SkillsLoader`。

use regex::Regex;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::sync::LazyLock;
use std::time::SystemTime;

use nexus_common::protocol::SkillFull;

/// SKILL.md frontmatter 解析正则（预编译）
static FRONTMATTER_RE: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"(?s)^---\n(.*?)\n---").expect("invalid frontmatter regex"));

/// Skill 元数据（从 SKILL.md frontmatter 解析）
#[derive(Debug, Clone)]
pub struct SkillMeta {
    pub name: String,
    pub description: String,
    pub always: bool,
    /// 所需二进制命令
    pub required_bins: Vec<String>,
    /// 所需环境变量
    pub required_env: Vec<String>,
}

/// Skill 信息
#[derive(Debug, Clone)]
pub struct Skill {
    pub meta: SkillMeta,
    /// SKILL.md 文件路径
    pub path: PathBuf,
    /// 最后修改时间（用于热加载检测）
    pub mtime: SystemTime,
}

/// 解析 SKILL.md 的 YAML frontmatter。
fn parse_frontmatter(content: &str) -> Option<SkillMeta> {
    let caps = FRONTMATTER_RE.captures(content)?;
    let yaml_block = caps.get(1)?.as_str();

    let mut name = None;
    let mut description = None;
    let mut always = false;
    let mut required_bins = Vec::new();
    let mut required_env = Vec::new();

    for line in yaml_block.lines() {
        let line = line.trim();
        if let Some(val) = line.strip_prefix("name:") {
            name = Some(val.trim().to_string());
        } else if let Some(val) = line.strip_prefix("description:") {
            description = Some(val.trim().to_string());
        } else if line == "always: true" || line == "always: true," {
            always = true;
        } else if let Some(val) = line.strip_prefix("requires:") {
            // 解析 requires 块（简化处理）
            let _ = val;
        } else if let Some(val) = line.strip_prefix("  bins:") {
            // 解析 bins
            let _ = val;
        } else if let Some(val) = line.strip_prefix("  env:") {
            // 解析 env
            let _ = val;
        } else if line.starts_with("- ") && !line.contains(":") {
            // 可能是 requires.bins 或 requires.env 中的项
            let val = line.trim_start_matches("- ").trim();
            if val.contains('/') || val.contains('\\') || val.is_empty() {
                continue;
            }
            // 简单判断：是否看起来像环境变量名
            if val.chars().all(|c| c.is_ascii_uppercase() || c == '_') {
                required_env.push(val.to_string());
            } else {
                required_bins.push(val.to_string());
            }
        }
    }

    let name = name?;
    let description = description.unwrap_or_default();

    Some(SkillMeta {
        name,
        description,
        always,
        required_bins,
        required_env,
    })
}

/// 扫描 skills 目录，返回所有 SkillFull。
///
/// always=false: content = None（正文由 Agent 自行 read_file）
/// always=true:  content = Some(正文)
pub fn scan_skills(skills_dir: &Path) -> Vec<SkillFull> {
    let mut skills = Vec::new();

    let entries = match fs::read_dir(skills_dir) {
        Ok(e) => e,
        Err(e) => {
            tracing::warn!("failed to read skills directory '{}': {}", skills_dir.display(), e);
            return skills;
        }
    };

    for entry in entries.flatten() {
        let path = entry.path();
        if !path.is_dir() {
            continue;
        }

        let skill_md = path.join("SKILL.md");
        if !skill_md.is_file() {
            continue;
        }

        let content = match fs::read_to_string(&skill_md) {
            Ok(c) => c,
            Err(e) => {
                tracing::warn!("failed to read '{}': {}", skill_md.display(), e);
                continue;
            }
        };

        let meta = match parse_frontmatter(&content) {
            Some(m) => m,
            None => {
                tracing::warn!("failed to parse frontmatter from '{}'", skill_md.display());
                continue;
            }
        };

        let _mtime = entry
            .metadata()
            .and_then(|m| m.modified())
            .unwrap_or(SystemTime::UNIX_EPOCH);

        // always=true 时，content 参与 hash 且发送给服务端；always=false 时不读正文
        let skill_content = if meta.always {
            // 去掉 frontmatter，只保留正文
            let body = content
                .split_once("---")
                .and_then(|(_, rest)| rest.split_once("---").map(|(_, body)| body.trim()));
            body.map(String::from)
        } else {
            None
        };

        tracing::debug!(
            "discovered skill: {} (always={}) at {}",
            meta.name,
            meta.always,
            skill_md.display()
        );

        skills.push(SkillFull {
            name: meta.name,
            description: meta.description,
            always: meta.always,
            content: skill_content,
        });
    }

    skills
}

/// 读取指定 skill 的 SKILL.md 原文。
pub fn read_skill_content(skills_dir: &Path, name: &str) -> Option<String> {
    let skill_md = skills_dir.join(name).join("SKILL.md");
    fs::read_to_string(&skill_md).ok()
}

/// 检查 skill 的依赖是否满足（bins 和 env）。
pub fn check_requirements(skills_dir: &Path, name: &str) -> bool {
    let skill_md = skills_dir.join(name).join("SKILL.md");
    let content = match fs::read_to_string(&skill_md) {
        Ok(c) => c,
        Err(_) => return false,
    };

    let meta = match parse_frontmatter(&content) {
        Some(m) => m,
        None => return false,
    };

    // 检查所需的二进制命令
    for bin in &meta.required_bins {
        if !is_bin_available(bin) {
            tracing::warn!("skill '{}' requires bin '{}' which is not available", name, bin);
            return false;
        }
    }

    // 检查所需的环境变量
    for env_var in &meta.required_env {
        if std::env::var(env_var).is_err() {
            tracing::warn!(
                "skill '{}' requires env '{}' which is not set",
                name,
                env_var
            );
            return false;
        }
    }

    true
}

/// 检查命令是否可用（通过 which 或 command -v）。
fn is_bin_available(bin: &str) -> bool {
    #[cfg(windows)]
    {
        let out = std::process::Command::new("where")
            .arg(bin)
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status();
        out.is_ok()
    }
    #[cfg(not(windows))]
    {
        let out = std::process::Command::new("which")
            .arg(bin)
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status();
        out.is_ok()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_frontmatter_basic() {
        let content = r#"---
name: code_review
description: A skill for reviewing code
always: false
---

# Code Review Skill

This is the skill content.
"#;
        let meta = parse_frontmatter(content).unwrap();
        assert_eq!(meta.name, "code_review");
        assert_eq!(meta.description, "A skill for reviewing code");
        assert!(!meta.always);
    }

    #[test]
    fn test_parse_frontmatter_always_true() {
        let content = r#"---
name: git_helper
description: Git helper
always: true
---

# Git Helper
"#;
        let meta = parse_frontmatter(content).unwrap();
        assert_eq!(meta.name, "git_helper");
        assert!(meta.always);
    }

    #[test]
    fn test_parse_frontmatter_invalid() {
        let content = "No frontmatter here";
        assert!(parse_frontmatter(content).is_none());
    }
}
