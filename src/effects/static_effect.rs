// src/effects/static_effect.rs

use smart_leds::RGB8;
use super::Effect;

/// Static Effect - Hiển thị một màu tĩnh
/// 
/// Effect đơn giản nhất, chỉ render một lần rồi không update nữa
/// cho đến khi màu được thay đổi. Rất tiết kiệm CPU.
pub struct StaticEffect {
    color: RGB8,
}

impl StaticEffect {
    pub fn new(color: RGB8) -> Self {
        Self { color }
    }
}

impl Effect for StaticEffect {
    fn update(&mut self, _now_us: u64, buffer: &mut [RGB8]) -> Option<u64> {
        // Đơn giản: luôn fill buffer
        buffer.fill(self.color);
        
        // Không cần update tự động nữa
        None
    }

    fn set_color(&mut self, color: RGB8) {
        self.color = color;
    }

    fn get_color(&self) -> Option<RGB8> {
        Some(self.color)
    }

    fn name(&self) -> &str {
        "Static"
    }
}