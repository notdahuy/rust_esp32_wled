use smart_leds::RGB8;
use crate::audio::AudioData;
use super::Effect;

pub struct RadialPulseEffect {
    num_leds: usize,
    speed: u8,
    stored_color: RGB8, 
    pulses: heapless::Vec<Pulse, 20>,
    last_update: u64,
    target_frame_time_us: u64,
    smoothed_bass: f32,
    smoothed_mid: f32,
    smoothed_treble: f32,
    smoothed_intensity: f32,
    
    bass_history: [f32; 6],   
    mid_history: [f32; 6],
    treble_history: [f32; 6],
    history_idx: usize,
}

#[derive(Clone, Copy)]
struct Pulse {
    radius: f32,
    max_radius: f32,  // Bán kính tối đa sóng có thể lan tới
    brightness: f32,
    speed: f32,
    color: RGB8,
    age: f32,
    width: f32,       // Độ dày của sóng
}

#[derive(Clone, Copy)]
enum FrequencyType {
    Bass,
    Mid,
    Treble,
}

impl RadialPulseEffect {
    pub fn new(num_leds: usize, color: RGB8, speed: u8) -> Self {
        Self {
            num_leds,
            speed: speed.clamp(1, 255),
            stored_color: color,
            pulses: heapless::Vec::new(),
            last_update: 0,
            target_frame_time_us: 1_000_000 / 60,
            smoothed_bass: 0.0,
            smoothed_mid: 0.0,
            smoothed_treble: 0.0,
            smoothed_intensity: 0.0,
            bass_history: [0.0; 6],
            mid_history: [0.0; 6],
            treble_history: [0.0; 6],
            history_idx: 0,
        }
    }
    
    fn smooth_value(current: f32, target: f32, factor: f32) -> f32 {
        current + (target - current) * factor.clamp(0.0, 1.0)
    }
    
    fn detect_peak(&self, current: f32, history: &[f32]) -> bool {
        if history.is_empty() { return false; }
        
        let avg = history.iter().sum::<f32>() / history.len() as f32;
        let max = history.iter().copied().fold(0.0f32, f32::max);
        
        // Logic peak detection nhạy hơn
        (current > avg * 1.3 && current > 0.15) || (current > 0.8 && current > max * 0.9)
    }
    
    fn get_pulse_color(&self, freq_type: FrequencyType, intensity: f32) -> RGB8 {
        let val = intensity.clamp(0.0, 1.0);
        
        match freq_type {
            FrequencyType::Bass => {
                Self::hsv_to_rgb(0.95 - (val * 0.15), 1.0, 1.0) 
            },
            FrequencyType::Mid => {
                Self::hsv_to_rgb(0.05 + (val * 0.25), 1.0, 1.0)
            },
            FrequencyType::Treble => {
                let sat = (1.0 - val * 0.5).clamp(0.0, 1.0);
                Self::hsv_to_rgb(0.5 + (val * 0.15), sat, 1.0)
            }
        }
    }
    
