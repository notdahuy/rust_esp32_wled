
use std::collections::HashMap;
use once_cell::sync::Lazy;
use smart_leds::RGB8;
use crate::audio::AudioData;

const FRAMETIME_US: u64 = 1_000_000 / 42; // 42 FPS như WLED
const SPEED_MULTIPLIER: f32 = 255.0; // Speed từ 1-255

// Declare submodules
mod static_effect;
mod blink_effect;
mod rainbow_effect;
mod vu_meter_effect;
mod breathe_effect;
mod chase_effect;
mod comet_effect;
mod sparkle_effect;
mod scan_effect;
mod fade_effect;


// Re-export các effects để code bên ngoài có thể dùng
pub use static_effect::StaticEffect;
pub use blink_effect::BlinkEffect;
pub use rainbow_effect::RainbowEffect;
pub use vu_meter_effect::VuMeterEffect;
pub use breathe_effect::BreatheEffect;
pub use chase_effect::ChaseEffect;
pub use comet_effect::CometEffect;
pub use sparkle_effect::SparkleEffect;
pub use scan_effect::ScanEffect;
pub use fade_effect::FadeEffect;

/// Enum định nghĩa các loại effect có sẵn
#[derive(Debug, Clone, PartialEq, Copy)]
pub enum EffectType {
    Static,
    Blink,
    Rainbow,
    VuMeter,
    Breathe,   
    Comet,     
    Sparkle,   
    Chase,     
    Fade,      
    Scan,      
}

/// Registry mapping từ tên string sang EffectType

pub static EFFECT_REGISTRY: Lazy<HashMap<&'static str, EffectType>> = Lazy::new(|| {
    let mut m = HashMap::new();
    m.insert("static", EffectType::Static);
    m.insert("blink", EffectType::Blink);
    m.insert("rainbow", EffectType::Rainbow);
    m.insert("vu", EffectType::VuMeter);
    m.insert("breathe", EffectType::Breathe);
    m.insert("comet", EffectType::Comet);
    m.insert("sparkle", EffectType::Sparkle);
    m.insert("chase", EffectType::Chase);
    m.insert("fade", EffectType::Fade);
    m.insert("scan", EffectType::Scan);
    m
});

pub trait Effect {
    fn update(&mut self, now_us: u64, buffer: &mut [RGB8]) -> Option<u64>;

    /// Set màu cho effect (nếu effect hỗ trợ)
    fn set_color(&mut self, _color: RGB8) {}
    
    /// Set tốc độ cho effect (nếu effect hỗ trợ)
    fn set_speed(&mut self, _speed: u8) {}

    /// Lấy màu hiện tại của effect (nếu có)
    fn get_color(&self) -> Option<RGB8> {
        None
    }
    
    /// Lấy tốc độ hiện tại của effect (nếu có)
    fn get_speed(&self) -> Option<u8> {
        None
    }

    /// Kiểm tra xem effect có phản ứng với audio không
    fn is_audio_reactive(&self) -> bool {
        false
    }

    /// Update effect với audio data (chỉ cho audio-reactive effects)
    fn update_audio(&mut self, now_us: u64, audio: &AudioData, buffer: &mut [RGB8]) -> Option<u64> {
        let _ = audio; // Ignore audio data
        self.update(now_us, buffer)
    }

    /// Trả về tên của effect để logging
    fn name(&self) -> &str;
}

fn speed_to_cycle_time_us(speed: u8) -> u64 {
    // Speed 1 = chậm (2 giây), Speed 255 = nhanh (50ms)
    let speed_clamped = speed.clamp(1, 255) as u64;
    let min_time = 50_000;    // 50ms
    let max_time = 2_000_000; // 2s
    max_time - ((speed_clamped * (max_time - min_time)) / 255)
}