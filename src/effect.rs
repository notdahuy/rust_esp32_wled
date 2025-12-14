use std::collections::HashMap;

use once_cell::sync::Lazy;
use smart_leds::RGB8;
use palette::{FromColor, Hsv, RgbHue, Srgb};
use crate::audio::AudioData;

pub static EFFECT_REGISTRY: Lazy<HashMap<&'static str, EffectType>> = Lazy::new(|| {
    let mut m = HashMap::new();

    m.insert("static",  EffectType::Static);
    m.insert("blink",   EffectType::Blink);
    m.insert("rainbow", EffectType::Rainbow);
    m.insert("vu",       EffectType::VuMeter);
    m
});

#[derive(Debug, Clone, PartialEq, Copy)]
pub enum EffectType {
    Static,
    Blink,
    Rainbow,
    VuMeter
}

/// Trait chung cho tất cả các hiệu ứng
pub trait Effect {
    fn update(&mut self, now_us: u64, buffer: &mut [RGB8]) -> Option<u64>;

    fn set_color(&mut self, _color: RGB8) {}
    fn set_speed(&mut self, _speed: u8) {}

    fn get_color(&self) -> Option<RGB8> { None }
    fn get_speed(&self) -> Option<u8> { None }

    fn is_audio_reactive(&self) -> bool { false }
    
    /// Update effect với audio data (chỉ cho audio-reactive effects)
    /// Default implementation để các non-audio effects không cần implement
    fn update_audio(&mut self, now_us: u64, audio: &AudioData, buffer: &mut [RGB8]) -> Option<u64> {
        // Default: gọi update() thông thường và ignore audio data
        let _ = audio;
        self.update(now_us, buffer)
    }

    fn name(&self) -> &str;
}


pub struct StaticEffect {
    color: RGB8,
    needs_initial_render: bool,
}

impl StaticEffect {
    pub fn new(color: RGB8) -> Self {
        Self { 
            color, 
            needs_initial_render: true, // Cần render lần đầu
        }
    }
}

impl Effect for StaticEffect {
    fn update(&mut self, _now_us: u64, buffer: &mut [RGB8]) -> Option<u64> {
        // Chỉ render nếu cần thiết (lần đầu hoặc sau khi đổi màu)
        if self.needs_initial_render {
            buffer.fill(self.color);
            self.needs_initial_render = false;
            
            // Đã render xong, không cần update nữa cho đến khi có thay đổi
            // Trả về None = "đừng gọi tôi nữa cho đến khi có lệnh mới"
            return None;
        }
        
        // Không nên đến đây vì controller chỉ gọi khi cần
        // Nhưng nếu có gọi thì cũng không cần update gì
        None
    }

    fn set_color(&mut self, color: RGB8) {
        if self.color != color {
            self.color = color;
            // Đánh dấu cần render lại với màu mới
            self.needs_initial_render = true;
        }
    }

    fn get_color(&self) -> Option<RGB8> {
        Some(self.color)
    }

    fn name(&self) -> &str {
        "Static"
    }
}

pub struct RainbowEffect {
    speed: u8,
    
    // Animation state - SỬ DỤNG ABSOLUTE TIME
    start_time: u64,        // Thời điểm effect bắt đầu (microseconds)
    hue_delta: f32,         // Khoảng cách màu giữa các LED liền kề
    
    // FPS control
    target_frame_time_us: u64, // Thời gian giữa các frame (microseconds)
}

impl RainbowEffect {
    pub fn new(speed: u8) -> Self {
        // Target 30 FPS cho rainbow effect (đủ mượt mà không tốn tài nguyên)
        let target_fps = 30;
        let target_frame_time_us = 1_000_000 / target_fps;
        
        Self {
            speed: speed.clamp(1, 255),
            start_time: 0, // Sẽ được set ở lần update đầu tiên
            hue_delta: 360.0 / 144.0, // 144 LED, mỗi LED cách nhau 2.5 độ màu
            target_frame_time_us,
        }
    }
    
    /// Tính tốc độ thay đổi màu dựa trên speed (degrees per second)
    fn hue_speed(&self) -> f32 {
        // Speed 1 = chậm nhất (10 độ/giây = 36 giây cho một vòng cầu vồng)
        // Speed 255 = nhanh nhất (360 độ/giây = 1 giây cho một vòng cầu vồng)
        let min_speed = 10.0;   // degrees per second
        let max_speed = 360.0;  // degrees per second
        min_speed + (self.speed as f32 / 255.0) * (max_speed - min_speed)
    }
}

