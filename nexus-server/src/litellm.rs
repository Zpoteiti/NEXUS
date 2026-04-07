//! LiteLLM proxy manager — creates venv, installs litellm, spawns/manages the process.

use std::path::PathBuf;
use std::sync::Arc;
use tokio::process::{Child, Command};
use tokio::sync::RwLock;
use tracing::{error, info, warn};

pub struct LiteLlmManager {
    venv_dir: PathBuf,
    port: u16,
    master_key: String,
    database_url: Option<String>,
    child: Arc<RwLock<Option<Child>>>,
}

impl LiteLlmManager {
    pub fn new(port: u16, database_url: Option<String>) -> Self {
        let venv_dir = dirs::home_dir()
            .unwrap_or_else(|| PathBuf::from("."))
            .join(".nexus")
            .join("litellm-venv");

        // Random master key for this session
        let raw = uuid::Uuid::new_v4().to_string().replace('-', "");
        let master_key = format!("sk-nexus-{}", &raw[..16]);

        Self {
            venv_dir,
            port,
            master_key,
            database_url,
            child: Arc::new(RwLock::new(None)),
        }
    }

    /// Ensure Python venv exists and litellm is installed
    pub async fn ensure_setup(&self) -> Result<(), String> {
        // Check Python available
        let python_check = Command::new("python3")
            .arg("--version")
            .output()
            .await
            .map_err(|_| {
                "Python 3 is required but not found. Install Python 3.8+ to continue.".to_string()
            })?;
        if !python_check.status.success() {
            return Err("Python 3 not found".into());
        }

        let venv_python = self.venv_dir.join("bin").join("python");
        if !venv_python.exists() {
            info!("Creating LiteLLM venv at {:?}...", self.venv_dir);
            let output = Command::new("python3")
                .args(["-m", "venv", self.venv_dir.to_str().unwrap()])
                .output()
                .await
                .map_err(|e| format!("failed to create venv: {}", e))?;
            if !output.status.success() {
                return Err(format!(
                    "venv creation failed: {}",
                    String::from_utf8_lossy(&output.stderr)
                ));
            }
        }

        // Check if litellm is installed
        let pip = self.venv_dir.join("bin").join("pip");
        let check = Command::new(&pip)
            .args(["show", "litellm"])
            .output()
            .await;
        if check.map(|o| !o.status.success()).unwrap_or(true) {
            info!("Installing litellm[proxy] (first time only, this may take a minute)...");
            let output = Command::new(&pip)
                .args(["install", "litellm[proxy]"])
                .output()
                .await
                .map_err(|e| format!("pip install failed: {}", e))?;
            if !output.status.success() {
                return Err(format!(
                    "litellm installation failed: {}",
                    String::from_utf8_lossy(&output.stderr)
                ));
            }
            info!("LiteLLM installed successfully");
        }

        Ok(())
    }

    /// Start the LiteLLM process
    pub async fn start(&self) -> Result<(), String> {
        let litellm_bin = self.venv_dir.join("bin").join("litellm");

        let mut cmd = Command::new(&litellm_bin);
        cmd.args(["--port", &self.port.to_string()])
            .args(["--host", "127.0.0.1"])
            .env("LITELLM_MASTER_KEY", &self.master_key);

        if let Some(ref db_url) = self.database_url {
            cmd.env("DATABASE_URL", db_url);
        }

        // Suppress litellm's verbose output
        cmd.stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::piped());

        let child = cmd
            .spawn()
            .map_err(|e| format!("failed to spawn litellm: {}", e))?;

        info!("LiteLLM proxy starting on port {}...", self.port);
        *self.child.write().await = Some(child);

        // Wait for health check
        self.wait_for_ready().await?;
        info!("LiteLLM proxy ready on localhost:{}", self.port);

        Ok(())
    }

    /// Wait until LiteLLM responds to health check
    async fn wait_for_ready(&self) -> Result<(), String> {
        let url = format!("http://127.0.0.1:{}/health", self.port);
        let client = reqwest::Client::new();

        for i in 0..30 {
            // 30 seconds timeout
            tokio::time::sleep(std::time::Duration::from_secs(1)).await;
            if let Ok(resp) = client.get(&url).send().await {
                if resp.status().is_success() {
                    return Ok(());
                }
            }
            if i % 5 == 4 {
                info!("Waiting for LiteLLM to start... ({}s)", i + 1);
            }
        }
        Err("LiteLLM failed to start within 30 seconds".into())
    }

    /// Add a model to LiteLLM via REST API
    pub async fn add_model(
        &self,
        provider: &str,
        model: &str,
        api_key: &str,
        api_base: Option<&str>,
    ) -> Result<(), String> {
        let url = format!("http://127.0.0.1:{}/model/new", self.port);
        let client = reqwest::Client::new();

        let litellm_model = format!("{}/{}", provider, model);
        let mut litellm_params = serde_json::json!({
            "model": litellm_model,
            "api_key": api_key,
        });
        if let Some(base) = api_base {
            litellm_params["api_base"] = serde_json::json!(base);
        }

        let body = serde_json::json!({
            "model_name": "default",
            "litellm_params": litellm_params,
        });

        let resp = client
            .post(&url)
            .header("Authorization", format!("Bearer {}", self.master_key))
            .json(&body)
            .send()
            .await
            .map_err(|e| format!("failed to add model: {}", e))?;

        if !resp.status().is_success() {
            let text = resp.text().await.unwrap_or_default();
            return Err(format!("LiteLLM add model failed: {}", text));
        }

        info!(
            "LiteLLM model added: {} ({}/{})",
            "default", provider, model
        );
        Ok(())
    }

    /// Delete a model from LiteLLM (for future use: config change hot-reload)
    #[allow(dead_code)]
    pub async fn delete_model(&self, model_id: &str) -> Result<(), String> {
        let url = format!("http://127.0.0.1:{}/model/delete", self.port);
        let client = reqwest::Client::new();

        let resp = client
            .post(&url)
            .header("Authorization", format!("Bearer {}", self.master_key))
            .json(&serde_json::json!({"id": model_id}))
            .send()
            .await
            .map_err(|e| format!("failed to delete model: {}", e))?;

        if !resp.status().is_success() {
            // Ignore delete errors -- model might not exist
            warn!(
                "LiteLLM delete model warning: {}",
                resp.text().await.unwrap_or_default()
            );
        }
        Ok(())
    }

    /// Get the OpenAI-compatible API base URL for the agent loop
    pub fn api_base(&self) -> String {
        format!("http://127.0.0.1:{}", self.port)
    }

    /// Get the master key (used as api_key when calling LiteLLM)
    pub fn api_key(&self) -> &str {
        &self.master_key
    }

    /// Stop the LiteLLM process
    pub async fn stop(&self) {
        if let Some(mut child) = self.child.write().await.take() {
            let _ = child.kill().await;
            info!("LiteLLM proxy stopped");
        }
    }

    /// Check if process is still running, restart if crashed (for future use: health monitoring)
    #[allow(dead_code)]
    pub async fn ensure_running(&self) -> bool {
        let mut guard = self.child.write().await;
        if let Some(ref mut child) = *guard {
            match child.try_wait() {
                Ok(None) => return true, // still running
                Ok(Some(status)) => {
                    warn!("LiteLLM exited with status: {}", status);
                }
                Err(e) => {
                    warn!("LiteLLM status check error: {}", e);
                }
            }
        }
        drop(guard);

        // Try to restart
        warn!("LiteLLM crashed, restarting...");
        match self.start().await {
            Ok(()) => true,
            Err(e) => {
                error!("LiteLLM restart failed: {}", e);
                false
            }
        }
    }
}
