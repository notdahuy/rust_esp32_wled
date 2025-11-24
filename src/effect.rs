use smart_leds::RGB8;
use palette::{FromColor, Hsv, RgbHue, Srgb};
use crate::audio::AudioData;
use std::cell::RefCell;

#[derive(Debug, Clone, PartialEq)]
pub enum EffectType {
    Static,
    Rainbow,
    Breathe,
    ColorWipe,
    Comet,
    Scanner,
    TheaterChase,
    Bounce,  
    AudioVolumeBar   
}

/// Trait chung cho tất cả các hiệu ứng
pub trait Effect {

    fn update(&mut self, delta_us: u64) -> bool;
    

    fn render(&self, buffer: &mut [RGB8]);

    fn render_audio(&mut self, buffer: &mut [RGB8], audio: &AudioData, now_us: u64) {
        // Default: chỉ gọi render bình thường
        self.render(buffer);
    }
    

    fn set_color(&mut self, color: RGB8) -> bool {
        false 
    }
    

    fn set_speed(&mut self, speed: u8) -> bool {
        false 
    }
    
    fn name(&self) -> &'static str;
    fn is_audio_reactive(&self) -> bool { false }
}


pub struct StaticEffect {
    color: RGB8,
}

impl StaticEffect {
    pub fn new(color: RGB8) -> Self { 
        Self { color } 
    }
}

impl Effect for StaticEffect {
    fn name(&self) -> &'static str { "Static" }

    fn update(&mut self, _delta_us: u64,) -> bool {
        false  // Static không bao giờ tự update
    }

    fn render(&self, buffer: &mut [RGB8]) {
        buffer.fill(self.color);
    }

    fn set_color(&mut self, color: RGB8) -> bool {
        if self.color != color {
            self.color = color;
            return true;  // Cần render ngay
        }
        false
    }
}


pub struct RainbowEffect {
    phase16: u16,
    speed: u8,
    phase_spacing: u16,
    lut: Vec<RGB8>,
}

impl RainbowEffect {
    pub fn new(num_leds: usize, speed: u8) -> Self {
        let mut lut = Vec::with_capacity(256);
        
        for i in 0..=255 {
            let hue = (i as f32 * 360.0) / 256.0;
            let color = Hsv::new(RgbHue::from_degrees(hue), 1.0, 1.0);
            let srgb: Srgb = Srgb::from_color(color);

            lut.push(RGB8 {
                r: (srgb.red * 255.0).round() as u8,
                g: (srgb.green * 255.0).round() as u8,
                b: (srgb.blue * 255.0).round() as u8,
            });
        }

        Self {
            phase16: 0,
            speed: speed.clamp(1, 255),
            phase_spacing: (65536_u32 / num_leds.max(1) as u32) as u16,
            lut,
        }
    }
}

impl Effect for RainbowEffect {
    fn name(&self) -> &'static str { "Rainbow" }

    fn update(&mut self, delta_us: u64) -> bool {
        // Tính phase increment với overflow protection
        let increment = ((self.speed as u64).saturating_mul(delta_us)) / 10000;
        
        // Chỉ update nếu có thay đổi
        if increment > 0 {
            self.phase16 = self.phase16.wrapping_add(increment as u16);
            return true;  // Phase thay đổi → cần render
        }
        
        false  // Không thay đổi (delta_us quá nhỏ)
    }

    fn render(&self, buffer: &mut [RGB8]) {
        for (i, pixel) in buffer.iter_mut().enumerate() {
            let pixel_phase = self.phase16
                .wrapping_add((i as u16).wrapping_mul(self.phase_spacing));
            let hue_index = (pixel_phase >> 8) as u8;
            *pixel = self.lut[hue_index as usize];
        }
    }
    
    fn set_speed(&mut self, speed: u8) -> bool {
        self.speed = speed.clamp(1, 255);
        false  // Speed không cần render ngay
    }
}


pub struct BreatheEffect {
    base_color: RGB8,
    current_color: RGB8,
    speed: u8,
    phase16: u16,
    lut: Vec<u8>,
}

