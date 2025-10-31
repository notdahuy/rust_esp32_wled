use std::sync::Arc;
use std::sync::Mutex;

use esp_idf_sys::esp_timer_get_time;
use log::warn;
use smart_leds::RGB8;
use ws2812_esp32_rmt_driver::Ws2812Esp32RmtDriver;
use palette::{FromColor, Hsv, RgbHue, Srgb};
use crate::audio::AudioProcessor;

struct FrameTimeDebugger {
    stutter_threshold_us: u64,
    last_warning_time: u64,
    warning_cooldown_ms: u64,
}

impl FrameTimeDebugger {
    fn new(threshold_us: u64) -> Self {
        Self {
            stutter_threshold_us: threshold_us,
            last_warning_time: 0,
            warning_cooldown_ms: 2000, // Chỉ in cảnh báo mỗi 2 giây
        }
    }

    fn check(&mut self, delta_us: u64, now: u64) {
        if delta_us > self.stutter_threshold_us {
            let now_ms = now / 1000;
            if now_ms - self.last_warning_time > self.warning_cooldown_ms {
                warn!("[DEBUG] Frame time spike detected! Delta: {} us", delta_us);
                self.last_warning_time = now_ms;
            }
        }
    }
}

#[derive(Debug, Clone)]
pub enum EffectType {
    Static,
    Rainbow,    
    Off,            
}

pub struct LedController<'a> {
    driver: Arc<Mutex<Ws2812Esp32RmtDriver<'a>>>,
    num_leds: usize,
    brightness: f32,
    color: RGB8,
    effect: EffectType,
    speed: u8,
    last_update: u64,
    frame_interval: u64,
    // time: f32,
    phase16: u16,
    audio_processor: Option<Arc<AudioProcessor>>,
    front_buffer: Vec<RGB8>,
    back_buffer: Vec<RGB8>,
    tx_buffer: Vec<u8>,
    needs_update: bool,
    debugger: FrameTimeDebugger,
}

impl<'a> LedController<'a> {
    pub fn new(driver: Ws2812Esp32RmtDriver<'a>, num_leds: usize) -> Self {
        Self {
            driver: Arc::new(Mutex::new(driver)),
            num_leds,
            brightness: 1.0,
            color: RGB8 { r: 150, g: 150, b: 150 },
            effect: EffectType::Static,
            speed: 128,
            last_update: unsafe { esp_timer_get_time() } as u64,
            frame_interval: 16_667,
            // time: 0.0,
            phase16: 0,
            audio_processor: None,
            front_buffer: vec![RGB8 { r: 0, g: 0, b: 0 }; num_leds],
            back_buffer: vec![RGB8 { r: 0, g: 0, b: 0 }; num_leds],
            tx_buffer: Vec::with_capacity(num_leds * 3),
            needs_update: true, 
            debugger: FrameTimeDebugger::new(45_000),
        }
    }

    pub fn set_audio_processor(&mut self, processor: Arc<AudioProcessor>) {
        self.audio_processor = Some(processor);
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

    pub fn update(&mut self) {
        let now = unsafe { esp_timer_get_time() } as u64;
        if now - self.last_update < self.frame_interval { return; }

        let delta_us = now - self.last_update;
        self.debugger.check(delta_us, now);
        self.last_update = now;

        let phase_increment = (self.speed as u64 * delta_us) / 16000;
        self.phase16 = self.phase16.wrapping_add(phase_increment as u16);

        // Render trực tiếp vào back_buffer
        self.render_to_back_buffer();

        // Chỉ swap và update nếu có thay đổi
        if self.back_buffer != self.front_buffer || self.needs_update {
            std::mem::swap(&mut self.front_buffer, &mut self.back_buffer);
            self.update_display();
            self.needs_update = false;
        }
    }

    fn render_to_back_buffer(&mut self) {
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

        if let Ok(mut driver) = self.driver.lock() {
            if let Err(e) = driver.write_blocking(self.tx_buffer.iter().cloned()) {
                warn!("LED write error: {:?}", e);
            }
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