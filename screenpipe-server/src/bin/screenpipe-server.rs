use std::{
    collections::HashMap, fs, io, net::SocketAddr, ops::Deref, path::PathBuf, sync::{atomic::AtomicBool, Arc}, time::Duration, env
};
use std::io::Write;

use clap::Parser;
#[allow(unused_imports)]
use colored::Colorize;
use crossbeam::queue::SegQueue;
use dirs::home_dir;
use futures::{pin_mut, stream::FuturesUnordered, StreamExt};
use highlightio::Highlight;
use log::{debug, error, info};
use screenpipe_audio::{
    default_input_device, default_output_device, list_audio_devices, parse_audio_device,
    AudioDevice, DeviceControl,
};
use screenpipe_core::find_ffmpeg_path;
use screenpipe_server::{
    cli::{Cli, CliAudioTranscriptionEngine, CliOcrEngine, Command, PipeCommand}, logs::SingleFileRollingWriter, start_continuous_recording, watch_pid, DatabaseManager, PipeManager, ResourceMonitor, Server
};
use screenpipe_vision::monitor::list_monitors;
use serde_json::{json, Value};
use tokio::{runtime::Runtime, signal, sync::broadcast};
use tracing_subscriber::prelude::__tracing_subscriber_SubscriberExt;
use tracing_subscriber::util::SubscriberInitExt;
use tracing_subscriber::Layer;
use tracing_subscriber::{fmt, EnvFilter};

fn print_devices(devices: &[AudioDevice]) {
    println!("available audio devices:");
    for device in devices.iter() {
        println!("  {}", device);
    }

    #[cfg(target_os = "macos")]
    println!("on macos, it's not intuitive but output devices are your displays");
}