impl BreatheEffect {
    pub fn new(color: RGB8, speed: u8) -> Self {
        let mut lut = Vec::with_capacity(256);
        
        // Tạo LUT sóng sin
        for i in 0..=255 {
            let rad = (i as f32 / 255.0) * std::f32::consts::PI;
            let sin_val = rad.sin();
            let brightness = (sin_val * 255.0).round() as u8;
            lut.push(brightness);
        }

        Self {
            base_color: color,
            current_color: RGB8::default(),
            speed: speed.clamp(1, 255),
            phase16: 0,
            lut,
        }
    }
}

impl Effect for BreatheEffect {
    fn name(&self) -> &'static str { "Breathe" }

    fn update(&mut self, delta_us: u64) -> bool {
        let increment = ((self.speed as u64).saturating_mul(delta_us)) / 10000;
        
        if increment > 0 {
            self.phase16 = self.phase16.wrapping_add(increment as u16);
            
            // Tính màu mới
            let brightness_index = (self.phase16 >> 8) as u8;
            let brightness_scale = self.lut[brightness_index as usize] as u16;

            let new_color = RGB8 {
                r: ((self.base_color.r as u16 * brightness_scale) >> 8) as u8,
                g: ((self.base_color.g as u16 * brightness_scale) >> 8) as u8,
                b: ((self.base_color.b as u16 * brightness_scale) >> 8) as u8,
            };
            
            // Chỉ render nếu màu thực sự thay đổi
            if self.current_color != new_color {
                self.current_color = new_color;
                return true;
            }
        }
        
        false
    }

    fn render(&self, buffer: &mut [RGB8]) {
        buffer.fill(self.current_color);
    }

    fn set_color(&mut self, color: RGB8) -> bool {
        if self.base_color != color {
            self.base_color = color;
            return true;  // Màu base đổi → cần render ngay
        }
        false
    }

    fn set_speed(&mut self, speed: u8) -> bool {
        self.speed = speed.clamp(1, 255);
        false
    }
}


pub struct ColorWipeEffect {
    color: RGB8,
    num_leds: usize,
    current_pixel: usize,
    time_accumulator: u64,
    pixel_interval_us: u64,
}

impl ColorWipeEffect {
    pub fn new(color: RGB8, speed: u8, num_leds: usize) -> Self {
        Self {
            color,
            num_leds,
            current_pixel: 0,
            time_accumulator: 0,
            pixel_interval_us: Self::map_speed_to_interval(speed),
        }
    }

    fn map_speed_to_interval(speed: u8) -> u64 {
        let inverted_speed = 256 - speed.max(1) as u64;
        let interval_ms = (inverted_speed * 148) / 254 + 2;
        interval_ms * 1000
    }
}

impl Effect for ColorWipeEffect {
    fn name(&self) -> &'static str { "Color Wipe" }

    fn update(&mut self, delta_us: u64) -> bool {
        self.time_accumulator += delta_us;

        if self.time_accumulator >= self.pixel_interval_us {
            self.time_accumulator -= self.pixel_interval_us;  // Giữ phần dư
            
            self.current_pixel += 1;

            if self.current_pixel > self.num_leds {
                self.current_pixel = 0;
            }
            
            return true;  // Pixel mới → cần render
        }
        
        false  // Chưa đến lúc update
    }

    fn render(&self, buffer: &mut [RGB8]) {
        if self.current_pixel == 0 {
            // Reset: tắt tất cả
            buffer.fill(RGB8::default());
        } else {
            // Bật từ pixel 0 đến current_pixel - 1
            let lit_count = self.current_pixel.min(buffer.len());
            
            // Fill phần sáng
            buffer[..lit_count].fill(self.color);
            
            // Fill phần tối (nếu có)
            if lit_count < buffer.len() {
                buffer[lit_count..].fill(RGB8::default());
            }
        }
    }

    fn set_color(&mut self, color: RGB8) -> bool {
        if self.color != color {
            self.color = color;
            return true;  // Màu đổi → render lại với màu mới
        }
        false
    }

    fn set_speed(&mut self, speed: u8) -> bool {
        self.pixel_interval_us = Self::map_speed_to_interval(speed);
        false
    }
}

pub struct CometEffect {
    color: RGB8,
    num_leds: usize,
    position: usize, 
    tail_len: usize,
    time_accumulator: u64,
    pixel_interval_us: u64,
}

