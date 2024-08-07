use chrono::Utc;
use image::ImageFormat::{self};
use log::{debug, error, info, warn};
use screenpipe_core::find_ffmpeg_path;
use screenpipe_vision::{continuous_capture, get_monitor, CaptureResult, OcrEngine};
use std::collections::VecDeque;
use std::path::PathBuf;
use std::process::Stdio;
use std::sync::Arc;
use tokio::io::AsyncWriteExt;
use tokio::io::{AsyncBufReadExt, BufReader};
use tokio::process::{Child, ChildStdin, Command};
use tokio::sync::mpsc::{channel, Receiver, Sender};
use tokio::sync::Mutex;

use std::time::Duration;

const MAX_FPS: f64 = 30.0; // Adjust based on your needs

#[derive(Debug, Clone)]
pub struct VideoCapture {
    frame_queue: Arc<Mutex<VecDeque<CaptureResult>>>,
    video_frame_queue: Arc<Mutex<VecDeque<CaptureResult>>>,
    pub ocr_frame_queue: Arc<Mutex<VecDeque<CaptureResult>>>,
    stop_signal: Arc<Mutex<bool>>,
}

impl VideoCapture {
    pub fn new(
        output_path: &str,
        fps: f64,
        new_chunk_callback: impl Fn(&str) + Send + Sync + 'static,
        save_text_files: bool,
        ocr_engine: Arc<OcrEngine>,
    ) -> Self {
        info!("Starting new video capture");
        let frame_queue = Arc::new(Mutex::new(VecDeque::new()));
        let video_frame_queue = Arc::new(Mutex::new(VecDeque::new()));
        let ocr_frame_queue = Arc::new(Mutex::new(VecDeque::new()));
        let new_chunk_callback = Arc::new(new_chunk_callback);
        let new_chunk_callback_clone = Arc::clone(&new_chunk_callback);
        let stop_signal = Arc::new(Mutex::new(false));
        let stop_signal_capture = Arc::new(Mutex::new(false));

        let capture_frame_queue = frame_queue.clone();
        let capture_video_frame_queue = video_frame_queue.clone();
        let capture_ocr_frame_queue = ocr_frame_queue.clone();
        let (result_sender, mut result_receiver) = channel(512);
        let _capture_thread = tokio::spawn(async move {
            continuous_capture(
                result_sender,
                Duration::from_secs_f64(1.0 / fps),
                save_text_files,
                ocr_engine,
                get_monitor().await,
                stop_signal_capture,
            )
            .await;
        });

        info!("Started capture thread");

        // Spawn another thread to handle receiving and queueing the results
        let _queue_thread = tokio::spawn(async move {
            while let Some(result) = result_receiver.recv().await {
                // throw error for testing smthing
                // if result.frame_number == 1 {
                //     error!("Error for testing");
                //     return;
                // }
                let frame_number = result.frame_number;
                debug!("Received frame {} for queueing", frame_number);
                let mut queue = capture_frame_queue.lock().await;
                let mut video_queue = capture_video_frame_queue.lock().await;
                let mut ocr_queue = capture_ocr_frame_queue.lock().await;
                queue.push_back(result.clone());
                video_queue.push_back(result.clone());
                ocr_queue.push_back(result);
                debug!("Frame {} pushed to queues. Queue length: {}, Video queue length: {}, OCR queue length: {}", frame_number, queue.len(), video_queue.len(), ocr_queue.len());

                // Clear the old queue after processing
                if queue.len() > 1 {
                    queue.pop_front();
                }
            }
        });

        let video_frame_queue_clone = video_frame_queue.clone();
        let output_path = output_path.to_string();
        let video_capture = VideoCapture {
            frame_queue,
            video_frame_queue,
            ocr_frame_queue,
            stop_signal,
        };
        let video_capture_clone = video_capture.clone();

        let _video_thread = tokio::spawn(async move {
            video_capture_clone
                .save_frames_as_video(
                    &video_frame_queue_clone,
                    &output_path,
                    fps,
                    new_chunk_callback_clone,
                )
                .await;
        });

        video_capture
    }

