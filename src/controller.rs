use std::sync::Arc;

use esp_idf_sys::esp_timer_get_time;
use log::warn;
use smart_leds::RGB8;
use ws2812_esp32_rmt_driver::Ws2812Esp32RmtDriver;
use palette::{FromColor, Hsv, RgbHue, Srgb};
use crate::audio::AudioData;

#[derive(Debug, Clone, PartialEq)]
pub enum EffectType {
    Static,
    Rainbow,    
    MusicBassPulse, // Ví dụ: Nháy màu tím theo bass
    MusicVU,        // Ví dụ: Thước đo VU màu xanh lá
    MusicSpectral,  // Ví dụ: RGB = Bass, Mid, Treble
    Off,            
}

impl EffectType {
    pub fn is_music_effect(&self) -> bool {
        match self {
            EffectType::MusicBassPulse |
            EffectType::MusicVU |
            EffectType::MusicSpectral => true,
            // Tất cả các hiệu ứng khác
            _ => false,
        }
    }
}

pub struct LedController<'a> {
    driver: Ws2812Esp32RmtDriver<'a>,
    num_leds: usize,
    brightness: f32,
    color: RGB8,
    effect: EffectType,
    speed: u8,
    last_update: u64,
    frame_interval: u64,
    phase16: u16,
    front_buffer: Vec<RGB8>,
    back_buffer: Vec<RGB8>,
    tx_buffer: Vec<u8>,
    needs_update: bool,
}

impl<'a> LedController<'a> {
    pub fn new(driver: Ws2812Esp32RmtDriver<'a>, num_leds: usize) -> Self {
        Self {
            driver: driver,
            num_leds,
            brightness: 1.0,
            color: RGB8 { r: 150, g: 150, b: 150 },
            effect: EffectType::Static,
            speed: 128,
            last_update: unsafe { esp_timer_get_time() } as u64,
            frame_interval: 33_333,
            phase16: 0,
            front_buffer: vec![RGB8 { r: 0, g: 0, b: 0 }; num_leds],
            back_buffer: vec![RGB8 { r: 0, g: 0, b: 0 }; num_leds],
            tx_buffer: Vec::with_capacity(num_leds * 3),
            needs_update: true,
        }
    }

    pub fn set_brightness(&mut self, level: f32) {
        let new_level = level.clamp(0.0, 1.0);
        
        if (self.brightness - new_level).abs() > 0.001 {
            log::info!("Brightness changed: {} -> {}", self.brightness, new_level);
            self.brightness = new_level;
            self.needs_update = true; 
        }
    }

    pub fn set_color(&mut self, color: RGB8) {   
        if self.color != color {
            self.color = color;
            self.needs_update = true;

        }
    }

    pub fn set_effect(&mut self, effect: EffectType) {
        self.effect = effect;
        self.needs_update = true;
    }

    pub fn set_speed(&mut self, speed: u8) {
        let old_speed = self.speed;
        self.speed = speed.clamp(1, 255);
        
        if old_speed != self.speed {
            self.needs_update = true;
        }
    }

    pub fn update(&mut self, audio_data: Option<&Arc<AudioData>>) {
        let now = unsafe { esp_timer_get_time() } as u64;
        if now - self.last_update < self.frame_interval { return; }

        let delta_us = now - self.last_update;
        self.last_update = now;

        let phase_increment = (self.speed as u64 * delta_us) / 16000;
        self.phase16 = self.phase16.wrapping_add(phase_increment as u16);

        let effective_audio = if self.effect.is_music_effect() {
            audio_data 
        } else {
            None 
        };

        // Render trực tiếp vào back_buffer
        self.render_to_back_buffer(effective_audio);

        // Chỉ swap và update nếu có thay đổi
        if self.back_buffer != self.front_buffer || self.needs_update {
            std::mem::swap(&mut self.front_buffer, &mut self.back_buffer);
            self.update_display();
            self.needs_update = false;
        }
    }

