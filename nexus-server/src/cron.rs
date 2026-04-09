//! Cron scheduler: polls DB every 10s for due jobs, injects into message bus.

use crate::bus::{self, InboundEvent};
use crate::state::AppState;
use nexus_common::consts::CRON_POLL_INTERVAL_SEC;
use std::sync::Arc;
use tracing::{error, info, warn};

/// Spawn the cron poller background task.
pub fn spawn_cron_poller(state: Arc<AppState>) {
    tokio::spawn(async move {
        let mut interval =
            tokio::time::interval(std::time::Duration::from_secs(CRON_POLL_INTERVAL_SEC));
        loop {
            interval.tick().await;
            if let Err(e) = poll_and_execute(&state).await {
                warn!("Cron poll error: {e}");
            }
        }
    });
}

async fn poll_and_execute(state: &Arc<AppState>) -> Result<(), String> {
    let due_jobs = crate::db::cron::find_due_jobs(&state.db)
        .await
        .map_err(|e| format!("Query due jobs: {e}"))?;

    for job in due_jobs {
        info!("Cron firing: {} [{}]", job.name, job.job_id);

        // Create InboundEvent
        let event = InboundEvent {
            session_id: format!("cron:{}", job.job_id),
            user_id: job.user_id.clone(),
            content: job.message.clone(),
            channel: job.channel.clone(),
            chat_id: Some(job.chat_id.clone()),
            sender_id: None,
            media: vec![],
            cron_job_id: Some(job.job_id.clone()),
            identity: None, // Cron = system-triggered, always partner context
            metadata: Default::default(),
        };

        // Publish to bus (rate limit exempt via cron_job_id)
        if let Err(e) = bus::publish_inbound(state, event).await {
            error!("Cron publish failed for {}: {e}", job.job_id);
        }

        // Update job state
        let next_run = compute_next_run(&job);

        if job.delete_after_run {
            let _ = crate::db::cron::delete_job(&state.db, &job.job_id, &job.user_id).await;
        } else if next_run.is_none() {
            // at-mode: disable instead of delete
            let _ = crate::db::cron::disable_job(&state.db, &job.job_id).await;
        } else {
            let _ = crate::db::cron::update_after_run(&state.db, &job.job_id, next_run).await;
        }
    }

    Ok(())
}

fn compute_next_run(job: &crate::db::cron::CronJob) -> Option<chrono::DateTime<chrono::Utc>> {
    if let Some(ref expr) = job.cron_expr {
        crate::server_tools::cron_tool::compute_next_cron_pub(expr, &job.timezone).ok()
    } else if let Some(secs) = job.every_seconds {
        Some(chrono::Utc::now() + chrono::Duration::seconds(secs as i64))
    } else {
        None
    }
}
