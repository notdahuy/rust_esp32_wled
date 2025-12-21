// src/effects/static_effect.rs

use smart_leds::RGB8;
use super::Effect;

pub struct StaticEffect {
    color: RGB8,
    dirty: bool,
}

impl StaticEffect {
    pub fn new(color: RGB8) -> Self {
        Self { 
            color,
            dirty: true 
        }
    }
}

impl Effect for StaticEffect {
    fn update(&mut self, _now_us: u64, buffer: &mut [RGB8]) -> Option<u64> {
        if !self.dirty {
            return None;
        }

        buffer.fill(self.color);
        self.dirty = false;
        Some(0) 
    }

    fn set_color(&mut self, color: RGB8) {
        if self.color != color {
            self.color = color;
            self.dirty = true; 
        }
    }

    fn get_color(&self) -> Option<RGB8> {
        Some(self.color)
    }

    fn name(&self) -> &str {
        "Static"
    }
}