    fn hsv_to_rgb(h: f32, s: f32, v: f32) -> RGB8 {
        let h = h - h.floor();
        let h = h * 6.0;
        
        let i = h as u8;
        let f = h - i as f32;
        let p = v * (1.0 - s);
        let q = v * (1.0 - f * s);
        let t = v * (1.0 - (1.0 - f) * s);
        
        let (r, g, b) = match i {
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

    fn spawn_pulse(&mut self, intensity: f32, freq: FrequencyType) {
        if self.pulses.is_full() { return; }

        let color = self.get_pulse_color(freq, intensity);
        let (base_speed, base_width) = match freq {
            FrequencyType::Bass => (12.0, 6.0),
            FrequencyType::Mid => (20.0, 4.0),
            FrequencyType::Treble => (35.0, 2.5),
        };

        // Boost tốc độ theo nhạc
        let speed = base_speed + (intensity * intensity * 15.0);
        let width = base_width + (intensity * 3.0);

        let _ = self.pulses.push(Pulse {
            radius: 0.0,
            max_radius: self.num_leds as f32 * 0.6, // Lan ra 60% dải led rồi tắt
            brightness: 0.6 + intensity * 0.4,
            speed,
            color,
            age: 0.0,
            width,
        });
    }
}

impl Effect for RadialPulseEffect {
    fn update(&mut self, _now_us: u64, _buffer: &mut [RGB8]) -> Option<u64> {
        None
    }
    
    fn update_audio(&mut self, now_us: u64, audio: &AudioData, buffer: &mut [RGB8]) -> Option<u64> {
        if self.last_update == 0 {
            self.last_update = now_us;
            return Some(now_us + self.target_frame_time_us);
        }
        
        let delta_us = now_us.saturating_sub(self.last_update);
        self.last_update = now_us;
        let delta_s = (delta_us as f32 / 1_000_000.0).clamp(0.001, 0.1);
        
        // Smooth audio parameters
        let smoothing = 0.2;
        self.smoothed_bass = Self::smooth_value(self.smoothed_bass, audio.bass, smoothing);
        self.smoothed_mid = Self::smooth_value(self.smoothed_mid, audio.mid, smoothing);
        self.smoothed_treble = Self::smooth_value(self.smoothed_treble, audio.treble, smoothing);
        self.smoothed_intensity = Self::smooth_value(self.smoothed_intensity, audio.volume, 0.1);
        
        // Update history
        self.bass_history[self.history_idx] = self.smoothed_bass;
        self.mid_history[self.history_idx] = self.smoothed_mid;
        self.treble_history[self.history_idx] = self.smoothed_treble;
        self.history_idx = (self.history_idx + 1) % self.bass_history.len();
        
        // Bass Pulse
        if self.detect_peak(self.smoothed_bass, &self.bass_history) {
            self.spawn_pulse(self.smoothed_bass, FrequencyType::Bass);
        }
        
        // Mid Pulse (Spawn nếu mid đủ lớn và không full buffer)
        if self.smoothed_mid > 0.25 && !self.pulses.is_full() {
            // Random chance để mid không spawn quá dày đặc
            if (now_us % 3) == 0 { 
                self.spawn_pulse(self.smoothed_mid, FrequencyType::Mid);
            }
        }
        
        // Treble Pulse 
        if self.smoothed_treble > 0.3 {
             self.spawn_pulse(self.smoothed_treble, FrequencyType::Treble);
        }
        
        // Tốc độ nhân với speed 
        let speed_mult = self.speed as f32 / 128.0; 

        self.pulses.retain_mut(|pulse| {
            pulse.age += delta_s;
            pulse.radius += pulse.speed * speed_mult * delta_s;
            
            // Fade out theo khoảng cách và thời gian
            pulse.brightness *= 0.95; 
            
            pulse.brightness > 0.01 && pulse.radius < (self.num_leds as f32 * 1.5)
        });
        
        // --- RENDER ---
        // 1. Fade toàn bộ background (tạo trail)
        for pixel in buffer.iter_mut() {
            pixel.r = (pixel.r as f32 * 0.85) as u8; // Fade nhanh hơn một chút để đỡ rối
            pixel.g = (pixel.g as f32 * 0.85) as u8;
            pixel.b = (pixel.b as f32 * 0.85) as u8;
        }
        
        let center = self.num_leds as f32 / 2.0;
        
        for pulse in &self.pulses {
            // Tính vùng ảnh hưởng của pulse để không loop hết cả dải LED
            let start_idx = ((center - pulse.radius - pulse.width).floor() as isize).max(0) as usize;
            let end_idx = ((center + pulse.radius + pulse.width).ceil() as usize).min(self.num_leds);

            // Pulse là 2 vòng tròn lan ra từ tâm về 2 phía
            let left_center = center - pulse.radius;
            let right_center = center + pulse.radius;

            for i in start_idx..end_idx {
                let pos = i as f32;
                
                // Tính khoảng cách tới sóng bên trái và bên phải
                let dist_left = (pos - left_center).abs();
                let dist_right = (pos - right_center).abs();
                
                // Chọn sóng gần nhất
                let dist = dist_left.min(dist_right);
                
                if dist < pulse.width {
                    // Hàm mũ chuông đơn giản hóa (Gaussian approx)
                    // 1.0 ở tâm sóng, 0.0 ở biên
                    let intensity = (1.0 - dist / pulse.width).powf(2.0); // powf(2) nhanh hơn powf(1.5)
                    let b = (pulse.brightness * intensity).clamp(0.0, 1.0);
                    
                    if b > 0.01 {
                        buffer[i].r = buffer[i].r.saturating_add((pulse.color.r as f32 * b) as u8);
                        buffer[i].g = buffer[i].g.saturating_add((pulse.color.g as f32 * b) as u8);
                        buffer[i].b = buffer[i].b.saturating_add((pulse.color.b as f32 * b) as u8);
                    }
                }
            }
        }
        
        // Center Glow (Tâm luôn sáng rực rỡ theo nhịp)
        let center_idx = self.num_leds / 2;
        if center_idx < buffer.len() {
            // Mix màu center dựa trên bass/mid/treble hiện tại
            let glow_r = (self.smoothed_bass * 255.0) as u8;
            let glow_g = (self.smoothed_mid * 200.0) as u8;
            let glow_b = (self.smoothed_treble * 255.0) as u8;
            
            // Lan tỏa glow ra 3 LED giữa
            for offset in 0..=1 {
                 if center_idx >= offset {
                    let idx = center_idx - offset;
                    buffer[idx].r = buffer[idx].r.saturating_add(glow_r >> offset);
                    buffer[idx].g = buffer[idx].g.saturating_add(glow_g >> offset);
                    buffer[idx].b = buffer[idx].b.saturating_add(glow_b >> offset);
                 }
                 if center_idx + offset < self.num_leds && offset > 0 {
                    let idx = center_idx + offset;
                    buffer[idx].r = buffer[idx].r.saturating_add(glow_r >> offset);
                    buffer[idx].g = buffer[idx].g.saturating_add(glow_g >> offset);
                    buffer[idx].b = buffer[idx].b.saturating_add(glow_b >> offset);
                 }
            }
        }
        
        Some(now_us + self.target_frame_time_us)
    }
    
    fn is_audio_reactive(&self) -> bool { true }
    
    fn set_color(&mut self, color: RGB8) { 
        self.stored_color = color;
    }
    
    fn get_color(&self) -> Option<RGB8> { Some(self.stored_color) }
    fn set_speed(&mut self, speed: u8) { self.speed = speed.clamp(1, 255); }
    fn get_speed(&self) -> Option<u8> { Some(self.speed) }
    fn name(&self) -> &str { "Radial Pulse" }
}