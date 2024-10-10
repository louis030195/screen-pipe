use std::{
    path::PathBuf,
    sync::{Arc, Mutex},
    time::{SystemTime, UNIX_EPOCH},
};

use anyhow::Result;
use candle::Tensor;
use chrono::Utc;
use log::{debug, error, info};
#[cfg(target_os = "macos")]
use objc::rc::autoreleasepool;

use candle_transformers::models::whisper::{self as m, audio};
use rubato::{
    Resampler, SincFixedIn, SincInterpolationParameters, SincInterpolationType, WindowFunction,
};

use crate::{
    audio_processing::normalize_v2,
    encode_single_audio, multilingual,
    vad_engine::{SileroVad, VadEngine, VadEngineEnum, VadSensitivity, WebRtcVad},
    whisper::{Decoder, WhisperModel},
    AudioDevice, AudioTranscriptionEngine, DeviceType,
};

use hound::{WavSpec, WavWriter};
use std::io::Cursor;
use reqwest::Client;
use serde_json::Value;

// Replace the get_deepgram_api_key function with this:
fn get_deepgram_api_key() -> String {
    "7ed2a159a094337b01fd8178b914b7ae0e77822d".to_string()
}

async fn transcribe_with_deepgram(
    api_key: &str,
    audio_data: &[f32],
    device: &str,
    sample_rate: u32,
) -> Result<String> {
    debug!("starting deepgram transcription");
    let client = Client::new();

    // Create a WAV file in memory
    let mut cursor = Cursor::new(Vec::new());
    {
        let spec = WavSpec {
            channels: 1,
            sample_rate: match sample_rate {
                88200 => 16000,       // Deepgram expects 16kHz for 88.2kHz
                _ => sample_rate / 3, // Fallback for other sample rates
            },
            bits_per_sample: 32,
            sample_format: hound::SampleFormat::Float,
        };
        let mut writer = WavWriter::new(&mut cursor, spec)?;
        for &sample in audio_data {
            writer.write_sample(sample)?;
        }
        writer.finalize()?;
    }

    // Get the WAV data from the cursor
    let wav_data = cursor.into_inner();

    let response = client
        .post("https://api.deepgram.com/v1/listen?model=nova-2&smart_format=true")
        .header("Content-Type", "audio/wav")
        .header("Authorization", format!("Token {}", api_key))
        .body(wav_data)
        .send();

    match response.await {
        Ok(resp) => {
            debug!("received response from deepgram api");
            match resp.json::<Value>().await {
                Ok(result) => {
                    debug!("successfully parsed json response");
                    if let Some(err_code) = result.get("err_code") {
                        error!(
                            "deepgram api error code: {:?}, result: {:?}",
                            err_code, result
                        );
                        return Err(anyhow::anyhow!("Deepgram API error: {:?}", result));
                    }
                    let transcription = result["results"]["channels"][0]["alternatives"][0]
                        ["transcript"]
                        .as_str()
                        .unwrap_or("");

                    if transcription.is_empty() {
                        info!(
                            "device: {}, transcription is empty. full response: {:?}",
                            device, result
                        );
                    } else {
                        info!(
                            "device: {}, transcription successful. length: {} characters",
                            device,
                            transcription.len()
                        );
                    }

                    Ok(transcription.to_string())
                }
                Err(e) => {
                    error!("Failed to parse JSON response: {:?}", e);
                    Err(anyhow::anyhow!("Failed to parse JSON response: {:?}", e))
                }
            }
        }
        Err(e) => {
            error!("Failed to send request to Deepgram API: {:?}", e);
            Err(anyhow::anyhow!(
                "Failed to send request to Deepgram API: {:?}",
                e
            ))
        }
    }
}

pub fn stt_sync(
    audio_input: &AudioInput,
    whisper_model: &WhisperModel,
    audio_transcription_engine: Arc<AudioTranscriptionEngine>,
    vad_engine: Arc<Mutex<Box<dyn VadEngine + Send>>>, // Changed type here
    deepgram_api_key: Option<String>,
    output_path: &PathBuf,
) -> Result<(String, String)> {
    let audio_input = audio_input.clone();
    let whisper_model = whisper_model.clone();
    let output_path = output_path.clone();
    let vad_engine = vad_engine.clone(); // Clone the Arc to move into the closure

    let handle = std::thread::spawn(move || {
        let rt = tokio::runtime::Runtime::new().unwrap();
        let mut vad_engine_guard = vad_engine.lock().unwrap();

        rt.block_on(stt(
            &audio_input,
            &whisper_model,
            audio_transcription_engine,
            &mut **vad_engine_guard, // Obtain &mut dyn VadEngine
            deepgram_api_key,
            &output_path,
            false,
        ))
    });

    handle.join().unwrap()
}

