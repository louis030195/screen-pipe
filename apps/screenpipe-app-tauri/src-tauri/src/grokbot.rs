// screenpipe — AI that knows everything you've seen, said, or heard
// https://screenpipe.com

//! Install a private skill in Grok Bot's shared skill store. The Bun bridge
//! keeps gateway credentials outside the renderer and verifies every mutation.

use serde_json::{json, Value};
use std::{
    path::{Path, PathBuf},
    process::Stdio,
    time::Duration,
};
use tauri::AppHandle;
use tokio::io::AsyncWriteExt;

pub(crate) static GROKBOT_CONNECTION_LOCK: tokio::sync::Mutex<()> =
    tokio::sync::Mutex::const_new(());

const INSTALLER: &str = include_str!("../../lib/grokbot-installer.mjs");
const API_SKILL: &str =
    include_str!("../../../../crates/screenpipe-core/assets/skills/screenpipe-api/SKILL.md");

async fn run_bridge(
    app: &AppHandle,
    home: &Path,
    bun: &Path,
    action: &str,
) -> Result<Value, String> {
    let data_dir = crate::log_files::get_active_data_dir(app.clone()).await?;
    let api = crate::recording::local_api_context_from_app(app);
    let input = json!({ "home": home, "bun": bun, "dataDir": data_dir,
        "port": api.port, "skill": API_SKILL,
        "starterSkills": screenpipe_core::starter_skills::STARTER_SKILLS, "action": action });
    run_bridge_input(bun, &input).await
}

async fn run_bridge_input(bun: &Path, input: &Value) -> Result<Value, String> {
    let mut command = tokio::process::Command::new(bun);
    command
        .args(["-e", INSTALLER, "--", "--screenpipe-grokbot-installer"])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .kill_on_drop(true);
    #[cfg(target_os = "windows")]
    command.creation_flags(0x08000000); // CREATE_NO_WINDOW
    let mut child = command
        .spawn()
        .map_err(|_| "Could not start Grok Bot setup.")?;
    if let Some(mut stdin) = child.stdin.take() {
        stdin
            .write_all(input.to_string().as_bytes())
            .await
            .map_err(|_| "Could not prepare Grok Bot setup.")?;
    }
    let output = tokio::time::timeout(Duration::from_secs(100), child.wait_with_output())
        .await
        .map_err(|_| "Grok Bot setup timed out. Open Grok Bot and retry.")?
        .map_err(|_| "Could not finish Grok Bot setup.")?;
    // Never include child output in errors: future runtime errors might contain
    // connection material. The bridge emits only a controlled status object.
    if !output.status.success() {
        return Err("Grok Bot setup failed. Open Grok Bot and retry.".into());
    }
    let result: Value = serde_json::from_slice(&output.stdout)
        .map_err(|_| "Grok Bot returned an invalid setup result.")?;
    if let Some(error) = result.get("error").and_then(Value::as_str) {
        return Err(error.to_owned());
    }
    Ok(result)
}

async fn connection_in(
    app: &AppHandle,
    home: &Path,
    bun: &Path,
    action: &str,
) -> Result<Value, String> {
    let _guard = GROKBOT_CONNECTION_LOCK.lock().await;
    connection_with_bridge(
        home,
        &crate::skills::ai_tool_auto_connect_opt_out_dir(),
        action,
        || run_bridge(app, home, bun, action),
    )
    .await
}

// Keep the credential operation behind an injectable boundary so passive
// status and failure ordering can be tested without another app's secrets.
async fn connection_with_bridge<F, Fut>(
    home: &Path,
    opt_out_dir: &Path,
    action: &str,
    bridge: F,
) -> Result<Value, String>
where
    F: FnOnce() -> Fut,
    Fut: std::future::Future<Output = Result<Value, String>>,
{
    if !["status", "connect", "disconnect"].contains(&action) {
        return Err("Invalid Grok Bot connection action.".into());
    }
    let opted_out = opt_out_dir.join("grokbot").is_file();
    let connected_marker = opt_out_dir.join("grokbot-last-connected");
    if action == "status" {
        // Reading status must not spawn Bun, decrypt another app's credential,
        // or contact its gateway. This marker records only our last result.
        return Ok(cached_connection_status(
            grokbot_app_data_in(home).is_dir(),
            connected_marker.is_file(),
            opted_out,
        ));
    }
    // Only explicit Connect/Disconnect reaches the credential bridge. Clear
    // the last result first so a failed/partial operation cannot claim success.
    if connected_marker.exists() {
        std::fs::remove_file(&connected_marker)
            .map_err(|_| "Could not clear the previous Grok Bot connection status.")?;
    }
    crate::skills::set_ai_tool_auto_connect_opt_out_in(
        opt_out_dir,
        "grokbot",
        action == "disconnect",
    )?;
    let mut result = bridge().await?;
    if action == "connect" && result["connected"] == true {
        std::fs::create_dir_all(opt_out_dir)
            .and_then(|_| std::fs::write(&connected_marker, b""))
            .map_err(|_| "Connected, but could not save Grok Bot's last connection status.")?;
    }
    result["optedOut"] = json!(action == "disconnect");
    Ok(result)
}

fn cached_connection_status(detected: bool, last_connected: bool, opted_out: bool) -> Value {
    json!({
        "detected": detected,
        "connected": detected && last_connected && !opted_out,
        "optedOut": opted_out,
        "cached": true,
        "message": "Status is saved locally. Connect to verify the installation in Grok Bot."
    })
}

