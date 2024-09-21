use criterion::{black_box, criterion_group, criterion_main, Criterion};
use memory_stats::memory_stats;
use screenpipe_audio::vad_engine::SileroVad;
use screenpipe_audio::{
    create_whisper_channel, stt, AudioTranscriptionEngine, VadEngineEnum, WhisperModel,
};
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

fn criterion_benchmark(c: &mut Criterion) {
    let audio_transcription_engine = Arc::new(AudioTranscriptionEngine::WhisperTiny);
    let whisper_model = WhisperModel::new(audio_transcription_engine.clone()).unwrap();
    let test_file_path = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("test_data")
        .join("selah.mp4");

    let mut group = c.benchmark_group("whisper_benchmarks");
    group.sample_size(10);
    group.measurement_time(Duration::from_secs(60));

    group.bench_function("create_whisper_channel", |b| {
        b.iter(|| {
            let _ = create_whisper_channel(
                black_box(audio_transcription_engine.clone()),
                black_box(VadEngineEnum::Silero),
                None,
            );
        })
    });

    group.bench_function("stt", |b| {
        b.iter(|| {
            let mut vad_engine = Box::new(SileroVad::new().unwrap());
            let _ = stt(
                black_box(test_file_path.to_string_lossy().as_ref()),
                black_box(&whisper_model),
                black_box(audio_transcription_engine.clone()),
                &mut *vad_engine,
                None,
            );
        })
    });

    group.bench_function("memory_usage_stt", |b| {
        b.iter_custom(|iters| {
            let mut total_duration = Duration::new(0, 0);
            for _ in 0..iters {
                let start = std::time::Instant::now();
                let before = memory_stats().unwrap().physical_mem;
                let mut vad_engine = Box::new(SileroVad::new().unwrap());
                let _ = stt(
                    test_file_path.to_string_lossy().as_ref(),
                    &whisper_model,
                    audio_transcription_engine.clone(),
                    &mut *vad_engine,
                    None,
                );
                let after = memory_stats().unwrap().physical_mem;
                let duration = start.elapsed();
                total_duration += duration;
                println!("Memory used: {} bytes", after - before);
            }
            total_duration
        })
    });

    group.finish();
}

criterion_group!(benches, criterion_benchmark);
criterion_main!(benches);