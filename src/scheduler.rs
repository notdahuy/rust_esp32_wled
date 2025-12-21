use crate::effects::EffectType;
use log::info;
use smart_leds::RGB8;

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct TimeOfDay {
    pub hour: u8,   // Giờ (0-23)
    pub minute: u8, // Phút (0-59)
}

impl TimeOfDay {
    // Khởi tạo thời gian, kiểm tra tính hợp lệ
    pub fn new(hour: u8, minute: u8) -> Result<Self, &'static str> {
        if hour > 23 || minute > 59 {
            return Err("Invalid time");
        }
        Ok(Self { hour, minute })
    }

    // Chuyển đổi sang tổng số phút để dễ so sánh
    pub fn to_minutes(&self) -> u16 {
        (self.hour as u16) * 60 + (self.minute as u16)
    }
}

#[derive(Debug, Clone)]
pub struct SchedulePreset {
    pub effect: EffectType,      // Loại hiệu ứng
    pub color: Option<RGB8>,     // Màu sắc (None nếu không dùng)
    pub brightness: Option<f32>, // Độ sáng
    pub speed: Option<u8>,       // Tốc độ hiệu ứng
}

impl SchedulePreset {
    pub fn new(effect: EffectType) -> Self {
        Self {
            effect,
            color: None,
            brightness: None,
            speed: None,
        }
    }

    pub fn with_color(effect: EffectType, color: RGB8) -> Self {
        Self {
            effect,
            color: Some(color),
            brightness: None,
            speed: None,
        }
    }

    pub fn with_all(
        effect: EffectType,
        color: Option<RGB8>,
        brightness: Option<f32>,
        speed: Option<u8>,
    ) -> Self {
        Self {
            effect,
            color,
            brightness,
            speed,
        }
    }

    // Tạo preset để tắt đèn (Static màu đen)
    pub fn off() -> Self {
        Self {
            effect: EffectType::Static,
            color: Some(RGB8 { r: 0, g: 0, b: 0 }),
            brightness: None,
            speed: None,
        }
    }

    // Kiểm tra xem preset này có phải là lệnh tắt không
    pub fn is_off(&self) -> bool {
        self.effect == EffectType::Static && self.color == Some(RGB8 { r: 0, g: 0, b: 0 })
    }

    // Tạo preset bật đèn với tham số tùy chỉnh
    pub fn on(
        effect: EffectType,
        color: Option<RGB8>,
        brightness: Option<f32>,
        speed: Option<u8>,
    ) -> Self {
        Self {
            effect,
            color,
            brightness,
            speed,
        }
    }
}

#[derive(Debug, Clone)]
pub struct Schedule {
    pub id: usize,          // ID định danh của lịch
    pub enabled: bool,      // Trạng thái kích hoạt (Bật/Tắt)
    pub preset: SchedulePreset, // Cấu hình hiệu ứng sẽ chạy
    pub time: TimeOfDay,    // Thời gian kích hoạt
    pub days: [bool; 7],    // Các ngày trong tuần (T2=0 ... CN=6)
}

impl Schedule {
    // Kiểm tra xem lịch có nên chạy tại thời điểm hiện tại không
    pub fn should_trigger(&self, current_time: TimeOfDay, current_day: u8) -> bool {
        if !self.enabled {
            return false;
        }

        if current_day > 6 {
            return false;
        }

        // Kiểm tra xem ngày hiện tại có được chọn không
        if !self.days[current_day as usize] {
            return false;
        }

        // So sánh giờ và phút
        self.time.hour == current_time.hour && self.time.minute == current_time.minute
    }

    // Tạo chuỗi hiển thị các ngày được chọn (VD: "0,1,2")
    pub fn days_string(&self) -> heapless::String<32> {
        let mut s = heapless::String::new();
        const DAY_STRS: [&str; 7] = ["0", "1", "2", "3", "4", "5", "6"];

        for (i, &enabled) in self.days.iter().enumerate() {
            if enabled {
                if !s.is_empty() {
                    let _ = s.push_str(",");
                }
                let _ = s.push_str(DAY_STRS[i]);
            }
        }
        s
    }

