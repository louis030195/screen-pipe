// screenpipe — AI that knows everything you've seen, said, or heard
// https://screenpipe.com

//! Offline storage lifecycle entry point; capture and upload remain stopped.
#[tokio::main]
async fn main() -> anyhow::Result<()> {
    tracing_subscriber::fmt()
        .with_writer(std::io::stderr)
        .with_ansi(false)
        .with_env_filter("screenpipe_db::storage=info")
        .init();
    let mut args = std::env::args().skip(1);
    let command = args.next().unwrap_or_default();
    let root=std::path::PathBuf::from(args.next().ok_or_else(||anyhow::anyhow!("usage: screenpipe-storage <init|migrate|verify|seal|reclaim|compact|backup|restore|export-sqlite|compare|cancel|status> <root> [destination]"))?);
    let destination = args.next().map(std::path::PathBuf::from);
    if args.next().is_some() {
        anyhow::bail!("unexpected storage command argument");
    }
    let result =
        screenpipe_db::storage::run_command(&command, &root, destination.as_deref()).await?;
    println!("{}", serde_json::to_string(&result)?);
    Ok(())
}
