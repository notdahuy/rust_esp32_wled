use esp_idf_hal::i2s::*;
use esp_idf_hal::gpio::*;
use log::{info, warn};
use std::sync::{Arc, RwLock};
use std::time::Instant;
use microfft::real::rfft_256;

const SAMPLE_RATE: u32 = 16000;
const BUFFER_SIZE: usize = 512;
const FFT_SIZE: usize = 256;

// Frequency bands (in Hz)
const BASS_MAX: f32 = 250.0;
const MID_MAX: f32 = 2000.0;
const TREBLE_MAX: f32 = 8000.0;

// Update throttling - chỉ update 30 FPS
const UPDATE_INTERVAL_MS: u64 = 33; // ~30 FPS

#[derive(Clone, Default)]
pub struct AudioData {
    pub amplitude: f32,
    pub bass: f32,
    pub mid: f32,
    pub treble: f32,
    pub peak_freq: f32,
}

pub struct AudioProcessor {
    public_data: Arc<RwLock<AudioData>>,
    raw_amplitude: f32,
    raw_bass: f32,
    raw_mid: f32,
    raw_treble: f32,
    peak_freq: f32,
    gain: f32,
    peak_history: Vec<f32>,
    last_update: Instant,
    fft_counter: u32,
}

impl AudioProcessor {
    fn new(public_data: Arc<RwLock<AudioData>>) -> Self {
        Self {
            public_data,
            raw_amplitude: 0.0,
            raw_bass: 0.0,
            raw_mid: 0.0,
            raw_treble: 0.0,
            peak_freq: 0.0,
            gain: 1.0,
            peak_history: Vec::with_capacity(100),
            last_update: Instant::now(),
            fft_counter: 0,
        }
    }

    fn process_audio_data(&mut self, samples: &[i32]) {
        if samples.is_empty() { return; }

        // --- 1. TÍNH TOÁN NHANH (KHÔNG KHÓA) ---
        
        // Tính RMS
        let sum_squares: f32 = samples.iter()
            .map(|&s| ((s as f32 * self.gain) / 2147483648.0).powi(2))
            .sum();
        let rms = (sum_squares / samples.len() as f32).sqrt();
        
        self.update_agc(rms);
        self.raw_amplitude = rms;

        // FFT - chỉ chạy mỗi 4 buffer để giảm tải CPU
        self.fft_counter += 1;
        if self.fft_counter % 4 == 0 && samples.len() >= FFT_SIZE {
            self.analyze_frequencies_fft(&samples[..FFT_SIZE]);
        }

        // --- 2. THROTTLE UPDATE - CHỈ CẬP NHẬT 30 FPS ---
        if self.last_update.elapsed().as_millis() >= UPDATE_INTERVAL_MS as u128 {
            // Sử dụng RwLock::write() thay vì Mutex
            if let Ok(mut data) = self.public_data.write() {
                // Cập nhật và làm mượt
                const AMPLITUDE_ALPHA: f32 = 0.15;
                const BAND_ALPHA: f32 = 0.12;

                data.amplitude = data.amplitude * (1.0 - AMPLITUDE_ALPHA) + self.raw_amplitude * AMPLITUDE_ALPHA;
                data.bass = data.bass * (1.0 - BAND_ALPHA) + self.raw_bass * BAND_ALPHA;
                data.mid = data.mid * (1.0 - BAND_ALPHA) + self.raw_mid * BAND_ALPHA;
                data.treble = data.treble * (1.0 - BAND_ALPHA) + self.raw_treble * BAND_ALPHA;
                data.peak_freq = self.peak_freq;
                
                self.last_update = Instant::now();
            }
        }
    }

    fn update_agc(&mut self, current_level: f32) {
        const TARGET_LEVEL: f32 = 0.7;
        const AGC_SPEED: f32 = 0.15;
        const MIN_GAIN: f32 = 0.5;
        const MAX_GAIN: f32 = 15.0;

        self.peak_history.push(current_level);
        if self.peak_history.len() > 100 {
            self.peak_history.remove(0);
        }
        
        let avg_peak = if !self.peak_history.is_empty() {
            self.peak_history.iter().sum::<f32>() / self.peak_history.len() as f32
        } else { 0.0 };

        if avg_peak > 0.01 {
            let target_gain = TARGET_LEVEL / avg_peak;
            let new_gain = self.gain + (target_gain - self.gain) * AGC_SPEED;
            self.gain = new_gain.clamp(MIN_GAIN, MAX_GAIN);
        }
    }