impl CometEffect {
    pub fn new(color: RGB8, speed: u8, num_leds: usize) -> Self {
        Self {
            color,
            num_leds,
            position: 0,
            tail_len: (num_leds / 5).max(3), // Đuôi dài 20% strip, tối thiểu 3
            time_accumulator: 0,
            pixel_interval_us: Self::map_speed_to_interval(speed),
        }
    }

    // Tốc độ nhanh hơn ColorWipe (max 100ms)
    fn map_speed_to_interval(speed: u8) -> u64 {
        let inverted_speed = 256 - speed.max(1) as u64;
        let interval_ms = (inverted_speed * 100) / 254 + 2; // 2ms - 102ms
        interval_ms * 1000
    }
}

impl Effect for CometEffect {
    fn name(&self) -> &'static str { "Comet" }

    fn update(&mut self, delta_us: u64) -> bool {
        self.time_accumulator += delta_us;

        if self.time_accumulator >= self.pixel_interval_us {
            self.time_accumulator -= self.pixel_interval_us;
            
            // Di chuyển vị trí, lặp lại khi đến cuối
            self.position = (self.position + 1) % self.num_leds;
            return true;
        }
        false
    }

    fn render(&self, buffer: &mut [RGB8]) {
        // 1. Xóa toàn bộ buffer (hoặc làm mờ nếu muốn hiệu ứng mượt hơn)
        buffer.fill(RGB8::default());

        // 2. Vẽ "đầu" sao chổi
        buffer[self.position] = self.color;

        // 3. Vẽ "đuôi"
        for i in 1..=self.tail_len {
            // Tính vị trí pixel của đuôi (vòng lặp lại)
            let pos = (self.position + self.num_leds - i) % self.num_leds;
            
            // Tính độ mờ (giảm dần)
            let fade_factor = 255 - (i * (255 / self.tail_len.max(1))) as u8;
            buffer[pos] = dim_color(self.color, fade_factor);
        }
    }

    fn set_color(&mut self, color: RGB8) -> bool {
        self.color = color;
        true // Cần render lại ngay
    }

    fn set_speed(&mut self, speed: u8) -> bool {
        self.pixel_interval_us = Self::map_speed_to_interval(speed);
        false
    }
}

pub struct ScannerEffect {
    color: RGB8,
    num_leds: usize,
    position: usize, // Vị trí "mắt"
    direction: i8, // 1 = sang phải, -1 = sang trái
    time_accumulator: u64,
    pixel_interval_us: u64,
}

impl ScannerEffect {
    pub fn new(color: RGB8, speed: u8, num_leds: usize) -> Self {
        Self {
            color,
            num_leds,
            position: 0,
            direction: 1,
            time_accumulator: 0,
            pixel_interval_us: Self::map_speed_to_interval(speed),
        }
    }
    
    // Tốc độ tương tự Comet
    fn map_speed_to_interval(speed: u8) -> u64 {
        let inverted_speed = 256 - speed.max(1) as u64;
        let interval_ms = (inverted_speed * 100) / 254 + 2;
        interval_ms * 1000
    }
}

impl Effect for ScannerEffect {
    fn name(&self) -> &'static str { "Scanner" }

    fn update(&mut self, delta_us: u64) -> bool {
        self.time_accumulator += delta_us;

        if self.time_accumulator >= self.pixel_interval_us {
            self.time_accumulator -= self.pixel_interval_us;
            
            // Logic đổi hướng khi chạm 2 đầu
            if self.direction > 0 {
                // Đang đi sang phải
                if self.position >= self.num_leds - 1 {
                    self.direction = -1; // Đổi hướng
                }
            } else {
                // Đang đi sang trái
                if self.position <= 0 {
                    self.direction = 1; // Đổi hướng
                }
            }
            
            // Di chuyển vị trí
            self.position = (self.position as i16 + self.direction as i16) as usize;
            return true;
        }
        false
    }

    fn render(&self, buffer: &mut [RGB8]) {
        // Xóa buffer
        buffer.fill(RGB8::default());

        
        if self.position < self.num_leds {
            buffer[self.position] = self.color;
        }
        
        
        let inner_dim = dim_color(self.color, 128); // 50%
        if self.position >= 1 { buffer[self.position - 1] = inner_dim; }
        if self.position + 1 < self.num_leds { buffer[self.position + 1] = inner_dim; }
        
    
        let outer_dim = dim_color(self.color, 64); // 25%
        if self.position >= 2 { buffer[self.position - 2] = outer_dim; }
        if self.position + 2 < self.num_leds { buffer[self.position + 2] = outer_dim; }
    }

