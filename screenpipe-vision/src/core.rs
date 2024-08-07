use image::DynamicImage;
use log::{debug, error};
use serde_json;
use std::{
    collections::{HashMap, HashSet},
    sync::Arc,
    time::{Duration, Instant},
};
use strsim::levenshtein;
use tokio::sync::{mpsc::Sender, Mutex}; // Corrected import for Mutex
use xcap::{Monitor, Window};

#[cfg(target_os = "windows")]
use crate::utils::perform_ocr_windows;
use crate::utils::OcrEngine;
use crate::utils::{
    capture_screenshot, compare_with_previous_image, perform_ocr_tesseract, save_text_files,
};
use rusty_tesseract::{Data, DataOutput}; // Add this import
use screenpipe_integrations::unstructured_ocr::perform_ocr_cloud;

pub struct DataOutputWrapper {
    pub data_output: rusty_tesseract::tesseract::output_data::DataOutput,
}

impl DataOutputWrapper {
    pub fn to_json(&self) -> String {
        let data_json: Vec<String> = self.data_output.data.iter().map(|d| {
            format!(
                r#"{{"level": {}, "page_num": {}, "block_num": {}, "par_num": {}, "line_num": {}, "word_num": {}, "left": {}, "top": {}, "width": {}, "height": {}, "conf": {}, "text": "{}"}}"#,
                d.level, d.page_num, d.block_num, d.par_num, d.line_num, d.word_num, d.left, d.top, d.width, d.height, d.conf, d.text
            )
        }).collect();
        format!(
            r#"{{"output": "{}", "data": [{}]}}"#,
            self.data_output.output,
            data_json.join(", ")
        )
    }
}

#[derive(Debug)]
pub struct CaptureResult {
    pub image: Arc<DynamicImage>,
    pub text: String,
    pub text_json: Vec<HashMap<String, String>>,
    pub new_text_json: Vec<HashMap<String, String>>,
    pub frame_number: u64,
    pub timestamp: Instant,
    pub data_output: DataOutput,
    pub app_name: String,
}

impl Clone for CaptureResult {
    fn clone(&self) -> Self {
        CaptureResult {
            image: Arc::clone(&self.image),
            text: self.text.clone(),
            text_json: self.text_json.clone(),
            new_text_json: self.new_text_json.clone(),
            frame_number: self.frame_number,
            timestamp: self.timestamp,
            data_output: DataOutput {
                output: self.data_output.output.clone(),
                data: self
                    .data_output
                    .data
                    .iter()
                    .map(|d| Data {
                        level: d.level,
                        page_num: d.page_num,
                        block_num: d.block_num,
                        par_num: d.par_num,
                        line_num: d.line_num,
                        word_num: d.word_num,
                        left: d.left,
                        top: d.top,
                        width: d.width,
                        height: d.height,
                        conf: d.conf,
                        text: d.text.clone(),
                    })
                    .collect(),
            },
            app_name: self.app_name.clone(),
        }
    }
}

pub struct OcrTaskData {
    pub image: Arc<DynamicImage>,
    pub frame_number: u64,
    pub timestamp: Instant,
    pub result_tx: Sender<CaptureResult>,
}

pub async fn get_monitor() -> Monitor {
    Monitor::all().unwrap().first().unwrap().clone()
}

pub async fn continuous_capture(
    result_tx: Sender<CaptureResult>,
    interval: Duration,
    save_text_files_flag: bool,
    ocr_engine: Arc<OcrEngine>,
    monitor: Monitor,
    stop_signal: Arc<Mutex<bool>>,
) {
    debug!("continuous_capture: Starting using monitor: {:?}", monitor);
    let previous_text_json = Arc::new(Mutex::new(None));
    let mut frame_counter: u64 = 0;
    // let start_time = Instant::now();
    let mut previous_image: Option<Arc<DynamicImage>> = None;
    let mut max_average: Option<MaxAverageFrame> = None;
    let mut max_avg_value = 0.0;

    loop {
        if *stop_signal.lock().await {
            debug!("Stopping continuous_capture loop");
            break;
        }
        let (image, image_hash, _capture_duration) = capture_screenshot(&monitor).await;
        let current_average = compare_with_previous_image(
            &previous_image,
            &image,
            &mut max_average,
            frame_counter,
            &mut max_avg_value,
        )
        .await;

        // Account for situation when there is no previous image
        let current_average = if previous_image.is_none() {
            1.0 // Default value to ensure the frame is processed
        } else {
            current_average
        };

        // Skip the frame if the current average difference is less than 0.006
        if current_average < 0.006 {
            debug!(
                "Skipping frame {} due to low average difference: {:.3}",
                frame_counter, current_average
            );
            frame_counter += 1;
            tokio::time::sleep(interval).await;
            continue;
        }

        if current_average > max_avg_value {
            max_average = Some(MaxAverageFrame {
                image: Arc::new(image.clone()),
                image_hash,
                frame_number: frame_counter,
                timestamp: Instant::now(),
                result_tx: result_tx.clone(),
                average: current_average,
            });
            max_avg_value = current_average;
        }

        previous_image = Some(Arc::new(image.clone()));
        // debug!("ocr_task_running {} BEFORE if if !ocr_task_running.load(Ordering::SeqCst)", ocr_task_running.load(Ordering::SeqCst));

        // debug!("max_avg_frame {} before if let Some(", max_avg_value);
        if let Some(max_avg_frame) = max_average.take() {
            // Use take() to move out the value
            let ocr_task_data = OcrTaskData {
                image: max_avg_frame.image.clone(),
                frame_number: max_avg_frame.frame_number,
                timestamp: max_avg_frame.timestamp,
                result_tx: result_tx.clone(),
            };

            let previous_text_json_clone = previous_text_json.clone();

            let ocr_engine_clone = ocr_engine.clone();
            let w = Window::all().unwrap().first().unwrap().clone();
            let app_name = w.app_name();
            if let Err(e) = process_ocr_task(
                ocr_task_data.image,
                ocr_task_data.frame_number,
                ocr_task_data.timestamp,
                ocr_task_data.result_tx,
                &previous_text_json_clone,
                save_text_files_flag, // Pass the flag here
                ocr_engine_clone,     // Pass the cloud_ocr flag here
                app_name.to_string().to_lowercase(),
            )
            .await
            {
                error!("Error processing OCR task: {}", e);
            }

            frame_counter = 0; // Reset frame_counter after OCR task is processed
                               // Reset max_average and max_avg_value after spawning the OCR task
            max_avg_value = 0.0;
        }

        frame_counter += 1;
        tokio::time::sleep(interval).await;
    }
}

