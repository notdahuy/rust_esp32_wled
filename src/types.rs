use serde::Deserialize;
use smart_leds::RGB8;

pub const LED_COUNT: usize = 144;

#[derive(Clone, Copy)]
pub enum EffectType {
    Static,
    Breathing,
    ColorWipe,
    Rainbow,
}

pub struct LedState {
    pub r: u8,
    pub g: u8,
    pub b: u8,
    pub brightness: u8,
    pub effect: EffectType,
    pub is_running: bool,
}

impl Default for LedState {
    fn default() -> Self {
        Self { 
            r: 255, 
            g: 255, 
            b: 255, 
            brightness: 100,
            effect: EffectType::Static,
            is_running: false
        }
    }
}

impl LedState {
    
    pub fn get_static_color(&self) -> RGB8 {
        RGB8 {
            r: scale_brightness(self.r, self.brightness),
            g: scale_brightness(self.g, self.brightness),
            b: scale_brightness(self.b, self.brightness),
        }
    }

     pub fn get_effect_colors(&mut self) -> Vec<RGB8> {
        match self.effect {
            EffectType::Static => vec![self.get_static_color(); LED_COUNT],
            EffectType::Rainbow => create_rainbow_effect(),
            EffectType::Breathing => create_breathing_effect(self),
            EffectType::ColorWipe => create_color_wipe_effect(self),
        }
    }
}

// Effect implementations
fn create_rainbow_effect() -> Vec<RGB8> {
    static mut OFFSET: u8 = 0;
    unsafe {
        OFFSET = OFFSET.wrapping_add(1); // dịch dần hue
        (0..LED_COUNT)
            .map(|i| {
                let hue = ((i as u16 * 256 / LED_COUNT as u16) as u8).wrapping_add(OFFSET);
                hsv_to_rgb(hue, 255, 255)
            })
            .collect()
    }
}


fn create_breathing_effect(state: &LedState) -> Vec<RGB8> {
    static mut BREATH_LEVEL: f32 = 0.0;
    unsafe {
        BREATH_LEVEL = (BREATH_LEVEL + 0.05) % (2.0 * std::f32::consts::PI);
        let brightness = ((BREATH_LEVEL.sin() + 1.0) / 2.0 * state.brightness as f32) as u8;
        
        vec![RGB8 {
            r: scale_brightness(state.r, brightness),
            g: scale_brightness(state.g, brightness),
            b: scale_brightness(state.b, brightness),
        }; LED_COUNT]
    }
}


fn create_color_wipe_effect(state: &LedState) -> Vec<RGB8> {
    static mut POSITION: usize = 0;
    static mut FORWARD: bool = true;
    unsafe {
        let mut pixels = vec![RGB8::default(); LED_COUNT];
        for i in 0..=POSITION {
            pixels[i] = state.get_static_color();
        }

        if FORWARD {
            POSITION += 1;
            if POSITION >= LED_COUNT - 1 {
                FORWARD = false;
            }
        } else {
            if POSITION > 0 {
                POSITION -= 1;
            } else {
                FORWARD = true;
            }
        }
        pixels
    }
}


// Helper function for rainbow effect
fn hsv_to_rgb(h: u8, s: u8, v: u8) -> RGB8 {
    let h = h as f32 / 255.0;
    let s = s as f32 / 255.0;
    let v = v as f32 / 255.0;

    let i = (h * 6.0).floor();
    let f = h * 6.0 - i;
    let p = v * (1.0 - s);
    let q = v * (1.0 - f * s);
    let t = v * (1.0 - (1.0 - f) * s);

    let (r, g, b) = match i as i32 % 6 {
        0 => (v, t, p),
        1 => (q, v, p),
        2 => (p, v, t),
        3 => (p, q, v),
        4 => (t, p, v),
        _ => (v, p, q),
    };

    RGB8 {
        r: (r * 255.0) as u8,
        g: (g * 255.0) as u8,
        b: (b * 255.0) as u8,
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

#[derive(Deserialize)]
pub struct EffectRequest {
    pub effect_type: String,
} 