// screenpipe — AI that knows everything you've seen, said, or heard
// https://screenpipe.com

//! Loopback HTTP benchmark host using the production router and private DB
//! copies. Capture and telemetry are disabled. See benchmark-storage-api.ts.

#[cfg(unix)]
use {
    anyhow::{ensure, Context},
    screenpipe_audio::audio_manager::AudioManagerBuilder,
    screenpipe_db::DatabaseManager,
    screenpipe_engine::SCServer,
    std::{path::PathBuf, sync::Arc},
};

#[cfg(unix)]
#[tokio::main]
async fn main() -> anyhow::Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter("warn")
        .with_writer(std::io::stderr)
        .init();
    let root = PathBuf::from(
        std::env::args()
            .nth(1)
            .context("private fixture root required")?,
    )
    .canonicalize()?;
    ensure!(
        root.join(".screenpipe-api-benchmark").is_file(),
        "root must be an explicitly prepared benchmark copy"
    );
    let db = Arc::new(
        DatabaseManager::new(root.join("db.sqlite").to_str().unwrap(), Default::default()).await?,
    );
    if root.join("storage.json").is_file() {
        let descriptor: screenpipe_db::storage::StorageDescriptor =
            serde_json::from_slice(&std::fs::read(root.join("storage.json"))?)?;
        ensure!(
            descriptor.privacy.required_surfaces == 0 && descriptor.privacy.identity.is_empty(),
            "benchmark fixture requires a completed migration with the default privacy policy"
        );
    }
    // Match engine startup with its optional text-PII worker disabled. This
    // handshake admits normal background Parquet publication on the fixture.
    db.set_frame_privacy_policy(&Default::default()).await?;
    let audio = Arc::new(
        AudioManagerBuilder::new()
            .is_disabled(true)
            .output_path(root.join("audio"))
            .build(db.clone())
            .await?,
    );
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await?;
    let addr = listener.local_addr()?;
    let mut server = SCServer::new(
        db.clone(),
        addr,
        root.clone(),
        true,
        true,
        audio,
        false,
        "balanced".into(),
    );
    server.timeline_disabled = true;
    server.advertise_mdns = false;
    let router = server.try_create_router().await?.route("/__benchmark_metrics", axum::routing::get(|| async {
        let usage = |who| {
            let mut value: libc::rusage = unsafe { std::mem::zeroed() };
            let ok = unsafe { libc::getrusage(who, &mut value) } == 0;
            assert!(ok);
            let ms = |time: libc::timeval| time.tv_sec as f64 * 1000.0 + time.tv_usec as f64 / 1000.0;
            (ms(value.ru_utime), ms(value.ru_stime), value.ru_maxrss)
        };
        let (user, system, rss) = usage(libc::RUSAGE_SELF);
        let (child_user, child_system, _) = usage(libc::RUSAGE_CHILDREN);
        axum::Json(serde_json::json!({"user_ms":user,"system_ms":system,"child_cpu_ms":child_user+child_system,"peak_rss_native":rss}))
    }));
    println!(
        "{}",
        serde_json::json!({"base_url":format!("http://{addr}"),"storage_mode":db.storage_mode(),"pid":std::process::id()})
    );
    let shutdown = async {
        #[cfg(unix)]
        {
            let mut term =
                tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate())
                    .expect("SIGTERM handler");
            tokio::select! { _ = term.recv() => {}, _ = tokio::signal::ctrl_c() => {} }
        }
        #[cfg(not(unix))]
        let _ = tokio::signal::ctrl_c().await;
    };
    SCServer::serve_router_with_listener_graceful(addr, listener, router, shutdown).await?;
    db.close().await;
    Ok(())
}

#[cfg(not(unix))]
fn main() {
    eprintln!("The storage HTTP benchmark host requires Unix.");
    std::process::exit(1);
}
