use esp_idf_hal::i2s::{self, config};
use esp_idf_hal::gpio::*;
use esp_idf_hal::i2s::I2S0;
use log::{info, warn};
use std::sync::Arc;
use std::sync::atomic::{AtomicU32, Ordering};
use core::f32::consts::PI;

/// Tuning parameters
const SAMPLE_RATE: u32 = 16000;    // Recommended for LED-reactive: 16kHz
const BUFFER_SIZE: usize = 128;    // bytes / 32-bit samples => 128 samples
// Note: if you need 44100 Hz, set SAMPLE_RATE = 44100 and adjust filter cutoffs below.

/// Band cutoffs (Hz) - chosen for SAMPLE_RATE = 16k
const BASS_CUTOFF: f32 = 250.0;    // <= 250Hz considered bass
const MID_LOW: f32 = 250.0;        // mid band low edge
const MID_HIGH: f32 = 3000.0;      // mid band high edge
const TREBLE_CUTOFF: f32 = 3000.0; // > 3k treble

/// AGC / envelope params
const TARGET_LEVEL: f32 = 0.6;
const AGC_ATTACK: f32 = 0.25;   // fast attack smoothing for envelope
const AGC_RELEASE: f32 = 0.02;  // slower release smoothing
const GAIN_MIN: f32 = 0.4;
const GAIN_MAX: f32 = 15.0;

/// AudioData lock-free shared state
pub struct AudioData {
    amplitude: AtomicU32,
    bass: AtomicU32,
    mid: AtomicU32,
    treble: AtomicU32,
}

impl AudioData {
    pub fn new() -> Self {
        Self {
            amplitude: AtomicU32::new(0),
            bass: AtomicU32::new(0),
            mid: AtomicU32::new(0),
            treble: AtomicU32::new(0),
        }
    }

    #[inline] fn set_amplitude(&self, v: f32) { self.amplitude.store(v.to_bits(), Ordering::Relaxed); }
    #[inline] fn set_bass(&self, v: f32)      { self.bass.store(v.to_bits(), Ordering::Relaxed); }
    #[inline] fn set_mid(&self, v: f32)       { self.mid.store(v.to_bits(), Ordering::Relaxed); }
    #[inline] fn set_treble(&self, v: f32)    { self.treble.store(v.to_bits(), Ordering::Relaxed); }

    #[inline] pub fn get_amplitude(&self) -> f32 { f32::from_bits(self.amplitude.load(Ordering::Relaxed)) }
    #[inline] pub fn get_bass(&self) -> f32      { f32::from_bits(self.bass.load(Ordering::Relaxed)) }
    #[inline] pub fn get_mid(&self) -> f32       { f32::from_bits(self.mid.load(Ordering::Relaxed)) }
    #[inline] pub fn get_treble(&self) -> f32    { f32::from_bits(self.treble.load(Ordering::Relaxed)) }
}

impl Default for AudioData { fn default() -> Self { Self::new() } }

/// First-order IIR helper (y[n] = a*y[n-1] + b*x[n])
#[derive(Clone, Copy)]
struct OnePole {
    a: f32,
    b: f32,
    y: f32,
}

impl OnePole {
    fn new(alpha: f32) -> Self { Self { a: alpha, b: 1.0 - alpha, y: 0.0 } }
    fn apply(&mut self, x: f32) -> f32 {
        self.y = self.a * self.y + self.b * x;
        self.y
    }
}

/// Audio processor: zero-alloc, per-sample processing
struct AudioProcessor {
    data: Arc<AudioData>,

    // smoothed display values
    smooth_amp: f32,
    smooth_bass: f32,
    smooth_mid: f32,
    smooth_treble: f32,

    // AGC / envelope / gain
    envelope: f32,
    gain: f32,

    // filters (lowpass for bass, band-pass via cascade)
    lp_bass: OnePole,
    lp_mid: OnePole,
    lp_treble: OnePole,

    // helper for mid band separation (we use hp = x - bass_lp)
    prev_hp_mid: f32,
}

impl AudioProcessor {
    fn new(data: Arc<AudioData>) -> Self {
        // compute alpha for one-pole: alpha = exp(-2*pi*fc/fs)
        fn alpha_for(fc: f32, fs: f32) -> f32 {
            let x = (-2.0 * PI * fc / fs).exp();
            // clamp to avoid NaN
            if x.is_finite() { x as f32 } else { 0.0 }
        }

        let fs = SAMPLE_RATE as f32;
        let alpha_bass = alpha_for(BASS_CUTOFF, fs);    // smooth lowpass for bass
        let alpha_mid = alpha_for((MID_LOW + MID_HIGH) * 0.5, fs); // mid smoother
        let alpha_treble = alpha_for(TREBLE_CUTOFF, fs);

        Self {
            data,
            smooth_amp: 0.0,
            smooth_bass: 0.0,
            smooth_mid: 0.0,
            smooth_treble: 0.0,
            envelope: 0.0,
            gain: 1.0,
            lp_bass: OnePole::new(alpha_bass),
            lp_mid: OnePole::new(alpha_mid),
            lp_treble: OnePole::new(alpha_treble),
            prev_hp_mid: 0.0,
        }
    }

