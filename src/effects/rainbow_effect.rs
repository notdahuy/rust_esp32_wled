

use smart_leds::RGB8;
use palette::{FromColor, Hsv, RgbHue, Srgb};
use super::Effect;


pub struct RainbowEffect {
    speed: u8,
    start_time: u64,
    hue_delta: f32,
    target_frame_time_us: u64,
}

impl RainbowEffect {

    pub fn new(speed: u8) -> Self {
        let target_fps = 30;
        let target_frame_time_us = 1_000_000 / target_fps;

        Self {
            speed: speed.clamp(1, 255),
            start_time: 0,
            hue_delta: 360.0 / 144.0, 
            target_frame_time_us,
        }
    }

    fn hue_speed(&self) -> f32 {
        let min_speed = 10.0;  
        let max_speed = 360.0; 
        min_speed + (self.speed as f32 / 255.0) * (max_speed - min_speed)
    }
}

impl Effect for RainbowEffect {
    fn update(&mut self, now_us: u64, buffer: &mut [RGB8]) -> Option<u64> {
        if self.start_time == 0 {
            self.start_time = now_us;
        }


        let elapsed_sec = (now_us - self.start_time) as f32 / 1_000_000.0;
        let hue_offset = (self.hue_speed() * elapsed_sec) % 360.0;

        for (i, pixel) in buffer.iter_mut().enumerate() {
            let hue = (hue_offset + (i as f32 * self.hue_delta)) % 360.0;
            let hsv = Hsv::new(RgbHue::from_degrees(hue), 1.0, 1.0);
            let rgb = Srgb::from_color(hsv);

            pixel.r = (rgb.red * 255.0) as u8;
            pixel.g = (rgb.green * 255.0) as u8;
            pixel.b = (rgb.blue * 255.0) as u8;
        }

        Some(now_us + self.target_frame_time_us)
    }

    fn set_speed(&mut self, speed: u8) {
        let new_speed = speed.clamp(1, 255);
        if self.speed != new_speed {
            // Điều chỉnh start_time để animation tiếp tục mượt mà
            let now_us = unsafe { esp_idf_sys::esp_timer_get_time() } as u64;
            let elapsed_sec = (now_us - self.start_time) as f32 / 1_000_000.0;
            let current_hue = (self.hue_speed() * elapsed_sec) % 360.0;

            self.speed = new_speed;

            let time_to_current_hue = current_hue / self.hue_speed();
            self.start_time = now_us - (time_to_current_hue * 1_000_000.0) as u64;
        }
    }

    fn get_speed(&self) -> Option<u8> {
        Some(self.speed)
    }

    fn name(&self) -> &str {
        "Rainbow"
    }
}