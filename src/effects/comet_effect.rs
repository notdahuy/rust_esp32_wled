use smart_leds::RGB8;

use crate::effects::Effect;

pub struct CometEffect {
    color: RGB8,
    speed: u8,
    position: f32,
    tail_length: usize,
    start_time: u64,
}

impl CometEffect {
    pub fn new(color: RGB8, speed: u8, num_leds: usize) -> Self {
        Self {
            color,
            speed: speed.clamp(1, 255),
            position: 0.0,
            tail_length: (num_leds / 4).max(5), // 25% của strip hoặc ít nhất 5 LEDs
            start_time: 0,
        }
    }
    
    fn speed_pixels_per_sec(&self) -> f32 {
        // Speed 1 = 10 pixels/sec, Speed 255 = 200 pixels/sec
        10.0 + (self.speed as f32 / 255.0) * 190.0
    }
}

impl Effect for CometEffect {
    fn update(&mut self, now_us: u64, buffer: &mut [RGB8]) -> Option<u64> {
        if self.start_time == 0 {
            self.start_time = now_us;
        }
        
        let elapsed_sec = (now_us - self.start_time) as f32 / 1_000_000.0;
        self.position = (self.speed_pixels_per_sec() * elapsed_sec) % (buffer.len() as f32);
        
        // Clear buffer
        buffer.fill(RGB8::default());
        
        let head_pos = self.position as usize;
        
        // Draw comet head và tail
        for i in 0..self.tail_length {
            let pos = (head_pos + buffer.len() - i) % buffer.len();
            
            // Brightness giảm dần theo tail
            let brightness = 1.0 - (i as f32 / self.tail_length as f32);
            let brightness = brightness.powf(2.0); // Exponential fade
            
            let r = (self.color.r as f32 * brightness) as u8;
            let g = (self.color.g as f32 * brightness) as u8;
            let b = (self.color.b as f32 * brightness) as u8;
            
            buffer[pos] = RGB8 { r, g, b };
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
        "Comet"
    }
}
