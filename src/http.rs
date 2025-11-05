use anyhow::{Result, anyhow};
use embedded_svc::http::Headers;
use esp_idf_svc::http::server::{EspHttpServer, Configuration};
use esp_idf_svc::io::{Read, Write};
use crate::controller::EffectType;
use log::{info, warn};
use heapless::spsc::Producer;
use heapless::Vec;
use std::sync::{Arc, Mutex};
// Thêm thư viện để ghi chuỗi (no alloc)
use core::fmt::Write as FmtWrite;

// --- LOẠI BỎ HOÀN TOÀN: serde, SuccessResponse, ErrorResponse ---
// use serde::Serialize;
// #[derive(Debug, Serialize)] struct SuccessResponse ...
// #[derive(Debug, Serialize)] struct ErrorResponse ...


pub enum LedCommand {
    SetEffect(EffectType),
    SetBrightness(f32),
    SetColor(u8, u8, u8),
    SetSpeed(u8),
}

pub fn start_http_server(producer: Arc<Mutex<Producer<'static, LedCommand>>>) -> Result<EspHttpServer<'static>> {
    let config = Configuration::default();
    let mut server = EspHttpServer::new(&config)?;
    
    info!("HTTP Server starting on port 80 (NO ALLOC Mode)");

    const MAX_BODY_SIZE: usize = 512;

    server.fn_handler::<anyhow::Error, _>("/led", esp_idf_svc::http::Method::Post, move |mut req| {
        
        // 1. Đọc body vào buffer (trên Stack)
        let mut buf = [0u8; MAX_BODY_SIZE];
        let len = req.content_len().unwrap_or(0) as usize;

        if len == 0 || len > MAX_BODY_SIZE {
            let mut response = req.into_status_response(400)?;
            // Phản hồi lỗi (no alloc)
            response.write_all(b"{\"status\":\"error\",\"message\":\"Invalid body length\"}")?;
            return Ok(());
        }

        req.read_exact(&mut buf[..len])?;
        let body_str = match std::str::from_utf8(&buf[..len]) {
            Ok(s) => s,
            Err(_) => {
                let mut response = req.into_status_response(400)?;
                // Phản hồi lỗi (no alloc)
                response.write_all(b"{\"status\":\"error\",\"message\":\"Invalid UTF-8 body\"}")?;
                return Ok(());
            }
        };

        // 2. TỰ PHÂN TÍCH (MANUAL PARSE)
        
        let mut commands_to_send: Vec<LedCommand, 4> = Vec::new();
        
        // Thay vì struct 'SuccessResponse', chúng ta lưu các giá trị phản hồi
        // vào các biến trên Stack.
        let mut resp_mode: Option<&str> = None;
        let mut resp_brightness: Option<u8> = None;
        let mut resp_speed: Option<u8> = None;
        let mut resp_color: Option<&str> = None; // Dùng &str (tham chiếu) thay vì String

        info!("Parsing lightweight body (no alloc): '{}'", body_str);

        for pair in body_str.split('&') {
            if let Some((key, value)) = pair.split_once('=') {
                match key {
                    "mode" => {
                        let (effect, mode_str) = match value {
                            "static" => (EffectType::Static, "static"),
                            "off" => (EffectType::Off, "off"),
                            "rainbow" => (EffectType::Rainbow, "rainbow"),
                            "bass" => (EffectType::MusicBassPulse, "bass"),
                            "spectral" => (EffectType::MusicSpectral, "spectral"),
                            "vu" => (EffectType::MusicVU, "vu"),
                            _ => continue,
                        };
                        commands_to_send.push(LedCommand::SetEffect(effect));
                        resp_mode = Some(mode_str); // Lưu &str
                    }
                    "brightness" => {
                        if let Ok(val) = value.parse::<u8>() {
                            let clamped = val.min(100);
                            let brightness_val = (clamped as f32) / 100.0;
                            commands_to_send.push(LedCommand::SetBrightness(brightness_val));
                            resp_brightness = Some(clamped);
                        }
                    }
                    "speed" => {
                        if let Ok(val) = value.parse::<u8>() {
                            commands_to_send.push(LedCommand::SetSpeed(val));
                            resp_speed = Some(val);
                        }
                    }
                    "color" => {
                        if parse_hex_color(value).is_ok() {
                            let (r, g, b) = parse_hex_color(value).unwrap(); // an toàn vì đã check
                            commands_to_send.push(LedCommand::SetColor(r, g, b));
                            resp_color = Some(value); // Lưu &str (vd: "FF00CC")
                        }
                    }
                    _ => {}
                }
            }
        }
        
        // 3. Gửi lệnh (Logic này không đổi và đã là 'no alloc')
        let mut send_success = true;
        if !commands_to_send.is_empty() {
            if let Ok(mut producer_guard) = producer.try_lock() {
                for cmd in commands_to_send {
                    if producer_guard.enqueue(cmd).is_err() {
                        warn!("Command queue is full!");
                        send_success = false; // Đánh dấu thất bại
                        break;
                    }
                }
            } else {
                warn!("Mutex lock failed!");
                send_success = false; // Đánh dấu thất bại
            }
        } 
        
        // 4. Trả về Response (Tự xây dựng JSON, 'no alloc')
        if send_success {
            let mut response = req.into_ok_response()?;
            
            // Tạo một String trên Stack
            let mut resp_str = heapless::String::<128>::new(); 
            
            // Tự xây dựng JSON bằng 'write!' (không cấp phát heap)
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
            
            // Ghi buffer từ Stack ra socket
            response.write_all(resp_str.as_bytes())?;

        } else {
            // THẤT BẠI: Trả về 503 (Service Unavailable)
            let mut response = req.into_status_response(503)?;
            // Phản hồi lỗi (no alloc)
            response.write_all(b"{\"status\":\"error\",\"message\":\"Device busy or queue full\"}")?;
        }

        Ok(())
    })?;

    // Handler /status này vốn dĩ đã là 'no alloc'
    server.fn_handler::<anyhow::Error, _>("/status", esp_idf_svc::http::Method::Get, |req| {
        info!("Status requested");
        let mut response = req.into_ok_response()?;
        response.write_all(b"{\"status\":\"ok\",\"device\":\"LED Controller\",\"version\":\"3.2-no-alloc\"}")?;
        Ok(())
    })?;


    Ok(server)
}

/// Hàm helper (không đổi, vốn dĩ đã là 'no alloc')
fn parse_hex_color(s: &str) -> Result<(u8, u8, u8), ()> {
    if s.len() != 6 {
        return Err(());
    }
    let r = u8::from_str_radix(&s[0..2], 16).map_err(|_| ())?;
    let g = u8::from_str_radix(&s[2..4], 16).map_err(|_| ())?;
    let b = u8::from_str_radix(&s[4..6], 16).map_err(|_| ())?;
    Ok((r, g, b))
}