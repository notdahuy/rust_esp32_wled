use std::sync::{Arc, Mutex}; 
use esp_idf_sys::esp_timer_get_time;
use log::{info, warn};
use smart_leds::RGB8;
use ws2812_esp32_rmt_driver::Ws2812Esp32RmtDriver;
use crate::audio::AudioData;
use crate::effect::*;

pub struct LedController<'a> {
    driver: Ws2812Esp32RmtDriver<'a>,
    num_leds: usize,
    brightness: u8,
    last_brightness: u8,
    
    // Pre-allocated buffers 
    buffer: Vec<RGB8>,
    tx_buffer: Vec<u8>,
    
    // Timing
    last_update: u64,
    frame_interval: u64,
    
    // Effect system
    current_effect: Box<dyn Effect>,
    needs_update: bool,
    last_set_color: RGB8,
    last_set_speed: u8,
    
    // Audio
    audio_data: Option<Arc<Mutex<AudioData>>>,
    
    // State
    force_update_count: u8,
    is_powered: bool,
    
    // Performance tracking
    slow_write_count: u32,
    last_write_us: u64,
}

impl<'a> LedController<'a> {
    pub fn new(driver: Ws2812Esp32RmtDriver<'a>, num_leds: usize) -> Self {
        let default_color = RGB8 { r: 255, g: 255, b: 255 };
        let default_speed = 128;
        
        // ✅ Pre-allocate tx_buffer to EXACT size (no future reallocs)
        let mut tx_buffer = Vec::with_capacity(num_leds * 3);
        unsafe { tx_buffer.set_len(num_leds * 3); }
        
        Self {
            driver,
            num_leds,
            brightness: 255,
            last_brightness: 255,
            buffer: vec![RGB8::default(); num_leds],
            tx_buffer,
            last_update: unsafe { esp_timer_get_time() } as u64,
            frame_interval: 33_333, // ✅ 30 FPS (was 60)
            current_effect: Box::new(StaticEffect::new(default_color)),
            needs_update: true,
            last_set_color: default_color,
            last_set_speed: default_speed,
            audio_data: None,
            force_update_count: 0,
            is_powered: true,
            slow_write_count: 0,
            last_write_us: 0,
        }
    }

    pub fn set_audio_data(&mut self, audio_data: Arc<Mutex<AudioData>>) {
        self.audio_data = Some(audio_data);
        info!("✅ Audio data connected");
    }

    pub fn set_brightness(&mut self, level: f32) {
        let new_level = (level.clamp(0.0, 1.0) * 255.0).round() as u8;
        
        if self.brightness != new_level {
            self.brightness = new_level;
            self.last_brightness = new_level;
            self.needs_update = true;
            self.force_update_count = self.force_update_count.max(2);
            info!("💡 Brightness: {}", new_level);
        }
    }

    pub fn set_color(&mut self, color: RGB8) {   
        self.last_set_color = color;
        if self.current_effect.set_color(color) {
            self.needs_update = true;
            self.force_update_count = self.force_update_count.max(2);
            info!("🎨 Color: #{:02X}{:02X}{:02X}", color.r, color.g, color.b);
        }
    }

    pub fn set_speed(&mut self, speed: u8) {
        self.last_set_speed = speed;
        if self.current_effect.set_speed(speed) {
            self.needs_update = true;
            self.force_update_count = self.force_update_count.max(1);
            info!("⚡ Speed: {}", speed);
        }
    }

    pub fn set_effect(&mut self, effect: EffectType) {
        let new_effect: Box<dyn Effect> = match effect {
            EffectType::Static => Box::new(StaticEffect::new(self.last_set_color)),
            EffectType::Rainbow => Box::new(RainbowEffect::new(self.num_leds, self.last_set_speed)),
            EffectType::Breathe => Box::new(BreatheEffect::new(self.last_set_color, self.last_set_speed)),
            EffectType::ColorWipe => Box::new(ColorWipeEffect::new(self.last_set_color, self.last_set_speed, self.num_leds)),
            EffectType::Comet => Box::new(CometEffect::new(self.last_set_color, self.last_set_speed, self.num_leds)),
            EffectType::Scanner => Box::new(ScannerEffect::new(self.last_set_color, self.last_set_speed, self.num_leds)),
            EffectType::TheaterChase => Box::new(TheaterChaseEffect::new(self.last_set_color, self.last_set_speed, self.num_leds)),
            EffectType::Bounce => Box::new(BounceEffect::new(self.last_set_speed, self.num_leds)),
            EffectType::AudioVolumeBar => Box::new(AudioVolumeBarEffect::new(self.last_set_color, self.num_leds)),
        };
        
        info!("✨ Effect: {}", new_effect.name());
        self.current_effect = new_effect;
        self.needs_update = true;
        self.force_update_count = self.force_update_count.max(3);
    }

    pub fn power_on(&mut self) {
        if !self.is_powered {
            self.is_powered = true;
            self.brightness = self.last_brightness;
            self.needs_update = true;
            self.force_update_count = 2;
            info!("💡 LED ON (brightness: {})", self.brightness);
        }
    }

    pub fn power_off(&mut self) {
        if self.is_powered {
            self.is_powered = false;
            self.last_brightness = self.brightness;
            self.brightness = 0;
            self.needs_update = true;
            self.force_update_count = 1;
            self.update_display_fast(); // ✅ Immediate off
            info!("🔌 LED OFF");
        }
    }


