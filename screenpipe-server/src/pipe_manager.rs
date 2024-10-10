use anyhow::Result;
use screenpipe_core::download_pipe;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::future::Future;
use std::path::PathBuf;
use tokio::fs::File;
use tokio::io::AsyncWriteExt;
use tracing::{debug, info, warn};

#[derive(Clone, Serialize, Deserialize, Debug)]
pub struct PipeInfo {
    pub id: String,
    pub enabled: bool,
    pub config: Value,
    pub source: String,
}

pub struct PipeManager {
    screenpipe_dir: PathBuf,
}

impl PipeManager {
    pub fn new(screenpipe_dir: PathBuf) -> Self {
        PipeManager { screenpipe_dir }
    }

    pub async fn start_pipe(
        &self,
        id: &str,
    ) -> Result<impl Future<Output = Result<(), anyhow::Error>>> {
        let pipes = self.list_pipes().await;

        if let Some(_) = pipes.iter().find(|pipe| pipe.id == id) {
            let pipe_id = id.to_string();
            let screenpipe_dir = self.screenpipe_dir.clone();

            let future = async move { screenpipe_core::run_pipe(&pipe_id, screenpipe_dir).await };

            self.update_config(
                id,
                serde_json::json!({
                    "enabled": true,
                }),
            )
            .await?;

            Ok(future)
        } else {
            Err(anyhow::anyhow!("pipe not found"))
        }
    }

    pub async fn update_config(&self, id: &str, new_config: Value) -> Result<()> {
        debug!("Updating config for pipe: {}", id);
        let pipe_dir = self.screenpipe_dir.join("pipes").join(id);

        if !pipe_dir.exists() {
            return Err(anyhow::anyhow!("pipe '{}' does not exist", id));
        }

        let config_path = pipe_dir.join("pipe.json");

        let mut config: Value = if config_path.exists() {
            let config_str = tokio::fs::read_to_string(&config_path).await?;
            serde_json::from_str(&config_str)?
        } else {
            tokio::fs::create_dir_all(&pipe_dir).await?;
            serde_json::json!({
                "id": id,
                "enabled": false
            })
        };

        if let Value::Object(existing_config) = &mut config {
            if let Value::Object(updates) = new_config {
                for (key, value) in updates {
                    existing_config.insert(key, value);
                }
            } else {
                return Err(anyhow::anyhow!("new configuration must be an object"));
            }
        } else {
            return Err(anyhow::anyhow!("existing configuration is not an object"));
        }

        let updated_config_str = serde_json::to_string_pretty(&config)?;

        let mut file = File::create(&config_path).await?;
        file.write_all(updated_config_str.as_bytes()).await?;

        Ok(())
    }

    pub async fn get_pipe_info(&self, id: &str) -> Option<PipeInfo> {
        let pipes = self.list_pipes().await;
        pipes.iter().find(|pipe| pipe.id == id).cloned()
    }

    async fn load_pipe_info(pipe_id: String, config_path: PathBuf) -> PipeInfo {
        let config = tokio::fs::read_to_string(&config_path)
            .await
            .and_then(|s| serde_json::from_str::<Value>(&s).map_err(Into::into))
            .unwrap_or_else(|_| {
                warn!("pipe {}: does not seem to have a config file", pipe_id);
                Value::Null
            });

        PipeInfo {
            id: pipe_id,
            enabled: config
                .get("enabled")
                .and_then(Value::as_bool)
                .unwrap_or(false),
            source: config
                .get("source")
                .unwrap_or(&Value::Null)
                .as_str()
                .unwrap_or("")
                .to_string(),
            config,
        }
    }
    pub async fn list_pipes(&self) -> Vec<PipeInfo> {
        let pipe_dir = self.screenpipe_dir.join("pipes");
        let mut pipe_infos = Vec::new();

        if let Ok(mut entries) = tokio::fs::read_dir(pipe_dir).await {
            while let Ok(Some(entry)) = entries.next_entry().await {
                let pipe_id = entry.file_name().to_string_lossy().into_owned();
                let config_path = entry.path().join("pipe.json");
                pipe_infos.push(Self::load_pipe_info(pipe_id, config_path));
            }
        }

        futures::future::join_all(pipe_infos).await
    }

    pub async fn download_pipe(&self, url: &str) -> Result<String> {
        // Remove any surrounding quotes and normalize backslashes
        let normalized_url = url.trim_matches('"').replace("\\", "/");

        let pipe_dir = download_pipe(&normalized_url, self.screenpipe_dir.clone()).await?;

        // update the config with the source url
        self.update_config(
            &pipe_dir.file_name().unwrap().to_string_lossy(),
            serde_json::json!({
                "source": normalized_url,
            }),
        )
        .await?;

        info!(
            "pipe {} downloaded",
            pipe_dir.file_name().unwrap().to_string_lossy()
        );

        Ok(pipe_dir.file_name().unwrap().to_string_lossy().into_owned())
    }

    pub async fn purge_pipes(&self) -> Result<()> {
        let pipe_dir = self.screenpipe_dir.join("pipes");
        tokio::fs::remove_dir_all(pipe_dir).await?;
        Ok(())
    }
}
