use std::sync::{Arc, Mutex}; 
use esp_idf_sys::esp_timer_get_time;
use log::{info, warn};
use smart_leds::RGB8;
use ws2812_esp32_rmt_driver::Ws2812Esp32RmtDriver;
use palette::{FromColor, Hsv, RgbHue, Srgb};
use crate::audio::AudioData;
use crate::effect::*;

pub struct LedController<'a> {
    driver: Ws2812Esp32RmtDriver<'a>,
    num_leds: usize,
    brightness: u8,
    buffer: Vec<RGB8>,
    tx_buffer: Vec<u8>,
    last_update: u64,
    frame_interval: u64, 
    current_effect: Box<dyn Effect>,
    needs_update: bool,
    last_set_color: RGB8,
    last_set_speed: u8,
    audio_data: Option<Arc<Mutex<AudioData>>>
}

impl<'a> LedController<'a> {
    pub fn new(driver: Ws2812Esp32RmtDriver<'a>, num_leds: usize) -> Self {
        let default_color = RGB8 { r: 0, g: 0, b: 0 };
        let default_speed = 128;
        
        Self {
            driver: driver,
            num_leds,
            brightness: 255,
            buffer: vec![RGB8 { r: 0, g: 0, b: 0 }; num_leds],
            tx_buffer: Vec::with_capacity(num_leds * 3),
            last_update: unsafe { esp_timer_get_time() } as u64,
            frame_interval: 33_333, // set fps
            current_effect: Box::new(StaticEffect::new(default_color)),
            needs_update: true,
            last_set_color: default_color,
            last_set_speed: default_speed,
            audio_data: None
        }
    }

    pub fn set_audio_data(&mut self, audio_data: Arc<Mutex<AudioData>>) {
        self.audio_data = Some(audio_data);
        info!("Audio data source connected to LED controller");
    }

    pub fn set_brightness(&mut self, level: f32) {
        let new_level = (level.clamp(0.0, 1.0) * 255.0).round() as u8;
        
        if self.brightness != new_level {
            self.brightness = new_level;
            self.needs_update = true; // Brightness là toàn cục
        }
    }

    pub fn set_color(&mut self, color: RGB8) {   
        self.last_set_color = color;
        if self.current_effect.set_color(color) {
            self.needs_update = true;
        }
    }

    pub fn set_speed(&mut self, speed: u8) {
        self.last_set_speed = speed;
        if self.current_effect.set_speed(speed) {
            self.needs_update = true;
        }
    }

    pub fn set_effect(&mut self, effect: EffectType) {
        let new_effect: Box<dyn Effect> = match effect {
            EffectType::Static => {
                Box::new(StaticEffect::new(self.last_set_color))
            }
            EffectType::Rainbow => {
                Box::new(RainbowEffect::new(self.num_leds, self.last_set_speed))
            }
            EffectType::Breathe => {
                Box::new(BreatheEffect::new(self.last_set_color, self.last_set_speed))
            }
            EffectType::ColorWipe => {
                Box::new(ColorWipeEffect::new(self.last_set_color, self.last_set_speed, self.num_leds))
            }
            EffectType::Comet => {
                Box::new(CometEffect::new(self.last_set_color, self.last_set_speed, self.num_leds))
            }
            EffectType::Scanner => {
                Box::new(ScannerEffect::new(self.last_set_color, self.last_set_speed, self.num_leds))
            }
             EffectType::TheaterChase => {
                Box::new(TheaterChaseEffect::new(self.last_set_color, self.last_set_speed, self.num_leds))
            }
             EffectType::Bounce => {
                Box::new(BounceEffect::new(self.last_set_speed, self.num_leds))
            }
            EffectType::AudioVolumeBar => {
                Box::new(AudioVolumeBarEffect::new(self.last_set_color, self.num_leds))
            }

        };
        
        info!("Effect changed to: {}", new_effect.name());
        self.current_effect = new_effect;
        self.needs_update = true; 
    }

    pub fn update(&mut self) {
        let now = unsafe { esp_timer_get_time() } as u64;
        
        if now - self.last_update < self.frame_interval { return; }
        let delta_us = now.saturating_sub(self.last_update);
        self.last_update = now;

        if self.current_effect.update(delta_us) {
            self.needs_update = true;
        }

        // Chỉ render nếu cần
        if self.needs_update {
            if self.current_effect.is_audio_reactive() {
                // Audio reactive effect - cần audio data
                if let Some(ref audio_data) = self.audio_data {
                    if let Ok(audio) = audio_data.lock() {
                        self.current_effect.render_audio(&mut self.buffer, &audio, now);
                    } else {
                        // Fallback nếu không lock được
                        self.current_effect.render(&mut self.buffer);
                    }
                } else {
                    // Không có audio data - render bình thường
                    warn!("Audio effect active but no audio data source!");
                    self.current_effect.render(&mut self.buffer);
                }
            } else {
                // Normal effect
                self.current_effect.render(&mut self.buffer);
            }
            
            self.update_display();
            self.needs_update = false;
        }
    }

    fn update_display(&mut self) {
        self.tx_buffer.clear();
        let brightness = self.brightness;

        if brightness == 255 { 
            for pixel in &self.buffer { 
                self.tx_buffer.extend_from_slice(&[pixel.g, pixel.r, pixel.b]);
            }
        } else {
            
            let scale = brightness as u16;
            
            for pixel in &self.buffer {
                let scaled = RGB8 {
                    r: ((pixel.r as u16 * scale) >> 8) as u8,
                    g: ((pixel.g as u16 * scale) >> 8) as u8,
                    b: ((pixel.b as u16 * scale) >> 8) as u8,
                };
                self.tx_buffer.extend_from_slice(&[scaled.g, scaled.r, scaled.b]);
            }
        }

       
        if let Err(e) = self.driver.write_blocking(self.tx_buffer.iter().cloned()) {
            warn!("LED write error: {:?}", e);
        }

    }
}