    pub fn update(&mut self) {
        // Skip if powered off and no forced updates
        if !self.is_powered && self.force_update_count == 0 {
            return;
        }

        let now = unsafe { esp_timer_get_time() } as u64;
        
        // ✅ 30 FPS frame rate limiting (was 60)
        if self.force_update_count == 0 && now - self.last_update < self.frame_interval {
            return;
        }
        
        let delta_us = now.saturating_sub(self.last_update);
        self.last_update = now;

        // ✅ Update effect state
        if self.current_effect.update(delta_us) {
            self.needs_update = true;
        }

        // ✅ Render and write if needed
        if self.needs_update || self.force_update_count > 0 {
            self.render_current_effect(now);
            self.update_display_optimized();
            
            self.needs_update = false;
            
            if self.force_update_count > 0 {
                self.force_update_count -= 1;
            }
        }
    }

    /// ✅ Render with optimized audio locking
    fn render_current_effect(&mut self, now: u64) {
        if self.current_effect.is_audio_reactive() {
            if let Some(ref audio_data) = self.audio_data {
                // ✅ try_lock with timeout fallback
                if let Ok(audio) = audio_data.try_lock() {
                    self.current_effect.render_audio(&mut self.buffer, &audio, now);
                } else {
                    // Lock failed - render without audio (no blocking!)
                    self.current_effect.render(&mut self.buffer);
                }
            } else {
                self.current_effect.render(&mut self.buffer);
            }
        } else {
            self.current_effect.render(&mut self.buffer);
        }
    }

    /// ✅ OPTIMIZED: Fast display update with bit-shift (no 64KB LUT!)
    fn update_display_optimized(&mut self) {
        let write_start = unsafe { esp_timer_get_time() } as u64;
        
        let brightness = self.brightness;
        let num_leds = self.num_leds;
        
        // ✅ Use bit-shift instead of 64KB LUT (compiler optimizes this well)
        match brightness {
            255 => {
                // Full brightness - direct copy (fastest path)
                let mut idx = 0;
                for pixel in &self.buffer[..num_leds] {
                    self.tx_buffer[idx] = pixel.g;
                    self.tx_buffer[idx + 1] = pixel.r;
                    self.tx_buffer[idx + 2] = pixel.b;
                    idx += 3;
                }
            }
            0 => {
                // Off - memset to zero (very fast)
                self.tx_buffer[..num_leds * 3].fill(0);
            }
            _ => {
                // ✅ Bit-shift scaling (faster than LUT, uses zero extra RAM)
                let mut idx = 0;
                let brightness_u16 = brightness as u16;
                
                for pixel in &self.buffer[..num_leds] {
                    self.tx_buffer[idx] = ((pixel.g as u16 * brightness_u16) >> 8) as u8;
                    self.tx_buffer[idx + 1] = ((pixel.r as u16 * brightness_u16) >> 8) as u8;
                    self.tx_buffer[idx + 2] = ((pixel.b as u16 * brightness_u16) >> 8) as u8;
                    idx += 3;
                }
            }
        }

        // ✅ Write to LEDs
        if let Err(e) = self.driver.write_blocking(self.tx_buffer.iter().cloned()) {
            warn!("⚠️ LED write error: {:?}", e);
        }
        
        // ✅ Performance monitoring
        let write_duration = unsafe { esp_timer_get_time() } as u64 - write_start;
        self.last_write_us = write_duration;
        
        if write_duration > 2000 { // > 2ms is slow
            self.slow_write_count += 1;
            if self.slow_write_count % 30 == 0 {
                warn!("⚠️ Slow RMT write: {}μs ({}x)", write_duration, self.slow_write_count);
            }
        }
    }

    /// Fast update for immediate changes (power off)
    fn update_display_fast(&mut self) {
        let brightness = self.brightness;
        
        if brightness == 0 {
            self.tx_buffer[..self.num_leds * 3].fill(0);
        } else {
            self.update_display_optimized();
            return;
        }
        
        let _ = self.driver.write_blocking(self.tx_buffer.iter().cloned());
    }

    /// Direct write (bypass effects) - for testing
    pub fn write_direct(&mut self, buffer: &[RGB8]) {
        let mut idx = 0;
        for pixel in &buffer[..self.num_leds.min(buffer.len())] {
            self.tx_buffer[idx] = pixel.g;
            self.tx_buffer[idx + 1] = pixel.r;
            self.tx_buffer[idx + 2] = pixel.b;
            idx += 3;
        }
        
        let _ = self.driver.write_blocking(self.tx_buffer.iter().cloned());
    }

    pub fn get_status(&self) -> LedStatus {
        LedStatus {
            is_powered: self.is_powered,
            brightness: self.brightness,
            effect_name: self.current_effect.name().to_string(),
            num_leds: self.num_leds,
            last_color: self.last_set_color,
            last_speed: self.last_set_speed,
            last_write_us: self.last_write_us,
            slow_writes: self.slow_write_count,
        }
    }
}

#[derive(Debug, Clone)]
pub struct LedStatus {
    pub is_powered: bool,
    pub brightness: u8,
    pub effect_name: String,
    pub num_leds: usize,
    pub last_color: RGB8,
    pub last_speed: u8,
    pub last_write_us: u64,
    pub slow_writes: u32,
}