    fn set_color(&mut self, color: RGB8) -> bool {
        self.color = color;
        true
    }

    fn set_speed(&mut self, speed: u8) -> bool {
        self.pixel_interval_us = Self::map_speed_to_interval(speed);
        false
    }
}


pub struct TheaterChaseEffect {
    color1: RGB8,
    color2: RGB8, // Màu nền (thường là đen)
    num_leds: usize,
    spacing: usize, // Khoảng cách giữa các pixel sáng
    position_offset: usize,
    time_accumulator: u64,
    pixel_interval_us: u64,
}

impl TheaterChaseEffect {
    pub fn new(color: RGB8, speed: u8, num_leds: usize) -> Self {
        Self {
            color1: color,
            color2: RGB8::default(), // Màu đen
            num_leds,
            spacing: 4, // Cứ 4 pixel thì sáng 1
            position_offset: 0,
            time_accumulator: 0,
            pixel_interval_us: Self::map_speed_to_interval(speed),
        }
    }

    // Tốc độ tương tự Comet
    fn map_speed_to_interval(speed: u8) -> u64 {
        let inverted_speed = 256 - speed.max(1) as u64;
        let interval_ms = (inverted_speed * 100) / 254 + 2;
        interval_ms * 1000
    }
}

impl Effect for TheaterChaseEffect {
    fn name(&self) -> &'static str { "Theater Chase" }

    fn update(&mut self, delta_us: u64) -> bool {
        self.time_accumulator += delta_us;

        if self.time_accumulator >= self.pixel_interval_us {
            self.time_accumulator -= self.pixel_interval_us;
            
            // Di chuyển offset, lặp lại theo `spacing`
            self.position_offset = (self.position_offset + 1) % self.spacing;
            return true;
        }
        false
    }

    fn render(&self, buffer: &mut [RGB8]) {
        for (i, pixel) in buffer.iter_mut().enumerate() {
            // (i + offset) % spacing == 0
            if (i + self.position_offset) % self.spacing == 0 {
                *pixel = self.color1;
            } else {
                *pixel = self.color2;
            }
        }
    }

    fn set_color(&mut self, color: RGB8) -> bool {
        self.color1 = color;
        true
    }

    fn set_speed(&mut self, speed: u8) -> bool {
        self.pixel_interval_us = Self::map_speed_to_interval(speed);
        false
    }
}



/// Bộ tạo số giả ngẫu nhiên (PRNG) đơn giản
struct FastRand {
    seed: u32,
}

impl FastRand {
    fn new(seed: u32) -> Self {
        Self { seed: if seed == 0 { 1 } else { seed } }
    }
    
    /// Trả về một số u32
    fn rand_u32(&mut self) -> u32 {
        self.seed = self.seed.wrapping_add(0xADC47F53);
        let mut tmp = self.seed.wrapping_mul(0x7FFFFFED);
        tmp ^= tmp >> 15;
        tmp ^= tmp << 13;
        tmp
    }
    
    /// Trả về một số u8
    fn rand_u8(&mut self) -> u8 {
        (self.rand_u32() >> 24) as u8
    }

    /// Trả về một số trong [0, max)
    fn rand_max(&mut self, max: usize) -> usize {
        (self.rand_u32() as u64 * max as u64 / u32::MAX as u64) as usize
    }
}

pub struct TwinkleEffect {
    base_color: RGB8,    // Màu nền
    sparkle_color: RGB8, // Màu lấp lánh
    num_leds: usize,
    density: u8, // 1-255: Cơ hội một pixel mới lấp lánh
    fade_speed: u8, // 1-255: Tốc độ mờ dần
    time_accumulator: u64,
    pixel_interval_us: u64,
    rand: RefCell<FastRand>,
}

