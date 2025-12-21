use smart_leds::RGB8;
use crate::effects::{Effect, FRAMETIME_US};

pub struct TheaterChaseEffect {
    color: RGB8,
    speed: u8,
    offset: f32,
    spacing: usize,
}

impl TheaterChaseEffect {
    pub fn new(color: RGB8, speed: u8) -> Self {
        Self {
            color,
            speed: speed.clamp(1, 255),
            offset: 0.0,
            spacing: 3,
        }
    }
}

impl Effect for TheaterChaseEffect {
    fn update(&mut self, now_us: u64, buffer: &mut [RGB8]) -> Option<u64> {
        // Tính tốc độ di chuyển
        let speed_factor = self.speed as f32 / 255.0;
        let step_per_frame = 0.05 + speed_factor * 0.4; // 0.05 - 0.45 per frame
        
        // Cập nhật offset
        self.offset += step_per_frame;
        if self.offset >= self.spacing as f32 {
            self.offset -= self.spacing as f32;
        }
        
        let offset_int = self.offset as usize;
        
        // Vẽ pattern theater chase
        for (i, pixel) in buffer.iter_mut().enumerate() {
            if (i + self.spacing - offset_int) % self.spacing == 0 {
                *pixel = self.color;
            } else {
                *pixel = RGB8 { r: 0, g: 0, b: 0 };
            }
        }
        
        Some(now_us + FRAMETIME_US)
    }

    fn set_color(&mut self, color: RGB8) {
        self.color = color;
    }

    fn set_speed(&mut self, speed: u8) {
        self.speed = speed.clamp(1, 255);
    }

    fn get_color(&self) -> Option<RGB8> {
        Some(self.color)
    }

    fn get_speed(&self) -> Option<u8> {
        Some(self.speed)
    }

    fn name(&self) -> &str {
        "TheaterChase"
    }
}