const DISPLAY: &str = r"
                                            _          
   __________________  ___  ____     ____  (_____  ___ 
  / ___/ ___/ ___/ _ \/ _ \/ __ \   / __ \/ / __ \/ _ \
 (__  / /__/ /  /  __/  __/ / / /  / /_/ / / /_/ /  __/
/____/\___/_/   \___/\___/_/ /_/  / .___/_/ .___/\___/ 
                                 /_/     /_/           

";

fn get_base_dir(custom_path: Option<String>) -> anyhow::Result<PathBuf> {
    let default_path = home_dir()
        .ok_or_else(|| anyhow::anyhow!("failed to get home directory"))?
        .join(".screenpipe");

    let base_dir = custom_path.map(PathBuf::from).unwrap_or(default_path);
    let data_dir = base_dir.join("data");

    fs::create_dir_all(&data_dir)?;
    Ok(base_dir)
}




#[tokio::main]
async fn main() -> anyhow::Result<()> {
    debug!("starting screenpipe server");
    let cli = Cli::parse();
    let local_data_dir = get_base_dir(cli.data_dir)?;
    let local_data_dir_clone = local_data_dir.clone();

    let pipe_manager = Arc::new(PipeManager::new(local_data_dir_clone.clone()));

    if let Some(pipe_command) = cli.command {
        match pipe_command {
            Command::Pipe { subcommand } => {
                handle_pipe_command(subcommand, &pipe_manager).await?;
                return Ok(());
            }
        }
    }

    if find_ffmpeg_path().is_none() {
        eprintln!("ffmpeg not found. please install ffmpeg and ensure it is in your path.");
        std::process::exit(1);
    }

    // Set up file appender
    let log_file_path = local_data_dir.join("screenpipe.log");
    let file_writer = SingleFileRollingWriter::new(log_file_path)?;

    // Create a custom layer for file logging
    let file_layer = fmt::layer()
        .with_writer(file_writer)
        .with_ansi(false)
        .with_filter(EnvFilter::new("info"));

    // Create a custom layer for console logging
    let console_layer = fmt::layer()
        .with_writer(std::io::stdout)
        .with_filter(EnvFilter::new("debug"));

    // Build the EnvFilter
    let env_filter = EnvFilter::from_default_env()
        .add_directive("info".parse().unwrap())
        .add_directive("tokenizers=error".parse().unwrap())
        .add_directive("rusty_tesseract=error".parse().unwrap())
        .add_directive("symphonia=error".parse().unwrap());

    // Add custom log levels for specific modules based on environment variables
    let env_filter = env::var("SCREENPIPE_LOG")
        .unwrap_or_default()
        .split(',')
        .filter(|s| !s.is_empty()) // Filter out empty strings
        .fold(env_filter, |filter, module_directive| {
            match module_directive.parse() {
                Ok(directive) => filter.add_directive(directive),
                Err(e) => {
                    eprintln!("warning: invalid log directive '{}': {}", module_directive, e);
                    filter
                }
            }
        });

    // Usage:
    //  SCREENPIPE_LOG=screenpipe_audio=debug ./screenpipe
    //  SCREENPIPE_LOG=screenpipe_audio=debug,screenpipe_vision=trace ./screenpipe

    let env_filter = if cli.debug {
        env_filter.add_directive("screenpipe=debug".parse().unwrap())
    } else {
        env_filter
    };

    // ignore mut warning 
    #[allow(unused_mut)]
    let mut h: Option<Highlight> = None;
    if !cli.disable_telemetry {
        // ! do not use yet - leaks too much privacy and does not catch errors properly 
        // h = Some(Highlight::init(HighlightConfig {
        //     project_id: "82688".to_string(),
        //     ..Default::default()
        // }).expect("Failed to initialize Highlight.io"));
    }
    // } else {
    tracing_subscriber::registry()
        .with(env_filter)
        .with(file_layer)
        .with(console_layer)
        .init();
    // }

    let all_audio_devices = list_audio_devices().await?;
    let mut devices_status = HashMap::new();
    if cli.list_audio_devices {
        print_devices(&all_audio_devices);
        return Ok(());
    }
    let all_monitors = list_monitors().await;
    if cli.list_monitors {
        println!("available monitors:");
        for monitor in all_monitors.iter() {
            println!("  {}. {:?}", monitor.id(), monitor);
        }
        return Ok(());
    }

    let mut audio_devices = Vec::new();

    let audio_devices_control = Arc::new(SegQueue::new());

    let audio_devices_control_server = audio_devices_control.clone();

    // Add all available audio devices to the controls
    for device in &all_audio_devices {
        let device_control = DeviceControl {
            is_running: false,
            is_paused: false,
        };
        devices_status.insert(device.clone(), device_control);
    }

    if !cli.disable_audio {
        if cli.audio_device.is_empty() {
            // Use default devices
            if let Ok(input_device) = default_input_device() {
                audio_devices.push(Arc::new(input_device.clone()));
                let device_control = DeviceControl {
                    is_running: true,
                    is_paused: false,
                };
                devices_status.insert(input_device, device_control);
            }
            // audio output only on macos <15.0 atm ?
            // see https://github.com/mediar-ai/screenpipe/pull/106
            if let Ok(output_device) = default_output_device() {
                audio_devices.push(Arc::new(output_device.clone()));
                let device_control = DeviceControl {
                    is_running: true,
                    is_paused: false,
                };
                devices_status.insert(output_device, device_control);
            }
        } else {
            // Use specified devices
            for d in &cli.audio_device {
                let device = parse_audio_device(d).expect("failed to parse audio device");
                audio_devices.push(Arc::new(device.clone()));
                let device_control = DeviceControl {
                    is_running: true,
                    is_paused: false,
                };
                devices_status.insert(device, device_control);
            }
        }

        if audio_devices.is_empty() {
            eprintln!("no audio devices available. audio recording will be disabled.");
        } else {
            for device in &audio_devices {
                info!("  {}", device);

                let device_control = DeviceControl {
                    is_running: true,
                    is_paused: false,
                };
                let device_clone = device.deref().clone();
                let sender_clone = audio_devices_control.clone();
                // send signal after everything started
                tokio::spawn(async move {
                    tokio::time::sleep(Duration::from_secs(15)).await;
                    sender_clone.push((device_clone, device_control));
                });
            }
        }
    }

    let resource_monitor = ResourceMonitor::new();
    resource_monitor.start_monitoring(Duration::from_secs(10));

    let db = Arc::new(
        DatabaseManager::new(&format!("{}/db.sqlite", local_data_dir.to_string_lossy()))
            .await
            .map_err(|e| {
                eprintln!("failed to initialize database: {:?}", e);
                e
            })?,
    );
    info!(
        "database initialized, will store files in {}",
        local_data_dir.to_string_lossy()
    );
    let db_server = db.clone();

    // Channel for controlling the recorder ! TODO RENAME SHIT
    let vision_control = Arc::new(AtomicBool::new(true));

    let vision_control_server_clone = vision_control.clone();

    // Before the loop starts, clone friend_wearable_uid
    let friend_wearable_uid = cli.friend_wearable_uid.clone();

    let warning_ocr_engine_clone = cli.ocr_engine.clone();
    let warning_audio_transcription_engine_clone = cli.audio_transcription_engine.clone();
    let monitor_ids = if cli.monitor_id.is_empty() {
        all_monitors.iter().map(|m| m.id()).collect::<Vec<_>>()
    } else {
        cli.monitor_id.clone()
    };

    let ocr_engine_clone = cli.ocr_engine.clone();
    let vad_engine = cli.vad_engine.clone();
    let vad_engine_clone = vad_engine.clone();
    let vad_sensitivity_clone = cli.vad_sensitivity.clone();
    let (shutdown_tx, _) = broadcast::channel::<()>(1);

    let audio_runtime = Runtime::new().unwrap();
    let vision_runtime = Runtime::new().unwrap();

    let audio_handle = audio_runtime.handle().clone();
    let vision_handle = vision_runtime.handle().clone();

    let db_clone = Arc::clone(&db);
    let output_path_clone = Arc::new(local_data_dir.join("data").to_string_lossy().into_owned());
    let vision_control_clone = Arc::clone(&vision_control);
    let shutdown_tx_clone = shutdown_tx.clone();
    let friend_wearable_uid_clone: Option<String> = friend_wearable_uid.clone(); // Clone here
    let monitor_ids_clone = monitor_ids.clone();
    let ignored_windows_clone = cli.ignored_windows.clone();
    let included_windows_clone = cli.included_windows.clone();

    let fps = if cli.fps.is_finite() && cli.fps > 0.0 {
        cli.fps
    } else {
        eprintln!("invalid fps value: {}. using default of 1.0", cli.fps);
        1.0
    };

    let audio_chunk_duration = Duration::from_secs(cli.audio_chunk_duration);

    let handle = {
        let runtime = &tokio::runtime::Handle::current();
        runtime.spawn(async move {
            loop {
                let vad_engine_clone = vad_engine.clone(); // Clone it here for each iteration
                let mut shutdown_rx = shutdown_tx_clone.subscribe();
                let recording_future = start_continuous_recording(
                    db_clone.clone(),
                    output_path_clone.clone(),
                    fps,
                    audio_chunk_duration, // use the new setting
                    Duration::from_secs(cli.video_chunk_duration),
                    vision_control_clone.clone(),
                    audio_devices_control.clone(),
                    cli.disable_audio,
                    cli.save_text_files,
                    Arc::new(cli.audio_transcription_engine.clone().into()),
                    Arc::new(cli.ocr_engine.clone().into()),
                    friend_wearable_uid_clone.clone(),
                    monitor_ids_clone.clone(),
                    cli.use_pii_removal,
                    cli.disable_vision,
                    vad_engine_clone,
                    &vision_handle,
                    &audio_handle,
                    &cli.ignored_windows,
                    &cli.included_windows,
                    cli.deepgram_api_key.clone(),
                    cli.vad_sensitivity.clone(),
                );

                let result = tokio::select! {
                    result = recording_future => result,
                    _ = shutdown_rx.recv() => {
                        info!("received shutdown signal for recording");
                        break;
                    }
                };

                if let Err(e) = result {
                    error!("continuous recording error: {:?}", e);
                }
            }

            drop(vision_runtime);
            drop(audio_runtime);
        })
    };

    let local_data_dir_clone_2 = local_data_dir_clone.clone();
    #[cfg(feature = "llm")]
    debug!("LLM initializing");

    #[cfg(feature = "llm")]
    let llm = {
        match cli.enable_llm {
            true => Some(screenpipe_core::LLM::new(screenpipe_core::ModelName::Llama)?),
            false => None,
        }
        }; 

    #[cfg(feature = "llm")]
    debug!("LLM initialized");

    let api_plugin = |req: &axum::http::Request<axum::body::Body>| {
        if req.uri().path() == "/search" {
            // Track search requests
        }
    };
    let server = Server::new(
        db_server,
        SocketAddr::from(([127, 0, 0, 1], cli.port)),
        vision_control_server_clone,
        audio_devices_control_server,
        local_data_dir_clone_2,
        pipe_manager.clone(),
        cli.disable_vision,
        cli.disable_audio,
        #[cfg(feature = "llm")]
        cli.enable_llm,
        #[cfg(feature = "llm")]
        llm,

    );

    let mut pipe_futures = FuturesUnordered::new();

    // print screenpipe in gradient
    println!("\n\n{}", DISPLAY.truecolor(147, 112, 219).bold());
    println!(
        "\n{}",
        "build ai apps that have the full context"
            .bright_yellow()
            .italic()
    );
    println!(
        "{}\n\n",
        "open source | runs locally | developer friendly".bright_green()
    );

    println!("┌───────────────────┬────────────────────────────────────┐");
    println!("│ setting             │ value                              │");
    println!("├─────────────────────┼────────────────────────────────────┤");
    println!("│ fps                 │ {:<34} │", cli.fps);
    println!(
        "│ audio chunk duration│ {:<34} │",
        format!("{} seconds", cli.audio_chunk_duration)
    );
    println!(
        "│ video chunk duration│ {:<34} │",
        format!("{} seconds", cli.video_chunk_duration)
    );
    println!("│ port                │ {:<34} │", cli.port);
    println!("│ audio disabled      │ {:<34} │", cli.disable_audio);
    println!("│ vision disabled     │ {:<34} │", cli.disable_vision);
    println!("│ save text files     │ {:<34} │", cli.save_text_files);
    println!(
        "│ audio engine        │ {:<34} │",
        format!("{:?}", warning_audio_transcription_engine_clone)
    );
    println!(
        "│ ocr engine          │ {:<34} │",
        format!("{:?}", ocr_engine_clone)
    );
    println!(
        "│ vad engine          │ {:<34} │",
        format!("{:?}", vad_engine_clone)
    );
    println!(
        "│ vad sensitivity     │ {:<34} │",
        format!("{:?}", vad_sensitivity_clone)
    );
    println!(
        "│ data directory      │ {:<34} │",
        local_data_dir_clone.display()
    );
    println!("│ debug mode          │ {:<34} │", cli.debug);
    println!("│ telemetry           │ {:<34} │", !cli.disable_telemetry);
    println!("│ local llm           │ {:<34} │", cli.enable_llm);

    println!("│ use pii removal     │ {:<34} │", cli.use_pii_removal);
    println!(
        "│ ignored windows     │ {:<34} │",
        format_cell(&format!("{:?}", &ignored_windows_clone), VALUE_WIDTH)
    );
    println!(
        "│ included windows    │ {:<34} │",
        format_cell(&format!("{:?}", &included_windows_clone), VALUE_WIDTH)
    );
    println!(
        "│ friend wearable uid │ {:<34} │",
        cli.friend_wearable_uid.as_deref().unwrap_or("not set")
    );
    const VALUE_WIDTH: usize = 34;

    // Function to truncate and pad strings
    fn format_cell(s: &str, width: usize) -> String {
        if s.len() > width {
            format!("{}...", &s[..width - 3])
        } else {
            format!("{:<width$}", s, width = width)
        }
    }

    // Add monitors section
    println!("├─────────────────────┼────────────────────────────────────┤");
    println!("│ monitors            │                                    │");
    const MAX_ITEMS_TO_DISPLAY: usize = 5;

    if cli.disable_vision {
        println!("│ {:<19} │ {:<34} │", "", "vision disabled");
    } else if monitor_ids.is_empty() {
        println!("│ {:<19} │ {:<34} │", "", "no monitors available");
    } else {
        let total_monitors = monitor_ids.len();
        for (_, monitor) in monitor_ids.iter().enumerate().take(MAX_ITEMS_TO_DISPLAY) {
            let monitor_str = format!("id: {}", monitor);
            let formatted_monitor = format_cell(&monitor_str, VALUE_WIDTH);
            println!("│ {:<19} │ {:<34} │", "", formatted_monitor);
        }
        if total_monitors > MAX_ITEMS_TO_DISPLAY {
            println!(
                "│ {:<19} │ {:<34} │",
                "",
                format!("... and {} more", total_monitors - MAX_ITEMS_TO_DISPLAY)
            );
        }
    }

    // Audio devices section
    println!("├─────────────────────┼────────────────────────────────────┤");
    println!("│ audio devices       │                                    │");

    if cli.disable_audio {
        println!("│ {:<19} │ {:<34} │", "", "disabled");
    } else if audio_devices.is_empty() {
        println!("│ {:<19} │ {:<34} │", "", "no devices available");
    } else {
        let total_devices = audio_devices.len();
        for (_, device) in audio_devices.iter().enumerate().take(MAX_ITEMS_TO_DISPLAY) {
            let device_str = device.deref().to_string();
            let formatted_device = format_cell(&device_str, VALUE_WIDTH);

            println!("│ {:<19} │ {:<34} │", "", formatted_device);
        }
        if total_devices > MAX_ITEMS_TO_DISPLAY {
            println!(
                "│ {:<19} │ {:<34} │",
                "",
                format!("... and {} more", total_devices - MAX_ITEMS_TO_DISPLAY)
            );
        }
    }

    // Pipes section
    println!("├─────────────────────┼────────────────────────────────────┤");
    println!("│ pipes               │                                    │");
    let pipes = pipe_manager.list_pipes().await;
    if pipes.is_empty() {
        println!("│ {:<19} │ {:<34} │", "", "no pipes available");
    } else {
        let total_pipes = pipes.len();
        for (_, pipe) in pipes.iter().enumerate().take(MAX_ITEMS_TO_DISPLAY) {
            let pipe_str = format!(
                "({}) {}",
                if pipe.enabled { "enabled" } else { "disabled" },
                pipe.id,
            );
            let formatted_pipe = format_cell(&pipe_str, VALUE_WIDTH);
            println!("│ {:<19} │ {:<34} │", "", formatted_pipe);
        }
        if total_pipes > MAX_ITEMS_TO_DISPLAY {
            println!(
                "│ {:<19} │ {:<34} │",
                "",
                format!("... and {} more", total_pipes - MAX_ITEMS_TO_DISPLAY)
            );
        }
    }

    println!("└─────────────────────┴────────────────────────────────────┘");

    // Add warning for cloud arguments and telemetry
    if warning_audio_transcription_engine_clone == CliAudioTranscriptionEngine::Deepgram
        || warning_ocr_engine_clone == CliOcrEngine::Unstructured
    {
        println!(
            "{}",
            "warning: you are using cloud now. make sure to understand the data privacy risks."
                .bright_yellow()
        );
    } else {
        println!(
            "{}",
            "you are using local processing. all your data stays on your computer.\n"
                .bright_yellow()
        );
    }

    // Add warning for telemetry
    if !cli.disable_telemetry {
        println!(
            "{}",
            "warning: telemetry is enabled. only error-level data will be sent to highlight.io.\n\
            to disable, use the --disable-telemetry flag."
                .bright_yellow()
        );
    } else {
        println!(
            "{}",
            "telemetry is disabled. no data will be sent to external services."
                .bright_green()
        );
    }

    // Start pipes
    debug!("starting pipes");
    let pipes = pipe_manager.list_pipes().await;
    for pipe in pipes {
        debug!("pipe: {:?}", pipe.id);
        if !pipe.enabled {
            debug!("pipe {} is disabled, skipping", pipe.id);
            continue;
        }
        match pipe_manager.start_pipe(&pipe.id).await {
            Ok(future) => pipe_futures.push(future),
            Err(e) => eprintln!("failed to start pipe {}: {}", pipe.id, e),
        }
    }

    let server_future = server.start(devices_status, api_plugin);
    pin_mut!(server_future);

    let pipes_future = async {
        loop {
            if let Some(result) = pipe_futures.next().await {
                info!("pipe completed: {:?}", result);
            } else {
                tokio::time::sleep(std::time::Duration::from_secs(1)).await;
            }
        }
    };
    pin_mut!(pipes_future);

    // Add auto-destruct watcher
    if let Some(pid) = cli.auto_destruct_pid {
        info!("watching pid {} for auto-destruction", pid);
        let shutdown_tx_clone = shutdown_tx.clone();
        tokio::spawn(async move {
            if watch_pid(pid).await {
                info!("watched pid {} has stopped, initiating shutdown", pid);
                let _ = shutdown_tx_clone.send(());
            }
        });
    }

    let ctrl_c_future = signal::ctrl_c();
    pin_mut!(ctrl_c_future);

    tokio::select! {
        _ = handle => info!("recording completed"),
        result = &mut server_future => {
            match result {
                Ok(_) => info!("server stopped normally"),
                Err(e) => error!("server stopped with error: {:?}", e),
            }
        }
        _ = &mut pipes_future => {
            info!("all pipes completed, but server is still running");
        }
        _ = ctrl_c_future => {
            info!("received ctrl+c, initiating shutdown");
            let _ = shutdown_tx.send(());
        }
    }

    info!("shutdown complete");

    if let Some(h) = h {
        h.shutdown();
    }

    Ok(())
}

async fn handle_pipe_command(pipe: PipeCommand, pipe_manager: &PipeManager) -> anyhow::Result<()> {
    // Handle pipe subcommands
    match pipe {
        PipeCommand::List => {
            let pipes = pipe_manager.list_pipes().await;
            println!("available pipes:");
            for pipe in pipes {
                println!("  id: {}, enabled: {}", pipe.id, pipe.enabled);
            }
        }
        PipeCommand::Download { url } => match pipe_manager.download_pipe(&url).await {
            Ok(pipe_id) => println!("pipe downloaded successfully. id: {}. now enable it with `screenpipe pipe enable {}`", pipe_id, pipe_id),
            Err(e) => eprintln!("failed to download pipe: {}", e),
        },
        PipeCommand::Info { id } => match pipe_manager.get_pipe_info(&id).await {
            Some(info) => println!("pipe info: {:?}", info),
            None => eprintln!("pipe not found"),
        },
        PipeCommand::Enable { id } => {
            match pipe_manager
                .update_config(&id, json!({"enabled": true}))
                .await
            {
                Ok(_) => println!("pipe {} enabled. now restart screenpipe with `screenpipe`", id),
                Err(e) => eprintln!("failed to enable pipe: {}", e),
            }
        }
        PipeCommand::Disable { id } => {
            match pipe_manager
                .update_config(&id, json!({"enabled": false}))
                .await
            {
                Ok(_) => println!("pipe {} disabled", id),
                Err(e) => eprintln!("failed to disable pipe: {}", e),
            }
        }
        PipeCommand::Update { id, config } => {
            let config: Value = serde_json::from_str(&config)
                .map_err(|e| anyhow::anyhow!("invalid json: {}", e))?;
            match pipe_manager.update_config(&id, config).await {
                Ok(_) => println!("pipe {} config updated", id),
                Err(e) => eprintln!("failed to update pipe config: {}", e),
            }
        }
        PipeCommand::Purge { yes } => {
            if yes {
                match pipe_manager.purge_pipes().await {
                    Ok(_) => println!("all pipes purged successfully."),
                    Err(e) => eprintln!("failed to purge pipes: {}", e),
                }
            } else {
                print!("are you sure you want to purge all pipes? this action cannot be undone. (y/N): ");
                io::stdout().flush()?;
                
                let mut input = String::new();
                io::stdin().read_line(&mut input)?;
                
                if input.trim().to_lowercase() == "y" {
                    match pipe_manager.purge_pipes().await {
                        Ok(_) => println!("all pipes purged successfully."),
                        Err(e) => eprintln!("failed to purge pipes: {}", e),
                    }
                } else {
                    println!("pipe purge cancelled.");
                }
            }
        },
    }
    Ok(())
}