impl TwinkleEffect {
    pub fn new(color: RGB8, speed: u8, num_leds: usize) -> Self {
        // Lấy seed ngẫu nhiên từ thời gian
        let seed = (unsafe { esp_idf_sys::esp_timer_get_time() } & 0xFFFFFFFF) as u32;

        Self {
            base_color: RGB8::default(), // Nền đen
            sparkle_color: color,
            num_leds,
            density: 128, // 50% cơ hội
            fade_speed: 100, // Tốc độ mờ
            time_accumulator: 0,
            pixel_interval_us: Self::map_speed_to_interval(speed),
            rand: RefCell::new(FastRand::new(seed)),
        }
    }

    // Tốc độ ở đây là tốc độ "tick" của hiệu ứng
    fn map_speed_to_interval(speed: u8) -> u64 {
        let inverted_speed = 256 - speed.max(1) as u64;
        let interval_ms = (inverted_speed * 50) / 254 + 5; // 5ms - 55ms
        interval_ms * 1000
    }
}

impl Effect for TwinkleEffect {
    fn name(&self) -> &'static str { "Twinkle" }

    fn update(&mut self, delta_us: u64) -> bool {
        self.time_accumulator += delta_us;

        // Chỉ update theo tốc độ đã định
        if self.time_accumulator >= self.pixel_interval_us {
            self.time_accumulator -= self.pixel_interval_us;
            return true; // Luôn cần render để xử lý fade
        }
        false
    }

    fn render(&self, buffer: &mut [RGB8]) {
        // 1. Làm mờ (fade) tất cả các pixel
        for pixel in buffer.iter_mut() {
            *pixel = fade_color(*pixel, self.fade_speed);
        }

        // 2. Thêm các pixel lấp lánh mới
        let mut rand_mut = self.rand.borrow_mut();
        if rand_mut.rand_u8() < self.density {
            // Chọn một vị trí ngẫu nhiên
            let pos = rand_mut.rand_max(self.num_leds);
            
            // Chỉ đặt nếu nó gần như đã tắt (tránh ghi đè pixel đang sáng)
            if buffer[pos].r < 10 && buffer[pos].g < 10 && buffer[pos].b < 10 {
                buffer[pos] = self.sparkle_color;
            }
        }
        
        // (Nếu bạn muốn nhiều pixel hơn, hãy đặt code trên trong một vòng lặp `for`)
    }

    fn set_color(&mut self, color: RGB8) -> bool {
        self.sparkle_color = color;
        false // Không cần render ngay, vòng update sau sẽ dùng màu mới
    }

    fn set_speed(&mut self, speed: u8) -> bool {
        self.pixel_interval_us = Self::map_speed_to_interval(speed);
        false
    }
}

fn fade_color(color: RGB8, fade_by: u8) -> RGB8 {
    RGB8 {
        r: color.r.saturating_sub(fade_by),
        g: color.g.saturating_sub(fade_by),
        b: color.b.saturating_sub(fade_by),
    }
}

fn dim_color(color: RGB8, scale: u8) -> RGB8 {
    RGB8 {
        r: ((color.r as u16 * scale as u16) >> 8) as u8,
        g: ((color.g as u16 * scale as u16) >> 8) as u8,
        b: ((color.b as u16 * scale as u16) >> 8) as u8,
    }
}

#[derive(Clone, Copy)]
struct Particle {
    position: f32, // Vị trí (float)
    velocity: f32, // Vận tốc (pixel / giây)
    color: RGB8,
}

pub struct BounceEffect {
    num_leds: usize,
    particles: Vec<Particle>,
    lut: Vec<RGB8>, // Bảng màu
    rand: RefCell<FastRand>,
}

impl BounceEffect {
    pub fn new(speed: u8, num_leds: usize) -> Self {
        let seed = (unsafe { esp_idf_sys::esp_timer_get_time() } & 0xFFFFFFFF) as u32;
        let mut rand = FastRand::new(seed);
        
        // Tạo LUT cầu vồng
        let mut lut = Vec::with_capacity(256);
        for i in 0..=255 {
            let hue = (i as f32 * 360.0) / 256.0;
            let color = Hsv::new(RgbHue::from_degrees(hue), 1.0, 1.0);
            let srgb: Srgb = Srgb::from_color(color);
            lut.push(RGB8 {
                r: (srgb.red * 255.0).round() as u8,
                g: (srgb.green * 255.0).round() as u8,
                b: (srgb.blue * 255.0).round() as u8,
            });
        }

        // Tạo các hạt
        let num_particles = (num_leds / 20).max(3); // 5% dải LED, tối thiểu 3
        let mut particles = Vec::with_capacity(num_particles);
        
        // Ánh xạ speed (1-255) sang vận tốc (10-60 pixels/sec)
        let max_vel = (speed as f32 / 255.0) * 50.0 + 10.0;

        for _ in 0..num_particles {
            // Vận tốc ngẫu nhiên (có thể âm hoặc dương)
            let vel = (rand.rand_u32() as f32 / u32::MAX as f32 - 0.5) * 2.0 * max_vel;
            
            particles.push(Particle {
                position: rand.rand_max(num_leds) as f32,
                velocity: vel.clamp(-max_vel, max_vel),
                color: lut[rand.rand_u8() as usize],
            });
        }

        Self {
            num_leds,
            particles,
            lut,
            rand: RefCell::new(rand),
        }
    }
    
