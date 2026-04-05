use async_trait::async_trait;
use nexus_common::error::{ErrorCode, NexusError};
use serde_json::{json, Value};
use std::sync::Arc;

use super::{ServerTool, ServerToolResult};
use crate::state::AppState;

// ── cron_create ──

pub struct CronCreateTool;

#[async_trait]
impl ServerTool for CronCreateTool {
    fn name(&self) -> &str { "cron_create" }

    fn schema(&self) -> Value {
        json!({
            "type": "function",
            "function": {
                "name": "cron_create",
                "description": "Create a scheduled task that fires at a specified time or interval. The task message will be processed by the agent when triggered.",
                "parameters": {
                    "type": "object",
                    "properties": {
                        "message": {
                            "type": "string",
                            "description": "The instruction/prompt for the agent to execute when the job fires."
                        },
                        "cron_expr": {
                            "type": "string",
                            "description": "Standard cron expression (e.g., '0 9 * * *' for 9am daily). Mutually exclusive with every_seconds and at."
                        },
                        "every_seconds": {
                            "type": "integer",
                            "description": "Recurring interval in seconds. Mutually exclusive with cron_expr and at."
                        },
                        "at": {
                            "type": "string",
                            "description": "ISO 8601 datetime for one-time execution (e.g., '2026-04-05T09:00:00Z'). Mutually exclusive with cron_expr and every_seconds."
                        },
                        "timezone": {
                            "type": "string",
                            "description": "IANA timezone (e.g., 'Asia/Shanghai'). Defaults to 'UTC'."
                        },
                        "channel": {
                            "type": "string",
                            "description": "The channel to deliver results to (e.g., 'discord')."
                        },
                        "chat_id": {
                            "type": "string",
                            "description": "The chat ID to deliver results to."
                        }
                    },
                    "required": ["message", "channel", "chat_id"]
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
        event_channel: &str,
        event_chat_id: &str,
    ) -> Result<ServerToolResult, NexusError> {
        let message = arguments.get("message").and_then(|v| v.as_str())
            .ok_or_else(|| NexusError::new(ErrorCode::ToolInvalidParams, "missing message"))?.to_string();
        // Default to current session's channel/chat_id for delivery
        let channel = arguments.get("channel").and_then(|v| v.as_str())
            .unwrap_or(event_channel).to_string();
        let chat_id = arguments.get("chat_id").and_then(|v| v.as_str())
            .unwrap_or(event_chat_id).to_string();
        let cron_expr = arguments.get("cron_expr").and_then(|v| v.as_str()).map(|s| s.to_string());
        let every_seconds = arguments.get("every_seconds").and_then(|v| v.as_i64()).map(|v| v as i32);
        let at = arguments.get("at").and_then(|v| v.as_str()).map(|s| s.to_string());
        let timezone = arguments.get("timezone").and_then(|v| v.as_str())
            .unwrap_or("UTC").to_string();

        // Validate: exactly one schedule type
        let schedule_count = [cron_expr.is_some(), every_seconds.is_some(), at.is_some()]
            .iter().filter(|&&v| v).count();
        if schedule_count == 0 {
            return Err(NexusError::new(ErrorCode::ToolInvalidParams, "must provide one of: cron_expr, every_seconds, or at"));
        }
        if schedule_count > 1 {
            return Err(NexusError::new(ErrorCode::ToolInvalidParams, "only one of cron_expr, every_seconds, or at may be set"));
        }
        // Validate at is parseable
        if let Some(ref at_str) = at {
            chrono::DateTime::parse_from_rfc3339(at_str)
                .map_err(|_| NexusError::new(ErrorCode::ToolInvalidParams, format!("invalid ISO 8601 datetime: {}", at_str)))?;
        }

        let name = message.chars().take(50).collect::<String>();
        let delete_after_run = at.is_some(); // one-shot jobs auto-delete

        let job_id = crate::db::create_cron_job(
            &state.db, user_id, &name,
            cron_expr.as_deref(), every_seconds, at.as_deref(),
            &timezone, &message, &channel, &chat_id, delete_after_run,
        ).await.map_err(|e| NexusError::new(ErrorCode::ExecutionFailed, format!("failed to create cron job: {}", e)))?;

        Ok(ServerToolResult {
            output: format!("Cron job created: id={}, name='{}'", job_id, name),
            media: vec![],
        })
    }
}

// ── cron_list ──

pub struct CronListTool;

#[async_trait]
impl ServerTool for CronListTool {
    fn name(&self) -> &str { "cron_list" }

    fn schema(&self) -> Value {
        json!({
            "type": "function",
            "function": {
                "name": "cron_list",
                "description": "List all scheduled cron jobs for the current user.",
                "parameters": {
                    "type": "object",
                    "properties": {},
                    "required": []
                }
            }
        })
    }

    async fn execute(
        &self,
        state: &Arc<AppState>,
        user_id: &str,
        _session_id: &str,
        _arguments: Value,
        _event_channel: &str,
        _event_chat_id: &str,
    ) -> Result<ServerToolResult, NexusError> {
        let jobs = crate::db::list_cron_jobs(&state.db, user_id).await
            .map_err(|e| NexusError::new(ErrorCode::ExecutionFailed, format!("failed to list cron jobs: {}", e)))?;

        if jobs.is_empty() {
            return Ok(ServerToolResult {
                output: "No cron jobs found.".into(),
                media: vec![],
            });
        }

        let output = serde_json::to_string_pretty(&jobs)
            .unwrap_or_else(|_| format!("{} jobs found", jobs.len()));

        Ok(ServerToolResult { output, media: vec![] })
    }
}

// ── cron_remove ──

pub struct CronRemoveTool;

#[async_trait]
impl ServerTool for CronRemoveTool {
    fn name(&self) -> &str { "cron_remove" }

    fn schema(&self) -> Value {
        json!({
            "type": "function",
            "function": {
                "name": "cron_remove",
                "description": "Remove a scheduled cron job by its ID.",
                "parameters": {
                    "type": "object",
                    "properties": {
                        "job_id": {
                            "type": "string",
                            "description": "The ID of the cron job to remove."
                        }
                    },
                    "required": ["job_id"]
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
        let job_id = arguments.get("job_id").and_then(|v| v.as_str())
            .ok_or_else(|| NexusError::new(ErrorCode::ToolInvalidParams, "missing job_id"))?;

        let deleted = crate::db::delete_cron_job(&state.db, user_id, job_id).await
            .map_err(|e| NexusError::new(ErrorCode::ExecutionFailed, format!("failed to delete cron job: {}", e)))?;

        if deleted {
            Ok(ServerToolResult {
                output: format!("Cron job '{}' removed.", job_id),
                media: vec![],
            })
        } else {
            Err(NexusError::new(ErrorCode::ToolNotFound, format!("Cron job '{}' not found or not owned by you.", job_id)))
        }
    }
}
