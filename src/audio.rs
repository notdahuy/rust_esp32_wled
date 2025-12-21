use esp_idf_hal::i2s::{self, I2sDriver, config};
use esp_idf_hal::gpio::*;
use esp_idf_hal::i2s::I2S0;
use esp_idf_hal::delay::FreeRtos;
use std::sync::Arc;

pub const SAMPLE_RATE: u32 = 16000;
pub const BUFFER_SIZE: usize = 256;
pub const NUM_BINS: usize = 8;

const SMOOTH_FACTOR: f32 = 0.55;     
const VOL_SCALE: f32 = 60.0;        
const BASS_SCALE: f32 = 15.0;    
const MID_SCALE: f32 = 18.0;          
const TREBLE_SCALE: f32 = 12.0;      
const PORT_MAX_DELAY: u32 = 0xFFFFFFFF;
const NOISE_FLOOR: f32 = 0.003; 


#[derive(Debug, Clone)]
pub struct AudioData {
    pub volume: f32,
    pub bass: f32,
    pub mid: f32,
    pub treble: f32,
    pub bins: [f32; NUM_BINS],
}

impl Default for AudioData {
    fn default() -> Self {
        Self {
            volume: 0.0,
            bass: 0.0,
            mid: 0.0,
            treble: 0.0,
            bins: [0.0; NUM_BINS],
        }
    }
}

#[inline(always)]
fn smooth(current: f32, target: f32, factor: f32) -> f32 {
    current * factor + target * (1.0 - factor)
}

#[inline(always)]
fn clamp(v: f32) -> f32 {
    v.clamp(0.0, 1.0)
}

#[inline(always)]
fn apply_noise_gate(value: f32, threshold: f32) -> f32 {
    if value < threshold {
        0.0
    } else {
        value
    }
}

#[inline]
fn calculate_rms(samples: &[i32]) -> f32 {
    if samples.is_empty() { return 0.0; }
    let mut sum_squares: u64 = 0;

    for &sample in samples {
        let s = sample as i64;
        sum_squares += (s * s) as u64;
    }
    
    let mean_square = sum_squares as f32 / samples.len() as f32;
    (mean_square.sqrt()) / (i32::MAX as f32)
}


#[inline]
fn calculate_zcr(samples: &[i32]) -> f32 {
    let mut crossings = 0u32;
    let mut prev_sign = samples[0] >= 0;
    
    for &sample in samples.iter().skip(1) {
        let curr_sign = sample >= 0;
        if curr_sign != prev_sign {
            crossings += 1;
        }
        prev_sign = curr_sign;
    }
    (crossings as f32) / (samples.len() as f32)
}

#[inline]
fn calculate_spectral_brightness(samples: &[i32], rms: f32) -> f32 {
    if rms < 0.005 {  
        return 0.0;
    }
    
    let mut high_freq_energy = 0.0f32;
    let mut total_energy = 0.0f32; 
    let threshold = (i32::MAX as f32 * rms * 0.12) as i32;  
    
    for i in (2..samples.len()).step_by(2) {
        let diff = samples[i] - samples[i - 1];
        let abs_diff = diff.abs() as f32;
        
        if diff.abs() > threshold {
            high_freq_energy += abs_diff;
        }
        
        total_energy += samples[i].abs() as f32;
    }
    
    if total_energy > 0.0 {
        (high_freq_energy / total_energy).clamp(0.0, 1.0)
    } else {
        0.0
    }
}

fn analyze_frequency_bands(samples: &[i32]) -> (f32, f32, f32) {
    let rms = calculate_rms(samples);

    if rms < 0.002 { 
        return (0.0, 0.0, 0.0);
    }
    
    let zcr = calculate_zcr(samples);
    let brightness = calculate_spectral_brightness(samples, rms);
    let bass = if zcr < 0.08 { 
        rms * 2.0 
    } else {
        rms * 0.1
    };
    
    let mid = if zcr >= 0.08 && zcr < 0.4 {
        rms * 1.5 
    } else {
        rms * 0.4
    };
    
    let treble = if zcr >= 0.4 || brightness > 0.15 {
        rms * brightness * 3.0 
    } else {
        0.0
    };
    
    (bass, mid, treble)
}

fn generate_simple_bins(samples: &[i32], bins: &mut [f32; NUM_BINS]) {
    let chunk_size = BUFFER_SIZE / NUM_BINS;
    
    for i in 0..NUM_BINS {
        let start = i * chunk_size;
        let end = start + chunk_size;
        
        if end > samples.len() {
            bins[i] = 0.0;
            continue;
        }
        
        let chunk = &samples[start..end];
        let chunk_rms = calculate_rms(chunk);
        let chunk_zcr = calculate_zcr(chunk);
        let freq_weight = if i < 3 {
            if chunk_zcr < 0.45 {
                1.6 - chunk_zcr * 0.8
            } else {
                0.4
            }
        } else if i < 6 {
            1.2
        } else {
            if chunk_zcr > 0.35 {
                0.7 + chunk_zcr * 0.8
            } else {
                0.5
            }
        };
        
        bins[i] = chunk_rms * freq_weight;
    }
}

