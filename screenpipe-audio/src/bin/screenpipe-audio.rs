use anyhow::{anyhow, Result};
use clap::Parser;
use log::info;
use screenpipe_audio::create_whisper_channel;
use screenpipe_audio::default_input_device;
use screenpipe_audio::default_output_device;
use screenpipe_audio::list_audio_devices;
use screenpipe_audio::parse_audio_device;
use screenpipe_audio::record_and_transcribe;
use screenpipe_audio::AudioDevice;
use std::path::PathBuf;
use std::sync::atomic::AtomicBool;
use std::sync::Arc;
use std::time::Duration;

#[derive(Parser, Debug)]
#[clap(author, version, about, long_about = None)]
struct Args {
    #[clap(
        short,
        long,
        help = "Audio device name (can be specified multiple times)"
    )]
    audio_device: Vec<String>,

    #[clap(long, help = "List available audio devices")]
    list_audio_devices: bool,

    #[clap(long, help = "Disable cloud audio processing")]
    use_cloud_audio: bool,
}

fn print_devices(devices: &[AudioDevice]) {
    println!("Available audio devices:");
    for (i, device) in devices.iter().enumerate() {
        println!("  {}. {}", i + 1, device);
    }
}

// TODO - kinda bad cli here

#[tokio::main]
async fn main() -> Result<()> {
    use env_logger::Builder;
    use log::LevelFilter;

    Builder::new()
        .filter(None, LevelFilter::Info)
        .filter_module("tokenizers", LevelFilter::Error)
        .init();

    let args = Args::parse();

    let devices = list_audio_devices()?;

    if args.list_audio_devices {
        print_devices(&devices);
        return Ok(());
    }

    let devices = if args.audio_device.is_empty() {
        vec![default_input_device()?, default_output_device()?]
    } else {
        args.audio_device
            .iter()
            .map(|d| parse_audio_device(d))
            .collect::<Result<Vec<_>>>()?
    };

    if devices.is_empty() {
        return Err(anyhow!("No audio input devices found"));
    }

    // delete .mp4 files (output*.mp4)
    std::fs::remove_file("output_0.mp4").unwrap_or_default();
    std::fs::remove_file("output_1.mp4").unwrap_or_default();

    let chunk_duration = Duration::from_secs(5);
    let output_path = PathBuf::from("output.mp4");
    let (whisper_sender, mut whisper_receiver) =
        create_whisper_channel(args.use_cloud_audio).await?;
    // Spawn threads for each device
    let recording_threads: Vec<_> = devices
        .into_iter()
        .enumerate()
        .map(|(i, device)| {
            let device = Arc::new(device);
            let whisper_sender = whisper_sender.clone();
            let output_path = output_path.with_file_name(format!("output_{}.mp4", i));
            let device_control = Arc::new(AtomicBool::new(true));
            let device_clone = Arc::clone(&device);

            tokio::spawn(async move {
                let device_control_clone = Arc::clone(&device_control);
                let device_clone_2 = Arc::clone(&device_clone);

                record_and_transcribe(
                    device_clone_2,
                    chunk_duration,
                    output_path,
                    whisper_sender,
                    device_control_clone,
                )
            })
        })
        .collect();
    let mut consecutive_timeouts = 0;
    let max_consecutive_timeouts = 3; // Adjust this value as needed

    // Main loop to receive and print transcriptions
    loop {
        match whisper_receiver.try_recv() {
            Ok(result) => {
                info!("Transcription: {:?}", result);
                consecutive_timeouts = 0; // Reset the counter on successful receive
            }
            Err(_) => {
                consecutive_timeouts += 1;
                if consecutive_timeouts >= max_consecutive_timeouts {
                    info!("No transcriptions received for a while, stopping...");
                    break;
                }
                continue;
            }
        }
    }

    // Wait for all recording threads to finish
    for (i, thread) in recording_threads.into_iter().enumerate() {
        let file_path = thread.await.unwrap().await;
        println!("Recording {} complete: {:?}", i, file_path);
    }

    Ok(())
}
