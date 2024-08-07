use std::{
    collections::HashMap,
    fs::{self, File},
    net::SocketAddr,
    ops::Deref,
    path::PathBuf,
    sync::Arc,
    time::Duration,
};

use clap::Parser;
#[allow(unused_imports)]
use colored::Colorize;
use crossbeam::queue::SegQueue;
use dirs::home_dir;
use log::{debug, error, info, warn, LevelFilter};
use screenpipe_audio::{
    default_input_device, list_audio_devices, parse_audio_device, AudioDevice, DeviceControl,
};
use screenpipe_vision::OcrEngine;
use std::io::Write;

use screenpipe_core::find_ffmpeg_path;
use screenpipe_server::{logs::MultiWriter, RecordingState};
use screenpipe_server::{start_continuous_recording, DatabaseManager, ResourceMonitor, Server};
use tokio::sync::{mpsc::channel, Mutex};

use clap::ValueEnum;
use screenpipe_vision::utils::OcrEngine as CoreOcrEngine;

#[derive(Clone, Debug, ValueEnum, PartialEq)]
enum CliOcrEngine {
    Unstructured,
    Tesseract,
    WindowsNative,
}

impl From<CliOcrEngine> for CoreOcrEngine {
    fn from(cli_engine: CliOcrEngine) -> Self {
        match cli_engine {
            CliOcrEngine::Unstructured => CoreOcrEngine::Unstructured,
            CliOcrEngine::Tesseract => CoreOcrEngine::Tesseract,
            CliOcrEngine::WindowsNative => CoreOcrEngine::WindowsNative,
        }
    }
}

