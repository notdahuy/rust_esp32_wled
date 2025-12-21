use anyhow::Result;
use esp_idf_svc::sntp::{EspSntp, SyncStatus};
use esp_idf_sys::{self as sys, time_t};
use log::{info, warn};
use std::sync::{Arc, Mutex};
use std::time::{SystemTime, UNIX_EPOCH};

const NTP_SERVERS: [&str; 3] = [
    "pool.ntp.org",
    "time.google.com", 
    "time.cloudflare.com"
];

/// Struct để lưu thông tin thời gian
#[derive(Debug, Clone, Copy)]
pub struct TimeInfo {
    pub hour: u8,      // 0-23
    pub minute: u8,    // 0-59
    pub second: u8,    // 0-59
    pub weekday: u8,   // 0=Monday, 6=Sunday
    pub day: u8,       // 1-31
    pub month: u8,     // 1-12
    pub year: u16,     // e.g., 2024
}

impl TimeInfo {
    /// Tạo TimeInfo từ Unix timestamp
    pub fn from_timestamp(timestamp: i64) -> Self {
        unsafe {
            let mut tm: sys::tm = std::mem::zeroed();
            let time = timestamp as time_t;
            sys::localtime_r(&time, &mut tm);
            
            Self {
                hour: tm.tm_hour as u8,
                minute: tm.tm_min as u8,
                second: tm.tm_sec as u8,
                weekday: if tm.tm_wday == 0 { 6 } else { (tm.tm_wday - 1) as u8 }, // Convert Sun=0 to Mon=0
                day: tm.tm_mday as u8,
                month: (tm.tm_mon + 1) as u8,
                year: (tm.tm_year + 1900) as u16,
            }
        }
    }
    
    /// Format thời gian thành string
    pub fn format(&self) -> heapless::String<32> {
        let mut s = heapless::String::new();
        use core::fmt::Write;
        write!(s, "{:04}-{:02}-{:02} {:02}:{:02}:{:02}", 
               self.year, self.month, self.day,
               self.hour, self.minute, self.second).ok();
        s
    }
    
    /// Lấy tên ngày trong tuần
    pub fn weekday_name(&self) -> &'static str {
        match self.weekday {
            0 => "Monday",
            1 => "Tuesday",
            2 => "Wednesday",
            3 => "Thursday",
            4 => "Friday",
            5 => "Saturday",
            6 => "Sunday",
            _ => "Unknown",
        }
    }
}

pub struct NtpManager {
    sntp: Arc<Mutex<Option<EspSntp<'static>>>>,
    timezone: String,
    sync_status: Arc<Mutex<bool>>,
}

impl NtpManager {
    /// Khởi tạo NTP Manager
    pub fn new(timezone: &str) -> Result<Self> {
        info!("🕐 Initializing NTP Manager");
        info!("   Timezone: {}", timezone);
        
        // Set timezone trước khi khởi tạo SNTP
        Self::set_timezone(timezone)?;
        
        Ok(Self {
            sntp: Arc::new(Mutex::new(None)),
            timezone: timezone.to_string(),
            sync_status: Arc::new(Mutex::new(false)),
        })
    }
    
    /// Start NTP sync (gọi sau khi WiFi connected)
    pub fn start_sync(&self) -> Result<()> {
        info!("🔄 Starting NTP synchronization...");
        
        // Tạo SNTP instance
        let sntp = EspSntp::new_default()?;
        
        *self.sntp.lock().unwrap() = Some(sntp);
        
        info!("   Waiting for time sync...");
        
        // Đợi sync trong background thread
        let sync_status = self.sync_status.clone();
        let sntp_clone = self.sntp.clone();
        
        std::thread::spawn(move || {
            let max_attempts = 30;
            let mut synced = false;
            
            for attempt in 1..=max_attempts {
                if let Some(ref sntp) = *sntp_clone.lock().unwrap() {
                    match sntp.get_sync_status() {
                        SyncStatus::Completed => {
                            info!("✅ NTP sync completed (attempt {})", attempt);
                            synced = true;
                            *sync_status.lock().unwrap() = true;
                            break;
                        }
                        SyncStatus::InProgress => {
                            if attempt % 5 == 0 {
                                info!("   Still syncing... ({}s)", attempt);
                            }
                        }
                        SyncStatus::Reset => {
                            warn!("   NTP sync reset");
                        }
                    }
                }
                
                std::thread::sleep(std::time::Duration::from_secs(1));
            }
            
            if !synced {
                warn!("⚠️  NTP sync timeout after {}s", max_attempts);
            }
        });
        
        Ok(())
    }
    
    /// Lấy thời gian hiện tại
    pub fn get_time(&self) -> Result<TimeInfo> {
        let timestamp = self.get_unix_timestamp()?;
        Ok(TimeInfo::from_timestamp(timestamp))
    }
    
    /// Lấy Unix timestamp (seconds since 1970-01-01)
    pub fn get_unix_timestamp(&self) -> Result<i64> {
        let duration = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map_err(|e| anyhow::anyhow!("Time error: {}", e))?;
        
        Ok(duration.as_secs() as i64)
    }
    
    /// Kiểm tra xem đã sync chưa
    pub fn is_synced(&self) -> bool {
        *self.sync_status.lock().unwrap()
    }
    
    /// Set timezone
    fn set_timezone(tz: &str) -> Result<()> {
        use std::ffi::CString;
        
        let tz_str = CString::new(tz)?;
        
        unsafe {
            sys::setenv(
                b"TZ\0".as_ptr() as *const u8,
                tz_str.as_ptr(),
                1
            );
            sys::tzset();
        }
        
        Ok(())
    }
    
    /// Reset và sync lại
    pub fn resync(&self) -> Result<()> {
        info!("🔄 Resyncing NTP...");
        
        // Reset sync status
        *self.sync_status.lock().unwrap() = false;
        
        // Drop old SNTP instance
        *self.sntp.lock().unwrap() = None;
        
        // Start new sync
        self.start_sync()?;
        
        Ok(())
    }
    
    /// Lấy thông tin debug
    pub fn get_debug_info(&self) -> String {
        let synced = self.is_synced();
        let tz = &self.timezone;
        
        let time_str = if synced {
            if let Ok(time) = self.get_time() {
                format!("{} ({})", time.format(), time.weekday_name())
            } else {
                "Error getting time".to_string()
            }
        } else {
            "Not synced yet".to_string()
        };
        
        format!(
            "NTP Status: {}\nTimezone: {}\nCurrent time: {}",
            if synced { "Synced" } else { "Not synced" },
            tz,
            time_str
        )
    }
}

/// Các timezone phổ biến cho Việt Nam
pub mod timezones {
    pub const VIETNAM: &str = "ICT-7";           // UTC+7
    pub const BANGKOK: &str = "ICT-7";           // UTC+7
    pub const SINGAPORE: &str = "SGT-8";         // UTC+8
    pub const TOKYO: &str = "JST-9";             // UTC+9
    pub const HONG_KONG: &str = "HKT-8";         // UTC+8
    pub const UTC: &str = "UTC0";                // UTC+0
    
    // Format: <STD><offset>[<DST>[<offset>]]
    // Ví dụ: "EST5EDT,M3.2.0/2,M11.1.0/2" cho Eastern Time US
}

#[cfg(test)]
mod tests {
    use super::*;
    
    #[test]
    fn test_time_info() {
        // Test với timestamp cụ thể: 2024-01-15 10:30:00 UTC
        let timestamp = 1705315800;
        let time = TimeInfo::from_timestamp(timestamp);
        
        println!("Time: {}", time.format());
        println!("Weekday: {}", time.weekday_name());
    }
}