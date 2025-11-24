use anyhow::Result;
use embedded_svc::http::Headers;
use esp_idf_svc::http::server::{EspHttpServer, Configuration};
use esp_idf_svc::io::{Read, Write};
use crate::effect::EffectType;
use log::{info, warn};
use heapless::spsc::Producer;
use heapless::Vec as HeaplessVec;
use std::sync::{Arc, Mutex};
use core::fmt::Write as FmtWrite;

pub enum LedCommand {
    SetEffect(EffectType),
    SetBrightness(f32),
    SetColor(u8, u8, u8),
    SetSpeed(u8),
}

pub fn start_http_server(producer: Arc<Mutex<Producer<'static, LedCommand>>>) -> Result<EspHttpServer<'static>> {
    let config = Configuration::default();
    let mut server = EspHttpServer::new(&config)?;
    
    info!("HTTP Server starting on port 80");

    const MAX_BODY_SIZE: usize = 512;

    server.fn_handler::<anyhow::Error, _>("/led", esp_idf_svc::http::Method::Post, move |mut req| {
        
        // Read body into buffer
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
        
        info!("Received: '{}'", body_str);
        
        // Parse commands (support up to 4 commands per request)
        let mut commands_to_send: HeaplessVec<LedCommand, 4> = HeaplessVec::new();
        
        // Response tracking
        let mut resp_mode: Option<&str> = None;
        let mut resp_brightness: Option<u8> = None;
        let mut resp_speed: Option<u8> = None;
        let mut resp_color: Option<&str> = None;

        // Parse form-urlencoded body: key=value&key=value
        for pair in body_str.split('&') {
            if let Some((key, value)) = pair.split_once('=') {
                match key {
                    "mode" => {
                        let (effect, mode_str) = match value {
                            "static" => (EffectType::Static, "static"),
                            "rainbow" => (EffectType::Rainbow, "rainbow"),
                            "breathe" => (EffectType::Breathe, "breathe"),
                            "colorwipe" => (EffectType::ColorWipe, "colorwipe"),
                            "comet" => (EffectType::Comet, "comet"),
                            "scanner" => (EffectType::Scanner, "scanner"),
                            "theaterchase" => (EffectType::TheaterChase, "theaterchase"),
                            "bounce" => (EffectType::Bounce, "bounce"),
                            "volumebar" => (EffectType::AudioVolumeBar, "volumebar"),
                            _ => {
                                warn!("Unknown mode: {}", value);
                                continue;
                            }
                        };
                        
                        // Prevent buffer overflow
                        if commands_to_send.push(LedCommand::SetEffect(effect)).is_err() {
                            warn!("Command buffer full, ignoring mode");
                            continue;
                        }
                        resp_mode = Some(mode_str);
                    }
                    
                    "brightness" => {
                        if let Ok(val) = value.parse::<u8>() {
                            let clamped = val.min(100);
                            let brightness_val = (clamped as f32) / 100.0;
                            
                            if commands_to_send.push(LedCommand::SetBrightness(brightness_val)).is_err() {
                                warn!("Command buffer full, ignoring brightness");
                                continue;
                            }
                            resp_brightness = Some(clamped);
                        } else {
                            warn!("Invalid brightness value: {}", value);
                        }
                    }
                    
                    "speed" => {
                        if let Ok(val) = value.parse::<u8>() {
                            if commands_to_send.push(LedCommand::SetSpeed(val)).is_err() {
                                warn!("Command buffer full, ignoring speed");
                                continue;
                            }
                            resp_speed = Some(val);
                        } else {
                            warn!("Invalid speed value: {}", value);
                        }
                    }
                    
                    "color" => {
                        match parse_hex_color(value) {
                            Ok((r, g, b)) => {
                                if commands_to_send.push(LedCommand::SetColor(r, g, b)).is_err() {
                                    warn!("Command buffer full, ignoring color");
                                    continue;
                                }
                                resp_color = Some(value);
                                info!("Color parsed: #{:02X}{:02X}{:02X}", r, g, b);
                            }
                            Err(_) => {
                                warn!("Invalid color format: {} (expected: RRGGBB)", value);
                            }
                        }
                    }
                    
                    _ => {
                        warn!("Unknown parameter: {}", key);
                    }
                }
            }
        }
        
        // Send commands to LED task
        let mut send_success = true;
        
        if !commands_to_send.is_empty() {
            match producer.try_lock() {
                Ok(mut producer_guard) => {
                    for cmd in commands_to_send {
                        if producer_guard.enqueue(cmd).is_err() {
                            warn!("⚠️ Command queue is FULL!");
                            send_success = false;
                            break;
                        }
                    }
                }
                Err(_) => {
                    warn!("⚠️ Mutex lock failed - concurrent access!");
                    send_success = false;
                }
            }
        } else {
            warn!("No valid commands parsed from body");
            send_success = false;
        }
        
        // Build response
        if send_success {
            let mut response = req.into_ok_response()?;
            
            // Build JSON response on stack (no heap allocation)
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
            
            info!("Response: {}", resp_str.as_str());
            response.write_all(resp_str.as_bytes())?;

        } else {
            let mut response = req.into_status_response(503)?;
            response.write_all(b"{\"status\":\"error\",\"message\":\"Device busy or invalid params\"}")?;
        }

        Ok(())
    })?;

    server.fn_handler::<anyhow::Error, _>("/status", esp_idf_svc::http::Method::Get, |req| {
        info!("Status requested");
        let mut response = req.into_ok_response()?;
        response.write_all(
            b"{\"status\":\"ok\",\"device\":\"WS2812 Controller\",\"version\":\"3.3\",\"firmware\":\"esp32-rust\"}"
        )?;
        Ok(())
    })?;

    server.fn_handler::<anyhow::Error, _>("/led", esp_idf_svc::http::Method::Options, |req| {
        let mut response = req.into_ok_response()?;
        response.write_all(b"")?;
        Ok(())
    })?;

    info!("✅ HTTP server configured successfully");
    Ok(server)
}

fn parse_hex_color(s: &str) -> Result<(u8, u8, u8), ()> {
    if s.len() != 6 {
        return Err(());
    }
    
    let r = u8::from_str_radix(&s[0..2], 16).map_err(|_| ())?;
    let g = u8::from_str_radix(&s[2..4], 16).map_err(|_| ())?;
    let b = u8::from_str_radix(&s[4..6], 16).map_err(|_| ())?;
    
    Ok((r, g, b))
}