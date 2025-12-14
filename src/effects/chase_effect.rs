use smart_leds::RGB8;
use esp_idf_sys::esp_timer_get_time;
use crate::effects::Effect;

pub struct ChaseEffect {
    color: RGB8,
    bg_color: RGB8,
    speed: u8,
    spacing: usize,
    offset: usize,
    start_time: u64,
}

impl ChaseEffect {
    pub fn new(color: RGB8, speed: u8) -> Self {
        Self {
            color,
            bg_color: RGB8::default(),
            speed: speed.clamp(1, 255),
            spacing: 3,
            offset: 0,
            start_time: 0,
        }
    }
    
    fn steps_per_sec(&self) -> f32 {
        1.0 + (self.speed as f32 / 255.0) * 19.0
    }
}

impl Effect for ChaseEffect {
    fn update(&mut self, now_us: u64, buffer: &mut [RGB8]) -> Option<u64> {
        if self.start_time == 0 {
            self.start_time = now_us;
        }
        
        let elapsed_sec = (now_us - self.start_time) as f32 / 1_000_000.0;
        self.offset = (self.steps_per_sec() * elapsed_sec) as usize % self.spacing;
        
        // Render pattern
        for (i, pixel) in buffer.iter_mut().enumerate() {
            if (i + self.offset) % self.spacing == 0 {
                *pixel = self.color;
            } else {
                *pixel = self.bg_color;
            }
        }
        
        Some(now_us + 1_000_000 / 30)
    }

    fn set_color(&mut self, color: RGB8) {
        self.color = color;
    }

    fn set_speed(&mut self, speed: u8) {
        let new_speed = speed.clamp(1, 255);
        if self.speed != new_speed {
            // Tính offset hiện tại
            let now_us = unsafe { esp_timer_get_time() } as u64;
            let elapsed_sec = (now_us - self.start_time) as f32 / 1_000_000.0;
            let current_offset = (self.steps_per_sec() * elapsed_sec) as usize % self.spacing;
            
            // Đổi speed
            self.speed = new_speed;
            
            // Điều chỉnh start_time để giữ offset hiện tại
            // current_offset = new_speed * (now - new_start_time)
            // => new_start_time = now - (current_offset / new_speed)
            let time_to_current = current_offset as f32 / self.steps_per_sec();
            self.start_time = now_us - (time_to_current * 1_000_000.0) as u64;
        }
    }

    fn get_color(&self) -> Option<RGB8> {
        Some(self.color)
    }

    fn get_speed(&self) -> Option<u8> {
        Some(self.speed)
    }

    fn name(&self) -> &str {
        "Chase"
    }
}