pub async fn stt(
    audio_input: &AudioInput,
    whisper_model: &WhisperModel,
    audio_transcription_engine: Arc<AudioTranscriptionEngine>,
    vad_engine: &mut dyn VadEngine,
    deepgram_api_key: Option<String>,
    output_path: &PathBuf,
    skip_encoding: bool,
) -> Result<(String, String)> {
    let model = &whisper_model.model;
    let tokenizer = &whisper_model.tokenizer;
    let device = &whisper_model.device;

    debug!("Loading mel filters");
    let mel_bytes = match model.config().num_mel_bins {
        80 => include_bytes!("../models/whisper/melfilters.bytes").as_slice(),
        128 => include_bytes!("../models/whisper/melfilters128.bytes").as_slice(),
        nmel => anyhow::bail!("unexpected num_mel_bins {nmel}"),
    };
    let mut mel_filters = vec![0f32; mel_bytes.len() / 4];
    <byteorder::LittleEndian as byteorder::ByteOrder>::read_f32_into(mel_bytes, &mut mel_filters);

    let audio_data = if audio_input.sample_rate != m::SAMPLE_RATE as u32 {
        info!(
            "device: {}, resampling from {} Hz to {} Hz",
            audio_input.device,
            audio_input.sample_rate,
            m::SAMPLE_RATE
        );
        resample(
            audio_input.data.as_ref(),
            audio_input.sample_rate,
            m::SAMPLE_RATE as u32,
        )?
    } else {
        audio_input.data.as_ref().to_vec()
    };

    let audio_data = if audio_input.device.device_type == DeviceType::Input {
        normalize_v2(&audio_data)
    } else { // ! for some reason buggy on output devices
        audio_data
    };

    let frame_size = 1600; // 100ms frame size for 16kHz audio
    let mut speech_frames = Vec::new();
    let mut total_frames = 0;
    let mut speech_frame_count = 0;

    let mut  noise = 0.;

    for chunk in audio_data.chunks(frame_size) {
        total_frames += 1;
        match vad_engine.audio_type(chunk) {
            Ok(status) => {
                match status {
                    VadStatus::Speech => {
                        let processed_audio = spectral_subtraction(chunk, noise)?;
                        speech_frames.extend(processed_audio);
                        speech_frame_count += 1;
                    },
                    VadStatus::Unknown => {
                        noise = average_noise_spectrum(chunk);
                    },
                    _  => {}
                }
            }
            Err(e) => {
                debug!("VAD failed for chunk: {:?}", e);
            }
        }
    }

    let speech_duration_ms = speech_frame_count * 100; // Each frame is 100ms
    let speech_ratio = speech_frame_count as f32 / total_frames as f32;
    let min_speech_ratio = vad_engine.get_min_speech_ratio();

    info!(
        "device: {}, total audio frames processed: {}, frames that include speech: {}, speech duration: {}ms, speech ratio: {:.2}, min required ratio: {:.2}",
        audio_input.device,
        total_frames,
        speech_frame_count,
        speech_duration_ms,
        speech_ratio,
        min_speech_ratio
    );

    // If no speech frames detected or speech ratio is too low, skip processing
    if speech_frames.is_empty() || speech_ratio < min_speech_ratio {
        debug!(
            "device: {}, insufficient speech detected (ratio: {:.2}, min required: {:.2}), skipping audio processing",
            audio_input.device,
            speech_ratio,
            min_speech_ratio
        );
        return Ok(("".to_string(), "".to_string()));
    }

    let transcription: Result<String> =
        if audio_transcription_engine == AudioTranscriptionEngine::Deepgram.into() {
            // Deepgram implementation
            //check if key is set or empty or no chars in it
            let api_key = if deepgram_api_key.clone().is_some()
                && !deepgram_api_key.clone().unwrap().is_empty()
                && deepgram_api_key.clone().unwrap().chars().count() > 0
            {
                deepgram_api_key.clone().unwrap()
            } else {
                get_deepgram_api_key()
            };
            info!(
                "device: {}, using deepgram api key: {}...",
                audio_input.device,
                &api_key[..8]
            );
            match transcribe_with_deepgram(
                &api_key,
                &speech_frames,
                &audio_input.device.name,
                audio_input.sample_rate,
            )
            .await
            {
                Ok(transcription) => Ok(transcription),
                Err(e) => {
                    error!(
                        "device: {}, deepgram transcription failed, falling back to Whisper: {:?}",
                        audio_input.device, e
                    );
                    // Existing Whisper implementation
                    debug!(
                        "device: {}, converting pcm to mel spectrogram",
                        audio_input.device
                    );
                    let mel = audio::pcm_to_mel(&model.config(), &speech_frames, &mel_filters);
                    let mel_len = mel.len();
                    debug!(
                        "device: {}, creating tensor from mel spectrogram",
                        audio_input.device
                    );
                    let mel = Tensor::from_vec(
                        mel,
                        (
                            1,
                            model.config().num_mel_bins,
                            mel_len / model.config().num_mel_bins,
                        ),
                        &device,
                    )?;

                    debug!("device: {}, detecting language", audio_input.device);
                    let language_token = Some(multilingual::detect_language(
                        &mut model.clone(),
                        &tokenizer,
                        &mel,
                    )?);
                    let mut model = model.clone();
                    debug!("device: {}, initializing decoder", audio_input.device);
                    let mut dc = Decoder::new(
                        &mut model,
                        tokenizer,
                        42,
                        &device,
                        language_token,
                        true,
                        false,
                    )?;
                    debug!("device: {}, starting decoding process", audio_input.device);
                    let segments = dc.run(&mel)?;
                    debug!("device: {}, decoding complete", audio_input.device);
                    Ok(segments
                        .iter()
                        .map(|s| s.dr.text.clone())
                        .collect::<Vec<String>>()
                        .join("\n"))
                }
            }
        } else {
            // Existing Whisper implementation
            debug!(
                "device: {}, starting whisper transcription",
                audio_input.device
            );
            debug!(
                "device: {}, converting pcm to mel spectrogram",
                audio_input.device
            );
            let mel = audio::pcm_to_mel(&model.config(), &speech_frames, &mel_filters);
            let mel_len = mel.len();
            debug!(
                "device: {}, creating tensor from mel spectrogram",
                audio_input.device
            );
            let mel = Tensor::from_vec(
                mel,
                (
                    1,
                    model.config().num_mel_bins,
                    mel_len / model.config().num_mel_bins,
                ),
                &device,
            )?;

            debug!("device: {}, detecting language", audio_input.device);
            let language_token = Some(multilingual::detect_language(
                &mut model.clone(),
                &tokenizer,
                &mel,
            )?);
            let mut model = model.clone();
            debug!("device: {}, initializing decoder", audio_input.device);
            let mut dc = Decoder::new(
                &mut model,
                tokenizer,
                42,
                &device,
                language_token,
                true,
                false,
            )?;
            debug!("device: {}, starting decoding process", audio_input.device);
            let segments = dc.run(&mel)?;
            debug!("device: {}, decoding complete", audio_input.device);
            Ok(segments
                .iter()
                .map(|s| s.dr.text.clone())
                .collect::<Vec<String>>()
                .join("\n"))
        };
    let new_file_name = Utc::now().format("%Y-%m-%d_%H-%M-%S").to_string();
    let sanitized_device_name = audio_input.device.to_string().replace(['/', '\\'], "_");
    let file_path = PathBuf::from(output_path)
        .join(format!("{}_{}.mp4", sanitized_device_name, new_file_name))
        .to_str()
        .expect("Failed to create valid path")
        .to_string();
    let file_path_clone = file_path.clone();
    // Run FFmpeg in a separate task
    if !skip_encoding {
        encode_single_audio(
            bytemuck::cast_slice(&audio_input.data),
            audio_input.sample_rate,
            audio_input.channels,
            &file_path.into(),
        )?;
    }

    Ok((transcription?, file_path_clone))
}

