use smart_leds::RGB8;

use crate::effects::Effect;

pub struct SparkleEffect {
    color: RGB8,
    speed: u8,
    last_update: u64,
    active_sparkles: Vec<(usize, u8)>, // (position, brightness)
}

impl SparkleEffect {
    pub fn new(color: RGB8, speed: u8) -> Self {
        Self {
            color,
            speed: speed.clamp(1, 255),
            last_update: 0,
            active_sparkles: Vec::new(),
        }
    }
    
    fn update_interval_us(&self) -> u64 {
        // Speed càng cao, update càng nhanh
        let min = 20_000;  // 50 FPS
        let max = 100_000; // 10 FPS
        max - (self.speed as u64 * (max - min) / 255)
    }
}

impl Effect for SparkleEffect {
    fn update(&mut self, now_us: u64, buffer: &mut [RGB8]) -> Option<u64> {
        if self.last_update == 0 {
            self.last_update = now_us;
        }
        
        // Clear buffer (background màu tối)
        buffer.fill(RGB8 { r: 0, g: 0, b: 0 });
        
        // Random add new sparkles
        let num_new = (self.speed as usize / 50).max(1);
        for _ in 0..num_new {
            if (now_us % 7) == 0 { // Pseudo random
                let pos = ((now_us / 1000) % buffer.len() as u64) as usize;
                self.active_sparkles.push((pos, 255));
            }
        }
        
        // Update và render sparkles
        self.active_sparkles.retain_mut(|(pos, brightness)| {
            if *brightness > 20 {
                // Fade out
                *brightness = brightness.saturating_sub(15);
                
                // Render
                let fade = *brightness as f32 / 255.0;
                buffer[*pos] = RGB8 {
                    r: (self.color.r as f32 * fade) as u8,
                    g: (self.color.g as f32 * fade) as u8,
                    b: (self.color.b as f32 * fade) as u8,
                };
                true
            } else {
                false
            }
        });
        
        self.last_update = now_us;
        Some(now_us + self.update_interval_us())
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
        "Sparkle"
    }
}