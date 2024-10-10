use anyhow::Result;
use realfft::num_complex::{Complex32, ComplexFloat};
use realfft::RealFftPlanner;

pub fn normalize_v2(audio: &[f32]) -> Vec<f32> {
    let rms = (audio.iter().map(|&x| x * x).sum::<f32>() / audio.len() as f32).sqrt();
    let peak = audio
        .iter()
        .fold(0.0f32, |max, &sample| max.max(sample.abs()));

    let target_rms = 0.2; // Adjust as needed
    let target_peak = 0.95; // Adjust as needed

    let rms_scaling = target_rms / rms;
    let peak_scaling = target_peak / peak;

    let scaling_factor = rms_scaling.min(peak_scaling);

    audio
        .iter()
        .map(|&sample| sample * scaling_factor)
        .collect()
}

pub fn spectral_subtraction(audio: &[f32], d: f32) -> Result<Vec<f32>> {
    let mut real_planner = RealFftPlanner::<f32>::new();
    let window_size = 1600; // 16k sample rate - 100ms
    let r2c = real_planner.plan_fft_forward(window_size);

    let mut y = r2c.make_output_vec();

    let mut padded_audio: [f32; 1600] = [0.0; 1600];

    for i in 0..audio.len() {
        padded_audio[i] = audio[i];
    }

    let mut indata = padded_audio;
    r2c.process(&mut indata, &mut y)?;

    let mut processed_audio = y.iter().map(|&x|{


        let magnitude_y = x.abs().powf(2.0);

        let div = 1.0 - (d / magnitude_y) as f32;


        let gain = {
            if div > 0.0 {
                f32::sqrt(div)
            } else {
                0.0f32
            }
        };

        x * gain

    }).collect::<Vec<Complex32>>();

    let c2r = real_planner.plan_fft_inverse(window_size);

    let mut outdata = c2r.make_output_vec();

    c2r.process(&mut processed_audio, &mut outdata)?;

    Ok(outdata)
}


// not an average of non-speech segments, but I don't know how much pause time we
// get. for now, we will just assume the noise is constant (kinda defeats the purpose)
// but oh well
pub fn average_noise_spectrum(audio: &[f32]) -> f32 {
        let mut total_sum = 0.0f32;

        for sample in audio {
            let magnitude = sample.abs();

            total_sum += magnitude.powf(2.0);
        }

    total_sum / audio.len() as f32
}
