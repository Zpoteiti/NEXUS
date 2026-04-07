//! Cron scheduler — polls DB for due jobs and injects prompts into agent loop via bus.

use std::sync::Arc;
use tokio::time::{Duration, sleep};
use tracing::{info, warn};

use crate::bus::InboundEvent;
use crate::state::AppState;

/// Runs the cron scheduler loop. Checks for due jobs every 10 seconds.
pub async fn run_cron_scheduler(state: Arc<AppState>) {
    let poll_interval = Duration::from_secs(10);
    info!("cron scheduler started");

    loop {
        sleep(poll_interval).await;

        match crate::db::get_due_cron_jobs(&state.db).await {
            Ok(jobs) => {
                for job in jobs {
                    info!("cron: firing job '{}' ({})", job.name, job.job_id);

                    let session_id = format!("cron:{}", job.job_id);
                    let reminder = format!(
                        "[Scheduled Task] Timer finished.\n\n\
                         Task '{}' has been triggered.\n\
                         Scheduled instruction: {}",
                        job.name, job.message
                    );

                    let event = InboundEvent {
                        channel: job.channel.clone(),
                        sender_id: job.user_id.clone(),
                        chat_id: job.chat_id.clone(),
                        content: reminder,
                        session_id,
                        timestamp: Some(chrono::Utc::now()),
                        media: vec![],
                        metadata: {
                            let mut m = std::collections::HashMap::new();
                            m.insert("cron_job_id".into(), serde_json::json!(job.job_id));
                            m
                        },
                    };

                    state.bus.publish_inbound(event).await;

                    if let Err(e) = crate::db::update_cron_job_after_run(
                        &state.db, &job.job_id, job.delete_after_run
                    ).await {
                        warn!("cron: failed to update job after run: {}", e);
                    }
                }
            }
            Err(e) => {
                warn!("cron: failed to query due jobs: {}", e);
            }
        }
    }
}
