#[cfg(test)]
mod tests {
    use super::*;
    use chrono::Utc;
    use log::{debug, LevelFilter};
    use screenpipe_audio::{
        default_output_device, list_audio_devices, stt, AudioTranscriptionEngine, WhisperModel,
    };
    use screenpipe_audio::{parse_audio_device, record_and_transcribe,create_vad_engine, VadEngineEnum,VadEngine};
    use std::path::PathBuf;
    use std::process::Command;
    use std::str::FromStr;
    use std::sync::atomic::{AtomicBool, Ordering};
    use std::sync::Arc;
    use std::time::{Duration, Instant};
    use tokio::sync::mpsc::unbounded_channel;

    fn setup() {
        // Initialize the logger with an info level filter
        match env_logger::builder()
            .filter_level(log::LevelFilter::Debug)
            .filter_module("tokenizers", LevelFilter::Error)
            .try_init()
        {
            Ok(_) => (),
            Err(_) => (),
        };
    }

    #[tokio::test]
    async fn test_webrtc_vad() {
        setup();

        let mut vad_engine = create_vad_engine(VadEngineEnum::WebRtc).unwrap();
        let audio_chunk = vec![0; 16000]; // Example silent audio chunk
        let is_voice = vad_engine.is_voice_segment(&audio_chunk)?;
        assert!(!is_voice, "WebRtc VAD should not detect voice in silent audio");

        Ok(())
    }

    #[tokio::test]
    async fn test_silero_vad() {
        setup();

        let mut vad_engine = create_vad_engine(VadEngineEnum::Silero).unwrap();
        let audio_chunk = vec![0; 16000]; // Example silent audio chunk
        let is_voice = vad_engine.is_voice_segment(&audio_chunk)?;
        assert!(!is_voice, "Silero VAD should not detect voice in silent audio");

        Ok(())
    }

    #[tokio::test]
    async fn test_webrtc_vad_with_voice() {
        setup();

        let mut vad_engine = create_vad_engine(VadEngineEnum::WebRtc).unwrap();
        let audio_chunk = vec![1; 16000]; // Example non-silent audio chunk
        let is_voice = vad_engine.is_voice_segment(&audio_chunk)?;
        assert!(is_voice, "WebRtc VAD should detect voice in non-silent audio");

        Ok(())
    }

    #[tokio::test]
    async fn test_silero_vad_with_voice() {
        setup();

        let mut vad_engine = create_vad_engine(VadEngineEnum::Silero).unwrap();
        let audio_chunk = vec![1; 16000]; // Example non-silent audio chunk
        let is_voice = vad_engine.is_voice_segment(&audio_chunk)?;
        assert!(is_voice, "Silero VAD should detect voice in non-silent audio");

        Ok(())
    }

    // ! what happen in github action?
    #[tokio::test]
    #[ignore]
    async fn test_list_audio_devices() {
        let devices = list_audio_devices().await.unwrap();
        assert!(!devices.is_empty());
    }

    #[test]
    fn test_parse_audio_device() {
        let spec = parse_audio_device("Test Device (input)").unwrap();
        assert_eq!(spec.to_string(), "Test Device (input)");
    }

    #[test]
    #[ignore]
    fn test_speech_to_text() {
        setup();
        println!("Starting speech to text test");
        println!("Loading audio file");
        let start = std::time::Instant::now();
        let whisper_model =
            WhisperModel::new(Arc::new(AudioTranscriptionEngine::WhisperTiny)).unwrap();

        let text = stt(
            "./test_data/selah.mp4",
            &whisper_model,
            Arc::new(AudioTranscriptionEngine::WhisperTiny),
        )
        .unwrap();
        let duration = start.elapsed();

        println!("Speech to text completed in {:?}", duration);
        println!("Transcribed text: {:?}", text);

        assert!(text.contains("love"));
    }

    #[tokio::test]
    #[ignore] // Add this if you want to skip this test in regular test runs
    async fn test_record_and_transcribe() {
        setup();

        // Setup
        let device_spec = Arc::new(default_output_device().await.unwrap());
        let duration = Duration::from_secs(30); // Record for 3 seconds
        let time = Utc::now().timestamp_millis();
        let output_path = PathBuf::from(format!("test_output_{}.mp4", time));
        let (sender, mut receiver) = unbounded_channel();
        let is_running = Arc::new(AtomicBool::new(true));

        // Act
        let start_time = Instant::now();
        println!("Starting record_and_transcribe");
        let result = record_and_transcribe(
            device_spec,
            duration,
            output_path.clone(),
            sender,
            is_running,
        )
        .await;
        println!("record_and_transcribe completed");
        let elapsed_time = start_time.elapsed();

        // Assert
        assert!(result.is_ok(), "record_and_transcribe should succeed");

        // Check if the recording duration is close to the specified duration
        assert!(
            elapsed_time >= duration && elapsed_time < duration + Duration::from_secs(3),
            "Recording duration should be close to the specified duration"
        );

        // Check if the file was created
        assert!(output_path.exists(), "Output file should exist");

        // Check if we received the correct AudioInput
        let audio_input = receiver.try_recv().unwrap();
        assert_eq!(audio_input.path, output_path.to_str().unwrap());
        println!("Audio input: {:?}", audio_input);

        // Verify file format (you might need to install the `infer` crate for this)
        let kind = infer::get_from_path(&output_path).unwrap().unwrap();
        assert_eq!(
            kind.mime_type(),
            "audio/mpeg",
            "File should be in mp3 format"
        );

        // Clean up
        std::fs::remove_file(output_path).unwrap();
    }