impl Effect for RainbowEffect {
    fn update(&mut self, now_us: u64, buffer: &mut [RGB8]) -> Option<u64> {
        // Lần đầu tiên được gọi - khởi tạo start_time
        if self.start_time == 0 {
            self.start_time = now_us;
        }
        
        // Tính elapsed time từ khi bắt đầu (ABSOLUTE TIME)
        // Cách này loại bỏ hoàn toàn accumulation error
        let elapsed_sec = (now_us - self.start_time) as f32 / 1_000_000.0;
        
        // Tính hue offset dựa trên elapsed time và speed
        // Công thức: hue = (speed * time) % 360
        // Điều này đảm bảo animation luôn mượt mà và chính xác
        let hue_offset = (self.hue_speed() * elapsed_sec) % 360.0;
        
        // Render rainbow vào buffer
        for (i, pixel) in buffer.iter_mut().enumerate() {
            // Tính hue cho LED này dựa trên vị trí và offset toàn cục
            let hue = (hue_offset + (i as f32 * self.hue_delta)) % 360.0;
            
            // Chuyển từ HSV sang RGB
            // Saturation = 1.0 (màu thuần, không pha trắng)
            // Value = 1.0 (độ sáng tối đa, brightness sẽ được apply bởi controller)
            let hsv = Hsv::new(RgbHue::from_degrees(hue), 1.0, 1.0);
            let rgb = Srgb::from_color(hsv);
            
            // Convert sang RGB8
            pixel.r = (rgb.red * 255.0) as u8;
            pixel.g = (rgb.green * 255.0) as u8;
            pixel.b = (rgb.blue * 255.0) as u8;
        }
        
        // Trả về thời điểm cần update tiếp theo (30 FPS)
        Some(now_us + self.target_frame_time_us)
    }

    fn set_speed(&mut self, speed: u8) {
        let new_speed = speed.clamp(1, 255);
        if self.speed != new_speed {
            // Khi thay đổi speed, ta cần điều chỉnh start_time để animation
            // tiếp tục mượt mà từ vị trí hiện tại thay vì nhảy đột ngột
            // 
            // Tính hue_offset hiện tại dựa trên speed cũ
            let now_us = unsafe { esp_idf_sys::esp_timer_get_time() } as u64;
            let elapsed_sec = (now_us - self.start_time) as f32 / 1_000_000.0;
            let current_hue = (self.hue_speed() * elapsed_sec) % 360.0;
            
            // Cập nhật speed
            self.speed = new_speed;
            
            // Tính lại start_time sao cho với speed mới, ta vẫn ở cùng hue hiện tại
            // current_hue = new_speed * (now - new_start_time)
            // => new_start_time = now - (current_hue / new_speed)
            let time_to_current_hue = current_hue / self.hue_speed();
            self.start_time = now_us - (time_to_current_hue * 1_000_000.0) as u64;
        }
    }

    fn get_speed(&self) -> Option<u8> {
        Some(self.speed)
    }

    fn name(&self) -> &str {
        "Rainbow"
    }
}

pub struct BlinkEffect {
    on: RGB8,
    off: RGB8,
    speed: u8,
    
    // State tracking
    current_state: bool, // true = ON, false = OFF
    next_transition_time: u64, // Thời điểm chuyển trạng thái tiếp theo
    cycle_time_us: u64, // Độ dài một chu kỳ (ON + OFF)
}

impl BlinkEffect {
    pub fn new(color: RGB8, speed: u8) -> Self {
        let cycle_time = cycle_time_us(speed);
        
        Self {
            on: color,
            off: RGB8::default(),
            speed: speed.clamp(1, 255),
            current_state: true, // Bắt đầu với ON
            next_transition_time: 0, // Sẽ được set ở lần update đầu tiên
            cycle_time_us: cycle_time,
        }
    }
    
    /// Tính thời điểm chuyển trạng thái tiếp theo dựa trên thời gian hiện tại
    fn calculate_next_transition(&self, now_us: u64) -> u64 {
        let half_cycle = self.cycle_time_us / 2;
        
        // Tìm chu kỳ hiện tại
        let current_cycle = now_us / self.cycle_time_us;
        let phase = now_us % self.cycle_time_us;
        
        // Tính thời điểm chuyển trạng thái tiếp theo
        if phase < half_cycle {
            // Đang ở nửa đầu (ON), chuyển sang OFF sau khi hết nửa đầu
            current_cycle * self.cycle_time_us + half_cycle
        } else {
            // Đang ở nửa sau (OFF), chuyển sang ON ở chu kỳ tiếp theo
            (current_cycle + 1) * self.cycle_time_us
        }
    }
}

