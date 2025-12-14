// src/effects/blink_effect.rs

use smart_leds::RGB8;
use super::Effect;

pub struct BlinkEffect {
    on: RGB8,
    off: RGB8,
    speed: u8,

    // State tracking
    current_state: bool,
    next_transition_time: u64,
    cycle_time_us: u64,
    
    // Flag để xử lý color change mượt mà
    needs_immediate_render: bool,
}

impl BlinkEffect {
    pub fn new(color: RGB8, speed: u8) -> Self {
        let cycle_time = cycle_time_us(speed);

        Self {
            on: color,
            off: RGB8::default(),
            speed: speed.clamp(1, 255),
            current_state: true,
            next_transition_time: 0,
            cycle_time_us: cycle_time,
            needs_immediate_render: false, // Khởi tạo flag
        }
    }
}

fn cycle_time_us(speed: u8) -> u64 {
    let min = 50_000;
    let max = 2_000_000;
    max - (speed as u64 * (max - min) / 255)
}

impl Effect for BlinkEffect {
    fn update(&mut self, now_us: u64, buffer: &mut [RGB8]) -> Option<u64> {
        // QUAN TRỌNG: Kiểm tra immediate render flag TRƯỚC mọi logic khác
        // Điều này đảm bảo rằng khi màu thay đổi, frame đầu tiên luôn đúng
        if self.needs_immediate_render {
            self.needs_immediate_render = false;
            
            // Nếu đang ở trạng thái ON, render màu mới ngay lập tức
            if self.current_state {
                buffer.fill(self.on);
                // Tiếp tục với timing bình thường, không reset next_transition_time
                // vì chúng ta không muốn làm gián đoạn chu kỳ blink
                return Some(self.next_transition_time);
            }
            // Nếu đang OFF, không cần làm gì, màu mới sẽ được dùng ở lần ON tiếp theo
        }
        
        // Lần đầu tiên được gọi, khởi tạo next_transition_time
        if self.next_transition_time == 0 {
            buffer.fill(self.on);
            let half_cycle = self.cycle_time_us / 2;
            self.next_transition_time = now_us + half_cycle;
            return Some(self.next_transition_time);
        }

        // Kiểm tra xem đã đến lúc chuyển trạng thái chưa
        if now_us >= self.next_transition_time {
            self.current_state = !self.current_state;
            buffer.fill(if self.current_state { self.on } else { self.off });

            let half_cycle = self.cycle_time_us / 2;
            self.next_transition_time += half_cycle;

            // Xử lý trường hợp bỏ lỡ nhiều transitions
            while self.next_transition_time <= now_us {
                self.next_transition_time += half_cycle;
                self.current_state = !self.current_state;
            }

            return Some(self.next_transition_time);
        }

        Some(self.next_transition_time)
    }

    fn set_color(&mut self, color: RGB8) {
        if self.on != color {
            self.on = color;
            // Đánh dấu cần render ngay lập tức trong lần update tiếp theo
            // Thay vì reset next_transition_time (gây gián đoạn timing),
            // chúng ta dùng flag để signal rằng cần update màu ngay
            self.needs_immediate_render = true;
        }
    }

    fn set_speed(&mut self, speed: u8) {
        let new_speed = speed.clamp(1, 255);
        if self.speed != new_speed {
            self.speed = new_speed;
            let new_cycle_time = cycle_time_us(new_speed);

            if new_cycle_time != self.cycle_time_us {
                self.cycle_time_us = new_cycle_time;
                // Khi đổi speed, chúng ta reset timing để tránh glitches
                self.next_transition_time = 0;
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