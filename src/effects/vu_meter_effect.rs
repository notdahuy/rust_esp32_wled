use smart_leds::RGB8;
use crate::audio::AudioData;
use super::Effect;

pub struct VuMeterEffect {
    num_leds: usize,
    speed: u8,
    
    // Smooth level
    current_level: f32,
    
    // Peak physics
    peak_level: f32,
    peak_hold_time: u64,
    last_update_time: u64,
    
    // Config constants (Pre-calculated for performance)
    target_frame_time_us: u64,
    peak_hold_duration_us: u64,
    peak_gravity: f32, // Tốc độ rơi của peak
}

impl VuMeterEffect {
    pub fn new(num_leds: usize, speed: u8) -> Self {
        Self {
            num_leds,
            speed: speed.clamp(1, 255),
            current_level: 0.0,
            peak_level: 0.0,
            peak_hold_time: 0,
            last_update_time: 0,
            // Cấu hình cứng
            target_frame_time_us: 1_000_000 / 60, // 60 FPS
            peak_hold_duration_us: 450_000,       // Giữ đỉnh 450ms
            peak_gravity: 0.005,                  // Tốc độ rơi mặc định
        }
    }

    // ✅ OPTIMIZED: Màu sắc mượt hơn (Gradient nhiệt độ: Xanh -> Vàng -> Đỏ)
    fn level_to_color(&self, pos: f32) -> RGB8 {
        // Pos đi từ 0.0 đến 1.0
        if pos < 0.5 {
            // 0.0 -> 0.5: Green to Yellow
            // Green (0, 255, 0) -> Yellow (255, 255, 0)
            let t = pos * 2.0; 
            RGB8 {
                r: (255.0 * t) as u8,
                g: 255, // Luôn sáng xanh tối đa ở nửa đầu
                b: 0,
            }
        } else {
            // 0.5 -> 1.0: Yellow to Red
            // Yellow (255, 255, 0) -> Red (255, 0, 0)
            let t = (pos - 0.5) * 2.0;
            RGB8 {
                r: 255, // Luôn sáng đỏ tối đa ở nửa sau
                g: (255.0 * (1.0 - t)) as u8,
                b: 0,
            }
        }
    }
}

impl Effect for VuMeterEffect {
    fn update(&mut self, _now_us: u64, _buffer: &mut [RGB8]) -> Option<u64> { None }

    fn update_audio(&mut self, now_us: u64, audio: &AudioData, buffer: &mut [RGB8]) -> Option<u64> {
        if self.last_update_time == 0 {
            self.last_update_time = now_us;
            return Some(now_us + self.target_frame_time_us);
        }

        // 1. Tính toán delta time để animation mượt không phụ thuộc FPS
        // Mặc định tính theo frame 60fps (~16ms)
        let _dt = (now_us - self.last_update_time) as f32 / 1_000_000.0;
        self.last_update_time = now_us;

        // 2. Xử lý Main Bar (Smoothing)
        let target_level = audio.volume.clamp(0.0, 1.0);
        
        // Tốc độ phản hồi dựa trên biến speed
        // Speed cao -> Attack nhanh (0.6), Release nhanh (0.2)
        let s_norm = self.speed as f32 / 255.0;
        let attack = 0.2 + s_norm * 0.6;  
        let release = 0.05 + s_norm * 0.15; 

        let smoothing = if target_level > self.current_level { attack } else { release };
        self.current_level = self.current_level + (target_level - self.current_level) * smoothing;

        // 3. Xử lý Peak Physics (Trọng lực)
        if self.current_level >= self.peak_level {
            // Đẩy peak lên
            self.peak_level = self.current_level;
            self.peak_hold_time = now_us;
        } else {
            // Peak đang treo hoặc rơi
            let time_held = now_us.saturating_sub(self.peak_hold_time);
            
            if time_held > self.peak_hold_duration_us {
                // Hết thời gian treo -> Rơi tự do (Gravity)
                // Rơi nhanh dần nếu muốn, ở đây rơi tuyến tính cho đơn giản
                self.peak_level -= self.peak_gravity * (1.0 + s_norm); 
                
                // Không rơi thấp hơn mức hiện tại
                if self.peak_level < self.current_level {
                    self.peak_level = self.current_level;
                }
            }
        }

        // 4. Render
        // Reset buffer (quan trọng)
        for p in buffer.iter_mut() { *p = RGB8::default(); }

        let len = self.num_leds as f32;
        let bar_len = (self.current_level * len).round() as usize;
        let peak_idx = (self.peak_level * len).floor() as usize;

        // Vẽ thanh VU
        for i in 0..bar_len.min(self.num_leds) {
            let pos = i as f32 / len;
            buffer[i] = self.level_to_color(pos);
        }

        // Vẽ Peak (Vẽ sau cùng để đè lên thanh VU)
        if peak_idx < self.num_leds {
            // Nếu peak trùng vị trí với bar, làm sáng màu lên (White/Cyan mix)
            if peak_idx < bar_len {
                buffer[peak_idx] = RGB8 { r: 255, g: 255, b: 255 }; // Đỉnh màu trắng khi đang đẩy
            } else {
                buffer[peak_idx] = RGB8 { r: 0, g: 255, b: 255 }; // Đỉnh màu Cyan khi đang rơi
            }
        }

        Some(now_us + self.target_frame_time_us)
    }
    
    // ... (Giữ nguyên các hàm trait khác)
    fn is_audio_reactive(&self) -> bool { true }
    fn set_speed(&mut self, speed: u8) { self.speed = speed.clamp(1, 255); }
    fn get_speed(&self) -> Option<u8> { Some(self.speed) }
    fn name(&self) -> &str { "VU Meter" }
}