    fn render_to_back_buffer(&mut self, audio_data: Option<&Arc<AudioData>>) {
        match self.effect {
            EffectType::Static => {
                for pixel in self.back_buffer.iter_mut() {
                    *pixel = self.color;
                }
            }
            EffectType::Off => {
                for pixel in self.back_buffer.iter_mut() {
                    *pixel = RGB8 { r: 0, g: 0, b: 0 };
                }
            }
            EffectType::Rainbow => {
                rainbow_effect(self.phase16, self.num_leds, &mut self.back_buffer);
            }
            EffectType::MusicBassPulse => {
                if let Some(audio) = audio_data {
                    render_music_bass_pulse(audio, &mut self.back_buffer);
                }
            }
            EffectType::MusicVU => {
                if let Some(audio) = audio_data {
                    render_music_vu(audio, self.num_leds, &mut self.back_buffer);
                }
            }
            EffectType::MusicSpectral => {
                if let Some(audio) = audio_data {
                    render_music_spectral(audio, &mut self.back_buffer);
                }
            }
        }
    }

    fn update_display(&mut self) {
        self.tx_buffer.clear();
    
        // Sử dụng FRONT BUFFER để hiển thị (buffer đã được swap)
        for pixel in &self.front_buffer {
            let scaled = RGB8 {
                r: ((pixel.r as f32) * self.brightness).round() as u8,
                g: ((pixel.g as f32) * self.brightness).round() as u8,
                b: ((pixel.b as f32) * self.brightness).round() as u8,
            };
            self.tx_buffer.extend_from_slice(&[scaled.g, scaled.r, scaled.b]);
        }

        if let Err(e) = self.driver.write_blocking(self.tx_buffer.iter().cloned()) {
                warn!("LED write error: {:?}", e);
            }
    }

    

}

fn rainbow_effect(phase16: u16, num_leds: usize, frame: &mut [RGB8]) {
    let phase_spacing = (65536_u32 / num_leds as u32) as u16;

    for (i, pixel) in frame.iter_mut().enumerate() {
        // Tất cả các phép tính đều là u16 và tự động wrap
        let pixel_phase = phase16.wrapping_add((i as u16).wrapping_mul(phase_spacing));
        let hue_u16 = ((pixel_phase as u32 * 360) >> 16) as u16; 
        let hue = hue_u16 as f32;

        let color = Hsv::new(RgbHue::from_degrees(hue), 1.0, 1.0);
        let srgb: Srgb = Srgb::from_color(color);

        *pixel = RGB8 {
            r: (srgb.red * 255.0).round() as u8,
            g: (srgb.green * 255.0).round() as u8,
            b: (srgb.blue * 255.0).round() as u8,
        };
    }
}   

// HIỆU ỨNG 1: Nháy theo Bass
fn render_music_bass_pulse(audio: &Arc<AudioData>, frame: &mut [RGB8]) {
    let bass = audio.get_bass(); // 0.0 - 1.0
    let color = RGB8 { r: 255, g: 0, b: 255 }; // Màu tím
    
    let scaled_color = scale_color(color, bass);
    frame.fill(scaled_color); // Tô toàn bộ dải LED
}

// HIỆU ỨNG 2: Thước đo VU
fn render_music_vu(audio: &Arc<AudioData>, num_leds: usize, frame: &mut [RGB8]) {
    let intensity = audio.get_amplitude(); // 0.0 - 1.0
    let leds_to_light = (intensity * num_leds as f32).round() as usize;

    for (i, pixel) in frame.iter_mut().enumerate() {
        if i < leds_to_light {
            // Hiển thị màu xanh lá
            *pixel = RGB8 {r: 0, g: 255, b: 0}; 
        } else {
            // Tắt
            *pixel = RGB8::default(); 
        }
    }
}

// HIỆU ỨNG 3: Phân tích phổ màu
fn render_music_spectral(audio: &Arc<AudioData>, frame: &mut [RGB8]) {
    // Bass = Đỏ, Mid = Xanh lá, Treble = Xanh dương
    let r = (audio.get_bass() * 255.0) as u8;
    let g = (audio.get_mid() * 255.0) as u8;
    let b = (audio.get_treble() * 255.0) as u8;
    
    frame.fill(RGB8 { r, g, b });
}

fn scale_color(color: RGB8, brightness: f32) -> RGB8 {
    RGB8 {
        r: (color.r as f32 * brightness).round() as u8,
        g: (color.g as f32 * brightness).round() as u8,
        b: (color.b as f32 * brightness).round() as u8,
    }
}