    // Hàm này sẽ được dùng trong set_speed
    fn update_speeds(&mut self, speed: u8) {
        let max_vel = (speed as f32 / 255.0) * 50.0 + 10.0;
        let mut rand = self.rand.borrow_mut();
        
        for p in self.particles.iter_mut() {
            let vel = (rand.rand_u32() as f32 / u32::MAX as f32 - 0.5) * 2.0 * max_vel;
            p.velocity = vel.clamp(-max_vel, max_vel);
        }
    }
}

impl Effect for BounceEffect {
    fn name(&self) -> &'static str { "Bounce" }

    fn update(&mut self, delta_us: u64) -> bool {
        // Chuyển delta_us sang giây (dưới dạng f32)
        let delta_sec = (delta_us as f32) / 1_000_000.0;
        let max_pos = (self.num_leds - 1) as f32;

        for p in self.particles.iter_mut() {
            // Tính vị trí mới
            let mut new_pos = p.position + p.velocity * delta_sec;

            // Kiểm tra va chạm
            if new_pos < 0.0 {
                new_pos = 0.0; // Đặt lại vị trí
                p.velocity = -p.velocity; // Đảo chiều
            } else if new_pos > max_pos {
                new_pos = max_pos; // Đặt lại vị trí
                p.velocity = -p.velocity; // Đảo chiều
            }
            
            p.position = new_pos;
        }

        true // Luôn luôn render
    }

    fn render(&self, buffer: &mut [RGB8]) {
        // 1. Xóa buffer
        buffer.fill(RGB8::default());

        // 2. Vẽ từng hạt
        for p in &self.particles {
            let pos_int = p.position.round() as usize;
            if pos_int < buffer.len() {
                // Thêm màu (additive) để các hạt giao nhau đẹp hơn
                buffer[pos_int].r = buffer[pos_int].r.saturating_add(p.color.r);
                buffer[pos_int].g = buffer[pos_int].g.saturating_add(p.color.g);
                buffer[pos_int].b = buffer[pos_int].b.saturating_add(p.color.b);
            }
        }
    }

    fn set_color(&mut self, _color: RGB8) -> bool {
        // Hiệu ứng này không dùng 1 màu
        false
    }

    fn set_speed(&mut self, speed: u8) -> bool {
        // Tính toán lại tất cả vận tốc
        self.update_speeds(speed);
        false
    }
}

pub struct AudioVolumeBarEffect {
    color: RGB8,
    num_leds: usize,
    center: usize,
    
    // Peak hold system (for both sides)
    peak_hold_left: usize,
    peak_hold_right: usize,
    peak_hold_time: u64,
    last_peak_update: u64,
    
    // Smoothing for natural movement
    current_level: f32,
    smooth_factor: f32,
    
    // Idle animation
    idle_phase: f32,
    idle_speed: f32,
    idle_amplitude: f32,
    
    // Background brightness
    bg_brightness: u8,
}

impl AudioVolumeBarEffect {
    pub fn new(color: RGB8, num_leds: usize) -> Self {
        Self {
            color,
            num_leds,
            center: num_leds / 2,
            peak_hold_left: num_leds / 2,
            peak_hold_right: num_leds / 2,
            peak_hold_time: 500_000, // 500ms
            last_peak_update: 0,
            current_level: 0.0,
            smooth_factor: 0.2,
            idle_phase: 0.0,
            idle_speed: 2.0,
            idle_amplitude: 0.15, // 15% breathing when idle
            bg_brightness: 20, // White background at 20/255 brightness
        }
    }
}