    pub async fn get_latest_frame(&self) -> Option<CaptureResult> {
        let mut queue = self.frame_queue.lock().await;
        let queue_length = queue.len();
        debug!("Number of frames in queue before popping: {}", queue_length);
        queue.pop_front()
    }

    pub fn get_video_frame_queue(&self) -> Arc<Mutex<VecDeque<CaptureResult>>> {
        Arc::clone(&self.video_frame_queue)
    }

    pub async fn stop(&mut self) {
        debug!("Stopping VideoCapture");
        let mut stop_signal = self.stop_signal.lock().await;
        *stop_signal = true;

        // Wait for a short time to allow other threads to finish
        tokio::time::sleep(Duration::from_secs(1)).await;
        debug!("VideoCapture stopped");
    }

    async fn save_frames_as_video(
        &self,
        frame_queue: &Arc<Mutex<VecDeque<CaptureResult>>>,
        output_path: &str,
        fps: f64,
        new_chunk_callback: Arc<dyn Fn(&str) + Send + Sync>,
    ) {
        debug!("Starting save_frames_as_video function");
        let frames_per_video = 30; // Adjust this value as needed
        let mut frame_count = 0;
        let (sender, mut receiver): (Sender<Vec<u8>>, Receiver<Vec<u8>>) = channel(512);
        let sender = Arc::new(sender);
        let mut current_ffmpeg: Option<Child> = None;
        let mut current_stdin: Option<ChildStdin> = None;

        loop {
            if *self.stop_signal.lock().await {
                debug!("Stopping save_frames_as_video loop");
                break;
            }
            if frame_count % frames_per_video == 0 || current_ffmpeg.is_none() {
                debug!("Starting new FFmpeg process");
                // Close previous FFmpeg process if exists
                if let Some(child) = current_ffmpeg.take() {
                    drop(current_stdin.take()); // Ensure stdin is closed
                    let output = child
                        .wait_with_output()
                        .await
                        .expect("ffmpeg process failed");
                    debug!("FFmpeg process exited with status: {}", output.status);
                    if !output.status.success() {
                        error!("FFmpeg stderr: {}", String::from_utf8_lossy(&output.stderr));
                    }
                }

                // Wait for at least one frame before starting a new FFmpeg process
                let first_frame = loop {
                    if let Some(result) = frame_queue.lock().await.pop_front() {
                        debug!("Got first frame for new chunk");
                        break result;
                    }
                    tokio::time::sleep(Duration::from_millis(10)).await;
                };

                // Encode the first frame
                let mut buffer = Vec::new();
                first_frame
                    .image
                    .write_to(&mut std::io::Cursor::new(&mut buffer), ImageFormat::Png)
                    .expect("Failed to encode first frame");

                let time = Utc::now();
                let formatted_time = time.format("%Y-%m-%d_%H-%M-%S").to_string();
                // Start new FFmpeg process with a new output file
                let output_file = PathBuf::from(output_path)
                    .join(format!("{}.mp4", formatted_time))
                    .to_str()
                    .expect("Failed to create valid path")
                    .to_string();

                // Call the callback with the new video chunk file path
                new_chunk_callback(&output_file);

                match start_ffmpeg_process(&output_file, fps).await {
                    Ok(mut child) => {
                        let mut stdin = child.stdin.take().expect("Failed to open stdin");
                        let stderr = child.stderr.take().expect("Failed to open stderr");
                        let stdout = child.stdout.take().expect("Failed to open stdout");

                        // Write the first frame to FFmpeg
                        stdin
                            .write_all(&buffer)
                            .await
                            .expect("Failed to write first frame to ffmpeg");
                        frame_count += 1;

                        // Spawn a task to log FFmpeg's stderr
                        tokio::spawn(async move {
                            let reader = BufReader::new(stderr);
                            let mut lines = reader.lines();
                            while let Ok(Some(line)) = lines.next_line().await {
                                debug!("FFmpeg: {}", line);
                            }
                        });

                        // Log FFmpeg's stdout
                        tokio::spawn(async move {
                            let reader = BufReader::new(stdout);
                            let mut lines = reader.lines();
                            while let Ok(Some(line)) = lines.next_line().await {
                                debug!("FFmpeg: {}", line);
                            }
                        });

                        current_ffmpeg = Some(child);
                        current_stdin = Some(stdin);
                        debug!("New FFmpeg process started for file: {}", output_file);
                    }
                    Err(e) => {
                        error!("Failed to start FFmpeg process: {}", e);
                        // Handle the error appropriately, maybe try to restart or exit
                    }
                }
            }

            if let Some(result) = frame_queue.lock().await.pop_front() {
                debug!("Processing frame in video.rs"); // {}", frame_count + 1
                let sender = Arc::clone(&sender);

                tokio::spawn(async move {
                    let mut buffer = Vec::new();
                    match result
                        .image
                        .write_to(&mut std::io::Cursor::new(&mut buffer), ImageFormat::Png)
                    {
                        Ok(_) => {
                            sender
                                .send(buffer)
                                .await
                                .expect("Failed to send encoded frame");
                        }
                        Err(e) => error!("Failed to encode image as PNG: {}", e),
                    }
                });
            } else {
                // debug!("No frames in queue, waiting...");
                tokio::time::sleep(Duration::from_millis(10)).await;
            }

            // Write encoded frames to FFmpeg
            while let Ok(buffer) = receiver.try_recv() {
                if let Some(stdin) = current_stdin.as_mut() {
                    match stdin.write_all(buffer.as_slice()).await {
                        Ok(_) => {
                            frame_count += 1;
                            debug!("Wrote frame {} to FFmpeg", frame_count);
                        }
                        Err(e) => {
                            error!("Failed to write frame to ffmpeg: {}", e);
                            if e.kind() == std::io::ErrorKind::BrokenPipe {
                                warn!("FFmpeg process may have terminated unexpectedly. Restarting...");
                                return;
                            }
                        }
                    }

                    // Flush every second
                    if frame_count % fps as usize == 0 {
                        debug!("Flushing FFmpeg input");
                        if let Err(e) = stdin.flush().await {
                            error!("Failed to flush FFmpeg input: {}", e);
                        }
                    }
                }
            }

            // Yield to other tasks periodically
            if frame_count % 100 == 0 {
                tokio::task::yield_now().await;
            }
        }
    }
}

