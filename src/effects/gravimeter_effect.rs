use smart_leds::RGB8;
use crate::audio::AudioData;
use super::Effect;

pub struct GravimeterEffect {
    num_leds: usize,
    speed: u8,
    // Vẫn giữ biến color để thỏa mãn trait Effect, nhưng không dùng để vẽ hạt
    stored_color: RGB8, 
    
    particles: heapless::Vec<Particle, 16>, 
    last_spawn_time: u64,
    spawn_interval_us: u64,
    
    last_update: u64,
    target_frame_time_us: u64,
    
    // History & Smooth logic
    bass_history: [f32; 6],
    mid_history: [f32; 6],
    treble_history: [f32; 6],
    history_idx: usize,
    
    smoothed_bass: f32,
    smoothed_mid: f32,
    smoothed_treble: f32,
}

#[derive(Clone, Copy)]
struct Particle {
    position: f32,
    velocity: f32,
    brightness: f32,
    color: RGB8,
    frequency_type: FrequencyType,
}

#[derive(Clone, Copy)]
enum FrequencyType {
    Bass,
    Mid,
    Treble,
}

impl GravimeterEffect {
    pub fn new(num_leds: usize, color: RGB8, speed: u8) -> Self {
        Self {
            num_leds,
            speed: speed.clamp(1, 255),
            stored_color: color,
            particles: heapless::Vec::new(),
            last_spawn_time: 0,
            spawn_interval_us: 60_000, 
            last_update: 0,
            target_frame_time_us: 1_000_000 / 60,
            bass_history: [0.0; 6],
            mid_history: [0.0; 6],
            treble_history: [0.0; 6],
            history_idx: 0,
            smoothed_bass: 0.0,
            smoothed_mid: 0.0,
            smoothed_treble: 0.0,
        }
    }
    
    fn speed_to_gravity(&self) -> f32 {
        0.08 + (self.speed as f32 / 255.0) * 0.35 
    }
    
    fn update_physics(&mut self, delta_time_s: f32) {
        let gravity = self.speed_to_gravity();
        let delta_scaled = (delta_time_s * 60.0).min(2.0);
        
        self.particles.retain_mut(|p| {
            if delta_scaled > 0.0 && delta_scaled.is_finite() {
                p.velocity += gravity * delta_scaled;
                p.position += p.velocity * delta_scaled;
            }
            
            p.brightness *= 0.988_f32; 
            
            p.position.is_finite() 
                && p.position >= -2.0 
                && p.position < (self.num_leds as f32 + 2.0)
                && p.brightness > 0.008 
        });
    }
    
    fn spawn_particle(&mut self, intensity: f32, freq_type: FrequencyType) {
        if self.particles.is_full() {
            return;
        }
        
        let intensity_clamped = intensity.clamp(0.0, 1.0);
        
        // ✅ FORCED SPECTRUM MODE:
        // Màu sắc được tính toán dựa trên độ lớn âm thanh (intensity)
        // Nhỏ: Đỏ -> Cam -> Vàng -> Lục -> Lam -> Tím: Lớn
        let particle_color = Self::hsv_to_rgb(intensity_clamped * 0.85, 0.95, 1.0);
        
        let base_velocity = match freq_type {
            FrequencyType::Bass => 0.25,
            FrequencyType::Mid => 0.55,
            FrequencyType::Treble => 1.0,
        };
        
        let velocity_boost = intensity_clamped.powf(0.8) * 2.8; 
        
        let _ = self.particles.push(Particle {
            position: 0.0,
            velocity: base_velocity + velocity_boost,
            brightness: 0.55 + intensity_clamped * 0.45,
            color: particle_color,
            frequency_type: freq_type,
        });
    }
    
    fn hsv_to_rgb(h: f32, s: f32, v: f32) -> RGB8 {
        let h = (h * 6.0).clamp(0.0, 6.0);
        let i = h as u8 % 6;
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
    
    fn detect_peak(&self, current: f32, history: &[f32]) -> bool {
        if history.is_empty() { return false; }
        
        let avg = history.iter().sum::<f32>() / history.len() as f32;
        let max = history.iter().copied().fold(0.0f32, f32::max);
        
        let threshold = avg * 1.3;
        let min_level = 0.12;
        
        (current > threshold && current > min_level) || 
        (current > max * 0.9 && current > 0.2)
    }
    
    fn smooth_value(current: f32, target: f32) -> f32 {
        current * 0.7 + target * 0.3
    }
}

impl Effect for GravimeterEffect {
    fn update(&mut self, _now_us: u64, _buffer: &mut [RGB8]) -> Option<u64> {
        None
    }
    