pub struct MaxAverageFrame {
    pub image: Arc<DynamicImage>,
    pub image_hash: u64,
    pub frame_number: u64,
    pub timestamp: Instant,
    pub result_tx: Sender<CaptureResult>,
    pub average: f64,
}

pub async fn process_ocr_task(
    image_arc: Arc<DynamicImage>,
    frame_number: u64,
    timestamp: Instant,
    result_tx: Sender<CaptureResult>,
    previous_text_json: &Arc<Mutex<Option<Vec<HashMap<String, String>>>>>,
    save_text_files_flag: bool, // Add this parameter
    ocr_engine: Arc<OcrEngine>, // Add this parameter
    app_name: String,
) -> Result<(), std::io::Error> {
    let start_time = Instant::now();

    // not to confuse with frame id which is wholly different thing
    debug!(
        "Performing OCR for frame number since beginning of program {}",
        frame_number
    );
    let (text, data_output, json_output) = match &*ocr_engine {
        OcrEngine::Unstructured => {
            perform_ocr_cloud(&image_arc).await
        }
        OcrEngine::Tesseract => {
            perform_ocr_tesseract(&image_arc)
        }
        #[cfg(target_os = "windows")]
        OcrEngine::WindowsNative => {
            perform_ocr_windows(&image_arc).await
        }
        _ => {
            error!("Unsupported OCR engine");
            return Err(std::io::Error::new(
                // TODO we should use anyhow everywhere for error
                std::io::ErrorKind::Other,
                "Unsupported OCR engine",
            ));
        }
    };

    let current_text_json: Vec<HashMap<String, String>> = serde_json::from_str(&json_output)
        .unwrap_or_else(|e| {
            error!("Failed to parse JSON output: {}", e);
            Vec::new()
        });

    let mut previous_text_json = previous_text_json.lock().await;
    let mut new_text_json = Vec::new();
    if let Some(prev_json) = &*previous_text_json {
        for current_record in &current_text_json {
            let confidence: f64 = current_record["confidence"].parse().unwrap_or(0.0);
            if confidence > 60.0 {
                let is_new = prev_json.iter().all(|prev_record| {
                    let distance = levenshtein(&current_record["text"], &prev_record["text"]);
                    let threshold = (prev_record["text"].len() as f64 * 0.1).ceil() as usize;
                    distance > threshold
                });
                if is_new {
                    new_text_json.push(current_record.clone());
                }
            }
        }
    } else {
        new_text_json = current_text_json
            .iter()
            .filter(|record| record["confidence"].parse::<f64>().unwrap_or(0.0) > 60.0)
            .cloned()
            .collect();
    }

    let mut seen_texts = HashSet::new();
    new_text_json.retain(|record| seen_texts.insert(record["text"].clone()));

    if save_text_files_flag {
        save_text_files(
            frame_number,
            &new_text_json,
            &current_text_json,
            &previous_text_json,
        )
        .await;
    }

    *previous_text_json = Some(current_text_json.clone());

    if let Err(e) = result_tx
        .send(CaptureResult {
            image: image_arc.into(),
            text: text.clone(),
            text_json: current_text_json,
            new_text_json,
            frame_number,
            timestamp,
            data_output,
            app_name,
        })
        .await
    {
        error!("Failed to send OCR result: {}", e);
        return Err(std::io::Error::new(
            std::io::ErrorKind::Other,
            "Failed to send OCR result",
        ));
    }
    let _duration = start_time.elapsed();
    debug!(
        "OCR task processed frame {} in {:?}",
        frame_number, _duration
    );
    Ok(())
}