use std::env;

async fn start_ffmpeg_process(output_file: &str, fps: f64) -> Result<Child, anyhow::Error> {
    // Overriding fps with max fps if over the max and warning user
    let fps = if fps > MAX_FPS {
        warn!("Overriding FPS from {} to {}", fps, MAX_FPS);
        MAX_FPS
    } else {
        fps
    };

    info!("Starting FFmpeg process for file: {}", output_file);
    let fps_str = fps.to_string();
    let mut command = Command::new(find_ffmpeg_path().unwrap());
    let mut args = vec![
        "-f",
        "image2pipe",
        "-vcodec",
        "png",
        "-r",
        &fps_str,
        "-i",
        "-",
    ];

    if env::consts::OS == "windows" {
        // TODO switch back to libx264 when ffmpeg is updated in pre_build.js
        // Use MPEG-4 encoder for Windows
        args.extend_from_slice(&[
            "-vcodec",
            "mpeg4",
            "-q:v",
            "5", // Adjust quality (1-31, lower is better)
            "-preset",
            "ultrafast",
        ]);
    } else {
        // Use libx264 for other platforms
        args.extend_from_slice(&["-vcodec", "libx264", "-preset", "ultrafast", "-crf", "23"]);
    }

    args.extend_from_slice(&["-pix_fmt", "yuv420p", output_file]);

    command
        .args(&args)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());

    debug!("FFmpeg command: {:?}", command);

    let child = command.spawn()?;
    debug!("FFmpeg process spawned");

    Ok(child)
}
