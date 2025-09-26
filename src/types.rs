use serde::Deserialize;
use smart_leds::RGB8;

pub const LED_COUNT: usize = 144;

pub struct LedState {
    pub r: u8,
    pub g: u8,
    pub b: u8,
    pub brightness: u8
}

impl Default for LedState {
    fn default() -> Self {
        Self { r: 255, g: 255, b: 255, brightness: 100}
    }
}

impl LedState {
    // Helper function để tính toán màu cuối cùng với brightness
    pub fn get_final_color(&self) -> RGB8 {
        RGB8 {
            r: scale_brightness(self.r, self.brightness),
            g: scale_brightness(self.g, self.brightness),
            b: scale_brightness(self.b, self.brightness),
        }
    }
}

pub fn scale_brightness(color: u8, brightness: u8) -> u8 {
    let brightness = brightness.min(100);
    
    if brightness == 0 {
        return 0;
    }  

    let brightness_factor = brightness as f32 / 100.0;
    
    let scaled = (color as f32 * brightness_factor).round() as u8;
    
    
    if brightness > 0 && scaled == 0 && color > 0 {
        1
    } else {
        scaled
    }
}

#[derive(Deserialize)]
pub struct ColorRequest {
    pub r: u8,
    pub g: u8,
    pub b: u8,
}

#[derive(Deserialize)]
pub struct BrightnessRequest {
    pub percent: u8,
}