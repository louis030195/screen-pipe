use clap::Parser;
use screenpipe_vision::{continuous_capture, get_monitor, OcrEngine};
use std::{sync::Arc, time::Duration};
use tokio::sync::{mpsc::channel, Mutex};

#[derive(Parser)]
#[command(author, version, about, long_about = None)]
struct Cli {
    /// Save text files
    #[arg(long, default_value_t = false)]
    save_text_files: bool,

    /// Disable cloud OCR processing
    #[arg(long, default_value_t = false)]
    cloud_ocr_off: bool, // Add this flag
}

#[tokio::main]
async fn main() {
    let cli = Cli::parse();

    let (result_tx, mut result_rx) = channel(512);

    let save_text_files = cli.save_text_files;
    let stop_signal = Arc::new(Mutex::new(false));

    let capture_thread = tokio::spawn(async move {
        continuous_capture(
            result_tx,
            Duration::from_secs(1),
            save_text_files,
            Arc::new(OcrEngine::Tesseract),
            get_monitor().await,
            stop_signal.clone(),
        )
        .await
    });

    // Example: Process results for 10 seconds, then pause for 5 seconds, then stop
    let start_time = std::time::Instant::now();
    loop {
        if let Some(result) = result_rx.recv().await {
            println!("OCR Text length: {}", result.text.len());
        }

        let elapsed = start_time.elapsed();
        if elapsed >= Duration::from_secs(15) {
            break;
        }

        tokio::time::sleep(Duration::from_millis(100)).await;
    }

    capture_thread.await.unwrap();
}