    #[tokio::test]
    #[ignore]
    async fn test_record_and_transcribe_interrupt_before_end() {
        setup();

        // Setup
        let device_spec = Arc::new(default_output_device().await.unwrap());
        let duration = Duration::from_secs(30);
        let time = Utc::now().timestamp_millis();
        let output_path = PathBuf::from(format!("test_output_interrupt_{}.mp4", time));
        let (sender, mut receiver) = unbounded_channel();
        let is_running = Arc::new(AtomicBool::new(true));
        let is_running_clone = Arc::clone(&is_running);

        // interrupt in 10 seconds
        tokio::spawn(async move {
            tokio::time::sleep(Duration::from_secs(10)).await;
            is_running_clone.store(false, Ordering::Relaxed);
        });

        // Act
        let start_time = Instant::now();

        record_and_transcribe(
            device_spec,
            duration,
            output_path.clone(),
            sender,
            is_running,
        )
        .await
        .unwrap();

        let elapsed_time = start_time.elapsed();

        println!("Elapsed time: {:?}", elapsed_time);
        // Assert
        assert!(
            elapsed_time < duration,
            "Recording should have been interrupted before the full duration"
        );
        assert!(
            elapsed_time >= Duration::from_secs(3),
            "Recording should have lasted at least 3 seconds"
        );

        // Check if the file was created
        assert!(output_path.exists(), "Output file should exist");

        // Check if we received the correct AudioInput
        let audio_input = receiver.try_recv().unwrap();
        assert_eq!(audio_input.path, output_path.to_str().unwrap());

        // Verify file format
        let kind = infer::get_from_path(&output_path).unwrap().unwrap();
        assert_eq!(
            kind.mime_type(),
            "audio/mpeg",
            "File should be in mp3 format"
        );

        // Verify file duration
        let file_duration = get_audio_duration(&output_path).unwrap();
        assert!(
            file_duration >= Duration::from_secs(3) && file_duration < duration,
            "File duration should be between 3 seconds and the full duration"
        );

        // Clean up
        std::fs::remove_file(output_path).unwrap();
    }

    // Helper function to get audio duration (you'll need to implement this)
    fn get_audio_duration(path: &PathBuf) -> Result<Duration, Box<dyn std::error::Error>> {
        let output = Command::new("ffprobe")
            .args(&[
                "-v",
                "error",
                "-show_entries",
                "format=duration",
                "-of",
                "default=noprint_wrappers=1:nokey=1",
                path.to_str().unwrap(),
            ])
            .output()?;

        let duration_str = String::from_utf8(output.stdout)?;
        let duration_secs = f64::from_str(duration_str.trim())?;

        Ok(Duration::from_secs_f64(duration_secs))
    }

    #[tokio::test]
    #[ignore]
    async fn test_audio_transcription() {
        setup();
        use screenpipe_audio::{create_whisper_channel, record_and_transcribe};
        use std::sync::Arc;
        use std::time::Duration;
        use tokio::time::timeout;

        // 1. start listening to https://music.youtube.com/watch?v=B6WAlAzuJb4&si=775sYWLG0b7XhQIH&t=50
        // 2. run the test
        // 3. the test should succeed (takes ~120s for some reason?) ! i think whisper is just slow as hell on cpu?

        // Setup
        let device_spec = Arc::new(default_output_device().await.unwrap());
        let output_path =
            PathBuf::from(format!("test_output_{}.mp4", Utc::now().timestamp_millis()));
        let output_path_2 = output_path.clone();
        let (whisper_sender, mut whisper_receiver, _) =
            create_whisper_channel(Arc::new(AudioTranscriptionEngine::WhisperTiny))
                .await
                .unwrap();
        let is_running = Arc::new(AtomicBool::new(true));
        // Start recording in a separate thread
        let recording_thread = tokio::spawn(async move {
            let device_spec = Arc::clone(&device_spec);
            let whisper_sender = whisper_sender.clone();
            record_and_transcribe(
                device_spec,
                Duration::from_secs(15),
                output_path.clone(),
                whisper_sender,
                is_running,
            )
            .await
            .unwrap();
        });

        // Wait for the recording to complete (with a timeout)
        let timeout_duration = Duration::from_secs(10); // Adjust as needed
        let result = timeout(timeout_duration, async {
            // Wait for the transcription result
            let transcription_result = whisper_receiver.try_recv().unwrap();
            debug!("Received transcription: {:?}", transcription_result);
            // Check if we received a valid transcription
            assert!(
                transcription_result.error.is_none(),
                "Transcription error occurred"
            );
            assert!(
                transcription_result.transcription.is_some(),
                "No transcription received"
            );

            let transcription = transcription_result.transcription.unwrap();
            assert!(!transcription.is_empty(), "Transcription is empty");

            println!("Received transcription: {}", transcription);

            assert!(
                transcription.contains("même")
                    || transcription.contains("tu m'aimes")
                    || transcription.contains("champs")
            );

            transcription
        })
        .await;

        // Check the result
        match result {
            Ok(transcription) => {
                println!("Test passed. Transcription: {}", transcription);
            }
            Err(_) => {
                panic!("Test timed out waiting for transcription");
            }
        }

        // Clean up
        let _ = recording_thread.abort();
        std::fs::remove_file(output_path_2).unwrap_or_default();
    }
}
