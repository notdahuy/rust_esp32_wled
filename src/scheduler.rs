use log::{info};
use crate::effect::EffectType;
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
pub struct ScheduleAction {
    pub power_on: bool,              // true = bật, false = tắt
    pub effect: Option<EffectType>,  // Hiệu ứng khi bật (None nếu chỉ tắt)
    pub color: Option<RGB8>,         // Màu cho hiệu ứng (nếu cần)
    pub brightness: Option<f32>,     // Độ sáng (0.0-1.0)
    pub speed: Option<u8>,           // Tốc độ hiệu ứng
}

impl ScheduleAction {
    pub fn power_off() -> Self {
        Self {
            power_on: false,
            effect: None,
            color: None,
            brightness: None,
            speed: None,
        }
    }
    
    pub fn full(power_on: bool, effect: Option<EffectType>, color: Option<RGB8>, 
                brightness: Option<f32>, speed: Option<u8>) -> Self {
        Self {
            power_on,
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
    pub action: ScheduleAction,
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
        let day_names = ["Mon", "Tue", "Wed", "Thu", "Fri", "Sat", "Sun"];
        let mut first = true;
        
        for (i, &enabled) in self.days.iter().enumerate() {
            if enabled {
                if !first {
                    let _ = s.push_str(",");
                }
                let _ = s.push_str(day_names[i]);
                first = false;
            }
        }
        
        if s.is_empty() {
            let _ = s.push_str("None");
        }
        
        s
    }
    
    pub fn effect_string(&self) -> &'static str {
        if !self.action.power_on {
            return "OFF";
        }
        
        match &self.action.effect {
            Some(EffectType::Static) => "static",
            Some(EffectType::Rainbow) => "rainbow",
            Some(EffectType::Breathe) => "breathe",
            Some(EffectType::ColorWipe) => "colorwipe",
            Some(EffectType::Comet) => "comet",
            Some(EffectType::Scanner) => "scanner",
            Some(EffectType::TheaterChase) => "theaterchase",
            Some(EffectType::Bounce) => "bounce",
            Some(EffectType::AudioVolumeBar) => "volumebar",
            None => "ON",
        }
    }
}

pub struct LedScheduler {
    schedules: heapless::Vec<Schedule, 16>,  // Max 16 schedules
    next_id: usize,
    last_check_minute: u16,  // Track last checked minute to avoid duplicate triggers
    led_state: bool,
}

impl LedScheduler {
    pub fn new() -> Self {
        Self {
            schedules: heapless::Vec::new(),
            next_id: 0,
            last_check_minute: 0xFFFF,
            led_state: true,
        }
    }
    
    pub fn add_schedule(&mut self, action: ScheduleAction, time: TimeOfDay, days: [bool; 7]) -> Result<usize, &'static str> {
        let id = self.next_id;
        self.next_id += 1;
        
        let schedule = Schedule {
            id,
            enabled: true,
            action: action.clone(),
            time,
            days,
        };
        
        self.schedules.push(schedule)
            .map_err(|_| "Schedule list full (max 16)")?;
        
        info!("Schedule {} added: {} at {:02}:{:02}", 
              id, if action.power_on { "ON" } else { "OFF" }, time.hour, time.minute);
        
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
  
    pub fn clear_all(&mut self) {
        self.schedules.clear();
        info!("All schedules cleared");
    }
    
    pub fn check_and_execute(&mut self, current_time: TimeOfDay, current_day: u8) -> Option<ScheduleAction> {
        let current_minute = current_time.to_minutes();
        
        // Only check once per minute
        if current_minute == self.last_check_minute {
            return None;
        }
        self.last_check_minute = current_minute;
        
        // Check all schedules
        for schedule in self.schedules.iter() {
            if schedule.should_trigger(current_time, current_day) {
                self.led_state = schedule.action.power_on;
                let action = schedule.action.clone();  // Clone action
                info!("Schedule {} triggered: LED {} - Effect: {}", 
                      schedule.id, 
                      if action.power_on { "ON" } else { "OFF" },
                      schedule.effect_string());
                return Some(action);
            }
        }
        
        None
    }
    
    pub fn get_all_schedules(&self) -> &[Schedule] {
        &self.schedules
    }
    
    pub fn set_led_state(&mut self, state: bool) {
        self.led_state = state;
    }
}