use std::sync::Arc;
use esp_idf_sys::esp_timer_get_time;
use log::{info, warn};
use smart_leds::RGB8;
use ws2812_esp32_rmt_driver::Ws2812Esp32RmtDriver;
use palette::{FromColor, Hsv, RgbHue, Srgb};
use crate::audio::AudioData;

#[derive(Debug, Clone, PartialEq)]
pub enum EffectType {
    Static,
    Rainbow,
    Off,
}

pub trait Effect {

    fn update(&mut self, delta_us: u64, audio: Option<&Arc<AudioData>>) -> bool;

    fn render(&self, buffer: &mut [RGB8]);


    fn set_color(&mut self, color: RGB8) -> bool {
        false 
    }
    
    fn set_speed(&mut self, speed: u8) -> bool {
        false 
    }

    fn name(&self) -> &'static str;
}


pub struct StaticEffect {
    color: RGB8,
}

impl StaticEffect {
    pub fn new(color: RGB8) -> Self { Self { color } }
}

impl Effect for StaticEffect {
    fn name(&self) -> &'static str { "Static" }

    fn update(&mut self, _delta_us: u64, _audio: Option<&Arc<AudioData>>) -> bool {
        false 
    }

    fn render(&self, buffer: &mut [RGB8]) {
        buffer.fill(self.color);
    }

    fn set_color(&mut self, color: RGB8) -> bool {
        if self.color != color {
            self.color = color;
            return true; // Báo cho controller biết cần render lại
        }
        false
    }
}


pub struct RainbowEffect {
    phase16: u16,
    speed: u8,
    phase_spacing: u16,
    lut: Vec<RGB8>,
}

impl RainbowEffect {
    pub fn new(num_leds: usize, speed: u8) -> Self {
        let mut lut = Vec::with_capacity(256); 
        
        
        for i in 0..=255 {
            let hue = (i as f32 * 360.0) / 256.0; 

            let color = Hsv::new(RgbHue::from_degrees(hue), 1.0, 1.0);
            let srgb: Srgb = Srgb::from_color(color);

            lut.push(RGB8 {
                r: (srgb.red * 255.0).round() as u8,
                g: (srgb.green * 255.0).round() as u8,
                b: (srgb.blue * 255.0).round() as u8,
            });
        }

        Self {
            phase16: 0,
            speed: speed.clamp(1, 255),
            phase_spacing: (65536_u32 / num_leds.max(1) as u32) as u16,
            lut: lut,
        }
    }
}

impl Effect for RainbowEffect {
    fn name(&self) -> &'static str { "Rainbow" }

    fn update(&mut self, delta_us: u64, _audio: Option<&Arc<AudioData>>) -> bool {
        let phase_increment = (self.speed as u64 * delta_us) / 16000;
        self.phase16 = self.phase16.wrapping_add(phase_increment as u16);
        true
    }

    fn render(&self, buffer: &mut [RGB8]) {
        for (i, pixel) in buffer.iter_mut().enumerate() {
            let pixel_phase = self.phase16.wrapping_add((i as u16).wrapping_mul(self.phase_spacing));
            let hue_index = (pixel_phase >> 8) as u8;
            *pixel = self.lut[hue_index as usize];
        }
    }
    
    fn set_speed(&mut self, speed: u8) -> bool {
        self.speed = speed.clamp(1, 255);
        false 
    }
}

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
}

impl<'a> LedController<'a> {
    pub fn new(driver: Ws2812Esp32RmtDriver<'a>, num_leds: usize) -> Self {
        let default_color = RGB8 { r: 150, g: 150, b: 150 };
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
        }
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
            EffectType::Off => {
                Box::new(StaticEffect::new(RGB8 { r:0, g:0, b:0 }))
            }
          
        };
        
        info!("Effect changed to: {}", new_effect.name());
        self.current_effect = new_effect;
        self.needs_update = true; 
    }

    pub fn update(&mut self, audio_data: Option<&Arc<AudioData>>) {
        let now = unsafe { esp_timer_get_time() } as u64;
        
        if now - self.last_update < self.frame_interval { return; }
        let delta_us = now.saturating_sub(self.last_update);
        self.last_update = now;

        if self.current_effect.update(delta_us, audio_data) {
            self.needs_update = true;
        }

        // Chỉ render nếu cần
        if self.needs_update {
            self.current_effect.render(&mut self.buffer);
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
