use std::sync::Arc;
use std::sync::Mutex;

use esp_idf_sys::esp_timer_get_time;
use log::warn;
use smart_leds::RGB8;
use ws2812_esp32_rmt_driver::Ws2812Esp32RmtDriver;
use palette::{FromColor, Hsv, RgbHue, Srgb};
use crate::audio::AudioProcessor;

#[derive(Debug, Clone)]
pub enum EffectType {
    Static,
    Rainbow,
    Blink,
    BlinkRainbow,     
    Aurora,
    Meteor,
    ColorWipe,
    Off,            
}

pub struct LedController<'a> {
    driver: Arc<Mutex<Ws2812Esp32RmtDriver<'a>>>,
    num_leds: usize,
    brightness: f32,
    color: RGB8,
    effect: EffectType,
    speed: u8,
    tick: u64,
    last_update: u64,
    frame_interval: u64,
    audio_processor: Option<Arc<AudioProcessor>>,
    prev_frame: Vec<RGB8>,
    needs_update: bool,
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
            tick: 0,
            last_update: 0,
            frame_interval: 33_333,
            audio_processor: None,
            prev_frame: vec![RGB8 { r: 0, g: 0, b: 0 }; num_leds],
            needs_update: true, 
        }
    }

    pub fn set_audio_processor(&mut self, processor: Arc<AudioProcessor>) {
        self.audio_processor = Some(processor);
    }

    pub fn set_brightness(&mut self, level: f32) {
        let new_level = level.clamp(0.0, 1.0);
        
        if (self.brightness - new_level).abs() > 0.001 {
            self.brightness = new_level;
            self.needs_update = true; 
        }
    }

    pub fn set_color(&mut self, color: RGB8) {
        self.color = color
    }

    pub fn set_effect(&mut self, effect: EffectType) {
        self.tick = 0;
        self.effect = effect;
        self.needs_update = true;
    }

    pub fn set_speed(&mut self, speed: u8) {
        self.speed = speed.clamp(1, 255);
    }

    pub fn update(&mut self) {
        let now = unsafe { esp_timer_get_time() } as u64;

        // Nếu chưa đủ thời gian để render frame mới thì return
        if now - self.last_update < self.frame_interval {
            return;
        }
        self.last_update = now;

        let speed_scaled = if self.speed < 50 {
            (self.speed as u32) / 5
        } else if self.speed < 150 {
            10 + ((self.speed as u32 - 50) * 30) / 100
        } else {
            40 + ((self.speed as u32 - 150) * 45) / 105
        };
        self.tick = self.tick.wrapping_add(speed_scaled as u64 + 1);

        // Static/Off effects
        if matches!(self.effect, EffectType::Static | EffectType::Off) {
            if !self.needs_update {
                return;
            }

            let frame = match self.effect {
                EffectType::Static => vec![self.color; self.num_leds],
                EffectType::Off => vec![RGB8 { r: 0, g: 0, b: 0 }; self.num_leds],
                _ => unreachable!(),
            };

            if frame != self.prev_frame {
                self.update_frame(&frame);
                self.prev_frame = frame;
            }
            self.needs_update = false;
            return;
        }
   
        let frame = match self.effect {
            EffectType::Rainbow => self.rainbow_effect(),
            EffectType::Blink => self.blink_effect(),
            EffectType::BlinkRainbow => self.blink_rainbow_effect(),
            EffectType::Aurora => self.aurora_effect(),
            EffectType::Meteor => self.meteor_effect(),
            EffectType::ColorWipe => self.colorwipe_effect(),
            _ => unreachable!(),
        };

        if frame != self.prev_frame || self.needs_update {
            self.update_frame(&frame);
            self.prev_frame = frame;
        }

        self.needs_update = false;
    }

    fn update_frame(&self, frame: &[RGB8]) {
        let mut pixel_bytes = Vec::with_capacity(self.num_leds * 3);
        for pixel in frame {
            let scaled = RGB8 {
                r: ((pixel.r as f32) * self.brightness) as u8,
                g: ((pixel.g as f32) * self.brightness) as u8,
                b: ((pixel.b as f32) * self.brightness) as u8,
            };
            pixel_bytes.extend_from_slice(&[scaled.g, scaled.r, scaled.b]);
        }

        if let Ok(mut driver) = self.driver.lock() {
            if let Err(e) = driver.write_blocking(pixel_bytes.iter().cloned()) {
                warn!("LED write error: {:?}", e);
            }
        }
    }

    fn rainbow_effect(&self) -> Vec<RGB8> {
        let mut frame = Vec::with_capacity(self.num_leds);

        // tick bạn đã tăng theo speed trong update()
        let offset = self.tick as usize;

        for i in 0..self.num_leds {
            // Hue tăng dần theo vị trí + offset → tạo chuyển động
            let hue = ((i * 360 / self.num_leds) + offset) % 360;

            let color = Hsv::new(RgbHue::from_degrees(hue as f32), 1.0, 1.0);
            let srgb: Srgb = Srgb::from_color(color);

            frame.push(RGB8 {
                r: (srgb.red * 255.0) as u8,
                g: (srgb.green * 255.0) as u8,
                b: (srgb.blue * 255.0) as u8,
            });
        }
        frame
    }


    fn blink_effect(&self) -> Vec<RGB8> {
        // Tick-based blinking với wrapping
        let blink_speed = (self.speed as f32 / 255.0) * 50.0 + 10.0; // 10-60 ticks per cycle
        let cycle_length = blink_speed as u64;
        
        let on = (self.tick.wrapping_rem(cycle_length)) < cycle_length / 2;
        
        let color = if on { self.color } else { RGB8 { r: 0, g: 0, b: 0 } };
        vec![color; self.num_leds]
    }

    fn blink_rainbow_effect(&self) -> Vec<RGB8> {
        // Tick-based blinking with color change
        let blink_speed = (self.speed as f32 / 255.0) * 40.0 + 15.0; // 15-55 ticks per cycle
        let cycle_length = blink_speed as u64;
        
        let on = (self.tick.wrapping_rem(cycle_length)) < cycle_length / 2;
        
        if on {
            // Color changes based on tick
            let color_change_speed = (self.speed as f32 / 255.0) * 30.0 + 10.0; // 10-40 ticks per color
            let color_cycle = (self.tick.wrapping_div(color_change_speed as u64)) as usize % 7;
            
            let color = match color_cycle {
                0 => RGB8 { r: 255, g: 0, b: 0 },
                1 => RGB8 { r: 255, g: 127, b: 0 },
                2 => RGB8 { r: 255, g: 255, b: 0 },
                3 => RGB8 { r: 0, g: 255, b: 0 },
                4 => RGB8 { r: 0, g: 127, b: 255 },
                5 => RGB8 { r: 0, g: 0, b: 255 },
                _ => RGB8 { r: 255, g: 0, b: 255 },
            };
            vec![color; self.num_leds]
        } else {
            vec![RGB8 { r: 0, g: 0, b: 0 }; self.num_leds]
        }
    }

    fn aurora_effect(&self) -> Vec<RGB8> {
        let mut frame = Vec::with_capacity(self.num_leds);
        
        let speed_factor = (self.speed as f32 / 255.0) * 0.1 + 0.02;
        let time = self.tick as f32 * speed_factor;
        
        for i in 0..self.num_leds {
            let pos = i as f32 / self.num_leds as f32;
            
            let wave1 = (time * 0.3 + pos * 3.141).sin();
            let wave2 = (time * 0.5 + pos * 2.356).cos() * 0.7;
            let wave3 = (time * 0.2 + pos * 4.0).sin() * 0.5;
            let wave4 = (time * 0.7 + pos * 1.570).cos() * 0.3;
            
            let combined_wave = (wave1 + wave2 + wave3 + wave4) * 0.4 + 0.5;
            let brightness = combined_wave.clamp(0.1, 1.0);
            
            let base_hue = 170.0;
            let hue_variation = (time * 0.05 + pos * 1.0).sin() * 40.0;
            let hue = (base_hue + hue_variation).rem_euclid(360.0);
            
            let saturation = 0.6 + brightness * 0.3;
            
            let hsv = Hsv::new(
                RgbHue::from_degrees(hue),
                saturation,
                brightness
            );
            
            let rgb: Srgb = Srgb::from_color(hsv);
            frame.push(RGB8 {
                r: (rgb.red * 255.0) as u8,
                g: (rgb.green * 255.0) as u8,
                b: (rgb.blue * 255.0) as u8,
            });
        }
        frame
    }

    fn colorwipe_effect(&self) -> Vec<RGB8> {
        let mut frame = vec![RGB8 { r: 0, g: 0, b: 0 }; self.num_leds];
        
        // Tick-based wiping với wrapping
        let wipe_speed = (self.speed as f32 / 255.0) * 80.0 + 20.0; // 20-100 ticks per cycle
        let cycle_length = wipe_speed as u64;
        
        let cycle_progress = (self.tick.wrapping_rem(cycle_length)) as f32 / cycle_length as f32;
        
        if cycle_progress < 0.5 {
            // Fill phase
            let fill_progress = cycle_progress * 2.0;
            let led_to_fill = (fill_progress * self.num_leds as f32).round() as usize;
            
            for i in 0..led_to_fill.min(self.num_leds) {
                frame[i] = self.color;
            }
        } else {
            // Clear phase
            let clear_progress = (cycle_progress - 0.5) * 2.0;
            let led_to_clear = (clear_progress * self.num_leds as f32).round() as usize;
            
            for i in 0..self.num_leds {
                frame[i] = if i < led_to_clear { 
                    RGB8 { r: 0, g: 0, b: 0 } 
                } else { 
                    self.color 
                };
            }
        }
        frame
    }

     fn meteor_effect(&self) -> Vec<RGB8> {
        let mut frame = vec![RGB8 { r: 0, g: 0, b: 0 }; self.num_leds];
        
        let speed_factor = (self.speed as f32 / 255.0) * 0.3 + 0.1;
        let meteor_length = 8;
        let num_meteors = 2;
        
        // Tick-based meteor position với wrapping
        let main_pos = ((self.tick as f32 * speed_factor) as usize) % (self.num_leds + meteor_length * 2);
        
        for meteor_idx in 0..num_meteors {
            let phase_shift = meteor_idx as f32 * 2.0 * std::f32::consts::PI / num_meteors as f32;
            let meteor_offset = (meteor_idx * self.num_leds) / num_meteors;
            let meteor_pos = (main_pos.wrapping_add(meteor_offset)) % (self.num_leds + meteor_length * 2);
            
            for i in 0..meteor_length {
                let pos = meteor_pos as i32 - i as i32;
                
                if pos >= 0 && pos < self.num_leds as i32 {
                    let idx = pos as usize;
                    
                    let hue_shift = (meteor_idx as f32 * 120.0 + i as f32 * 10.0) % 360.0;
                    let fade = 1.0 - (i as f32 / meteor_length as f32);
                    let exponential_fade = fade.powf(1.5);
                    
                    let hsv = Hsv::new(
                        RgbHue::from_degrees(hue_shift), 
                        0.9, 
                        exponential_fade
                    );
                    let rgb: Srgb = Srgb::from_color(hsv);
                    
                    let meteor_color = RGB8 {
                        r: (rgb.red * 255.0) as u8,
                        g: (rgb.green * 255.0) as u8,
                        b: (rgb.blue * 255.0) as u8,
                    };
                    
                    frame[idx] = RGB8 {
                        r: meteor_color.r.max(frame[idx].r),
                        g: meteor_color.g.max(frame[idx].g),
                        b: meteor_color.b.max(frame[idx].b),
                    };
                }
            }
        }
    
        // Fade out
        let fade_factor = 0.92 + (self.speed as f32 / 255.0) * 0.05;
        for pixel in frame.iter_mut() {
            pixel.r = (pixel.r as f32 * fade_factor) as u8;
            pixel.g = (pixel.g as f32 * fade_factor) as u8;
            pixel.b = (pixel.b as f32 * fade_factor) as u8;
        }
    
        frame
    }
}