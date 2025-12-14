use smart_leds::RGB8;
use crate::effects::{Effect, speed_to_cycle_time_us, FRAMETIME_US};


pub struct BreatheEffect {
    color: RGB8,
    speed: u8,
    // WLED style: lưu step counter thay vì start_time
    step: u32,
}

impl BreatheEffect {
    pub fn new(color: RGB8, speed: u8) -> Self {
        Self {
            color,
            speed: speed.clamp(1, 255),
            step: 0,
        }
    }
    
    fn get_brightness(&self) -> f32 {
        // Tính cycle time dựa trên speed
        let cycle_time_us = speed_to_cycle_time_us(self.speed);
        let steps_per_cycle = cycle_time_us / FRAMETIME_US;
        
        // Tính phase trong cycle (0.0 - 1.0)
        let phase = (self.step % steps_per_cycle as u32) as f32 / steps_per_cycle as f32;
        
        // Sin wave cho breathing
        let angle = phase * 2.0 * std::f32::consts::PI;
        (angle.sin() + 1.0) / 2.0 // 0.0 - 1.0
    }
}

impl Effect for BreatheEffect {
    fn update(&mut self, now_us: u64, buffer: &mut [RGB8]) -> Option<u64> {
        // WLED style: tăng step counter mỗi frame
        self.step = self.step.wrapping_add(1);
        
        let brightness = self.get_brightness();
        
        // Apply brightness
        let r = (self.color.r as f32 * brightness) as u8;
        let g = (self.color.g as f32 * brightness) as u8;
        let b = (self.color.b as f32 * brightness) as u8;
        
        buffer.fill(RGB8 { r, g, b });
        
        Some(now_us + FRAMETIME_US)
    }

    fn set_color(&mut self, color: RGB8) {
        self.color = color;
    }

    fn set_speed(&mut self, speed: u8) {
        // WLED style: không cần điều chỉnh gì khi đổi speed
        // Animation tự nhiên thay đổi tốc độ mà không nhảy
        self.speed = speed.clamp(1, 255);
    }

    fn get_color(&self) -> Option<RGB8> {
        Some(self.color)
    }

    fn get_speed(&self) -> Option<u8> {
        Some(self.speed)
    }

    fn name(&self) -> &str {
        "Breathe"
    }
}