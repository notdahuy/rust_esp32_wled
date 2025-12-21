use anyhow::Result;
use esp_idf_svc::sntp::{EspSntp, SyncStatus};
use esp_idf_sys::{self as sys, time_t};
use log::{info, warn};
use std::sync::{Arc, Mutex};
use std::time::{SystemTime, UNIX_EPOCH};

const NTP_SERVERS: [&str; 3] = ["pool.ntp.org", "time.google.com", "time.cloudflare.com"];

/// Struct để lưu thông tin thời gian chi tiết
#[derive(Debug, Clone, Copy)]
pub struct TimeInfo {
    pub hour: u8,    // Giờ (0-23)
    pub minute: u8,  // Phút (0-59)
    pub second: u8,  // Giây (0-59)
    pub weekday: u8, // Thứ (0=Thứ Hai, 6=Chủ Nhật)
    pub day: u8,     // Ngày (1-31)
    pub month: u8,   // Tháng (1-12)
    pub year: u16,   // Năm (ví dụ 2024)
}

impl TimeInfo {
    /// Chuyển đổi từ Unix timestamp sang TimeInfo
    pub fn from_timestamp(timestamp: i64) -> Self {
        unsafe {
            let mut tm: sys::tm = std::mem::zeroed();
            let time = timestamp as time_t;
            // Sử dụng hàm C localtime_r để chuyển đổi thời gian
            sys::localtime_r(&time, &mut tm);

            Self {
                hour: tm.tm_hour as u8,
                minute: tm.tm_min as u8,
                second: tm.tm_sec as u8,
                // Chuyển đổi format Chủ Nhật từ 0 sang 6 để thống nhất logic
                weekday: if tm.tm_wday == 0 {
                    6
                } else {
                    (tm.tm_wday - 1) as u8
                },
                day: tm.tm_mday as u8,
                month: (tm.tm_mon + 1) as u8,
                year: (tm.tm_year + 1900) as u16,
            }
        }
    }

    /// Định dạng thời gian thành chuỗi (YYYY-MM-DD HH:MM:SS)
    pub fn format(&self) -> heapless::String<32> {
        let mut s = heapless::String::new();
        use core::fmt::Write;
        write!(
            s,
            "{:04}-{:02}-{:02} {:02}:{:02}:{:02}",
            self.year, self.month, self.day, self.hour, self.minute, self.second
        )
        .ok();
        s
    }

    /// Lấy tên thứ trong tuần dạng chuỗi
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
    sntp: Arc<Mutex<Option<EspSntp<'static>>>>, // Instance SNTP được bảo vệ bởi Mutex
    timezone: String,                           // Múi giờ
    sync_status: Arc<Mutex<bool>>,              // Trạng thái đồng bộ
}

impl NtpManager {
    /// Khởi tạo NTP Manager
    pub fn new(timezone: &str) -> Result<Self> {
        info!("Initializing NTP Manager");
        info!("   Timezone: {}", timezone);

        // Thiết lập múi giờ trước khi khởi tạo SNTP
        Self::set_timezone(timezone)?;

        Ok(Self {
            sntp: Arc::new(Mutex::new(None)),
            timezone: timezone.to_string(),
            sync_status: Arc::new(Mutex::new(false)),
        })
    }

    /// Bắt đầu quá trình đồng bộ thời gian (gọi sau khi có WiFi)
    pub fn start_sync(&self) -> Result<()> {
        info!("Starting NTP synchronization...");

        // Tạo instance SNTP mặc định
        let sntp = EspSntp::new_default()?;

        *self.sntp.lock().unwrap() = Some(sntp);

        info!("   Waiting for time sync...");

        // Tạo luồng nền để chờ đồng bộ hoàn tất
        let sync_status = self.sync_status.clone();
        let sntp_clone = self.sntp.clone();

        std::thread::spawn(move || {
            let max_attempts = 30;
            let mut synced = false;

            for attempt in 1..=max_attempts {
                if let Some(ref sntp) = *sntp_clone.lock().unwrap() {
                    match sntp.get_sync_status() {
                        SyncStatus::Completed => {
                            info!("NTP sync completed (attempt {})", attempt);
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
                warn!("NTP sync timeout after {}s", max_attempts);
            }
        });

        Ok(())
    }

    /// Lấy thông tin thời gian hiện tại
    pub fn get_time(&self) -> Result<TimeInfo> {
        let timestamp = self.get_unix_timestamp()?;
        Ok(TimeInfo::from_timestamp(timestamp))
    }

    /// Lấy Unix timestamp (số giây tính từ 1970-01-01)
    pub fn get_unix_timestamp(&self) -> Result<i64> {
        let duration = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map_err(|e| anyhow::anyhow!("Time error: {}", e))?;

        Ok(duration.as_secs() as i64)
    }

    /// Kiểm tra xem đã đồng bộ thành công chưa
    pub fn is_synced(&self) -> bool {
        *self.sync_status.lock().unwrap()
    }

    /// Thiết lập biến môi trường cho múi giờ (sử dụng C API)
    fn set_timezone(tz: &str) -> Result<()> {
        use std::ffi::CString;

        let tz_str = CString::new(tz)?;

        unsafe {
            sys::setenv(b"TZ\0".as_ptr() as *const u8, tz_str.as_ptr(), 1);
            sys::tzset();
        }

        Ok(())
    }

    /// Reset và thực hiện đồng bộ lại
    pub fn resync(&self) -> Result<()> {
        info!("Resyncing NTP...");

        // Đặt lại trạng thái chưa đồng bộ
        *self.sync_status.lock().unwrap() = false;

        // Xóa instance SNTP cũ
        *self.sntp.lock().unwrap() = None;

        // Bắt đầu quy trình đồng bộ mới
        self.start_sync()?;

        Ok(())
    }

    /// Lấy chuỗi thông tin trạng thái để debug
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

/// Các múi giờ phổ biến cho Việt Nam
pub mod timezones {
    pub const VIETNAM: &str = "ICT-7";
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