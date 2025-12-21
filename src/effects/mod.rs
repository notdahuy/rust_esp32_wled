
use std::collections::HashMap;
use once_cell::sync::Lazy;
use smart_leds::RGB8;
use crate::audio::AudioData;

const FRAMETIME_US: u64 = 1_000_000 / 42; // 42 FPS như WLED

// submodules
mod static_effect;
mod blink_effect;
mod rainbow_effect;
mod vu_meter_effect;
mod breathe_effect;
mod comet_effect;
mod scanner_effect;
mod theater_chase_effect;
mod color_wipe_effect;
mod bounce_effect;
mod gravimeter_effect;
mod radialpulse_effect;


// Re-export các effects 
pub use static_effect::StaticEffect;
pub use rainbow_effect::RainbowEffect;
pub use vu_meter_effect::VuMeterEffect;
pub use breathe_effect::BreatheEffect;
pub use comet_effect::CometEffect;
pub use scanner_effect::ScannerEffect;
pub use theater_chase_effect::TheaterChaseEffect;
pub use bounce_effect::BounceEffect;
pub use color_wipe_effect::ColorWipeEffect;
pub use gravimeter_effect::GravimeterEffect;
pub use radialpulse_effect::RadialPulseEffect;


/// Enum định nghĩa các loại effect có sẵn
#[derive(Debug, Clone, PartialEq, Copy, Eq, Hash)]
pub enum EffectType {
    Static,
    Rainbow,
    VuMeter,
    Breathe,   
    Comet,
    Scanner,
    TheaterChase,
    Bounce, 
    ColorWipe,
    Gravimeter,
    RadialPulseEffect
}

/// Registry mapping từ tên string sang EffectType
pub static EFFECT_REGISTRY: Lazy<HashMap<&'static str, EffectType>> = Lazy::new(|| {
    let mut m = HashMap::new();
    m.insert("static", EffectType::Static);
    m.insert("rainbow", EffectType::Rainbow);
    m.insert("vu", EffectType::VuMeter);
    m.insert("breathe", EffectType::Breathe);
    m.insert("comet", EffectType::Comet);
    m.insert("colorwipe", EffectType::ColorWipe);
    m.insert("bounce", EffectType::Bounce);
    m.insert("theaterchase", EffectType::TheaterChase);
    m.insert("scanner", EffectType::Scanner);
    m.insert("gravimeter", EffectType::Gravimeter);
    m.insert("pulse", EffectType::RadialPulseEffect);

    m
});

pub trait Effect {
    fn update(&mut self, now_us: u64, buffer: &mut [RGB8]) -> Option<u64>;
    fn set_color(&mut self, _color: RGB8) {}
    fn set_speed(&mut self, _speed: u8) {}
    fn get_color(&self) -> Option<RGB8> { None }
    fn get_speed(&self) -> Option<u8> { None }
    fn is_audio_reactive(&self) -> bool { false }
    fn update_audio(&mut self, now_us: u64, audio: &AudioData, buffer: &mut [RGB8]) -> Option<u64> {
        let _ = audio; 
        self.update(now_us, buffer)
    }
    fn name(&self) -> &str;
}

fn speed_to_cycle_time_us(speed: u8) -> u64 {
    let speed_clamped = speed.clamp(1, 255) as u64;
    let min_time = 50_000;    
    let max_time = 2_000_000;
    max_time - ((speed_clamped * (max_time - min_time)) / 255)
}