#[tauri::command]
#[specta::specta]
pub async fn grokbot_connection(app: AppHandle, action: String) -> Result<Value, String> {
    let home = dirs::home_dir().ok_or("Home directory is unavailable.")?;
    let bun = if action == "status" {
        String::new()
    } else {
        crate::pi::find_bun_executable().ok_or("Screenpipe's bundled runtime is unavailable.")?
    };
    connection_in(&app, &home, Path::new(&bun), &action).await
}

fn grokbot_app_data_in(home: &Path) -> PathBuf {
    // Match the bridge's appDataPath, including platform config overrides only
    // for the real home. Fixtures and isolated dev builds never use real data.
    #[cfg(target_os = "windows")]
    if dirs::home_dir().as_deref() == Some(home) {
        if let Some(config) = std::env::var_os("APPDATA").filter(|path| !path.is_empty()) {
            return PathBuf::from(config).join("Grok Bot");
        }
    }
    #[cfg(target_os = "macos")]
    let config = home.join("Library/Application Support");
    #[cfg(target_os = "windows")]
    let config = home.join("AppData/Roaming");
    #[cfg(not(any(target_os = "macos", target_os = "windows")))]
    let config = home.join(".config");
    config.join("Grok Bot")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn status_never_calls_the_credential_bridge_even_with_an_existing_install() {
        let home = tempfile::tempdir().unwrap();
        let intent = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(grokbot_app_data_in(home.path())).unwrap();
        for last_connected in [false, true] {
            if last_connected {
                std::fs::write(intent.path().join("grokbot-last-connected"), b"").unwrap();
            }
            let result = connection_with_bridge(home.path(), intent.path(), "status", || async {
                panic!("passive status accessed Grok Bot credentials")
            }).await.unwrap();
            assert_eq!(result["connected"], last_connected);
            assert_eq!(result["cached"], true);
        }
        std::fs::write(intent.path().join("grokbot"), b"").unwrap();
        let result = connection_with_bridge(home.path(), intent.path(), "status", || async {
            panic!("opted-out status accessed Grok Bot credentials")
        }).await.unwrap();
        assert_eq!(result["connected"], false);
        assert_eq!(result["optedOut"], true);
    }

    #[tokio::test]
    async fn only_verified_explicit_connect_records_success_and_failure_clears_it() {
        let home = tempfile::tempdir().unwrap();
        let intent = tempfile::tempdir().unwrap();
        let marker = intent.path().join("grokbot-last-connected");
        connection_with_bridge(home.path(), intent.path(), "connect", || async {
            Ok(json!({"detected": true, "connected": true}))
        }).await.unwrap();
        assert!(marker.is_file());
        let result = connection_with_bridge(home.path(), intent.path(), "connect", || async {
            Err("gateway unavailable".into())
        }).await;
        assert!(result.is_err());
        assert!(!marker.exists());
    }

    #[tokio::test]
    async fn failed_disconnect_saves_opt_out_before_accessing_the_gateway() {
        let home = tempfile::tempdir().unwrap();
        let intent = tempfile::tempdir().unwrap();
        std::fs::write(intent.path().join("grokbot-last-connected"), b"").unwrap();
        let result = connection_with_bridge(home.path(), intent.path(), "disconnect", || async {
            assert!(intent.path().join("grokbot").is_file());
            assert!(!intent.path().join("grokbot-last-connected").exists());
            Err("gateway unavailable".into())
        }).await;
        assert!(result.is_err());
        assert!(intent.path().join("grokbot").is_file());
    }

    #[tokio::test]
    async fn bundled_bridge_executes_with_an_isolated_home_without_connecting_real_accounts() {
        let fixture = tempfile::tempdir().unwrap();
        // `cargo test` puts this executable in target/.../deps, outside an
        // app bundle. Use the sidecar prepared by the required build script.
        let platform = match std::env::consts::OS {
            "macos" => "apple-darwin",
            "windows" => "pc-windows-msvc",
            "linux" => "unknown-linux-gnu",
            other => panic!("unsupported test platform: {other}"),
        };
        let bun = Path::new(env!("CARGO_MANIFEST_DIR")).join(format!(
            "bun-{}-{platform}{}",
            std::env::consts::ARCH,
            std::env::consts::EXE_SUFFIX,
        ));
        assert!(bun.is_file(), "run the test through bun run test:tauri");
        let result = run_bridge_input(
            &bun,
            &json!({
                "home": fixture.path(), "bun": bun, "dataDir": fixture.path().join("data"),
                "port": 3137, "skill": "fixture skill", "action": "connect"
            }),
        )
        .await
        .unwrap();
        assert_eq!(result["detected"], false);
        assert_eq!(result["connected"], false);
        assert_eq!(std::fs::read_dir(fixture.path()).unwrap().count(), 0);
    }

    #[test]
    fn grokbot_disconnect_intent_persists_even_if_remote_removal_cannot_run() {
        let fixture = tempfile::tempdir().unwrap();
        crate::skills::set_ai_tool_auto_connect_opt_out_in(fixture.path(), "grokbot", true)
            .unwrap();
        assert!(fixture.path().join("grokbot").is_file());
        crate::skills::set_ai_tool_auto_connect_opt_out_in(fixture.path(), "grokbot", false)
            .unwrap();
        assert!(!fixture.path().join("grokbot").exists());
    }
}