#[inline]
fn detect_peak(current: f32, history: &[f32; 8]) -> f32 {
    let avg: f32 = history.iter().sum::<f32>() / history.len() as f32;
    let threshold = avg * 1.4;
    
    if current > threshold && avg > 0.01 {
        ((current - threshold) / threshold).min(1.0)
    } else {
        0.0
    }
}

fn remove_dc_offset(samples: &mut [i32]) {
    let mut sum: i64 = 0;
    for &s in samples.iter() {
        sum += s as i64;
    }
    let mean = (sum / samples.len() as i64) as i32;
    
    for s in samples.iter_mut() {
        *s = *s - mean; 
    }
}

pub fn audio_processing_blocking(
    i2s: I2S0,
    sck: Gpio33,
    ws: Gpio25,
    sd: Gpio32,
    audio_data: Arc<std::sync::Mutex<AudioData>>,
) -> Result<(), anyhow::Error> {
    let config = config::StdConfig::philips(
        SAMPLE_RATE,
        config::DataBitWidth::Bits32
    );
    
    let mut driver: I2sDriver<'_, i2s::I2sRx> = I2sDriver::new_std_rx(
        i2s,
        &config,
        sck,
        sd,
        None::<Gpio0>,
        ws
    )?;

    driver.rx_enable()?;

    let mut raw_bytes = vec![0u8; BUFFER_SIZE * 4];
    let mut samples = vec![0i32; BUFFER_SIZE];
    
    let mut smooth_volume = 0.0f32;
    let mut smooth_bass = 0.0f32;
    let mut smooth_mid = 0.0f32;
    let mut smooth_treble = 0.0f32;
    let mut smooth_bins = [0.0f32; NUM_BINS];
    
    let mut volume_history = [0.0f32; 8];
    let mut history_idx = 0;

    loop {
        if driver.read(&mut *raw_bytes, PORT_MAX_DELAY).is_err() {
            FreeRtos::delay_ms(10);
            continue;
        }

        for i in 0..BUFFER_SIZE {
            let idx = i * 4;
            samples[i] = i32::from_le_bytes([
                raw_bytes[idx],
                raw_bytes[idx + 1],
                raw_bytes[idx + 2],
                raw_bytes[idx + 3],
            ]);
        }
        remove_dc_offset(&mut samples);
        let mut volume = calculate_rms(&samples) * VOL_SCALE;
        volume = apply_noise_gate(volume, NOISE_FLOOR);
        let (mut bass, mut mid, mut treble) = analyze_frequency_bands(&samples);
        bass = apply_noise_gate(bass, NOISE_FLOOR);
        mid = apply_noise_gate(mid, NOISE_FLOOR);
        treble = apply_noise_gate(treble, NOISE_FLOOR);

        let mut bins = [0.0f32; NUM_BINS];
        generate_simple_bins(&samples, &mut bins);
        
        for bin in bins.iter_mut() {
            *bin = apply_noise_gate(*bin, NOISE_FLOOR);
        }
        
        volume_history[history_idx] = volume;
        history_idx = (history_idx + 1) % volume_history.len();
        let beat_intensity = detect_peak(volume, &volume_history);
        
        smooth_volume = smooth(smooth_volume, volume, SMOOTH_FACTOR);
        smooth_bass = smooth(smooth_bass, bass * BASS_SCALE, SMOOTH_FACTOR);
        smooth_mid = smooth(smooth_mid, mid * MID_SCALE, SMOOTH_FACTOR);
        smooth_treble = smooth(smooth_treble, treble * TREBLE_SCALE, SMOOTH_FACTOR);
        
        for i in 0..NUM_BINS {
            smooth_bins[i] = smooth(smooth_bins[i], bins[i], SMOOTH_FACTOR);
        }
        
        let beat_boost = 1.0 + beat_intensity * 0.4;

        if let Ok(mut data) = audio_data.try_lock() {
            data.volume = clamp(smooth_volume * beat_boost);
            data.bass   = clamp(smooth_bass * beat_boost);
            data.mid    = clamp(smooth_mid);
            data.treble = clamp(smooth_treble);

            for i in 0..NUM_BINS {
                data.bins[i] = clamp(smooth_bins[i] * beat_boost);
            }
        }
    }
}