impl Effect for AudioVolumeBarEffect {
    fn name(&self) -> &'static str { "Audio Volume Bar" }

    fn update(&mut self, _delta_us: u64) -> bool {
        true
    }

    fn render(&self, buffer: &mut [RGB8]) {
        buffer.fill(RGB8::default());
    }
    
    fn render_audio(&mut self, buffer: &mut [RGB8], audio: &AudioData, now_us: u64) {
        // Step 1: Fill background with dim white
        let bg_color = RGB8 {
            r: self.bg_brightness,
            g: self.bg_brightness,
            b: self.bg_brightness,
        };
        buffer.fill(bg_color);
        
        // Step 2: Update breathing phase for idle animation
        let delta_sec = 0.033; // ~30 FPS
        self.idle_phase += delta_sec * self.idle_speed;
        if self.idle_phase > core::f32::consts::PI * 2.0 {
            self.idle_phase -= core::f32::consts::PI * 2.0;
        }
        
        let breath = self.idle_phase.sin() * 0.5 + 0.5; // 0.0 to 1.0
        
        // Step 3: Calculate spread level
        let has_audio = audio.volume > 0.02;
        
        let spread: f32 = if has_audio {
            // Smooth audio response with subtle breathing
            let target = audio.volume;
            self.current_level += (target - self.current_level) * self.smooth_factor;
            self.current_level * 0.9 + breath * 0.1
        } else {
            // Idle breathing animation
            self.current_level *= 0.95; // Decay
            self.idle_amplitude * breath
        };
        
        // Step 4: Calculate LEDs to light from center
        let half_spread = ((spread * (self.num_leds / 2) as f32) as usize).min(self.num_leds / 2);
        
        // Step 5: Render user color from center spreading out
        // Left side
        for i in 0..half_spread {
            let pos = self.center.saturating_sub(i + 1);
            if pos < self.num_leds {
                buffer[pos] = self.color;
            }
        }
        
        // Right side
        for i in 0..half_spread {
            let pos = self.center + i + 1;
            if pos < self.num_leds {
                buffer[pos] = self.color;
            }
        }
        
        // Center LED (always user color when active)
        if spread > 0.01 {
            buffer[self.center] = self.color;
        }
        
        // Step 6: Peak hold system (only when audio active)
        if has_audio {
            let left_peak_pos = self.center.saturating_sub(half_spread);
            let right_peak_pos = (self.center + half_spread).min(self.num_leds - 1);
            
            // Update peaks
            if left_peak_pos < self.peak_hold_left {
                self.peak_hold_left = left_peak_pos;
                self.last_peak_update = now_us;
            }
            if right_peak_pos > self.peak_hold_right {
                self.peak_hold_right = right_peak_pos;
                self.last_peak_update = now_us;
            }
            
            // Peak decay
            if now_us - self.last_peak_update > self.peak_hold_time {
                if self.peak_hold_left < self.center {
                    self.peak_hold_left += 1;
                }
                if self.peak_hold_right > self.center {
                    self.peak_hold_right = self.peak_hold_right.saturating_sub(1);
                }
                self.last_peak_update = now_us;
            }
            
            // Render peak markers (brighter version of user color)
            if self.peak_hold_left < self.center && self.peak_hold_left < self.num_leds {
                buffer[self.peak_hold_left] = RGB8 {
                    r: self.color.r.saturating_add(50).min(255),
                    g: self.color.g.saturating_add(50).min(255),
                    b: self.color.b.saturating_add(50).min(255),
                };
            }
            if self.peak_hold_right > self.center && self.peak_hold_right < self.num_leds {
                buffer[self.peak_hold_right] = RGB8 {
                    r: self.color.r.saturating_add(50).min(255),
                    g: self.color.g.saturating_add(50).min(255),
                    b: self.color.b.saturating_add(50).min(255),
                };
            }
        } else {
            // Reset peaks when idle
            self.peak_hold_left = self.center;
            self.peak_hold_right = self.center;
        }
    }

    fn set_color(&mut self, color: RGB8) -> bool {
        self.color = color;
        true // Need re-render with new color
    }
    
    fn is_audio_reactive(&self) -> bool { 
        true 
    }
}