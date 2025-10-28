use std::sync::mpsc::Sender;
use anyhow::Result;
use esp_idf_svc::http::server::{EspHttpServer, Configuration};
use esp_idf_svc::io::Write;
use crate::controller::EffectType;
use log::info;

pub enum LedCommand {
    SetEffect(EffectType),
    SetBrightness(f32),
    SetColor(u8, u8, u8),
    SetSpeed(u8),
}

pub fn start_http_server(tx: Sender<LedCommand>) -> Result<EspHttpServer<'static>> {
    let config = Configuration::default();
    let mut server = EspHttpServer::new(&config)?;
    
    info!("HTTP Server starting on port 80...");

    // POST /led - Main control endpoint
    let tx_led = tx.clone();
    server.fn_handler::<anyhow::Error, _>("/led", esp_idf_svc::http::Method::Post, move |req| {
        let uri = req.uri().to_string();
        
        if let Some(query) = uri.split('?').nth(1) {
            let mut mode = None;
            let mut brightness = None;
            let mut speed = None;
            let mut r = None;
            let mut g = None;
            let mut b = None;
            
            for param in query.split('&') {
                if let Some((key, value)) = param.split_once('=') {
                    match key {
                        "mode" => mode = Some(value.to_string()),
                        "brightness" => brightness = value.parse::<u8>().ok(),
                        "speed" => speed = value.parse::<u8>().ok(),
                        "r" => r = value.parse::<u8>().ok(),
                        "g" => g = value.parse::<u8>().ok(),
                        "b" => b = value.parse::<u8>().ok(),
                        _ => {}
                    }
                }
            }
            
            let mut response_parts = Vec::new();
            let mut has_error = false;
            let mut error_msg = String::new();
            
            // Handle color FIRST
            if let (Some(red), Some(green), Some(blue)) = (r, g, b) {
                info!("LED: Set color to R:{} G:{} B:{}", red, green, blue);
                let _ = tx_led.send(LedCommand::SetColor(red, green, blue));
                response_parts.push(format!("\"color\":{{\"r\":{},\"g\":{},\"b\":{}}}", red, green, blue));
            }
            
            // Handle brightness
            if let Some(level) = brightness {
                let brightness_val = (level.min(100) as f32) / 100.0;
                info!("LED: Set brightness to {}%", level);
                let _ = tx_led.send(LedCommand::SetBrightness(brightness_val));
                response_parts.push(format!("\"brightness\":{}", level));
            }

            // Handle speed
            if let Some(spd) = speed {
                info!("LED: Set speed to {}", spd);
                let _ = tx_led.send(LedCommand::SetSpeed(spd));
                response_parts.push(format!("\"speed\":{}", spd));
            }
            
            // Handle mode LAST
            if let Some(m) = mode {
                let effect = match m.as_str() {
                    // Basic effects
                    "on" | "static" => Some(EffectType::Static),
                    "off" => Some(EffectType::Off),
                    "rainbow" => Some(EffectType::Rainbow),
                    "blink" => Some(EffectType::Blink),
                    "blink_rainbow" => Some(EffectType::BlinkRainbow),    
                    "aurora" => Some(EffectType::Aurora),
                    "meteor" => Some(EffectType::Meteor),
                    "colorwipe" => Some(EffectType::ColorWipe),
                    _ => None,
                };
                
                if let Some(eff) = effect {
                    info!("LED: Mode changed to {:?}", eff);
                    let _ = tx_led.send(LedCommand::SetEffect(eff));
                    response_parts.push(format!("\"mode\":\"{}\"", m));
                } else {
                    has_error = true;
                    error_msg = format!("Invalid mode: {}", m);
                }
            }
            
            // Send response
            if has_error {
                let mut response = req.into_status_response(400)?;
                response.write_all(
                    format!("{{\"status\":\"error\",\"message\":\"{}\"}}", error_msg).as_bytes()
                )?;
                return Ok(());
            }
            
            if !response_parts.is_empty() {
                let mut response = req.into_ok_response()?;
                response.write_all(
                    format!("{{\"status\":\"ok\",{}}}", response_parts.join(",")).as_bytes()
                )?;
                return Ok(());
            }
        }
        
        let mut response = req.into_status_response(400)?;
        response.write_all(b"{\"status\":\"error\",\"message\":\"Missing parameters\"}")?;
        Ok(())
    })?;

    // GET /status - Get current status
    server.fn_handler::<anyhow::Error, _>("/status", esp_idf_svc::http::Method::Get, |req| {
        info!("Status requested");
        let mut response = req.into_ok_response()?;
        response.write_all(b"{\"status\":\"ok\",\"device\":\"LED Controller\",\"version\":\"2.0\"}")?;
        Ok(())
    })?;

    info!("HTTP Server started successfully");
    Ok(server)
}