fn resample(input: &[f32], from_sample_rate: u32, to_sample_rate: u32) -> Result<Vec<f32>> {
    debug!("Resampling audio");
    let params = SincInterpolationParameters {
        sinc_len: 256,
        f_cutoff: 0.95,
        interpolation: SincInterpolationType::Linear,
        oversampling_factor: 256,
        window: WindowFunction::BlackmanHarris2,
    };

    let mut resampler = SincFixedIn::<f32>::new(
        to_sample_rate as f64 / from_sample_rate as f64,
        2.0,
        params,
        input.len(),
        1,
    )?;

    let waves_in = vec![input.to_vec()];
    debug!("Performing resampling");
    let waves_out = resampler.process(&waves_in, None)?;
    debug!("Resampling complete");
    Ok(waves_out.into_iter().next().unwrap())
}

#[derive(Debug, Clone)]
pub struct AudioInput {
    pub data: Arc<Vec<f32>>,
    pub sample_rate: u32,
    pub channels: u16,
    pub device: Arc<AudioDevice>,
}

#[derive(Debug, Clone)]
pub struct TranscriptionResult {
    pub path: String,
    pub input: AudioInput,
    pub transcription: Option<String>,
    pub timestamp: u64,
    pub error: Option<String>,
}
use std::sync::atomic::{AtomicBool, Ordering};
use vad_rs::VadStatus;
use crate::audio_processing::{average_noise_spectrum, spectral_subtraction};

