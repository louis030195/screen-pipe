// screenpipe — AI that knows everything you've seen, said, or heard
// https://screenpipe.com

//! Pinned Orukeet files for audiopipe's existing Parakeet ONNX runtime.
//! Only installation accesses Hugging Face; cached startup and inference are offline.

use anyhow::{ensure, Context, Result};
use hf_hub::{api::sync::ApiBuilder, Cache, Repo, RepoType};
use std::path::{Path, PathBuf};
use std::sync::Mutex;
use std::time::{Duration, Instant};

const REVISION: &str = "e31d65f0e6aaeec1cd4e84ed55e52ceef6a75717";
const DIRECTORY: &str = "onnx/combined-v0.1.0-int8";
const FILES: &[&str] = &[
    "encoder-model.int8.onnx",
    "decoder_joint-model.int8.onnx",
    "vocab.txt",
    "LICENSE-WEIGHTS",
    "LICENSE-CONVERTER.txt",
    "NOTICE.md",
    "config.json",
];

fn repo() -> Repo {
    Repo::with_revision("oruk/orukeet".into(), RepoType::Model, REVISION.into())
}

fn validate_config(path: &Path) -> Result<()> {
    let config: serde_json::Value = serde_json::from_slice(&std::fs::read(path)?)?;
    ensure!(
        config["model_type"] == "nemo-conformer-tdt"
            && config["features_size"] == 128
            && config["subsampling_factor"] == 8,
        "Orukeet configuration is incompatible with the Parakeet runtime"
    );
    Ok(())
}

fn cached_in(cache: &Cache) -> Option<PathBuf> {
    let cache = cache.repo(repo());
    let mut directory = None;
    for file in FILES {
        let path = cache.get(&format!("{DIRECTORY}/{file}"))?;
        if !path.is_file() || path.metadata().ok()?.len() == 0 {
            return None;
        }
        directory = path.parent().map(Path::to_path_buf);
    }
    directory
}

pub fn cached_model_dir() -> Option<PathBuf> {
    cached_in(&Cache::default())
}

/// Blocking installer for CLI/evals; the recorder calls this in a background task.
/// The configuration is saved and consumed, so real installations use HF's normal
/// NeMo JSON download-counting path without adding a telemetry request.
pub fn download_model() -> Result<PathBuf> {
    let cache = Cache::default();
    if let Some(directory) = cached_in(&cache) {
        validate_config(&directory.join("config.json"))?;
        return Ok(directory);
    }
    let api = ApiBuilder::from_cache(cache).with_progress(false).build()?;
    let model = api.repo(repo());
    let mut directory = None;
    for file in FILES {
        let path = model
            .get(&format!("{DIRECTORY}/{file}"))
            .with_context(|| format!("download Orukeet {file}"))?;
        directory = path.parent().map(Path::to_path_buf);
    }
    let directory = directory.context("Orukeet model directory missing")?;
    validate_config(&directory.join("config.json"))?;
    Ok(directory)
}

pub(crate) fn load_cached() -> Result<Option<audiopipe::Model>> {
    let Some(directory) = cached_model_dir() else {
        return Ok(None);
    };
    validate_config(&directory.join("config.json"))?;
    audiopipe::Model::from_dir(&directory, "parakeet")
        .map(Some)
        .map_err(|e| anyhow::anyhow!("load Orukeet: {e}"))
}

// One download across microphone/system-audio refreshes, with bounded retries.
// Holding this owned state in an async task keeps capture startup non-blocking.
static DOWNLOAD: Mutex<(bool, Option<Instant>)> = Mutex::new((false, None));

pub(crate) fn spawn_download() {
    let Ok(mut state) = DOWNLOAD.lock() else {
        return;
    };
    if state.0
        || state
            .1
            .is_some_and(|last| last.elapsed() < Duration::from_secs(60))
    {
        return;
    }
    state.0 = true;
    drop(state);
    tokio::spawn(async {
        match tokio::task::spawn_blocking(download_model).await {
            Ok(Ok(_)) => tracing::info!("Orukeet model ready in Hugging Face cache"),
            result => tracing::warn!("Orukeet download failed: {result:?}"),
        }
        if let Ok(mut state) = DOWNLOAD.lock() {
            *state = (false, Some(Instant::now()));
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cache_requires_complete_pinned_installation() {
        let temp = tempfile::tempdir().unwrap();
        let cache = Cache::new(temp.path().to_path_buf());
        let root = temp.path().join("models--oruk--orukeet");
        std::fs::create_dir_all(root.join("refs")).unwrap();
        std::fs::write(root.join("refs").join(REVISION), REVISION).unwrap();
        let directory = root.join("snapshots").join(REVISION).join(DIRECTORY);
        std::fs::create_dir_all(&directory).unwrap();
        for file in &FILES[..FILES.len() - 1] {
            std::fs::write(directory.join(file), b"fixture").unwrap();
        }
        assert!(cached_in(&cache).is_none());
        let config = directory.join("config.json");
        std::fs::write(&config, b"").unwrap();
        assert!(cached_in(&cache).is_none());
        std::fs::write(
            &config,
            br#"{"model_type":"nemo-conformer-tdt","features_size":128,"subsampling_factor":8}"#,
        )
        .unwrap();
        assert_eq!(cached_in(&cache), Some(directory));
        validate_config(&config).unwrap();
        std::fs::write(root.join("refs").join(REVISION), "different-revision").unwrap();
        assert!(cached_in(&cache).is_none());
    }

    #[test]
    fn incompatible_or_malformed_configuration_is_rejected() {
        let temp = tempfile::NamedTempFile::new().unwrap();
        for content in [
            "<html>proxy login</html>",
            "{}",
            r#"{"model_type":"nemo-conformer-tdt","features_size":80,"subsampling_factor":8}"#,
        ] {
            std::fs::write(temp.path(), content).unwrap();
            assert!(validate_config(temp.path()).is_err());
        }
    }
}
