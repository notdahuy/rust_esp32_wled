use smart_leds::RGB8;
use crate::effects::Effect;

pub struct FadeEffect {
    color1: RGB8,
    color2: RGB8,
    speed: u8,
    start_time: u64,
}

impl FadeEffect {
    pub fn new(color1: RGB8, speed: u8) -> Self {
        // Color2 mặc định là đen (tắt)
        let color2 = RGB8::default();
        Self {
            color1,
            color2,
            speed: speed.clamp(1, 255),
            start_time: 0,
        }
    }
    
    fn cycles_per_sec(&self) -> f32 {
        // Speed 1 = 0.1 cycles/sec (10s per cycle)
        // Speed 255 = 2 cycles/sec (0.5s per cycle)
        0.1 + (self.speed as f32 / 255.0) * 1.9
    }
}

impl Effect for FadeEffect {
    fn update(&mut self, now_us: u64, buffer: &mut [RGB8]) -> Option<u64> {
        if self.start_time == 0 {
            self.start_time = now_us;
        }
        
        let elapsed_sec = (now_us - self.start_time) as f32 / 1_000_000.0;
        let phase = (self.cycles_per_sec() * elapsed_sec * 2.0 * std::f32::consts::PI).sin();
        let blend = (phase + 1.0) / 2.0; // 0.0 - 1.0
        
        // Blend giữa 2 màu
        let r = (self.color1.r as f32 * (1.0 - blend) + self.color2.r as f32 * blend) as u8;
        let g = (self.color1.g as f32 * (1.0 - blend) + self.color2.g as f32 * blend) as u8;
        let b = (self.color1.b as f32 * (1.0 - blend) + self.color2.b as f32 * blend) as u8;
        
        buffer.fill(RGB8 { r, g, b });
        
        Some(now_us + 1_000_000 / 30) // 30 FPS
    }

    fn set_color(&mut self, color: RGB8) {
        self.color1 = color;
    }

    fn set_speed(&mut self, speed: u8) {
        self.speed = speed.clamp(1, 255);
    }

    fn get_color(&self) -> Option<RGB8> {
        Some(self.color1)
    }

    fn get_speed(&self) -> Option<u8> {
        Some(self.speed)
    }

    fn name(&self) -> &str {
        "Fade"
    }
}