    /// Process one 32-bit PCM sample (left channel). Normalized into [-1,1]
    #[inline]
    fn process_sample(&mut self, sample: i32) {
        // normalize 32-bit sample
        let x = (sample as f32) / 2147483648.0_f32;

        // envelope (peak) follower with attack/release
        let abs_x = x.abs();
        if abs_x > self.envelope {
            self.envelope = self.envelope * (1.0 - AGC_ATTACK) + abs_x * AGC_ATTACK;
        } else {
            self.envelope = self.envelope * (1.0 - AGC_RELEASE) + abs_x * AGC_RELEASE;
        }

        // update gain smoothly toward target
        if self.envelope > 1e-6 {
            let target_gain = TARGET_LEVEL / self.envelope;
            self.gain = (self.gain * 0.98 + target_gain * 0.02).clamp(GAIN_MIN, GAIN_MAX);
        }

        // apply gain
        let g_x = (x * self.gain).clamp(-1.0, 1.0);

        // filter chain:
        // bass low-pass
        let bass = self.lp_bass.apply(g_x);
        // high-pass approx for mid/treble: hp = g_x - bass
        let hp = g_x - bass;

        // mid band smoothing (a bit slower)
        let mid = self.lp_mid.apply(hp);

        // treble: remaining high-frequency (hp - mid)
        let treble_raw = hp - mid;
        let treble = self.lp_treble.apply(treble_raw);

        // accumulate smoothing for display (exponential)
        const DISP_ALPHA: f32 = 0.12;
        // amplitude: use envelope scaled
        let amp = (self.envelope * self.gain).min(1.0);
        self.smooth_amp = self.smooth_amp * (1.0 - DISP_ALPHA) + amp * DISP_ALPHA;

        // energies: take absolute (approximate magnitude)
        let bass_val = bass.abs();
        let mid_val = mid.abs();
        let treble_val = treble.abs();

        self.smooth_bass = self.smooth_bass * (1.0 - DISP_ALPHA) + bass_val * DISP_ALPHA;
        self.smooth_mid  = self.smooth_mid  * (1.0 - DISP_ALPHA) + mid_val * DISP_ALPHA;
        self.smooth_treble = self.smooth_treble * (1.0 - DISP_ALPHA) + treble_val * DISP_ALPHA;
    }

    /// Write smoothed values to shared atomic state (call periodically)
    fn publish(&self) {
        self.data.set_amplitude(self.smooth_amp);
        self.data.set_bass(self.smooth_bass);
        self.data.set_mid(self.smooth_mid);
        self.data.set_treble(self.smooth_treble);
    }
}

/// Start I2S audio task (spawns std thread). Returns Arc<AudioData>
pub fn start_i2s_audio_task(
    i2s: I2S0,
    sck_pin: Gpio33,
    ws_pin: Gpio25,
    sd_pin: Gpio32,
) -> Result<Arc<AudioData>, anyhow::Error> {
    let audio_data = Arc::new(AudioData::new());
    let audio_data_clone = audio_data.clone();

    std::thread::spawn(move || {
        if let Err(e) = run_audio_loop(i2s, sck_pin, ws_pin, sd_pin, audio_data) {
            warn!("Audio task failed: {:?}", e);
        }
    });

    Ok(audio_data_clone)
}

/// Main audio loop - zero-copy, blocking read, per-sample processing
fn run_audio_loop(
    i2s: I2S0,
    sck_pin: Gpio33,
    ws_pin: Gpio25,
    sd_pin: Gpio32,
    audio_data: Arc<AudioData>,
) -> Result<(), anyhow::Error> {
    info!("Starting audio capture task (optimized). Sample rate: {} Hz", SAMPLE_RATE);

    // I2S config
    let channel_cfg = config::Config::default();
    let clk_cfg = config::StdClkConfig::from_sample_rate_hz(SAMPLE_RATE);
    let slot_cfg = config::StdSlotConfig::philips_slot_default(
        config::DataBitWidth::Bits32,
        config::SlotMode::Mono,
    );
    let gpio_cfg = config::StdGpioConfig::default();

    let i2s_config = config::StdConfig::new(channel_cfg, clk_cfg, slot_cfg, gpio_cfg);

    // Initialize driver
    let mut driver = i2s::I2sDriver::new_std_rx(
        i2s,
        &i2s_config,
        sck_pin,
        sd_pin,
        None::<Gpio0>,
        ws_pin,
    )?;

    driver.rx_enable()?;

    // Reuse static buffers on stack (no allocations)
    const BUFFER_BYTES: usize = BUFFER_SIZE * 4; // 4 bytes per sample (32-bit)
    let mut buffer: [u8; BUFFER_BYTES] = [0u8; BUFFER_BYTES]; // read bytes
    // temporary sample conversion per-read, we process per-sample immediately
    let mut processor = AudioProcessor::new(audio_data.clone());

    // We will publish display values at a lower rate to avoid too-frequent atomics.
    // publish every N reads (approx every ~32..64 ms depending on buffer size).
    let mut publish_counter: usize = 0;
    let publish_interval_reads = 4usize; // tweak: 4 reads * BUFFER_SIZE / SAMPLE_RATE seconds

    loop {
        // Blocking read with moderate timeout (ms). Use 200 to avoid busy spin.
        match driver.read(&mut buffer, 200) {
            Ok(bytes_read) if bytes_read >= 4 => {
                // convert each 4-byte little-endian to i32 and process immediately
                let num_samples = bytes_read / 4;
                for i in 0..num_samples {
                    let base = i * 4;
                    let sample = i32::from_le_bytes([
                        buffer[base],
                        buffer[base + 1],
                        buffer[base + 2],
                        buffer[base + 3],
                    ]);
                    processor.process_sample(sample);
                }

                publish_counter = publish_counter.wrapping_add(1);
                if publish_counter >= publish_interval_reads {
                    publish_counter = 0;
                    processor.publish();
                }
            }
            Ok(_) => {
                // no data - yield thread a bit
                std::thread::yield_now();
            }
            Err(e) => {
                warn!("I2S read error: {:?}", e);
                // short sleep to avoid tight error loop
                std::thread::sleep(std::time::Duration::from_millis(20));
            }
        }
    }
}