    fn analyze_frequencies_fft(&mut self, samples: &[i32]) {
        let mut input: [f32; FFT_SIZE] = [0.0; FFT_SIZE];
        for (i, &sample) in samples.iter().enumerate() {
            let normalized = (sample as f32) / 2147483648.0;
            // Hamming window
            let window = 0.54 - 0.46 * f32::cos(2.0 * core::f32::consts::PI * i as f32 / (FFT_SIZE - 1) as f32);
            input[i] = normalized * window;
        }

        let spectrum = rfft_256(&mut input);
        let freq_resolution = SAMPLE_RATE as f32 / FFT_SIZE as f32;
        
        let mut bass_energy = 0.0f32;
        let mut mid_energy = 0.0f32;
        let mut treble_energy = 0.0f32;
        let mut max_magnitude = 0.0f32;
        let mut peak_bin = 0usize;
        
        for (i, bin) in spectrum.iter().enumerate().take(FFT_SIZE / 2) {
            let magnitude = bin.l1_norm();
            let freq = i as f32 * freq_resolution;
            
            if magnitude > max_magnitude {
                max_magnitude = magnitude;
                peak_bin = i;
            }
            if freq < BASS_MAX { bass_energy += magnitude; }
            else if freq < MID_MAX { mid_energy += magnitude; }
            else if freq < TREBLE_MAX { treble_energy += magnitude; }
        }
        
        // Cập nhật biến nội bộ (không khóa)
        let total_energy = bass_energy + mid_energy + treble_energy;
        if total_energy > 0.0 {
            self.raw_bass = bass_energy / total_energy;
            self.raw_mid = mid_energy / total_energy;
            self.raw_treble = treble_energy / total_energy;
        }
        self.peak_freq = peak_bin as f32 * freq_resolution;
    }
}

pub fn start_i2s_audio_task(
    i2s: I2S0,
    sck_pin: Gpio33,
    ws_pin: Gpio25,
    sd_pin: Gpio32,
) -> Result<Arc<RwLock<AudioData>>, anyhow::Error> {
    
    // Sử dụng RwLock thay vì Mutex để nhiều reader có thể đọc đồng thời
    let public_data = Arc::new(RwLock::new(AudioData::default()));
    let public_data_clone = public_data.clone();

    let mut processor = AudioProcessor::new(public_data);

    std::thread::spawn(move || {
        info!("Starting I2S audio capture (INMP441)...");
        info!("I2S audio thread running on core: {:?}", esp_idf_svc::hal::cpu::core());
        
        // Create I2S configs
        let channel_cfg = config::Config::default();
        let clk_cfg = config::StdClkConfig::from_sample_rate_hz(SAMPLE_RATE);
        let slot_cfg = config::StdSlotConfig::philips_slot_default(
            config::DataBitWidth::Bits32,
            config::SlotMode::Mono,
        );
        let gpio_cfg = config::StdGpioConfig::default();
        
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
        let mut read_count = 0u32;
        let mut last_log = std::time::Instant::now();

        loop {
            // Giảm timeout xuống 100ms
            match i2s_driver.read(&mut buffer, 100) {
                Ok(bytes_read) => {
                    if bytes_read > 0 {
                        read_count += 1;
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
                        
                        processor.process_audio_data(&sample_buffer);
                    } else {
                        // Không có data - yield CPU
                        std::thread::sleep(std::time::Duration::from_millis(10));
                    }
                }
                Err(e) => {
                    warn!("I2S read error: {:?}", e);
                    std::thread::sleep(std::time::Duration::from_millis(100));
                }
            }
            
            std::thread::sleep(std::time::Duration::from_micros(100));
        }
    });

    Ok(public_data_clone)
}

pub struct AudioLedParams {
    pub brightness: f32,
    pub speed: u8,
    pub hue_shift: u16,
    pub intensity: f32,
}

// Helper function - đọc nhanh với RwLock::read()
pub fn audio_to_led_params(audio_data: &Arc<RwLock<AudioData>>) -> AudioLedParams {
    // Sử dụng read() thay vì lock() - không block các reader khác
    if let Ok(data) = audio_data.read() {
        AudioLedParams {
            brightness: (data.amplitude * 100.0).min(100.0),
            speed: ((data.bass + data.mid) * 100.0).max(10.0).min(100.0) as u8,
            hue_shift: (data.peak_freq / 20.0).min(360.0) as u16,
            intensity: data.amplitude,
        }
    } else {
        // Fallback nếu không đọc được
        AudioLedParams {
            brightness: 50.0,
            speed: 50,
            hue_shift: 0,
            intensity: 0.5,
        }
    }
}