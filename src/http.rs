use anyhow::{Result, anyhow};
use embedded_svc::http::Headers;
use esp_idf_svc::http::server::{EspHttpServer, Configuration};
use esp_idf_svc::io::{Read, Write};
use crate::controller::EffectType;
use log::{info, warn};
use heapless::spsc::Producer;
use std::sync::{Arc, Mutex};

use serde::Serialize;

pub enum LedCommand {
    SetEffect(EffectType),
    SetBrightness(f32),
    SetColor(u8, u8, u8),
    SetSpeed(u8),
}

// --- XÓA CÁC STRUCT LedRequest, ColorRequest, ModeRequest ---

#[derive(Debug, Serialize)]
struct SuccessResponse {
    status: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    mode: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    brightness: Option<u8>,
    #[serde(skip_serializing_if = "Option::is_none")]
    speed: Option<u8>,
    #[serde(skip_serializing_if = "Option::is_none")]
    color: Option<String>, // Gửi lại màu dạng Hex
}

#[derive(Debug, Serialize)]
struct ErrorResponse {
    status: String,
    message: String,
}

pub fn start_http_server(producer: Arc<Mutex<Producer<'static, LedCommand>>>) -> Result<EspHttpServer<'static>> {
    let config = Configuration::default();
    let mut server = EspHttpServer::new(&config)?;
    
    info!("HTTP Server starting on port 80 (Optimized Mode)");

    const MAX_BODY_SIZE: usize = 512;

    server.fn_handler::<anyhow::Error, _>("/led", esp_idf_svc::http::Method::Post, move |mut req| {
        
        // 1. Đọc body vào buffer (giống như trước)
        let mut buf = [0u8; MAX_BODY_SIZE];
        let len = req.content_len().unwrap_or(0) as usize;

        if len == 0 || len > MAX_BODY_SIZE {
            // (Xử lý lỗi body rỗng hoặc quá lớn)
            let mut response = req.into_status_response(400)?;
            let error = ErrorResponse { status: "error".to_string(), message: "Invalid body length".to_string() };
            response.write_all(serde_json::to_string(&error)?.as_bytes())?;
            return Ok(());
        }

        req.read_exact(&mut buf[..len])?;
        let body_str = match std::str::from_utf8(&buf[..len]) {
            Ok(s) => s,
            Err(_) => {
                // (Xử lý lỗi UTF-8)
                let mut response = req.into_status_response(400)?;
                let error = ErrorResponse { status: "error".to_string(), message: "Invalid UTF-8 body".to_string() };
                response.write_all(serde_json::to_string(&error)?.as_bytes())?;
                return Ok(());
            }
        };

        // 2. TỰ PHÂN TÍCH (MANUAL PARSE) - CỰC NHANH, KHÔNG CẦN SERDE
        // Đây là phần thay thế cho serde_json::from_slice
        
        let mut commands_to_send: Vec<LedCommand> = Vec::with_capacity(4);
        let mut success_response = SuccessResponse {
            status: "ok".to_string(),
            mode: None,
            brightness: None,
            speed: None,
            color: None,
        };

        info!("Parsing lightweight body: '{}'", body_str);

        for pair in body_str.split('&') {
            if let Some((key, value)) = pair.split_once('=') {
                match key {
                    "mode" => {
                        let (effect, mode_str) = match value {
                            "static" => (EffectType::Static, "static"),
                            "off" => (EffectType::Off, "off"),
                            "rainbow" => (EffectType::Rainbow, "rainbow"),
                            "music" => (EffectType::MusicReactive, "music"),
                            _ => continue, // Bỏ qua mode không hợp lệ
                        };
                        commands_to_send.push(LedCommand::SetEffect(effect));
                        success_response.mode = Some(mode_str.to_string());
                    }
                    "brightness" => {
                        if let Ok(val) = value.parse::<u8>() {
                            let clamped = val.min(100);
                            let brightness_val = (clamped as f32) / 100.0;
                            commands_to_send.push(LedCommand::SetBrightness(brightness_val));
                            success_response.brightness = Some(clamped);
                        }
                    }
                    "speed" => {
                        if let Ok(val) = value.parse::<u8>() {
                            commands_to_send.push(LedCommand::SetSpeed(val));
                            success_response.speed = Some(val);
                        }
                    }
                    "color" => {
                        // Phân tích màu Hex (ví dụ: FF00CC)
                        if let Ok((r, g, b)) = parse_hex_color(value) {
                            commands_to_send.push(LedCommand::SetColor(r, g, b));
                            success_response.color = Some(value.to_string());
                        }
                    }
                    _ => {
                        // Bỏ qua các key không biết
                    }
                }
            }
        }

        // 3. Gửi lệnh (Giống như trước)
        if !commands_to_send.is_empty() {
            if let Ok(mut producer_guard) = producer.lock() {
                for cmd in commands_to_send {
                    if producer_guard.enqueue(cmd).is_err() {
                        warn!("Command queue is full!");
                        break;
                    }
                }
            } else {
                warn!("Mutex lock failed!");
            }
        }

        // 4. Trả về Response
        let mut response = req.into_ok_response()?;
        response.write_all(serde_json::to_string(&success_response)?.as_bytes())?;
        Ok(())
    })?;

    // ... (handler /status giữ nguyên) ...
    server.fn_handler::<anyhow::Error, _>("/status", esp_idf_svc::http::Method::Get, |req| {
        info!("Status requested");
        let mut response = req.into_ok_response()?;
        response.write_all(b"{\"status\":\"ok\",\"device\":\"LED Controller\",\"version\":\"3.1-optimized\"}")?;
        Ok(())
    })?;


    Ok(server)
}

/// Hàm helper siêu nhẹ để phân tích màu Hex (vd: "FF00CC")
fn parse_hex_color(s: &str) -> Result<(u8, u8, u8), ()> {
    if s.len() != 6 {
        return Err(());
    }
    let r = u8::from_str_radix(&s[0..2], 16).map_err(|_| ())?;
    let g = u8::from_str_radix(&s[2..4], 16).map_err(|_| ())?;
    let b = u8::from_str_radix(&s[4..6], 16).map_err(|_| ())?;
    Ok((r, g, b))
}