use esp_idf_hal::i2s::*;
use esp_idf_hal::gpio::*;
use log::{info, warn};
use std::sync::{Arc, Mutex};
use microfft::real::rfft_256;

const SAMPLE_RATE: u32 = 16000;
const BUFFER_SIZE: usize = 512;
const FFT_SIZE: usize = 256;

// Frequency bands (in Hz)
const BASS_MAX: f32 = 250.0;
const MID_MAX: f32 = 2000.0;
const TREBLE_MAX: f32 = 8000.0;

pub struct AudioProcessor {
    // Raw values
    amplitude: Arc<Mutex<f32>>,
    bass_level: Arc<Mutex<f32>>,
    mid_level: Arc<Mutex<f32>>,
    treble_level: Arc<Mutex<f32>>,
    peak_freq: Arc<Mutex<f32>>,
    gain: Arc<Mutex<f32>>,
    peak_history: Arc<Mutex<Vec<f32>>>,
    // Smoothed values for LED effects
    smooth_amplitude: Arc<Mutex<f32>>,
    smooth_bass: Arc<Mutex<f32>>,
    smooth_mid: Arc<Mutex<f32>>,
    smooth_treble: Arc<Mutex<f32>>,
}

impl AudioProcessor {
    pub fn new() -> Self {
        Self {
            amplitude: Arc::new(Mutex::new(0.0)),
            bass_level: Arc::new(Mutex::new(0.0)),
            mid_level: Arc::new(Mutex::new(0.0)),
            treble_level: Arc::new(Mutex::new(0.0)),
            peak_freq: Arc::new(Mutex::new(0.0)),
            gain: Arc::new(Mutex::new(1.0)),
            peak_history: Arc::new(Mutex::new(Vec::with_capacity(100))),
            smooth_amplitude: Arc::new(Mutex::new(0.0)),
            smooth_bass: Arc::new(Mutex::new(0.0)),
            smooth_mid: Arc::new(Mutex::new(0.0)),
            smooth_treble: Arc::new(Mutex::new(0.0)),
        }
    }

    // Getters return smoothed values
    pub fn get_amplitude(&self) -> f32 {
        *self.smooth_amplitude.lock().unwrap()
    }

    pub fn get_bass_level(&self) -> f32 {
        *self.smooth_bass.lock().unwrap()
    }

    pub fn get_mid_level(&self) -> f32 {
        *self.smooth_mid.lock().unwrap()
    }

    pub fn get_treble_level(&self) -> f32 {
        *self.smooth_treble.lock().unwrap()
    }

    pub fn get_peak_frequency(&self) -> f32 {
        *self.peak_freq.lock().unwrap()
    }

    fn process_audio_data(&self, samples: &[i32]) {
        if samples.is_empty() {
            return;
        }

        let current_gain = *self.gain.lock().unwrap();
        
        // Calculate amplitude (RMS)
        let sum_squares: f32 = samples.iter()
            .map(|&s| ((s as f32 * current_gain) / 2147483648.0).powi(2))
            .sum();
        let rms = (sum_squares / samples.len() as f32).sqrt();
        
        self.update_agc(rms);
        *self.amplitude.lock().unwrap() = rms;

        // Smooth amplitude
        self.smooth_value(&self.amplitude, &self.smooth_amplitude, 0.15);

        // FFT analysis
        if samples.len() >= FFT_SIZE {
            self.analyze_frequencies_fft(&samples[..FFT_SIZE]);
            
            // Smooth frequency bands
            self.smooth_value(&self.bass_level, &self.smooth_bass, 0.12);
            self.smooth_value(&self.mid_level, &self.smooth_mid, 0.12);
            self.smooth_value(&self.treble_level, &self.smooth_treble, 0.12);
        }
    }

    // Exponential moving average for smooth transitions
    fn smooth_value(&self, source: &Arc<Mutex<f32>>, target: &Arc<Mutex<f32>>, alpha: f32) {
        let current = *source.lock().unwrap();
        let mut smoothed = target.lock().unwrap();
        *smoothed = *smoothed * (1.0 - alpha) + current * alpha;
    }

    fn update_agc(&self, current_level: f32) {
        const TARGET_LEVEL: f32 = 0.7;      // Higher = more sensitive
        const AGC_SPEED: f32 = 0.15;        // Higher = faster response
        const MIN_GAIN: f32 = 0.5;          // Higher gain floor
        const MAX_GAIN: f32 = 15.0;         // Higher gain ceiling

        let mut peak_hist = self.peak_history.lock().unwrap();
        peak_hist.push(current_level);
        
        if peak_hist.len() > 100 {
            peak_hist.remove(0);
        }

        let avg_peak = if !peak_hist.is_empty() {
            peak_hist.iter().sum::<f32>() / peak_hist.len() as f32
        } else {
            0.0
        };

        if avg_peak > 0.01 {
            let mut gain = self.gain.lock().unwrap();
            let target_gain = TARGET_LEVEL / avg_peak;
            let new_gain = *gain + (target_gain - *gain) * AGC_SPEED;
            *gain = new_gain.clamp(MIN_GAIN, MAX_GAIN);
        }
    }

