use smart_leds::RGB8;

use crate::effects::Effect;

pub struct ScanEffect {
    color: RGB8,
    speed: u8,
    position: f32,
    direction: f32, // 1.0 hoặc -1.0
    eye_size: usize,
}

impl ScanEffect {
    pub fn new(color: RGB8, speed: u8, num_leds: usize) -> Self {
        Self {
            color,
            speed: speed.clamp(1, 255),
            position: 0.0,
            direction: 1.0,
            eye_size: (num_leds / 20).max(3), // 5% hoặc ít nhất 3 LEDs
        }
    }
    
    fn speed_pixels_per_sec(&self) -> f32 {
        10.0 + (self.speed as f32 / 255.0) * 190.0
    }
}

impl Effect for ScanEffect {
    fn update(&mut self, now_us: u64, buffer: &mut [RGB8]) -> Option<u64> {
        // Update position
        let delta = self.speed_pixels_per_sec() / 60.0; // 60 FPS
        self.position += delta * self.direction;
        
        // Bounce at edges
        if self.position >= buffer.len() as f32 - self.eye_size as f32 {
            self.position = buffer.len() as f32 - self.eye_size as f32;
            self.direction = -1.0;
        } else if self.position <= 0.0 {
            self.position = 0.0;
            self.direction = 1.0;
        }
        
        // Clear buffer
        buffer.fill(RGB8::default());
        
        // Draw eye
        let center = self.position as usize;
        for i in 0..self.eye_size {
            let pos = center + i;
            if pos < buffer.len() {
                // Brightness cao nhất ở giữa
                let dist_from_center = (i as f32 - self.eye_size as f32 / 2.0).abs();
                let brightness = 1.0 - (dist_from_center / (self.eye_size as f32 / 2.0));
                
                buffer[pos] = RGB8 {
                    r: (self.color.r as f32 * brightness) as u8,
                    g: (self.color.g as f32 * brightness) as u8,
                    b: (self.color.b as f32 * brightness) as u8,
                };
            }
        }
        
        Some(now_us + 1_000_000 / 60) // 60 FPS
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
        "Scan"
    }
}