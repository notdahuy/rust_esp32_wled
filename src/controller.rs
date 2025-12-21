use std::sync::{Arc, Mutex}; 
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
    last_show_us: u64,
    target_fps: u32,
    frame_time_us: u64,
    force_render: bool,         
    render_decay_counter: u8,   
    last_keep_alive_us: u64,   
}

impl<'a> LedController<'a> {
    pub fn new(driver: Ws2812Esp32RmtDriver<'a>, num_leds: usize) -> Self {
        let target_fps = 60;
        Self {
            driver,
            num_leds,
            brightness: 255,
            buffer: vec![RGB8::default(); num_leds],
            tx_buffer: vec![0u8; num_leds * 3],
            current_effect: Box::new(StaticEffect::new(RGB8 { r: 0, g: 0, b: 0 })),
            audio_data: None,
            last_show_us: 0,
            target_fps,
            frame_time_us: 1_000_000 / target_fps as u64,
            force_render: true, 
            render_decay_counter: 20, 
            last_keep_alive_us: 0,
        }
    }

    pub fn set_audio_data(&mut self, audio_data: Arc<Mutex<AudioData>>) {
        self.audio_data = Some(audio_data);
    }

    pub fn set_brightness(&mut self, level: f32) {
        let new_brightness = (level.clamp(0.0, 1.0) * 255.0) as u8;
        if self.brightness != new_brightness {
            self.brightness = new_brightness;
            self.trigger_redundant_render(); 
        }
    }

    pub fn set_color(&mut self, color: RGB8) {
        self.current_effect.set_color(color);
        self.trigger_redundant_render(); 
    }
    
    pub fn set_speed(&mut self, speed: u8) {
        self.current_effect.set_speed(speed);
        self.trigger_redundant_render();
    }
    
    pub fn set_fps(&mut self, fps: u32) {
        self.target_fps = fps.clamp(1, 120);
        self.frame_time_us = 1_000_000 / self.target_fps as u64;
    }
    
    fn trigger_redundant_render(&mut self) {
        self.force_render = true;
        self.render_decay_counter = 15; 
    }

    pub fn set_effect(&mut self, effect_type: EffectType) {
        let color = self.current_effect.get_color()
            .unwrap_or(RGB8 { r: 255, g: 255, b: 255 });
        let speed = self.current_effect.get_speed().unwrap_or(128);

        self.current_effect = match effect_type {
            EffectType::Static => Box::new(StaticEffect::new(color)),
            EffectType::Rainbow => Box::new(RainbowEffect::new(speed)),
            EffectType::VuMeter => Box::new(VuMeterEffect::new(self.num_leds, speed)),
            EffectType::Breathe => Box::new(BreatheEffect::new(color, speed)),
            EffectType::Comet => Box::new(CometEffect::new(color, speed, self.num_leds)),
            EffectType::Bounce => Box::new(BounceEffect::new(color, speed)),
            EffectType::Scanner => Box::new(ScannerEffect::new(color, speed)),
            EffectType::ColorWipe => Box::new(ColorWipeEffect::new(color, speed)),
            EffectType::TheaterChase => Box::new(TheaterChaseEffect::new(color, speed)),
            EffectType::Gravimeter => Box::new(GravimeterEffect::new(self.num_leds, color, speed)),
            EffectType::RadialPulseEffect => Box::new(RadialPulseEffect::new(self.num_leds, color, speed)),
        };

        info!("Effect: {}", self.current_effect.name());
        self.trigger_redundant_render();
    }

    pub fn needs_update(&self, now_us: u64) -> bool {
        now_us >= self.last_show_us + self.frame_time_us
    }

    pub fn get_delay_ms(&self, now_us: u64) -> u32 {
        if now_us >= self.last_show_us + self.frame_time_us {
            return 0;
        }
        let remaining_us = (self.last_show_us + self.frame_time_us) - now_us;
        (remaining_us / 1000) as u32
    }

    pub fn update(&mut self, now_us: u64) {
        if now_us < self.last_show_us + self.frame_time_us {
            return;
        }
    
        let effect_result = if self.current_effect.is_audio_reactive() {
            if let Some(ref audio_data) = self.audio_data {
                if let Ok(audio) = audio_data.lock() {
                    self.current_effect.update_audio(now_us, &audio, &mut self.buffer)
                } else { None }
            } else { None }
        } else {
            self.current_effect.update(now_us, &mut self.buffer)
        };

        let mut should_render = false;
        if effect_result.is_some() || self.current_effect.is_audio_reactive() {
            should_render = true;
            self.render_decay_counter = 0; 
        } 
        else if self.force_render {
            should_render = true;
            self.force_render = false;
        }
        
        if self.render_decay_counter > 0 {
            should_render = true;
            self.render_decay_counter -= 1;
        }

        if now_us > self.last_keep_alive_us + 2_000_000 {
            should_render = true;
            self.last_keep_alive_us = now_us;
        }

        if should_render {
            self.render();
        }
        
        self.last_show_us = now_us;
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