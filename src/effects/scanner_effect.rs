use smart_leds::RGB8;
use crate::effects::{Effect, FRAMETIME_US};

pub struct ScannerEffect {
    color: RGB8,
    speed: u8,
    position: f32,
    direction: i8, // 1 = forward, -1 = backward
    tail_length: usize,
}

impl ScannerEffect {
    pub fn new(color: RGB8, speed: u8) -> Self {
        Self {
            color,
            speed: speed.clamp(1, 255),
            position: 0.0,
            direction: 1,
            tail_length: 5,
        }
    }
}

impl Effect for ScannerEffect {
    fn update(&mut self, now_us: u64, buffer: &mut [RGB8]) -> Option<u64> {
        let num_leds = buffer.len();
        
        // Fade tất cả LEDs
        for pixel in buffer.iter_mut() {
            pixel.r = (pixel.r as u16 * 230 / 256) as u8;
            pixel.g = (pixel.g as u16 * 230 / 256) as u8;
            pixel.b = (pixel.b as u16 * 230 / 256) as u8;
        }
        
        // Tính tốc độ di chuyển
        let speed_factor = self.speed as f32 / 255.0;
        let pixels_per_frame = 0.3 + speed_factor * 2.5; // 0.3 - 2.8 pixels/frame
        
        // Cập nhật vị trí
        self.position += pixels_per_frame * self.direction as f32;
        
        // Đổi hướng khi chạm biên
        if self.position >= (num_leds - 1) as f32 {
            self.position = (num_leds - 1) as f32;
            self.direction = -1;
        } else if self.position <= 0.0 {
            self.position = 0.0;
            self.direction = 1;
        }
        
        let pos = self.position as usize;
        
        // Vẽ đèn chính và đuôi
        for i in 0..self.tail_length {
            let tail_pos = if self.direction == 1 {
                pos.saturating_sub(i)
            } else {
                pos.saturating_add(i).min(num_leds - 1)
            };
            
            if tail_pos < num_leds {
                let brightness = 1.0 - (i as f32 / self.tail_length as f32);
                
                buffer[tail_pos].r = (self.color.r as f32 * brightness) as u8;
                buffer[tail_pos].g = (self.color.g as f32 * brightness) as u8;
                buffer[tail_pos].b = (self.color.b as f32 * brightness) as u8;
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
        "Scanner"
    }
}