use esp_idf_hal::i2s::{self, I2sDriver, config};
use esp_idf_hal::gpio::*;
use esp_idf_hal::i2s::I2S0;
use esp_idf_hal::delay::FreeRtos;
use log::info;
use std::sync::Arc;

pub const SAMPLE_RATE: u32 = 16000;
pub const BUFFER_SIZE: usize = 128;
pub const NUM_BINS: usize = 8;


const SMOOTH_FACTOR: f32 = 0.65;     
const VOL_SCALE: f32 = 25.0;          
const BASS_SCALE: f32 = 5.0;         
const MID_SCALE: f32 = 4.0;           
const TREBLE_SCALE: f32 = 6.0;        

const PORT_MAX_DELAY: u32 = 0xFFFFFFFF;

// Noise gate - lọc nhiễu nền
const NOISE_FLOOR: f32 = 0.005;       // Dưới ngưỡng này = nhiễu

/// AudioData - lightweight
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

/// Smooth value over time - faster response
#[inline(always)]
fn smooth(current: f32, target: f32, factor: f32) -> f32 {
    current * factor + target * (1.0 - factor)
}

/// Clamp between 0.0 and 1.0
#[inline(always)]
fn clamp(v: f32) -> f32 {
    if v < 0.0 { 0.0 } else if v > 1.0 { 1.0 } else { v }
}

/// Apply noise gate
#[inline(always)]
fn apply_noise_gate(value: f32, threshold: f32) -> f32 {
    if value < threshold {
        0.0
    } else {
        value
    }
}

/// Calculate RMS (Root Mean Square) - measures volume
#[inline]
fn calculate_rms(samples: &[i32]) -> f32 {
    let mut sum = 0.0f32;
    for &s in samples.iter() {
        let normalized = (s as f32) / (i32::MAX as f32);
        sum += normalized * normalized;
    }
    (sum / samples.len() as f32).sqrt()
}

/// Simple zero-crossing rate - estimates pitch/frequency
#[inline]
fn calculate_zcr(samples: &[i32]) -> f32 {
    let mut crossings = 0;
    for i in 1..samples.len() {
        if (samples[i] >= 0 && samples[i-1] < 0) || 
           (samples[i] < 0 && samples[i-1] >= 0) {
            crossings += 1;
        }
    }
    crossings as f32 / samples.len() as f32
}

/// Simple spectral brightness approximation
#[inline]
fn calculate_spectral_brightness(samples: &[i32]) -> f32 {
    let mut high_freq_energy = 0.0f32;
    let mut low_freq_energy = 0.0f32;
    
    // Giảm threshold để nhạy hơn với treble
    const THRESHOLD: i32 = i32::MAX / 20; // 5% threshold (giảm từ 10%)
    
    for i in 1..samples.len() {
        let diff = (samples[i] - samples[i-1]).abs();
        if diff > THRESHOLD {
            high_freq_energy += diff as f32;
        }
        low_freq_energy += samples[i].abs() as f32;
    }
    
    if low_freq_energy > 0.0 {
        high_freq_energy / low_freq_energy
    } else {
        0.0
    }
}

/// Simple frequency band detection using time-domain analysis
fn analyze_frequency_bands(samples: &[i32]) -> (f32, f32, f32) {
    let rms = calculate_rms(samples);
    let zcr = calculate_zcr(samples);
    let brightness = calculate_spectral_brightness(samples);
    
    // Điều chỉnh ngưỡng ZCR để nhạy hơn với bass
    let bass = if zcr < 0.35 {  // Tăng từ 0.3
        rms * (1.0 - zcr) * 1.2  // Thêm boost 20%
    } else { 
        rms * 0.3 
    };
    
    let treble = rms * brightness * 1.3; // Boost treble thêm 30%
    
    let mid = rms - (bass + treble) * 0.5;
    
    (bass, mid.max(0.0), treble)
}

/// Generate simple frequency bins using windowed RMS
fn generate_simple_bins(samples: &[i32], bins: &mut [f32; NUM_BINS]) {
    let window_size = samples.len() / NUM_BINS;
    
    for i in 0..NUM_BINS {
        let start = i * window_size;
        let end = ((i + 1) * window_size).min(samples.len());
        
        if end > start {
            let window = &samples[start..end];
            bins[i] = calculate_rms(window);
            
            // Tăng trọng số cho bins cao (treble nhạy hơn)
            let weight = 1.0 + (i as f32 / NUM_BINS as f32) * 0.8; // Tăng từ 0.5
            bins[i] *= weight;
        }
    }
}

