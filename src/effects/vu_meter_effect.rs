

use smart_leds::RGB8;
use crate::audio::AudioData;
use super::Effect;

pub struct VuMeterEffect {
    num_leds: usize,
    speed: u8,
    current_level: f32,
    peak_level: f32,
    peak_hold_time: u64,
    peak_hold_duration_us: u64,
    last_update_time: u64,
    target_frame_time_us: u64,
}

impl VuMeterEffect {
    pub fn new(num_leds: usize, speed: u8) -> Self {
        let target_fps = 60;
        let target_frame_time_us = 1_000_000 / target_fps;

        Self {
            num_leds,
            speed: speed.clamp(1, 255),
            current_level: 0.0,
            peak_level: 0.0,
            peak_hold_time: 0,
            peak_hold_duration_us: 500_000,
            last_update_time: 0,
            target_frame_time_us,
        }
    }

    fn smoothing_factor(&self) -> (f32, f32) {
        let base_attack = 0.3;
        let base_release = 0.1;
        let speed_factor = self.speed as f32 / 255.0;

        let attack = base_attack + speed_factor * 0.4;
        let release = base_release + speed_factor * 0.2;

        (attack, release)
    }


    fn level_to_color(&self, normalized_position: f32) -> RGB8 {
        if normalized_position < 0.33 {
            let t = normalized_position / 0.33;
            RGB8 {
                r: 0,
                g: (255.0 * t) as u8,
                b: 0,
            }
        } else if normalized_position < 0.66 {
            let t = (normalized_position - 0.33) / 0.33;
            RGB8 {
                r: (255.0 * t) as u8,
                g: 255,
                b: 0,
            }
        } else {
            let t = (normalized_position - 0.66) / 0.34;
            RGB8 {
                r: 255,
                g: (255.0 * (1.0 - t)) as u8,
                b: 0,
            }
        }
    }
}

impl Effect for VuMeterEffect {
    
    fn update(&mut self, _now_us: u64, _buffer: &mut [RGB8]) -> Option<u64> {
        None
    }

    fn update_audio(&mut self, now_us: u64, audio: &AudioData, buffer: &mut [RGB8]) -> Option<u64> {
        if self.last_update_time == 0 {
            self.last_update_time = now_us;
        }

        let target_level = audio.volume.clamp(0.0, 1.0);

        
        let (attack, release) = self.smoothing_factor();
        let smoothing = if target_level > self.current_level {
            attack
        } else {
            release
        };

        self.current_level = self.current_level * (1.0 - smoothing) + target_level * smoothing;

        
        if self.current_level > self.peak_level {
            self.peak_level = self.current_level;
            self.peak_hold_time = now_us;
        } else if now_us - self.peak_hold_time > self.peak_hold_duration_us {
            self.peak_level = self.current_level;
        }

        let num_lit = (self.current_level * self.num_leds as f32) as usize;
        let peak_position = (self.peak_level * self.num_leds as f32) as usize;

        
        for (i, pixel) in buffer.iter_mut().enumerate() {
            if i < num_lit {
                let position = i as f32 / self.num_leds as f32;
                *pixel = self.level_to_color(position);
            } else if i == peak_position {
                *pixel = RGB8 { r: 0, g: 255, b: 255 };
            } else {
                *pixel = RGB8::default();
            }
        }

        self.last_update_time = now_us;
        Some(now_us + self.target_frame_time_us)
    }

    fn is_audio_reactive(&self) -> bool {
        true
    }

    fn set_speed(&mut self, speed: u8) {
        self.speed = speed.clamp(1, 255);
    }

    fn get_speed(&self) -> Option<u8> {
        Some(self.speed)
    }

    fn name(&self) -> &str {
        "VU Meter"
    }
}