use anyhow::Result;
use core::fmt::Write as FmtWrite;
use embedded_svc::http::Headers;
use esp_idf_hal::delay::FreeRtos;
use esp_idf_svc::http::server::{Configuration, EspHttpServer};
use esp_idf_svc::io::{Read, Write};
use heapless::spsc::Producer;
use heapless::Vec as HeaplessVec;
use log::{info, warn};
use smart_leds::RGB8;
use std::sync::{Arc, Mutex};
use std::thread;

use crate::effects::{EffectType, EFFECT_REGISTRY};
use crate::scheduler::{LedScheduler, SchedulePreset, TimeOfDay};
use crate::wifi::WifiManager;

// Enum định nghĩa các lệnh điều khiển LED gửi vào hàng đợi
pub enum LedCommand {
    SetEffect(EffectType),
    SetBrightness(f32),
    SetColor(u8, u8, u8),
    SetSpeed(u8),
    SetPowerState(bool),
}

// Hàm khởi tạo và cấu hình HTTP Server
pub fn start_http_server(
    producer: Arc<Mutex<Producer<'static, LedCommand>>>, // Kênh gửi lệnh tới LED task
    wifi_manager: Arc<WifiManager>,                      // Quản lý WiFi
    scheduler: Arc<Mutex<LedScheduler>>,                 // Quản lý lịch trình
) -> Result<EspHttpServer<'static>> {
    let config = Configuration::default();
    let mut server = EspHttpServer::new(&config)?;

    const MAX_BODY_SIZE: usize = 1024;

    // --- Endpoint điều khiển LED (/led) ---
    let producer_clone = producer.clone();
    server.fn_handler::<anyhow::Error, _>(
        "/led",
        esp_idf_svc::http::Method::Post,
        move |mut req| {
            // Đọc body request
            let mut buf = [0u8; MAX_BODY_SIZE];
            let len = req.content_len().unwrap_or(0) as usize;
            if len == 0 || len > MAX_BODY_SIZE {
                let mut response = req.into_status_response(400)?;
                response.write_all(b"{\"status\":\"error\",\"message\":\"Invalid body length\"}")?;
                return Ok(());
            }

            req.read_exact(&mut buf[..len])?;
            let body_str = match std::str::from_utf8(&buf[..len]) {
                Ok(s) => s,
                Err(_) => {
                    let mut response = req.into_status_response(400)?;
                    response.write_all(b"{\"status\":\"error\",\"message\":\"Invalid UTF-8\"}")?;
                    return Ok(());
                }
            };

            // Phân tích tham số từ body (dạng key=value&...)
            let mut commands_to_send: HeaplessVec<LedCommand, 8> = HeaplessVec::new();
            let mut resp_mode: Option<&str> = None;
            let mut resp_brightness: Option<u8> = None;
            let mut resp_speed: Option<u8> = None;
            let mut resp_color: Option<&str> = None;

            for pair in body_str.split('&') {
                if let Some((key, value)) = pair.split_once('=') {
                    match key {
                        "mode" => {
                            if let Some(&effect) = EFFECT_REGISTRY.get(value) {
                                if commands_to_send
                                    .push(LedCommand::SetEffect(effect))
                                    .is_err()
                                {
                                    warn!("Command buffer full, ignoring mode");
                                    continue;
                                }
                                resp_mode = Some(value);
                            } else {
                                warn!("Unknown mode: {}", value);
                            }
                        }
                        "brightness" => {
                            if let Ok(val) = value.parse::<u8>() {
                                let clamped = val.min(100);
                                let brightness_val = (clamped as f32) / 100.0;
                                if commands_to_send
                                    .push(LedCommand::SetBrightness(brightness_val))
                                    .is_err()
                                {
                                    continue;
                                }
                                resp_brightness = Some(clamped);
                            }
                        }
                        "speed" => {
                            if let Ok(val) = value.parse::<u8>() {
                                if commands_to_send.push(LedCommand::SetSpeed(val)).is_err() {
                                    continue;
                                }
                                resp_speed = Some(val);
                            }
                        }
                        "color" => match parse_hex_color(value) {
                            Ok((r, g, b)) => {
                                if commands_to_send
                                    .push(LedCommand::SetColor(r, g, b))
                                    .is_err()
                                {
                                    continue;
                                }
                                resp_color = Some(value);
                            }
                            Err(_) => warn!("Invalid color: {}", value),
                        },
                        _ => warn!("Unknown parameter: {}", key),
                    }
                }
            }

            // Gửi các lệnh vào hàng đợi
            let mut send_success = true;
            if !commands_to_send.is_empty() {
                match producer_clone.try_lock() {
                    Ok(mut producer_guard) => {
                        for cmd in commands_to_send {
                            if producer_guard.enqueue(cmd).is_err() {
                                send_success = false;
                                break;
                            }
                        }
                    }
                    Err(_) => send_success = false,
                }
            } else {
                send_success = false;
            }

            // Phản hồi JSON kết quả
            if send_success {
                let mut response = req.into_ok_response()?;
                let mut resp_str = heapless::String::<256>::new();
                write!(resp_str, "{{\"status\":\"ok\"").unwrap();
                if let Some(mode) = resp_mode {
                    write!(resp_str, ",\"mode\":\"{}\"", mode).unwrap();
                }
                if let Some(brightness) = resp_brightness {
                    write!(resp_str, ",\"brightness\":{}", brightness).unwrap();
                }
                if let Some(speed) = resp_speed {
                    write!(resp_str, ",\"speed\":{}", speed).unwrap();
                }
                if let Some(color) = resp_color {
                    write!(resp_str, ",\"color\":\"{}\"", color).unwrap();
                }
                write!(resp_str, "}}").unwrap();

                response.write_all(resp_str.as_bytes())?;
            } else {
                let mut response = req.into_status_response(503)?;
                response.write_all(b"{\"status\":\"error\",\"message\":\"Command failed\"}")?;
            }

            Ok(())
        },
    )?;

    // --- Endpoint quét WiFi (/wifi/scan) ---
    let wifi_mgr_scan = wifi_manager.clone();
    server.fn_handler::<anyhow::Error, _>(
        "/wifi/scan",
        esp_idf_svc::http::Method::Get,
        move |req| {
            match wifi_mgr_scan.scan_networks() {
                Ok(networks) => {
                    let mut response = req.into_ok_response()?;
                    response.write_all(b"{\"networks\":[")?;

                    let max_networks = networks.len().min(25); // Giới hạn 25 mạng
                    for (i, net) in networks.iter().take(max_networks).enumerate() {
                        if i > 0 {
                            response.write_all(b", ")?;
                        }
                        let mut item = heapless::String::<256>::new();
                        write!(
                            item,
                            "{{\"ssid\":\"{}\",\"rssi\":{},\"auth\":\"{}\"}}",
                            net.ssid, net.rssi, net.auth
                        )
                        .unwrap();
                        response.write_all(item.as_bytes())?;
                    }
                    response.write_all(b"]}")?;
                }
                Err(e) => {
                    warn!("Scan failed: {:?}", e);
                    let mut response = req.into_status_response(500)?;
                    response.write_all(b"{\"status\":\"error\",\"message\":\"Scan failed\"}")?;
                }
            }
            Ok(())
        },
    )?;

    // --- Endpoint kết nối WiFi (/wifi/connect) ---
    let wifi_mgr_connect = wifi_manager.clone();
    server.fn_handler::<anyhow::Error, _>(
        "/wifi/connect",
        esp_idf_svc::http::Method::Post,
        move |mut req| {
            info!("WiFi connect requested");
            let mut buf = [0u8; MAX_BODY_SIZE];
            let len = req.content_len().unwrap_or(0) as usize;
            if len == 0 || len > MAX_BODY_SIZE {
                let mut response = req.into_status_response(400)?;
                response.write_all(b"{\"status\":\"error\",\"message\":\"Invalid body\"}")?;
                return Ok(());
            }

            req.read_exact(&mut buf[..len])?;
            let body_str = std::str::from_utf8(&buf[..len]).unwrap_or("");
            let mut ssid: Option<String> = None;
            let mut password: Option<String> = None;

            // Parse tham số ssid và password
            for pair in body_str.split('&') {
                if let Some((key, value)) = pair.split_once('=') {
                    match key {
                        "ssid" => {
                            let decoded = url_decode(value);
                            info!("   Raw SSID: '{}' -> Decoded: '{}'", value, decoded);
                            ssid = Some(decoded);
                        }
                        "password" => {
                            let decoded = url_decode(value);
                            password = Some(decoded);
                        }
                        _ => {}
                    }
                }
            }

            if let (Some(ssid), Some(pass)) = (ssid, password) {
                info!("Attempting to connect to: {}", ssid);
                match wifi_mgr_connect.save_credentials(&ssid, &pass) {
                    Ok(_) => {
                        let mut response = req.into_ok_response()?;
                        response.write_all(
                            b"{\"status\":\"ok\",\"message\":\"Saved! Reconnecting in 2 seconds...\"}"
                        )?;
                        
                        // Tạo luồng riêng để thực hiện kết nối lại
                        let wifi_mgr_clone = wifi_mgr_connect.clone();
                        thread::spawn(move || {
                            info!("Waiting 2 seconds before reconnecting...");
                            FreeRtos::delay_ms(2000);

                            info!("Starting reconnection...");
                            if let Err(e) = wifi_mgr_clone.reconnect_saved() {
                                warn!("Reconnection failed: {:?}", e);
                            } else {
                                info!("Reconnection completed");
                            }
                        });

                        info!("Response sent, reconnection scheduled");
                    }
                    Err(e) => {
                        warn!("Connection failed: {:?}", e);
                        let mut response = req.into_status_response(400)?;
                        response.write_all(
                            b"{\"status\":\"error\",\"message\":\"Connection failed. Check credentials.\"}"
                        )?;
                    }
                }
            } else {
                let mut response = req.into_status_response(400)?;
                response
                    .write_all(b"{\"status\":\"error\",\"message\":\"Missing ssid or password\"}")?;
            }
            Ok(())
        },
    )?;

    // --- Endpoint trạng thái WiFi (/wifi/status) ---
    let wifi_mgr_status = wifi_manager.clone();
    server.fn_handler::<anyhow::Error, _>(
        "/wifi/status",
        esp_idf_svc::http::Method::Get,
        move |req| {
            let status = wifi_mgr_status.get_status();
            let mut response = req.into_ok_response()?;
            let mut json = heapless::String::<256>::new();
            write!(json, "{{\"connected\":{}", status.connected).unwrap();
            if let Some(ip) = status.ip {
                write!(json, ",\"ip\":\"{}\"", ip).unwrap();
            }
            write!(json, "}}").unwrap();
            response.write_all(json.as_bytes())?;
            Ok(())
        },
    )?;

    // --- Endpoint xóa thông tin WiFi (/wifi/credentials) ---
    let wifi_mgr_clear = wifi_manager.clone();
    server.fn_handler::<anyhow::Error, _>(
        "/wifi/credentials",
        esp_idf_svc::http::Method::Delete,
        move |req| {
            info!("Clear credentials requested");
            match wifi_mgr_clear.clear_credentials() {
                Ok(_) => {
                    let mut response = req.into_ok_response()?;
                    response.write_all(b"{\"status\":\"ok\",\"message\":\"Credentials cleared. Restart to reconfigure.\"}")?;
                    
                    let wifi_mgr_clone = wifi_mgr_clear.clone();
                    thread::spawn(move || {
                        info!("Waiting 2 seconds before restarting...");
                        FreeRtos::delay_ms(2000);

                        info!("Restarting AP mode...");
                        if let Err(e) = wifi_mgr_clone.restart_ap_mode() {
                            warn!("Restart failed: {:?}", e);
                        } else {
                            info!("AP mode restarted");
                        }
                    });
                }
                Err(e) => {
                    warn!("Clear failed: {:?}", e);
                    let mut response = req.into_status_response(500)?;
                    response.write_all(b"{\"status\":\"error\"}")?;
                }
            }
            Ok(())
        },
    )?;

    // --- Endpoint thêm lịch trình (/schedule/add) ---
    let scheduler_clone = scheduler.clone();
    server.fn_handler::<anyhow::Error, _>(
        "/schedule/add",
        esp_idf_svc::http::Method::Post,
        move |mut req| {
            let mut body_buf = [0u8; 256];
            let len = req.content_len().unwrap_or(0) as usize;
            let len = len.min(256);
            if len == 0 {
                let mut resp = req.into_status_response(400)?;
                resp.write_all(b"{\"status\":\"error\",\"message\":\"Empty body\"}")?;
                return Ok(());
            }

            req.read_exact(&mut body_buf[..len])?;
            let body_str = String::from_utf8_lossy(&body_buf[..len]).into_owned();
            let pairs = parse_form_pairs(&body_str);
            let mut hour: Option<u8> = None;
            let mut minute: Option<u8> = None;
            let mut days = [false; 7];
            let mut mode = "static";
            let mut color: Option<RGB8> = None;
            let mut brightness: Option<f32> = None;
            let mut speed: Option<u8> = None;

            for (k, v) in pairs {
                match k {
                    "hour" => hour = v.parse::<u8>().ok(),
                    "minute" => minute = v.parse::<u8>().ok(),
                    "days" => {
                        let decoded = url_decode(v);
                        for day_str in decoded.split(',') {
                            if let Ok(day) = day_str.parse::<u8>() {
                                if day < 7 {
                                    days[day as usize] = true;
                                }
                            }
                        }
                    }
                    "mode" => mode = v,
                    "color" => {
                        if let Ok((r, g, b)) = parse_hex_color(v) {
                            color = Some(RGB8 { r, g, b });
                        }
                    }
                    "brightness" => {
                        if let Ok(val) = v.parse::<u8>() {
                            brightness = Some((val.min(100) as f32) / 100.0);
                        }
                    }
                    "speed" => speed = v.parse::<u8>().ok(),
                    _ => {}
                }
            }

            // Kiểm tra tham số bắt buộc
            if hour.is_none() || minute.is_none() {
                let mut resp = req.into_status_response(400)?;
                resp.write_all(b"{\"status\":\"error\",\"message\":\"Missing hour or minute\"}")?;
                return Ok(());
            }
            let time = match TimeOfDay::new(hour.unwrap(), minute.unwrap()) {
                Ok(t) => t,
                Err(_) => {
                    let mut resp = req.into_status_response(400)?;
                    resp.write_all(b"{\"status\":\"error\",\"message\":\"Invalid time\"}")?;
                    return Ok(());
                }
            };

            // Xác định effect từ mode string
            let effect = if let Some(&effect_type) = EFFECT_REGISTRY.get(mode) {
                effect_type
            } else {
                EffectType::Static
            };

            // Tạo cấu hình preset cho lịch
            let preset = if mode == "off" {
                // Tắt = Static màu đen
                SchedulePreset {
                    effect: EffectType::Static,
                    color: Some(RGB8 { r: 0, g: 0, b: 0 }),
                    brightness: None,
                    speed: None,
                }
            } else {
                // Bật với effect và màu từ request
                SchedulePreset::with_all(effect, color, brightness, speed)
            };

            // Thêm vào scheduler
            if let Ok(mut sched) = scheduler_clone.try_lock() {
                match sched.add_schedule(preset, time, days) {
                    Ok(id) => {
                        let mut resp = req.into_ok_response()?;
                        write!(resp, "{{\"status\":\"ok\",\"schedule_id\":{}}}", id)?;
                    }
                    Err(e) => {
                        let mut resp = req.into_status_response(400)?;
                        write!(resp, "{{\"status\":\"error\",\"message\":\"{}\"}}", e)?;
                    }
                }
            } else {
                let mut resp = req.into_status_response(503)?;
                resp.write_all(b"{\"status\":\"error\",\"message\":\"Busy\"}")?;
            }

            Ok(())
        },
    )?;

    // --- Endpoint xóa lịch trình (/schedule/remove) ---
    let scheduler_clone = scheduler.clone();
    server.fn_handler::<anyhow::Error, _>(
        "/schedule/remove",
        esp_idf_svc::http::Method::Post,
        move |mut req| {
            let mut buf = [0u8; 128];
            let len = req.content_len().unwrap_or(0) as usize;
            let len = len.min(128);

            if len == 0 {
                let mut resp = req.into_status_response(400)?;
                resp.write_all(b"{\"status\":\"error\",\"message\":\"Empty body\"}")?;
                return Ok(());
            }

            req.read_exact(&mut buf[..len])?;
            let body_str = String::from_utf8_lossy(&buf[..len]).into_owned();
            let pairs = parse_form_pairs(&body_str);
            let mut id: Option<usize> = None;

            for (key, value) in pairs {
                if key == "id" {
                    id = value.parse::<usize>().ok();
                    break;
                }
            }

            if let Some(id) = id {
                let success = scheduler_clone.lock().unwrap().remove_schedule(id);

                let mut resp = req.into_ok_response()?;
                if success {
                    resp.write_all(b"{\"status\":\"ok\"}")?;
                } else {
                    resp.write_all(b"{\"status\":\"error\",\"message\":\"Not found\"}")?;
                }
            } else {
                let mut resp = req.into_status_response(400)?;
                resp.write_all(b"{\"status\":\"error\",\"message\":\"Missing id\"}")?;
            }

            Ok(())
        },
    )?;

    // --- Endpoint lấy danh sách lịch (/schedule/list) ---
    let scheduler_clone = scheduler.clone();
    server.fn_handler::<anyhow::Error, _>(
        "/schedule/list",
        esp_idf_svc::http::Method::Get,
        move |req| {
            let schedules = scheduler_clone.lock().unwrap();
            let all_schedules = schedules.get_all_schedules();
            let mut resp = req.into_ok_response()?;
            write!(resp, "{{\"status\":\"ok\",\"schedules\":[")?;

            for (i, schedule) in all_schedules.iter().enumerate() {
                if i > 0 {
                    write!(resp, ",")?;
                }
                // Lấy thông tin days
                let mut days_buf = heapless::String::<32>::new();
                write!(days_buf, "{}", schedule.days_string()).ok();
                
                // Xác định schedule là "on" hay "off"
                let is_off = schedule.is_off();
                let action = if is_off { "off" } else { "on" };
                let effect_str = schedule.effect_string();
                
                write!(
                    resp,
                    "{{\"id\":{},\"enabled\":{},\"action\":\"{}\",\"time\":\"{:02}:{:02}\",\"days\":\"{}\",\"mode\":\"{}\"",
                    schedule.id,
                    schedule.enabled,
                    action,
                    schedule.time.hour,
                    schedule.time.minute,
                    days_buf,
                    effect_str
                )?;
                
                // Thêm các trường tùy chọn
                if let Some(color) = schedule.preset.color {
                    write!(
                        resp,
                        ",\"color\":\"{:02X}{:02X}{:02X}\"",
                        color.r, color.g, color.b
                    )?;
                }
                if let Some(brightness) = schedule.preset.brightness {
                    write!(resp, ",\"brightness\":{}", (brightness * 100.0) as u8)?;
                }
                if let Some(speed) = schedule.preset.speed {
                    write!(resp, ",\"speed\":{}", speed)?;
                }
                write!(resp, "}}")?;
            }
            write!(resp, "]}}")?;
            Ok(())
        },
    )?;

    // --- Endpoint bật/tắt lịch trình (/schedule/toggle) ---
    let scheduler_clone = scheduler.clone();
    server.fn_handler::<anyhow::Error, _>(
        "/schedule/toggle",
        esp_idf_svc::http::Method::Post,
        move |mut req| {
            let mut buf = [0u8; 128];
            let len = req.content_len().unwrap_or(0) as usize;
            let len = len.min(128);
            if len == 0 {
                let mut resp = req.into_status_response(400)?;
                resp.write_all(b"{\"status\":\"error\",\"message\":\"Empty body\"}")?;
                return Ok(());
            }

            req.read_exact(&mut buf[..len])?;
            let body_str = String::from_utf8_lossy(&buf[..len]).into_owned();
            let pairs = parse_form_pairs(&body_str);
            let mut id: Option<usize> = None;
            let mut enable: Option<bool> = None;
            for (key, value) in pairs {
                match key {
                    "id" => id = value.parse::<usize>().ok(),
                    "enable" => enable = Some(value == "true" || value == "1"),
                    _ => {}
                }
            }

            if let (Some(id), Some(enable)) = (id, enable) {
                let success = scheduler_clone.lock().unwrap().toggle_schedule(id, enable);
                let mut resp = req.into_ok_response()?;
                if success {
                    resp.write_all(b"{\"status\":\"ok\"}")?;
                } else {
                    resp.write_all(b"{\"status\":\"error\",\"message\":\"Not found\"}")?;
                }
            } else {
                let mut resp = req.into_status_response(400)?;
                resp.write_all(b"{\"status\":\"error\",\"message\":\"Missing id or enable\"}")?;
            }
            Ok(())
        },
    )?;

    // --- Endpoint bật/tắt nguồn (/power) ---
    let producer_clone = producer.clone();
    server.fn_handler::<anyhow::Error, _>(
        "/power",
        esp_idf_svc::http::Method::Post,
        move |mut req| {
            let mut buf = [0u8; 128];
            let len = req.content_len().unwrap_or(0) as usize;
            let len = len.min(128);
            if len == 0 {
                let mut resp = req.into_status_response(400)?;
                resp.write_all(b"{\"status\":\"error\",\"message\":\"Empty body\"}")?;
                return Ok(());
            }

            req.read_exact(&mut buf[..len])?;
            let body_str = String::from_utf8_lossy(&buf[..len]).into_owned();
            let pairs = parse_form_pairs(&body_str);
            let mut state: Option<bool> = None;

            for (key, value) in pairs {
                if key == "state" {
                    state = Some(value == "on" || value == "1" || value == "true");
                }
            }
            if let Some(state) = state {
                if let Ok(mut prod) = producer_clone.try_lock() {
                    let _ = prod.enqueue(LedCommand::SetPowerState(state));
                }

                let mut resp = req.into_ok_response()?;
                write!(
                    resp,
                    "{{\"status\":\"ok\",\"state\":\"{}\"}}",
                    if state { "on" } else { "off" }
                )?;
            } else {
                let mut resp = req.into_status_response(400)?;
                resp.write_all(b"{\"status\":\"error\",\"message\":\"Missing state\"}")?;
            }
            Ok(())
        },
    )?;

    // --- Các Endpoint thông tin chung ---
    server.fn_handler::<anyhow::Error, _>("/status", esp_idf_svc::http::Method::Get, |req| {
        let mut response = req.into_ok_response()?;
        response
            .write_all(b"{\"status\":\"ok\",\"device\":\"ESP32-LED\",\"version\":\"4.0\"}")?;
        Ok(())
    })?;

    // --- Xử lý OPTIONS cho CORS ---
    server.fn_handler::<anyhow::Error, _>("/led", esp_idf_svc::http::Method::Options, |req| {
        let mut response = req.into_ok_response()?;
        response.write_all(b"")?;
        Ok(())
    })?;

    server.fn_handler::<anyhow::Error, _>(
        "/wifi/scan",
        esp_idf_svc::http::Method::Options,
        |req| {
            let mut response = req.into_ok_response()?;
            response.write_all(b"")?;
            Ok(())
        },
    )?;

    server.fn_handler::<anyhow::Error, _>(
        "/wifi/connect",
        esp_idf_svc::http::Method::Options,
        |req| {
            let mut response = req.into_ok_response()?;
            response.write_all(b"")?;
            Ok(())
        },
    )?;

    info!("HTTP server configured successfully");
    Ok(server)
}