    fn analyze_frequencies_fft(&self, samples: &[i32]) {
        // Prepare FFT input buffer
        let mut input: [f32; FFT_SIZE] = [0.0; FFT_SIZE];
        
        // Apply Hamming window and normalize
        for (i, &sample) in samples.iter().enumerate() {
            let normalized = (sample as f32) / 2147483648.0;
            // Hamming window
            let window = 0.54 - 0.46 * f32::cos(2.0 * core::f32::consts::PI * i as f32 / (FFT_SIZE - 1) as f32);
            input[i] = normalized * window;
        }

        // Perform FFT
        let spectrum = rfft_256(&mut input);
        
        // Calculate magnitudes and frequency bands
        let freq_resolution = SAMPLE_RATE as f32 / FFT_SIZE as f32;
        
        let mut bass_energy = 0.0f32;
        let mut mid_energy = 0.0f32;
        let mut treble_energy = 0.0f32;
        let mut max_magnitude = 0.0f32;
        let mut peak_bin = 0usize;
        
        // Only use first half of FFT output (Nyquist)
        for (i, bin) in spectrum.iter().enumerate().take(FFT_SIZE / 2) {
            let magnitude = bin.l1_norm();
            let freq = i as f32 * freq_resolution;
            
            // Track peak frequency
            if magnitude > max_magnitude {
                max_magnitude = magnitude;
                peak_bin = i;
            }
            
            // Accumulate energy by frequency band
            if freq < BASS_MAX {
                bass_energy += magnitude;
            } else if freq < MID_MAX {
                mid_energy += magnitude;
            } else if freq < TREBLE_MAX {
                treble_energy += magnitude;
            }
        }

        // Normalize to 0-1 range
        let total_energy = bass_energy + mid_energy + treble_energy;
        if total_energy > 0.0 {
            *self.bass_level.lock().unwrap() = bass_energy / total_energy;
            *self.mid_level.lock().unwrap() = mid_energy / total_energy;
            *self.treble_level.lock().unwrap() = treble_energy / total_energy;
        }

        // Store peak frequency
        let peak_frequency = peak_bin as f32 * freq_resolution;
        *self.peak_freq.lock().unwrap() = peak_frequency;
    }
}

pub fn start_i2s_audio_task(
    i2s: I2S0,
    sck_pin: Gpio32,
    ws_pin: Gpio25,
    sd_pin: Gpio33,
) -> Result<Arc<AudioProcessor>, anyhow::Error> {
    
    let processor = Arc::new(AudioProcessor::new());
    let processor_clone = processor.clone();

    std::thread::spawn(move || {
        info!("Starting I2S audio capture (INMP441)...");
        info!("I2S audio thread running on core: {:?}", esp_idf_svc::hal::cpu::core());
        
        // Create separate configs
        let channel_cfg = config::Config::default();
        let clk_cfg = config::StdClkConfig::from_sample_rate_hz(SAMPLE_RATE);
        let slot_cfg = config::StdSlotConfig::philips_slot_default(
            config::DataBitWidth::Bits32,
            config::SlotMode::Mono,
        );
        let gpio_cfg = config::StdGpioConfig::default();
        
        // Create StdConfig from 4 parameters
        let i2s_config = config::StdConfig::new(
            channel_cfg,
            clk_cfg,
            slot_cfg,
            gpio_cfg,
        );

        // Initialize I2S driver
        let mut i2s_driver = match I2sDriver::new_std_rx(
            i2s,
            &i2s_config,
            sck_pin,
            sd_pin,
            None::<Gpio0>,
            ws_pin,
        ) {
            Ok(driver) => driver,
            Err(e) => {
                warn!("Failed to initialize I2S: {:?}", e);
                return;
            }
        };

        if let Err(e) = i2s_driver.rx_enable() {
            warn!("Failed to enable I2S RX: {:?}", e);
            return;
        }

        info!("I2S driver initialized successfully");

        let mut buffer = vec![0u8; BUFFER_SIZE * 4];
        let mut sample_buffer = Vec::with_capacity(BUFFER_SIZE);

        loop {
            match i2s_driver.read(&mut buffer, 1000) {
                Ok(bytes_read) => {
                    if bytes_read > 0 {
                        sample_buffer.clear();
                        
                        let num_samples = bytes_read / 4;
                        for i in 0..num_samples {
                            let idx = i * 4;
                            let sample = i32::from_le_bytes([
                                buffer[idx],
                                buffer[idx + 1],
                                buffer[idx + 2],
                                buffer[idx + 3],
                            ]);
                            sample_buffer.push(sample);
                        }
                        
                        processor_clone.process_audio_data(&sample_buffer);
                    }
                }
                Err(e) => {
                    warn!("I2S read error: {:?}", e);
                    std::thread::sleep(std::time::Duration::from_millis(100));
                }
            }
        }
    });

    Ok(processor)
}

// Helper functions
pub fn audio_to_led_params(processor: &AudioProcessor) -> AudioLedParams {
    let amplitude = processor.get_amplitude();
    let bass = processor.get_bass_level();
    let mid = processor.get_mid_level();
    let treble = processor.get_treble_level();
    let peak_freq = processor.get_peak_frequency();

    AudioLedParams {
        brightness: (amplitude * 100.0).min(100.0),
        speed: ((bass + mid) * 100.0).max(10.0).min(100.0) as u8,
        hue_shift: (peak_freq / 20.0).min(360.0) as u16,
        intensity: amplitude,
    }
}

pub struct AudioLedParams {
    pub brightness: f32,
    pub speed: u8,
    pub hue_shift: u16,
    pub intensity: f32,
}