    fn update_audio(&mut self, now_us: u64, audio: &AudioData, buffer: &mut [RGB8]) -> Option<u64> {
        if self.last_update == 0 {
            self.last_update = now_us;
            return Some(now_us.saturating_add(self.target_frame_time_us));
        }
        
        let delta_time_us = now_us.saturating_sub(self.last_update);
        self.last_update = now_us;
        
        let delta_time_s = ((delta_time_us as f32) / 1_000_000.0).clamp(0.001, 0.1);
        
        self.smoothed_bass = Self::smooth_value(self.smoothed_bass, audio.bass);
        self.smoothed_mid = Self::smooth_value(self.smoothed_mid, audio.mid);
        self.smoothed_treble = Self::smooth_value(self.smoothed_treble, audio.treble);
        
        self.bass_history[self.history_idx] = self.smoothed_bass;
        self.mid_history[self.history_idx] = self.smoothed_mid;
        self.treble_history[self.history_idx] = self.smoothed_treble;
        self.history_idx = (self.history_idx + 1) % self.bass_history.len();
        
        let time_since_spawn = now_us.saturating_sub(self.last_spawn_time);
        let should_check_spawn = time_since_spawn > self.spawn_interval_us;
        
        if self.detect_peak(self.smoothed_bass, &self.bass_history) {
            self.spawn_particle(self.smoothed_bass, FrequencyType::Bass);
            if should_check_spawn {
                self.last_spawn_time = now_us;
            }
        }
        
        if should_check_spawn {
            if self.smoothed_mid > 0.20 {
                self.spawn_particle(self.smoothed_mid, FrequencyType::Mid);
            }
        }
        
        if self.smoothed_treble > 0.25 {
            if self.detect_peak(self.smoothed_treble, &self.treble_history) || 
               self.smoothed_treble > 0.4 {
                self.spawn_particle(self.smoothed_treble, FrequencyType::Treble);
            }
        }
        
        self.update_physics(delta_time_s);
        
        let fade_factor = if self.particles.len() > 12 { 0.88 } else { 0.92 };
        
        for pixel in buffer.iter_mut() {
            pixel.r = (pixel.r as f32 * fade_factor) as u8;
            pixel.g = (pixel.g as f32 * fade_factor) as u8;
            pixel.b = (pixel.b as f32 * fade_factor) as u8;
        }
        
        for particle in &self.particles {
            if !particle.position.is_finite() { continue; }
            
            let pos = particle.position;
            let pos_int = pos as isize;
            
            let frac = pos - pos_int as f32;
            let brightness = particle.brightness.clamp(0.0, 1.0);
            
            if pos_int >= 0 && (pos_int as usize) < buffer.len() {
                let idx = pos_int as usize;
                let main_brightness = brightness * (1.0 - frac * 0.5);
                
                let r = (particle.color.r as f32 * main_brightness) as u8;
                let g = (particle.color.g as f32 * main_brightness) as u8;
                let b = (particle.color.b as f32 * main_brightness) as u8;
                
                buffer[idx].r = buffer[idx].r.saturating_add(r);
                buffer[idx].g = buffer[idx].g.saturating_add(g);
                buffer[idx].b = buffer[idx].b.saturating_add(b);
            }
            
            if pos_int + 1 >= 0 && ((pos_int + 1) as usize) < buffer.len() && frac > 0.3 {
                let idx = (pos_int + 1) as usize;
                let sub_brightness = brightness * frac * 0.6;
                
                let r = (particle.color.r as f32 * sub_brightness) as u8;
                let g = (particle.color.g as f32 * sub_brightness) as u8;
                let b = (particle.color.b as f32 * sub_brightness) as u8;
                
                buffer[idx].r = buffer[idx].r.saturating_add(r);
                buffer[idx].g = buffer[idx].g.saturating_add(g);
                buffer[idx].b = buffer[idx].b.saturating_add(b);
            }
        }
        
        Some(now_us.saturating_add(self.target_frame_time_us))
    }
    
    fn is_audio_reactive(&self) -> bool { true }
    
    // Set color chỉ cập nhật biến lưu trữ chứ KHÔNG đổi mode nữa
    fn set_color(&mut self, color: RGB8) { 
        self.stored_color = color;
    }
    
    fn get_color(&self) -> Option<RGB8> { Some(self.stored_color) }
    fn set_speed(&mut self, speed: u8) { self.speed = speed.clamp(1, 255); }
    fn get_speed(&self) -> Option<u8> { Some(self.speed) }
    fn name(&self) -> &str { "Gravimeter" }
}