use std::sync::{Arc, Mutex}; 
use esp_idf_sys::esp_timer_get_time;
use log::info;
use smart_leds::RGB8;
use ws2812_esp32_rmt_driver::Ws2812Esp32RmtDriver;
use crate::audio::AudioData;
use crate::effects::*;

pub struct LedController<'a> {
    driver: Ws2812Esp32RmtDriver<'a>,
    num_leds: usize,
    brightness: u8,
    buffer: Vec<RGB8>,
    tx_buffer: Vec<u8>,
    current_effect: Box<dyn Effect>,
    audio_data: Option<Arc<Mutex<AudioData>>>,
    next_update_time: Option<u64>,
}

impl<'a> LedController<'a> {
    pub fn new(driver: Ws2812Esp32RmtDriver<'a>, num_leds: usize) -> Self {
        Self {
            driver,
            num_leds,
            brightness: 255,
            buffer: vec![RGB8::default(); num_leds],
            tx_buffer: vec![0u8; num_leds * 3],
            current_effect: Box::new(StaticEffect::new(RGB8 { r: 150, g: 150, b: 0 })),
            audio_data: None,
            next_update_time: Some(0),
        }
    }

    pub fn set_audio_data(&mut self, audio_data: Arc<Mutex<AudioData>>) {
        self.audio_data = Some(audio_data);
    }

    pub fn set_brightness(&mut self, level: f32) {
        let new = (level.clamp(0.0, 1.0) * 255.0) as u8;
        if self.brightness != new {
            self.brightness = new;
        }
    }

    pub fn set_color(&mut self, color: RGB8) {
        self.current_effect.set_color(color);
    }
    
    pub fn set_speed(&mut self, speed: u8) {
        self.current_effect.set_speed(speed);
    }

    pub fn set_effect(&mut self, effect_type: EffectType) {
        let color = self.current_effect.get_color()
            .unwrap_or(RGB8 { r: 255, g: 255, b: 255 });
        let speed = self.current_effect.get_speed().unwrap_or(128);

        self.current_effect = match effect_type {
            EffectType::Static => Box::new(StaticEffect::new(color)),
            EffectType::Blink => Box::new(BlinkEffect::new(color, speed)),
            EffectType::Rainbow => Box::new(RainbowEffect::new(speed)),
            EffectType::VuMeter => Box::new(VuMeterEffect::new(self.num_leds, speed)),
            EffectType::Breathe => Box::new(BreatheEffect::new(color, speed)),
            EffectType::Comet => Box::new(CometEffect::new(color, speed, self.num_leds)),
            EffectType::Sparkle => Box::new(SparkleEffect::new(color, speed)),
            EffectType::Chase => Box::new(ChaseEffect::new(color, speed)),
            EffectType::Fade => Box::new(FadeEffect::new(color, speed)),
            EffectType::Scan => Box::new(ScanEffect::new(color, speed, self.num_leds)),
        };

        info!("Effect: {}", self.current_effect.name());
    }

    pub fn needs_update(&self, now_us: u64) -> bool {
        match self.next_update_time {
            None => false,
            Some(time) => now_us >= time,
        }
    }

    pub fn get_delay_ms(&self, now_us: u64) -> u32 {
        match self.next_update_time {
            None => 10,
            Some(time) => {
                if now_us >= time {
                    1
                } else {
                    let us = time - now_us;
                    ((us / 1000).min(10)) as u32
                }
            }
        }
    }

    pub fn update(&mut self, now_us: u64) {
        let next_time = if self.current_effect.is_audio_reactive() {
            if let Some(ref audio_data) = self.audio_data {
                if let Ok(audio) = audio_data.lock() {
                    self.current_effect.update_audio(now_us, &audio, &mut self.buffer)
                } else {
                    Some(now_us + 1000)
                }
            } else {
                None
            }
        } else {
            self.current_effect.update(now_us, &mut self.buffer)
        };
        
        self.next_update_time = next_time;
        self.render();
    }

    fn render(&mut self) {
        let brightness = self.brightness as u16;

        for (i, pixel) in self.buffer.iter().enumerate() {
            let base = i * 3;
            
            let r = (pixel.r as u16 * brightness / 255) as u8;
            let g = (pixel.g as u16 * brightness / 255) as u8;
            let b = (pixel.b as u16 * brightness / 255) as u8;

            self.tx_buffer[base] = g;
            self.tx_buffer[base + 1] = r;
            self.tx_buffer[base + 2] = b;
        }

        let _ = self.driver.write_blocking(self.tx_buffer.iter().cloned());
    }
}