// keep in mind this is the most important feature ever // TODO: add a pipe and a ⭐️ e.g screen | ⭐️ somehow in ascii ♥️🤓
const DISPLAY: &str = r"
                                            _          
   __________________  ___  ____     ____  (_____  ___ 
  / ___/ ___/ ___/ _ \/ _ \/ __ \   / __ \/ / __ \/ _ \
 (__  / /__/ /  /  __/  __/ / / /  / /_/ / / /_/ /  __/
/____/\___/_/   \___/\___/_/ /_/  / .___/_/ .___/\___/ 
                                 /_/     /_/           

";

#[derive(Parser)]
#[command(author, version, about, long_about = None)]
struct Cli {
    /// FPS for continuous recording
    /// 1 FPS = 30 GB / month
    /// 5 FPS = 150 GB / month
    /// Optimise based on your needs.
    /// You rarely change change more than 1 times within a second, right?
    #[arg(short, long, default_value_t = 1.0)]
    fps: f64,

    /// Audio chunk duration in seconds
    #[arg(short, long, default_value_t = 30)]
    audio_chunk_duration: u64,

    /// Port to run the server on
    #[arg(short, long, default_value_t = 3030)]
    port: u16,

    /// Disable audio recording
    #[arg(long, default_value_t = false)]
    disable_audio: bool,

    /// EXPERIMENTAL: Enable self healing when detecting unhealthy state based on /health endpoint.
    /// This feature will automatically restart the recording tasks while keeping the API alive.
    #[arg(long, default_value_t = false)]
    self_healing: bool,

    /// Audio devices to use (can be specified multiple times)
    #[arg(long)]
    audio_device: Vec<String>,

    /// List available audio devices
    #[arg(long)]
    list_audio_devices: bool,

    /// Data directory. Default to $HOME/.screenpipe
    #[arg(long)]
    data_dir: Option<String>,

    /// Enable debug logging for screenpipe modules
    #[arg(long)]
    debug: bool,

    /// Save text files
    #[arg(long, default_value_t = false)]
    save_text_files: bool,

    /// Enable cloud audio processing
    #[arg(long, default_value_t = false)]
    cloud_audio_on: bool,

    /// OCR engine to use. Tesseract is a local OCR engine (default).
    /// WindowsNative is a local OCR engine for Windows.
    /// Unstructured is a cloud OCR engine (free of charge on us)
    #[arg(long, value_enum, default_value_t = CliOcrEngine::Tesseract)]
    ocr_engine: CliOcrEngine,

    /// UID key for sending data to friend wearable (if not provided, data won't be sent)
    #[arg(long)]
    friend_wearable_uid: Option<String>,
}

fn get_base_dir(custom_path: Option<String>) -> anyhow::Result<PathBuf> {
    let default_path = home_dir()
        .ok_or_else(|| anyhow::anyhow!("Failed to get home directory"))?
        .join(".screenpipe");

    let base_dir = custom_path.map(PathBuf::from).unwrap_or(default_path);
    let data_dir = base_dir.join("data");

    fs::create_dir_all(&data_dir)?;
    Ok(base_dir)
}

fn initialize_audio_devices(
    audio_devices: &Vec<Arc<AudioDevice>>,
    audio_devices_control: Arc<SegQueue<(AudioDevice, DeviceControl)>>,
) {
    for device in audio_devices {
        info!("  {}", device);

        let device_control = DeviceControl {
            is_running: true,
            is_paused: false,
        };
        let device_clone = device.deref().clone();
        let sender_clone = audio_devices_control.clone();

        tokio::spawn(async move {
            tokio::time::sleep(Duration::from_secs(15)).await;
            let _ = sender_clone.push((device_clone, device_control));
        });
    }
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    if find_ffmpeg_path().is_none() {
        eprintln!("ffmpeg not found. Please install ffmpeg and ensure it is in your PATH.");
        std::process::exit(1);
    }

    // Initialize logging
    let cli = Cli::parse();

    let mut builder = env_logger::Builder::new();
    builder
        .filter(None, LevelFilter::Info)
        .filter_module("tokenizers", LevelFilter::Error)
        .filter_module("rusty_tesseract", LevelFilter::Error)
        .filter_module("symphonia", LevelFilter::Error)
        .filter_module("external_cloud_integrations", LevelFilter::Debug); // Add this line

    if cli.debug {
        builder.filter_module("screenpipe", LevelFilter::Debug);
    }
    // Example usage of the new flag
    if cli.save_text_files {
        debug!("Text files will be saved.");
    }

    // builder.init();
    // tokio-console
    // console_subscriber::init();
    let local_data_dir = get_base_dir(cli.data_dir)?;

    let log_file = File::create(format!(
        "{}/screenpipe.log",
        local_data_dir.to_string_lossy()
    ))
    .unwrap();
    let multi_writer = MultiWriter::new(vec![
        Box::new(log_file) as Box<dyn Write + Send>,
        Box::new(std::io::stdout()) as Box<dyn Write + Send>,
    ]);

    builder.target(env_logger::Target::Pipe(Box::new(multi_writer)));
    builder.format_timestamp_secs().init();

    // Add warning for Linux and Windows users
    #[cfg(any(target_os = "linux", target_os = "windows"))]
    {
        use log::warn;
        warn!("Screenpipe hasn't been extensively tested on this OS. We'd love your feedback!");
        println!(
            "{}",
            "Would love your feedback on the UX, let's a 15 min call soon:".bright_yellow()
        );
        println!(
            "{}",
            "https://cal.com/louis030195/screenpipe"
                .bright_blue()
                .underline()
        );
    }
    let all_audio_devices = list_audio_devices()?;
    let mut devices_status = HashMap::new();
    if cli.list_audio_devices {
        println!("Available audio devices:");
        for (i, device) in all_audio_devices.iter().enumerate() {
            println!("  {}. {}", i + 1, device);
        }
        return Ok(());
    }

    let mut audio_devices = Vec::new();

    let audio_devices_control = Arc::new(SegQueue::new());

    let audio_devices_control_server = audio_devices_control.clone();

    info!("Available audio devices:");
    // Add all available audio devices to the controls
    for device in &all_audio_devices {
        let device_control = DeviceControl {
            is_running: false,
            is_paused: false,
        };
        devices_status.insert(device.clone(), device_control);
        info!("  {}", device);
    }

    if !cli.disable_audio {
        if cli.audio_device.is_empty() {
            debug!("Using default devices");
            // Use default devices
            if let Ok(input_device) = default_input_device() {
                audio_devices.push(Arc::new(input_device.clone()));
                let device_control = DeviceControl {
                    is_running: true,
                    is_paused: false,
                };
                devices_status.insert(input_device, device_control);
            }

            // audio output only supported on linux atm
            // see https://github.com/louis030195/screen-pipe/pull/106
            if cfg!(target_os = "linux") {
                use screenpipe_audio::default_output_device;
                if let Ok(output_device) = default_output_device() {
                    audio_devices.push(Arc::new(output_device.clone()));
                    let device_control = DeviceControl {
                        is_running: true,
                        is_paused: false,
                    };
                    devices_status.insert(output_device, device_control);
                }
            }
        } else {
            // Use specified devices
            for d in &cli.audio_device {
                let device = parse_audio_device(d).expect("Failed to parse audio device");
                audio_devices.push(Arc::new(device.clone()));
                let device_control = DeviceControl {
                    is_running: true,
                    is_paused: false,
                };
                devices_status.insert(device, device_control);
            }
        }
    }

    let (restart_sender, mut restart_receiver) = channel(10);
    let resource_monitor =
        ResourceMonitor::new(cli.self_healing, Duration::from_secs(5), 3, restart_sender);
    resource_monitor.start_monitoring(Duration::from_secs(10));

    let db = Arc::new(
        DatabaseManager::new(&format!("{}/db.sqlite", local_data_dir.to_string_lossy()))
            .await
            .map_err(|e| {
                eprintln!("Failed to initialize database: {:?}", e);
                e
            })?,
    );
    info!(
        "Database initialized, will store files in {}",
        local_data_dir.to_string_lossy()
    );
    let db_server = db.clone();

    let recording_state = Arc::new(Mutex::new(RecordingState::new()));

    // Before the loop starts, clone friend_wearable_uid
    let friend_wearable_uid = cli.friend_wearable_uid.clone();

    let warning_ocr_engine_clone = cli.ocr_engine.clone();

    // Function to start or restart the recording task
    let _start_recording = tokio::spawn(async move {
        loop {
            let db_clone = db.clone();
            let local_data_dir = local_data_dir.clone();
            let recording_state = Arc::clone(&recording_state);
            let recording_state_clone = recording_state.clone();
            let recording_state_clone_2 = recording_state.clone();
            let recording_state_clone_3 = recording_state.clone();
            let audio_devices_control = audio_devices_control.clone();
            let friend_wearable_uid_clone = friend_wearable_uid.clone();

            let core_ocr_engine: CoreOcrEngine = cli.ocr_engine.clone().into();
            let ocr_engine = Arc::new(OcrEngine::from(core_ocr_engine));

            {
                let mut state = recording_state.lock().await;
                if state.is_running {
                    warn!("Recording is already running. Waiting before retry.");
                    drop(state);
                    tokio::time::sleep(Duration::from_secs(30)).await;
                    continue;
                }
                state.is_running = true;
            }

            // Reinitialize audio devices on restart
            initialize_audio_devices(&audio_devices, audio_devices_control.clone());

            let recording_task = tokio::spawn(async move {
                let result = start_continuous_recording(
                    db_clone,
                    Arc::new(local_data_dir.join("data").to_string_lossy().into_owned()),
                    cli.fps,
                    Duration::from_secs(cli.audio_chunk_duration),
                    recording_state.clone(),
                    audio_devices_control,
                    cli.save_text_files,
                    cli.cloud_audio_on,
                    ocr_engine,
                    friend_wearable_uid_clone,
                )
                .await;

                if let Err(e) = result {
                    error!("Continuous recording error: {:?}", e);
                }

                let mut state = recording_state.lock().await;
                state.is_running = false;
            });

            tokio::select! {
                _ = recording_task => {
                    debug!("Recording task ended.");
                }
                Some(_) = restart_receiver.recv() => {
                    info!("Received restart signal. Cancelling current recording task...");
                    let state = recording_state_clone.lock().await;
                    state.cancel();
                    drop(state);
                    recording_state_clone_2.lock().await.cancel();
                }
            }

            // Wait for the recording task to finish
            // let _ = tokio::time::timeout(Duration::from_secs(10), recording_task).await;

            // Reset the recording state
            {
                let mut state = recording_state_clone_3.lock().await;
                state.reset();
            }

            // Cooldown period before attempting to restart
        }
    });

    tokio::spawn(async move {
        let api_plugin = |req: &axum::http::Request<axum::body::Body>| {
            // Custom plugin logic here
            // For example, using PostHog for tracking:
            if req.uri().path() == "/search" {
                // Track search requests
                // posthog.capture("search_request", {...})
            }
        };
        let server = Server::new(
            db_server,
            SocketAddr::from(([0, 0, 0, 0], cli.port)),
            audio_devices_control_server,
        );
        server.start(devices_status, api_plugin).await.unwrap();
    });

    // Wait for the server to start
    info!("Server started on http://localhost:{}", cli.port);

    // print screenpipe in gradient
    println!("\n\n{}", DISPLAY.truecolor(147, 112, 219).bold());
    println!(
        "\n{}",
        "Build AI apps that have the full context"
            .bright_yellow()
            .italic()
    );
    println!(
        "{}\n\n",
        "Open source | Runs locally | Developer friendly".bright_green()
    );

    // Add warning for cloud arguments
    if cli.cloud_audio_on || warning_ocr_engine_clone == CliOcrEngine::Unstructured {
        println!(
            "{}",
            "WARNING: You are using cloud now. Make sure to understand the data privacy risks."
                .bright_yellow()
        );
    } else {
        println!(
            "{}",
            "You are using local processing. All your data stays on your computer.\n"
                .bright_yellow()
        );
    }

    // Keep the main thread running
    loop {
        tokio::time::sleep(Duration::from_secs(1)).await;
    }
}