fn cycle_time_us(speed: u8) -> u64 {
    // Speed 1..255 → cycle time ~2000ms .. ~50ms
    // Tốc độ càng cao, chu kỳ càng ngắn, blink càng nhanh
    let min = 50_000;
    let max = 2_000_000;
    max - (speed as u64 * (max - min) / 255)
}

impl Effect for BlinkEffect {
    fn update(&mut self, now_us: u64, buffer: &mut [RGB8]) -> Option<u64> {
        // Lần đầu tiên được gọi, khởi tạo next_transition_time
        if self.next_transition_time == 0 {
            // Render trạng thái ban đầu (ON)
            buffer.fill(self.on);
            
            // Tính thời điểm chuyển sang OFF
            let half_cycle = self.cycle_time_us / 2;
            self.next_transition_time = now_us + half_cycle;
            
            // Trả về thời điểm cần update tiếp theo
            return Some(self.next_transition_time);
        }
        
        // Kiểm tra xem đã đến lúc chuyển trạng thái chưa
        if now_us >= self.next_transition_time {
            // Đã đến lúc chuyển trạng thái
            self.current_state = !self.current_state;
            
            // Render trạng thái mới
            buffer.fill(if self.current_state { self.on } else { self.off });
            
            // Tính thời điểm chuyển trạng thái tiếp theo
            // Thêm nửa chu kỳ vào thời điểm hiện tại
            let half_cycle = self.cycle_time_us / 2;
            self.next_transition_time += half_cycle;
            
            // Xử lý trường hợp đã bỏ lỡ nhiều transitions (CPU quá bận)
            // Đảm bảo next_transition_time luôn ở tương lai
            while self.next_transition_time <= now_us {
                self.next_transition_time += half_cycle;
                self.current_state = !self.current_state;
            }
            
            // Trả về thời điểm cần update tiếp theo
            return Some(self.next_transition_time);
        }
        
        // Chưa đến lúc chuyển, không cần làm gì cả
        // Nhưng vẫn trả về thời điểm cần update tiếp theo
        Some(self.next_transition_time)
    }

    fn set_color(&mut self, color: RGB8) {
        if self.on != color {
            self.on = color;
            // Nếu đang ở trạng thái ON, cần update ngay để thấy màu mới
            // Set next_transition_time = 0 để trigger update ngay lập tức
            if self.current_state {
                self.next_transition_time = 0;
            }
        }
    }

    fn set_speed(&mut self, speed: u8) {
        let new_speed = speed.clamp(1, 255);
        if self.speed != new_speed {
            self.speed = new_speed;
            let new_cycle_time = cycle_time_us(new_speed);
            
            if new_cycle_time != self.cycle_time_us {
                self.cycle_time_us = new_cycle_time;
                // Recalculate next transition với cycle time mới
                // Để đảm bảo chuyển đổi mượt mà
                self.next_transition_time = 0; // Trigger recalculation
            }
        }
    }

    fn get_color(&self) -> Option<RGB8> {
        Some(self.on)
    }

    fn get_speed(&self) -> Option<u8> {
        Some(self.speed)
    }

    fn name(&self) -> &str {
        "Blink"
    }
}



pub struct VuMeterEffect {
    // Cấu hình
    num_leds: usize,
    speed: u8, // Tốc độ phản ứng với âm thanh (attack/release)
    
    // State
    current_level: f32,         // Mức hiện tại (0.0 - 1.0)
    peak_level: f32,            // Đỉnh gần đây nhất
    peak_hold_time: u64,        // Thời gian giữ peak
    peak_hold_duration_us: u64, // Thời gian giữ peak (microseconds)
    last_update_time: u64,
    
    // FPS control
    target_frame_time_us: u64,
}

impl VuMeterEffect {
    pub fn new(num_leds: usize, speed: u8) -> Self {
        // Target 60 FPS cho sound reactive effect (cần responsive cao)
        let target_fps = 60;
        let target_frame_time_us = 1_000_000 / target_fps;
        
        Self {
            num_leds,
            speed: speed.clamp(1, 255),
            current_level: 0.0,
            peak_level: 0.0,
            peak_hold_time: 0,
            peak_hold_duration_us: 500_000, // Giữ peak trong 0.5 giây
            last_update_time: 0,
            target_frame_time_us,
        }
    }
    
