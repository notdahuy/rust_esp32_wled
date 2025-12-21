
use log::info;
use crate::effects::EffectType;
use smart_leds::RGB8;

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct TimeOfDay {
    pub hour: u8,    // 0-23
    pub minute: u8,  // 0-59
}

impl TimeOfDay {
    pub fn new(hour: u8, minute: u8) -> Result<Self, &'static str> {
        if hour > 23 || minute > 59 {
            return Err("Invalid time");
        }
        Ok(Self { hour, minute })
    }
    
    pub fn to_minutes(&self) -> u16 {
        (self.hour as u16) * 60 + (self.minute as u16)
    }
}

#[derive(Debug, Clone)]
pub struct SchedulePreset {
    pub effect: EffectType,     // Hiệu ứng (tắt = Static)
    pub color: Option<RGB8>,    // Màu (tắt = Some(RGB8 { r: 0, g: 0, b: 0 }))
    pub brightness: Option<f32>, // Độ sáng
    pub speed: Option<u8>,      // Tốc độ
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
    
    pub fn with_all(effect: EffectType, color: Option<RGB8>, brightness: Option<f32>, speed: Option<u8>) -> Self {
        Self {
            effect,
            color,
            brightness,
            speed,
        }
    }
    
    // Tạo preset tắt (static màu đen)
    pub fn off() -> Self {
        Self {
            effect: EffectType::Static,
            color: Some(RGB8 { r: 0, g: 0, b: 0 }),
            brightness: None,
            speed: None,
        }
    }
    
    // Kiểm tra có phải preset tắt không
    pub fn is_off(&self) -> bool {
        self.effect == EffectType::Static && 
        self.color == Some(RGB8 { r: 0, g: 0, b: 0 })
    }
    
    // Tạo preset bật với effect cụ thể
    pub fn on(effect: EffectType, color: Option<RGB8>, brightness: Option<f32>, speed: Option<u8>) -> Self {
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
    pub id: usize,
    pub enabled: bool,
    pub preset: SchedulePreset,
    pub time: TimeOfDay,
    pub days: [bool; 7],        // Mon=0, Tue=1, ..., Sun=6
}

impl Schedule {
    pub fn should_trigger(&self, current_time: TimeOfDay, current_day: u8) -> bool {
        if !self.enabled {
            return false;
        }
        
        if current_day > 6 {
            return false;
        }
        
        // Check if this day is enabled
        if !self.days[current_day as usize] {
            return false;
        }
        
        // Check if time matches
        self.time.hour == current_time.hour && self.time.minute == current_time.minute
    }
    
    pub fn days_string(&self) -> heapless::String<32> {
        let mut s = heapless::String::new();
        
        // Array chứa các số dạng string
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
    schedules: heapless::Vec<Schedule, 16>,  // Max 16 schedules
    next_id: usize,
    last_check_minute: u16,  // Track last checked minute to avoid duplicate triggers
}

impl LedScheduler {
    pub fn new() -> Self {
        Self {
            schedules: heapless::Vec::new(),
            next_id: 0,
            last_check_minute: 0xFFFF,
        }
    }
    
    pub fn add_schedule(&mut self, preset: SchedulePreset, time: TimeOfDay, days: [bool; 7]) -> Result<usize, &'static str> {
        let id = self.next_id;
        self.next_id += 1;
        
        let schedule = Schedule {
            id,
            enabled: true,
            preset: preset.clone(),
            time,
            days,
        };
        
        self.schedules.push(schedule)
            .map_err(|_| "Schedule list full (max 16)")?;
            
        Ok(id)
    }
    
    pub fn remove_schedule(&mut self, id: usize) -> bool {
        if let Some(pos) = self.schedules.iter().position(|s| s.id == id) {
            self.schedules.swap_remove(pos);
            info!("Schedule {} removed", id);
            true
        } else {
            false
        }
    }
    
    pub fn toggle_schedule(&mut self, id: usize, enable: bool) -> bool {
        if let Some(schedule) = self.schedules.iter_mut().find(|s| s.id == id) {
            schedule.enabled = enable;
            info!("Schedule {} {}", id, if enable { "enabled" } else { "disabled" });
            true
        } else {
            false
        }
    }
  
    pub fn clear_all(&mut self) {
        self.schedules.clear();
        info!("All schedules cleared");
    }
    
    pub fn check_and_execute(&mut self, current_time: TimeOfDay, current_day: u8) -> Option<SchedulePreset> {
        let current_minute = current_time.to_minutes();
        
        // Only check once per minute
        if current_minute == self.last_check_minute {
            return None;
        }
        self.last_check_minute = current_minute;
        
        // Check all schedules
        for schedule in self.schedules.iter() {
            if schedule.should_trigger(current_time, current_day) {
                let preset = schedule.preset.clone();
                info!("Schedule {} triggered: {} at {:02}:{:02}", 
                      schedule.id, 
                      if schedule.is_off() { "OFF" } else { schedule.effect_string() },
                      schedule.time.hour, 
                      schedule.time.minute);
                return Some(preset);
            }
        }
        
        None
    }
    
    pub fn get_all_schedules(&self) -> &[Schedule] {
        &self.schedules
    }
}