    // Trả về tên hiệu ứng dạng chuỗi
    pub fn effect_string(&self) -> &'static str {
        if self.preset.is_off() {
            return "off";
        }

        match self.preset.effect {
            EffectType::Static => "static",
            EffectType::Rainbow => "rainbow",
            EffectType::Breathe => "breathe",
            EffectType::Comet => "comet",
            EffectType::VuMeter => "vumeter",
            EffectType::Scanner => "scanner",
            EffectType::TheaterChase => "theaterchase",
            EffectType::Bounce => "bounce",
            EffectType::ColorWipe => "colorwipe",
            EffectType::Gravimeter => "gravimeter",
            EffectType::RadialPulseEffect => "pulse",
        }
    }

    pub fn is_off(&self) -> bool {
        self.preset.is_off()
    }
}

pub struct LedScheduler {
    schedules: heapless::Vec<Schedule, 16>, // Danh sách lịch (tối đa 16)
    next_id: usize,                         // ID tự tăng cho lịch mới
    last_check_minute: u16,                 // Lưu phút kiểm tra gần nhất để tránh trùng lặp
}

impl LedScheduler {
    pub fn new() -> Self {
        Self {
            schedules: heapless::Vec::new(),
            next_id: 0,
            last_check_minute: 0xFFFF,
        }
    }

    // Thêm lịch mới vào danh sách
    pub fn add_schedule(
        &mut self,
        preset: SchedulePreset,
        time: TimeOfDay,
        days: [bool; 7],
    ) -> Result<usize, &'static str> {
        let id = self.next_id;
        self.next_id += 1;

        let schedule = Schedule {
            id,
            enabled: true,
            preset: preset.clone(),
            time,
            days,
        };

        self.schedules
            .push(schedule)
            .map_err(|_| "Schedule list full (max 16)")?;

        Ok(id)
    }

    // Xóa lịch dựa theo ID
    pub fn remove_schedule(&mut self, id: usize) -> bool {
        if let Some(pos) = self.schedules.iter().position(|s| s.id == id) {
            self.schedules.swap_remove(pos);
            info!("Schedule {} removed", id);
            true
        } else {
            false
        }
    }

    // Bật hoặc tắt trạng thái kích hoạt của một lịch
    pub fn toggle_schedule(&mut self, id: usize, enable: bool) -> bool {
        if let Some(schedule) = self.schedules.iter_mut().find(|s| s.id == id) {
            schedule.enabled = enable;
            info!(
                "Schedule {} {}",
                id,
                if enable { "enabled" } else { "disabled" }
            );
            true
        } else {
            false
        }
    }

    // Xóa toàn bộ lịch
    pub fn clear_all(&mut self) {
        self.schedules.clear();
        info!("All schedules cleared");
    }

    // Hàm chính: Kiểm tra thời gian và thực thi lịch nếu khớp
    pub fn check_and_execute(
        &mut self,
        current_time: TimeOfDay,
        current_day: u8,
    ) -> Option<SchedulePreset> {
        let current_minute = current_time.to_minutes();

        // Chỉ kiểm tra một lần mỗi phút
        if current_minute == self.last_check_minute {
            return None;
        }
        self.last_check_minute = current_minute;

        // Duyệt qua tất cả các lịch
        for schedule in self.schedules.iter() {
            if schedule.should_trigger(current_time, current_day) {
                let preset = schedule.preset.clone();
                info!(
                    "Schedule {} triggered: {} at {:02}:{:02}",
                    schedule.id,
                    if schedule.is_off() {
                        "OFF"
                    } else {
                        schedule.effect_string()
                    },
                    schedule.time.hour,
                    schedule.time.minute
                );
                return Some(preset);
            }
        }

        None
    }

    // Lấy danh sách tất cả các lịch
    pub fn get_all_schedules(&self) -> &[Schedule] {
        &self.schedules
    }
}