// Hàm phụ trợ: Escape chuỗi JSON để đảm bảo format đúng
fn json_escape(input: &str) -> heapless::String<256> {
    let mut escaped = heapless::String::<256>::new();
    for c in input.chars() {
        match c {
            '"' => {
                let _ = escaped.push_str("\\\"");
            }
            '\\' => {
                let _ = escaped.push_str("\\\\");
            }
            '\u{08}' => {
                let _ = escaped.push_str("\\b");
            }
            '\u{0C}' => {
                let _ = escaped.push_str("\\f");
            }
            '\n' => {
                let _ = escaped.push_str("\\n");
            }
            '\r' => {
                let _ = escaped.push_str("\\r");
            }
            '\t' => {
                let _ = escaped.push_str("\\t");
            }
            _ if c < ' ' || c > '~' => {
                // Các ký tự không in được hoặc ngoài ASCII cơ bản
                let _ = write!(escaped, "\\u{:04x}", c as u32);
            }
            _ => {
                let _ = escaped.push(c);
            }
        }
    }
    escaped
}

// Hàm phụ trợ: Decode URL encoded string (ví dụ: %20 -> space)
fn url_decode(s: &str) -> String {
    let mut result = String::new();
    let mut chars = s.chars();

    while let Some(c) = chars.next() {
        match c {
            '+' => result.push(' '), // + = space
            '%' => {
                // %XX hex encoding
                if let (Some(h1), Some(h2)) = (chars.next(), chars.next()) {
                    if let Ok(byte) = u8::from_str_radix(&format!("{}{}", h1, h2), 16) {
                        result.push(byte as char);
                    }
                }
            }
            _ => result.push(c),
        }
    }
    result
}

// Hàm phụ trợ: Parse màu hex (RRGGBB) thành (u8, u8, u8)
fn parse_hex_color(s: &str) -> Result<(u8, u8, u8), ()> {
    if s.len() != 6 {
        return Err(());
    }

    let r = u8::from_str_radix(&s[0..2], 16).map_err(|_| ())?;
    let g = u8::from_str_radix(&s[2..4], 16).map_err(|_| ())?;
    let b = u8::from_str_radix(&s[4..6], 16).map_err(|_| ())?;

    Ok((r, g, b))
}

// Hàm phụ trợ: Parse chuỗi form data (key=value&key2=value2)
fn parse_form_pairs<'a>(body_str: &'a str) -> heapless::Vec<(&'a str, &'a str), 16> {
    let mut out: heapless::Vec<(&str, &str), 16> = heapless::Vec::new();
    for pair in body_str.split('&') {
        if let Some((k, v)) = pair.split_once('=') {
            let _ = out.push((k, v));
        }
    }
    out
}