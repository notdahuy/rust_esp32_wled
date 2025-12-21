use smart_leds::RGB8;
use crate::effects::{Effect, speed_to_cycle_time_us, FRAMETIME_US};

pub struct ColorWipeEffect {
    color: RGB8,
    speed: u8,
    position: f32, // Dùng float để chuyển động mượt
}

impl ColorWipeEffect {
    pub fn new(color: RGB8, speed: u8) -> Self {
        Self {
            color,
            speed: speed.clamp(1, 255),
            position: 0.0,
        }
    }
}

impl Effect for ColorWipeEffect {
    fn update(&mut self, now_us: u64, buffer: &mut [RGB8]) -> Option<u64> {
        let num_leds = buffer.len();
        
        // Tính tốc độ di chuyển dựa trên speed
        // Speed cao = di chuyển nhanh hơn mỗi frame
        let speed_factor = self.speed as f32 / 255.0;
        let pixels_per_frame = 0.5 + speed_factor * 3.0; // 0.5 - 3.5 pixels/frame
        
        // Cập nhật vị trí
        self.position += pixels_per_frame;
        
        // Reset khi hoàn thành chu kỳ
        if self.position >= num_leds as f32 {
            self.position = 0.0;
        }
        
        // Fill màu từ đầu đến position
        let pos = self.position as usize;
        for i in 0..num_leds {
            if i <= pos {
                buffer[i] = self.color;
            } else {
                buffer[i] = RGB8 { r: 0, g: 0, b: 0 };
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
        "ColorWipe"
    }
}