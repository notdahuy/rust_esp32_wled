use smart_leds::RGB8;
use crate::effects::{Effect, FRAMETIME_US};

pub struct BounceEffect {
    color: RGB8,
    speed: u8,
    position: f32,
    velocity: f32,
    ball_size: usize,
}

impl BounceEffect {
    pub fn new(color: RGB8, speed: u8) -> Self {
        Self {
            color,
            speed: speed.clamp(1, 255),
            position: 0.0,
            velocity: 1.0,
            ball_size: 3,
        }
    }
}

impl Effect for BounceEffect {
    fn update(&mut self, now_us: u64, buffer: &mut [RGB8]) -> Option<u64> {
        let num_leds = buffer.len();
        
        // Fade tất cả LEDs
        for pixel in buffer.iter_mut() {
            pixel.r = (pixel.r as u16 * 200 / 256) as u8;
            pixel.g = (pixel.g as u16 * 200 / 256) as u8;
            pixel.b = (pixel.b as u16 * 200 / 256) as u8;
        }
        
        // Tính tốc độ di chuyển
        let speed_factor = self.speed as f32 / 255.0;
        let base_velocity = 0.3 + speed_factor * 2.5; // 0.3 - 2.8 pixels/frame
        
        // Cập nhật vị trí với velocity hiện tại
        self.position += self.velocity;
        
        // Bounce khi chạm biên
        if self.position >= (num_leds - 1) as f32 {
            self.position = (num_leds - 1) as f32;
            self.velocity = -base_velocity;
        } else if self.position <= 0.0 {
            self.position = 0.0;
            self.velocity = base_velocity;
        }
        
        let pos = self.position as usize;
        
        // Vẽ quả bóng với gradient
        let half_size = self.ball_size / 2;
        for i in 0..self.ball_size {
            let offset = i as i32 - half_size as i32;
            let idx = pos as i32 + offset;
            
            if idx >= 0 && idx < num_leds as i32 {
                let distance = offset.abs() as f32 / half_size.max(1) as f32;
                let brightness = (1.0 - distance).max(0.0).min(1.0);
                
                let idx = idx as usize;
                let new_r = (self.color.r as f32 * brightness) as u8;
                let new_g = (self.color.g as f32 * brightness) as u8;
                let new_b = (self.color.b as f32 * brightness) as u8;
                
                // Lấy giá trị lớn nhất để tạo trail
                buffer[idx].r = buffer[idx].r.max(new_r);
                buffer[idx].g = buffer[idx].g.max(new_g);
                buffer[idx].b = buffer[idx].b.max(new_b);
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
        "Bounce"
    }
}