    /// Tính hệ số smoothing dựa trên speed
    fn smoothing_factor(&self) -> (f32, f32) {
        // Speed càng cao, phản ứng càng nhanh (smoothing thấp)
        // Attack (tăng) nhanh hơn release (giảm) để có cảm giác punchy
        let base_attack = 0.3;   // Attack nhanh
        let base_release = 0.1;  // Release chậm hơn
        
        let speed_factor = self.speed as f32 / 255.0;
        
        let attack = base_attack + speed_factor * 0.4;   // 0.3 - 0.7
        let release = base_release + speed_factor * 0.2; // 0.1 - 0.3
        
        (attack, release)
    }
    
    /// Chuyển đổi level (0-1) sang màu gradient
    /// Xanh lá (thấp) -> Vàng (trung bình) -> Đỏ (cao)
    fn level_to_color(&self, normalized_position: f32) -> RGB8 {
        if normalized_position < 0.33 {
            // Vùng xanh lá (thấp)
            let t = normalized_position / 0.33;
            RGB8 {
                r: 0,
                g: (255.0 * t) as u8,
                b: 0,
            }
        } else if normalized_position < 0.66 {
            // Chuyển từ xanh lá sang vàng
            let t = (normalized_position - 0.33) / 0.33;
            RGB8 {
                r: (255.0 * t) as u8,
                g: 255,
                b: 0,
            }
        } else {
            // Chuyển từ vàng sang đỏ
            let t = (normalized_position - 0.66) / 0.34;
            RGB8 {
                r: 255,
                g: (255.0 * (1.0 - t)) as u8,
                b: 0,
            }
        }
    }
}

impl Effect for VuMeterEffect {
    fn update(&mut self, _now_us: u64, _buffer: &mut [RGB8]) -> Option<u64> {
        None
    }
    fn update_audio(&mut self, now_us: u64, audio: &AudioData, buffer: &mut [RGB8]) -> Option<u64> {
        // Khởi tạo lần đầu
        if self.last_update_time == 0 {
            self.last_update_time = now_us;
        }
        
        // Lấy biên độ âm thanh từ audio.volume
        // AudioData của bạn đã có volume (0.0 - 1.0) với noise gate và smoothing
        let target_level = audio.volume.clamp(0.0, 1.0);
        
        // Smooth transition sử dụng exponential moving average
        let (attack, release) = self.smoothing_factor();
        let smoothing = if target_level > self.current_level {
            attack  // Attack: tăng nhanh
        } else {
            release // Release: giảm chậm
        };
        
        self.current_level = self.current_level * (1.0 - smoothing) + target_level * smoothing;
        
        // Xử lý peak hold
        if self.current_level > self.peak_level {
            // Level mới cao hơn peak, cập nhật peak
            self.peak_level = self.current_level;
            self.peak_hold_time = now_us;
        } else if now_us - self.peak_hold_time > self.peak_hold_duration_us {
            // Đã giữ peak đủ lâu, để peak rơi xuống
            self.peak_level = self.current_level;
        }
        
        // Tính số LED cần sáng dựa trên level
        let num_lit = (self.current_level * self.num_leds as f32) as usize;
        let peak_position = (self.peak_level * self.num_leds as f32) as usize;
        
        // Render VU meter vào buffer
        // Bottom-up: LED ở index 0 là dưới cùng, index max là trên cùng
        for (i, pixel) in buffer.iter_mut().enumerate() {
            if i < num_lit {
                // LED này nằm trong vùng sáng, tính màu gradient
                let position = i as f32 / self.num_leds as f32;
                *pixel = self.level_to_color(position);
            } else if i == peak_position {
                // LED này là peak, sáng màu trắng/cyan để nổi bật
                *pixel = RGB8 { r: 0, g: 255, b: 255 };
            } else {
                // LED này tắt
                *pixel = RGB8 { r: 0, g: 0, b: 0 };
            }
        }
        
        self.last_update_time = now_us;
        
        // Trả về thời điểm update tiếp theo (60 FPS cho sound reactive)
        Some(now_us + self.target_frame_time_us)
    }

    fn is_audio_reactive(&self) -> bool {
        true // Đây là effect phản ứng với âm thanh
    }

    fn set_speed(&mut self, speed: u8) {
        self.speed = speed.clamp(1, 255);
    }

    fn get_speed(&self) -> Option<u8> {
        Some(self.speed)
    }

    fn name(&self) -> &str {
        "VU Meter"
    }
}