/// Peak detection for beat/transient detection - more sensitive
#[inline]
fn detect_peak(current: f32, history: &[f32; 4]) -> f32 {
    let avg: f32 = history.iter().sum::<f32>() / history.len() as f32;
    let threshold = avg * 1.3; // Giảm từ 1.5 → dễ phát hiện peak hơn
    
    if current > threshold {
        (current - threshold) / threshold
    } else {
        0.0
    }
}

/// Simple audio processing - More sensitive settings
pub fn audio_processing_blocking(
    i2s: I2S0,
    sck: Gpio33,
    ws: Gpio25,
    sd: Gpio32,
    audio_data: Arc<std::sync::Mutex<AudioData>>,
) -> Result<(), anyhow::Error> {
    // I2S config
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

    // Allocate buffers on heap
    let mut raw_bytes = vec![0u8; BUFFER_SIZE * 4];
    let mut samples = vec![0i32; BUFFER_SIZE];
    
    // Smoothed values
    let mut smooth_volume = 0.0f32;
    let mut smooth_bass = 0.0f32;
    let mut smooth_mid = 0.0f32;
    let mut smooth_treble = 0.0f32;
    let mut smooth_bins = [0.0f32; NUM_BINS];
    
    // Peak detection history
    let mut volume_history = [0.0f32; 4];
    let mut history_idx = 0;
    
    info!("Audio processing started - SENSITIVE MODE");
    info!("Sample rate: {}Hz, Buffer: {} samples", SAMPLE_RATE, BUFFER_SIZE);
    info!("Scales - Vol:{} Bass:{} Mid:{} Treble:{}", 
          VOL_SCALE, BASS_SCALE, MID_SCALE, TREBLE_SCALE);

    loop {
        // Read I2S data
        if let Err(_) = driver.read(&mut *raw_bytes, PORT_MAX_DELAY) {
            FreeRtos::delay_ms(10);
            continue;
        }

        // Convert bytes to i32 samples
        for i in 0..BUFFER_SIZE {
            let idx = i * 4;
            samples[i] = i32::from_le_bytes([
                raw_bytes[idx],
                raw_bytes[idx + 1],
                raw_bytes[idx + 2],
                raw_bytes[idx + 3],
            ]);
        }

        // Calculate volume (RMS)
        let mut volume = calculate_rms(&samples) * VOL_SCALE;
        volume = apply_noise_gate(volume, NOISE_FLOOR); // Lọc nhiễu
        
        // Frequency band analysis
        let (mut bass, mut mid, mut treble) = analyze_frequency_bands(&samples);
        
        // Apply noise gate to bands
        bass = apply_noise_gate(bass, NOISE_FLOOR);
        mid = apply_noise_gate(mid, NOISE_FLOOR);
        treble = apply_noise_gate(treble, NOISE_FLOOR);
        
        // Generate simple bins
        let mut bins = [0.0f32; NUM_BINS];
        generate_simple_bins(&samples, &mut bins);
        
        // Apply noise gate to bins
        for bin in bins.iter_mut() {
            *bin = apply_noise_gate(*bin, NOISE_FLOOR);
        }
        
        // Peak detection for beat
        volume_history[history_idx] = volume;
        history_idx = (history_idx + 1) % volume_history.len();
        let beat_intensity = detect_peak(volume, &volume_history);
        
        // Apply smoothing (faster response than before)
        smooth_volume = smooth(smooth_volume, volume, SMOOTH_FACTOR);
        smooth_bass = smooth(smooth_bass, bass * BASS_SCALE, SMOOTH_FACTOR);
        smooth_mid = smooth(smooth_mid, mid * MID_SCALE, SMOOTH_FACTOR);
        smooth_treble = smooth(smooth_treble, treble * TREBLE_SCALE, SMOOTH_FACTOR);
        
        for i in 0..NUM_BINS {
            smooth_bins[i] = smooth(smooth_bins[i], bins[i], SMOOTH_FACTOR);
        }
        
        // Beat boost (tăng từ 0.5 lên 0.7)
        let beat_boost = 1.0 + beat_intensity * 0.7;

        // Update shared data
        if let Ok(mut data) = audio_data.lock() {
            data.volume = clamp(smooth_volume * beat_boost);
            data.bass = clamp(smooth_bass * beat_boost);
            data.mid = clamp(smooth_mid);
            data.treble = clamp(smooth_treble);
            
            for i in 0..NUM_BINS {
                data.bins[i] = clamp(smooth_bins[i] * beat_boost);
            }
        }

        // Fast update rate
        FreeRtos::delay_ms(5);
    }
}