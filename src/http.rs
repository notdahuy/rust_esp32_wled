use std::sync::mpsc::Sender;
use anyhow::Result;
use esp_idf_svc::http::server::{EspHttpServer, Configuration};
use esp_idf_svc::io::Write;
use crate::controller::EffectType;
use log::{info, warn};
use serde::{Deserialize, Serialize};

pub enum LedCommand {
    SetEffect(EffectType),
    SetBrightness(f32),
    SetColor(u8, u8, u8),
    SetSpeed(u8),
}

#[derive(Debug, Deserialize)]
struct ColorRequest {
    r: u8,
    g: u8,
    b: u8,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "lowercase")]
enum ModeRequest {
    On,
    Static,
    Off,
    Rainbow,

}

#[derive(Debug, Deserialize)]
struct LedRequest {
    #[serde(default)]
    mode: Option<ModeRequest>,
    
    #[serde(default)]
    brightness: Option<u8>,
    
    #[serde(default)]
    speed: Option<u8>,
    
    #[serde(default)]
    color: Option<ColorRequest>,
}

#[derive(Debug, Serialize)]
struct ColorResponse {
    r: u8,
    g: u8,
    b: u8,
}

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
    color: Option<ColorResponse>,
}

#[derive(Debug, Serialize)]
struct ErrorResponse {
    status: String,
    message: String,
}

pub fn start_http_server(tx: Sender<LedCommand>) -> Result<EspHttpServer<'static>> {
    let config = Configuration::default();
    let mut server = EspHttpServer::new(&config)?;
    
    info!("HTTP Server starting on port 80...");

    // POST /led - Main control endpoint with JSON body
    let tx_led = tx.clone();
    server.fn_handler::<anyhow::Error, _>("/led", esp_idf_svc::http::Method::Post, move |mut req| {
        // Read body
        let mut buf = vec![0u8; 512];
        let bytes_read = req.read(&mut buf)?;
        
        if bytes_read == 0 {
            let mut response = req.into_status_response(400)?;
            let error = ErrorResponse {
                status: "error".to_string(),
                message: "Empty request body".to_string(),
            };
            response.write_all(serde_json::to_string(&error)?.as_bytes())?;
            return Ok(());
        }

        // Parse JSON
        let body = &buf[..bytes_read];
        let led_req: LedRequest = match serde_json::from_slice(body) {
            Ok(req) => req,
            Err(e) => {
                warn!("JSON parse error: {}", e);
                let mut response = req.into_status_response(400)?;
                let error = ErrorResponse {
                    status: "error".to_string(),
                    message: format!("Invalid JSON: {}", e),
                };
                response.write_all(serde_json::to_string(&error)?.as_bytes())?;
                return Ok(());
            }
        };

        info!("LED Request: {:?}", led_req);

        let mut success_response = SuccessResponse {
            status: "ok".to_string(),
            mode: None,
            brightness: None,
            speed: None,
            color: None,
        };

        // Handle color FIRST
        if let Some(color) = led_req.color {
            info!("LED: Set color to R:{} G:{} B:{}", color.r, color.g, color.b);
            let _ = tx_led.send(LedCommand::SetColor(color.r, color.g, color.b));
            success_response.color = Some(ColorResponse {
                r: color.r,
                g: color.g,
                b: color.b,
            });
        }

        // Handle brightness
        if let Some(level) = led_req.brightness {
            let clamped = level.min(100);
            let brightness_val = (clamped as f32) / 100.0;
            info!("LED: Set brightness to {}%", clamped);
            let _ = tx_led.send(LedCommand::SetBrightness(brightness_val));
            success_response.brightness = Some(clamped);
        }

        // Handle speed
        if let Some(spd) = led_req.speed {
            info!("LED: Set speed to {}", spd);
            let _ = tx_led.send(LedCommand::SetSpeed(spd));
            success_response.speed = Some(spd);
        }

        // Handle mode LAST
        if let Some(mode) = led_req.mode {
            let (effect, mode_str) = match mode {
                ModeRequest::On | ModeRequest::Static => (EffectType::Static, "static"),
                ModeRequest::Off => (EffectType::Off, "off"),
                ModeRequest::Rainbow => (EffectType::Rainbow, "rainbow"),
            };
            
            info!("LED: Mode changed to {:?}", effect);
            let _ = tx_led.send(LedCommand::SetEffect(effect));
            success_response.mode = Some(mode_str.to_string());
        }

        // Send success response
        let mut response = req.into_ok_response()?;
        response.write_all(serde_json::to_string(&success_response)?.as_bytes())?;
        Ok(())
    })?;

    // GET /status - Get current status
    server.fn_handler::<anyhow::Error, _>("/status", esp_idf_svc::http::Method::Get, |req| {
        info!("Status requested");
        let mut response = req.into_ok_response()?;
        response.write_all(b"{\"status\":\"ok\",\"device\":\"LED Controller\",\"version\":\"3.0\"}")?;
        Ok(())
    })?;

    info!("HTTP Server started successfully");
    Ok(server)
}