pub async fn create_whisper_channel(
    audio_transcription_engine: Arc<AudioTranscriptionEngine>,
    vad_engine: VadEngineEnum,
    deepgram_api_key: Option<String>,
    output_path: &PathBuf,
    vad_sensitivity: VadSensitivity,
) -> Result<(
    crossbeam::channel::Sender<AudioInput>,
    crossbeam::channel::Receiver<TranscriptionResult>,
    Arc<AtomicBool>, // Shutdown flag
)> {
    let whisper_model = WhisperModel::new(&audio_transcription_engine)?;
    let (input_sender, input_receiver): (
        crossbeam::channel::Sender<AudioInput>,
        crossbeam::channel::Receiver<AudioInput>,
    ) = crossbeam::channel::bounded(20);
    let (output_sender, output_receiver): (
        crossbeam::channel::Sender<TranscriptionResult>,
        crossbeam::channel::Receiver<TranscriptionResult>,
    ) = crossbeam::channel::bounded(20);
    let mut vad_engine: Box<dyn VadEngine + Send> = match vad_engine {
        VadEngineEnum::WebRtc => Box::new(WebRtcVad::new()),
        VadEngineEnum::Silero => Box::new(SileroVad::new().await?),
    };
    vad_engine.set_sensitivity(vad_sensitivity);
    let vad_engine = Arc::new(Mutex::new(vad_engine));
    let shutdown_flag = Arc::new(AtomicBool::new(false));
    let shutdown_flag_clone = shutdown_flag.clone();
    let output_path = output_path.clone();

    tokio::spawn(async move {
        loop {
            if shutdown_flag_clone.load(Ordering::Relaxed) {
                info!("Whisper channel shutting down");
                break;
            }
            debug!("Waiting for input from input_receiver");

            crossbeam::select! {
                recv(input_receiver) -> input_result => {
                    match input_result {
                        Ok(input) => {
                            debug!("Received input from input_receiver");
                            let timestamp = SystemTime::now()
                                .duration_since(UNIX_EPOCH)
                                .expect("Time went backwards")
                                .as_secs();

                            let transcription_result = if cfg!(target_os = "macos") {
                                #[cfg(target_os = "macos")]
                                {
                                    autoreleasepool(|| {
                                        match stt_sync(&input, &whisper_model, audio_transcription_engine.clone(), vad_engine.clone(), deepgram_api_key.clone(), &output_path) {
                                            Ok((transcription, path)) => TranscriptionResult {
                                                input: input.clone(),
                                                transcription: Some(transcription),
                                                path,
                                                timestamp,
                                                error: None,
                                            },
                                            Err(e) => {
                                                error!("STT error for input {}: {:?}", input.device, e);
                                                TranscriptionResult {
                                                    input: input.clone(),
                                                    transcription: None,
                                                    path: "".to_string(),
                                                    timestamp,
                                                    error: Some(e.to_string()),
                                                }
                                            },
                                        }
                                    })
                                }
                                #[cfg(not(target_os = "macos"))]
                                {
                                    unreachable!("This code should not be reached on non-macOS platforms")
                                }
                            } else {
                                match stt_sync(&input, &whisper_model, audio_transcription_engine.clone(), vad_engine.clone(), deepgram_api_key.clone(), &output_path) {
                                    Ok((transcription, path)) => TranscriptionResult {
                                        input: input.clone(),
                                        transcription: Some(transcription),
                                        path,
                                        timestamp,
                                        error: None,
                                    },
                                    Err(e) => {
                                        error!("STT error for input {}: {:?}", input.device, e);
                                        TranscriptionResult {
                                            input: input.clone(),
                                            transcription: None,
                                            path: "".to_string(),
                                            timestamp,
                                            error: Some(e.to_string()),
                                        }
                                    },
                                }
                            };

                            if output_sender.send(transcription_result).is_err() {
                                break;
                            }
                        },
                        Err(e) => {
                            error!("Error receiving input: {:?}", e);
                            // Depending on the error type, you might want to break the loop or continue
                            // For now, we'll continue to the next iteration
                            break;
                        }
                    }
                },
            }
        }
        // Cleanup code here (if needed)
    });

    Ok((input_sender, output_receiver, shutdown_flag))
}
