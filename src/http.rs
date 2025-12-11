use anyhow::Result;
use embedded_svc::http::Headers;
use esp_idf_svc::http::server::{EspHttpServer, Configuration};
use esp_idf_svc::io::{Read, Write};
use smart_leds::RGB8;
use crate::effect::EffectType;
use log::{info};
use heapless::spsc::Producer;
use std::sync::{Arc, Mutex};
use core::fmt::Write as FmtWrite;
use crate::scheduler::{LedScheduler, ScheduleAction, TimeOfDay};

pub enum LedCommand {
    SetEffect(EffectType),
    SetBrightness(f32),
    SetColor(u8, u8, u8),
    SetSpeed(u8),
    SetPowerState(bool),
}

pub fn start_http_server(
    producer: Arc<Mutex<Producer<'static, LedCommand>>>,
    scheduler: Arc<Mutex<LedScheduler>>,
) -> Result<EspHttpServer<'static>> {
    let config = Configuration::default();
    let mut server = EspHttpServer::new(&config)?;
    
    info!("HTTP Server starting on port 80");

    let producer_clone = producer.clone();
    server.fn_handler::<anyhow::Error, _>("/led", esp_idf_svc::http::Method::Post, move |mut req| {
        // Stack: ~256 bytes buffer instead of heap Vec
        let mut body_buf = [0u8; 512];
        let len = req.content_len().unwrap_or(0) as usize;
        let len = len.min(512);
        
        if len == 0 {
            let mut resp = req.into_status_response(400)?;
            resp.write_all(b"{\"status\":\"error\",\"message\":\"Empty body\"}")?;
            return Ok(());
        }
        
        req.read_exact(&mut body_buf[..len])?;
        let body_str = match std::str::from_utf8(&body_buf[..len]) {
            Ok(s) => s,
            Err(_) => {
                let mut resp = req.into_status_response(400)?;
                resp.write_all(b"{\"status\":\"error\",\"message\":\"Invalid UTF-8\"}")?;
                return Ok(());
            }
        };

        let pairs = parse_form_pairs(body_str);
        let mut commands: heapless::Vec<LedCommand, 8> = heapless::Vec::new();
        
        // Track response fields
        let mut resp_mode: Option<&str> = None;
        let mut resp_brightness: Option<u8> = None;
        let mut resp_speed: Option<u8> = None;
        let mut resp_color: Option<&str> = None;

        for (key, value) in pairs {
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
                        _ => continue,
                    };
                    let _ = commands.push(LedCommand::SetEffect(effect));
                    resp_mode = Some(mode_str);
                },
                "brightness" => {
                    if let Ok(val) = value.parse::<u8>() {
                        let clamped = val.min(100);
                        let _ = commands.push(LedCommand::SetBrightness(clamped as f32 / 100.0));
                        resp_brightness = Some(clamped);
                    }
                },
                "speed" => {
                    if let Ok(val) = value.parse::<u8>() {
                        let _ = commands.push(LedCommand::SetSpeed(val));
                        resp_speed = Some(val);
                    }
                },
                "color" => {
                    if let Ok((r,g,b)) = parse_hex_color(value) {
                        let _ = commands.push(LedCommand::SetColor(r,g,b));
                        resp_color = Some(value);
                    }
                },
                _ => {}
            }
        }

        // Send commands
        let send_ok = if !commands.is_empty() {
            producer_clone.try_lock()
                .ok()
                .and_then(|mut p| {
                    for cmd in commands {
                        if p.enqueue(cmd).is_err() { return None; }
                    }
                    Some(())
                })
                .is_some()
        } else {
            true
        };

        // Stream response directly
        if send_ok {
            let mut resp = req.into_ok_response()?;
            write!(resp, "{{\"status\":\"ok\"")?;
            if let Some(m) = resp_mode { write!(resp, ",\"mode\":\"{}\"", m)?; }
            if let Some(b) = resp_brightness { write!(resp, ",\"brightness\":{}", b)?; }
            if let Some(s) = resp_speed { write!(resp, ",\"speed\":{}", s)?; }
            if let Some(c) = resp_color { write!(resp, ",\"color\":\"{}\"", c)?; }
            write!(resp, "}}")?;
        } else {
            let mut resp = req.into_status_response(503)?;
            resp.write_all(b"{\"status\":\"error\",\"message\":\"Queue full\"}")?;
        }

        Ok(())
    })?;

    // ========================================================================
    // /schedule/add - OPTIMIZED
    // ========================================================================
    let scheduler_clone = scheduler.clone();
    server.fn_handler::<anyhow::Error, _>("/schedule/add", esp_idf_svc::http::Method::Post, move |mut req| {
        let mut body_buf = [0u8; 512];
        let len = req.content_len().unwrap_or(0) as usize;
        let len = len.min(512);
        
        if len == 0 {
            let mut resp = req.into_status_response(400)?;
            resp.write_all(b"{\"status\":\"error\",\"message\":\"Empty body\"}")?;
            return Ok(());
        }
        
        req.read_exact(&mut body_buf[..len])?;
        let body_str = std::str::from_utf8(&body_buf[..len])
            .map_err(|_| anyhow::anyhow!("UTF-8 error"))?;

        let pairs = parse_form_pairs(body_str);

        let mut power_on: Option<bool> = None;
        let mut hour: Option<u8> = None;
        let mut minute: Option<u8> = None;
        let mut days_mask = [false; 7];
        let mut effect: Option<EffectType> = None;
        let mut color: Option<RGB8> = None;
        let mut brightness: Option<f32> = None;
        let mut speed: Option<u8> = None;

        for (k, v) in pairs {
            match k {
                "action" => power_on = Some(v == "on" || v == "1" || v == "true"),
                "hour" => hour = v.parse::<u8>().ok(),
                "minute" => minute = v.parse::<u8>().ok(),
                "days" => {
                    for day_str in v.split(',') {
                        if let Ok(day) = day_str.parse::<u8>() {
                            if day < 7 { days_mask[day as usize] = true; }
                        }
                    }
                }
                "effect" => {
                    effect = match v {
                        "static" => Some(EffectType::Static),
                        "rainbow" => Some(EffectType::Rainbow),
                        "breathe" => Some(EffectType::Breathe),
                        "colorwipe" => Some(EffectType::ColorWipe),
                        "comet" => Some(EffectType::Comet),
                        "scanner" => Some(EffectType::Scanner),
                        "theaterchase" => Some(EffectType::TheaterChase),
                        "bounce" => Some(EffectType::Bounce),
                        "volumebar" => Some(EffectType::AudioVolumeBar),
                        _ => None,
                    };
                }
                "color" => {
                    if let Ok((r,g,b)) = parse_hex_color(v) {
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

        if power_on.is_none() || hour.is_none() || minute.is_none() {
            let mut resp = req.into_status_response(400)?;
            resp.write_all(b"{\"status\":\"error\",\"message\":\"Missing fields\"}")?;
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

        let action = if power_on.unwrap() {
            ScheduleAction::full(true, effect, color, brightness, speed)
        } else {
            ScheduleAction::power_off()
        };

        if let Ok(mut sched) = scheduler_clone.try_lock() {
            match sched.add_schedule(action, time, days_mask) {
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
    })?;

    // ========================================================================
    // /schedule/remove - OPTIMIZED
    // ========================================================================
    let scheduler_clone = scheduler.clone();
    server.fn_handler::<anyhow::Error, _>("/schedule/remove", esp_idf_svc::http::Method::Post, move |mut req| {
        let mut buf = [0u8; 128];
        let len = req.content_len().unwrap_or(0) as usize;
        let len = len.min(128);

        if len == 0 {
            let mut resp = req.into_status_response(400)?;
            resp.write_all(b"{\"status\":\"error\",\"message\":\"Empty body\"}")?;
            return Ok(());
        }

        req.read_exact(&mut buf[..len])?;
        let body_str = std::str::from_utf8(&buf[..len])
            .map_err(|_| anyhow::anyhow!("UTF-8 error"))?;
        
        let mut id: Option<usize> = None;
        
        for pair in body_str.split('&') {
            if let Some((key, value)) = pair.split_once('=') {
                if key == "id" {
                    id = value.parse::<usize>().ok();
                }
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
    })?;

    // ========================================================================
    // /schedule/list - STREAMING WRITE (no 2KB buffer!)
    // ========================================================================
    let scheduler_clone = scheduler.clone();
    server.fn_handler::<anyhow::Error, _>("/schedule/list", esp_idf_svc::http::Method::Get, move |req| {
        let schedules = scheduler_clone.lock().unwrap();
        let all_schedules = schedules.get_all_schedules();
        
        let mut resp = req.into_ok_response()?;
        
        // Stream directly - no intermediate buffer!
        write!(resp, "{{\"status\":\"ok\",\"schedules\":[")?;
        
        for (i, schedule) in all_schedules.iter().enumerate() {
            if i > 0 {
                write!(resp, ",")?;
            }
            
            // Use small stack buffer for days string
            let mut days_buf = heapless::String::<32>::new();
            write!(days_buf, "{}", schedule.days_string()).ok();
            
            let effect_str = schedule.effect_string();
            
            write!(resp, 
                "{{\"id\":{},\"enabled\":{},\"power\":\"{}\",\"time\":\"{:02}:{:02}\",\"days\":\"{}\",\"effect\":\"{}\"",
                schedule.id,
                schedule.enabled,
                if schedule.action.power_on { "on" } else { "off" },
                schedule.time.hour,
                schedule.time.minute,
                days_buf,
                effect_str
            )?;
            
            // Optional fields
            if let Some(color) = schedule.action.color {
                write!(resp, ",\"color\":\"{:02X}{:02X}{:02X}\"", color.r, color.g, color.b)?;
            }
            if let Some(brightness) = schedule.action.brightness {
                write!(resp, ",\"brightness\":{}", (brightness * 100.0) as u8)?;
            }
            if let Some(speed) = schedule.action.speed {
                write!(resp, ",\"speed\":{}", speed)?;
            }
            
            write!(resp, "}}")?;
        }
        
        write!(resp, "]}}")?;
        
        Ok(())
    })?;

    // ========================================================================
    // /schedule/clear
    // ========================================================================
    let scheduler_clone = scheduler.clone();
    server.fn_handler::<anyhow::Error, _>("/schedule/clear", esp_idf_svc::http::Method::Post, move |req| {
        scheduler_clone.lock().unwrap().clear_all();
        
        let mut resp = req.into_ok_response()?;
        resp.write_all(b"{\"status\":\"ok\"}")?;
        
        Ok(())
    })?;

    // ========================================================================
    // /power - OPTIMIZED
    // ========================================================================
    let producer_clone = producer.clone();
    let scheduler_clone = scheduler.clone();
    server.fn_handler::<anyhow::Error, _>("/power", esp_idf_svc::http::Method::Post, move |mut req| {
        let mut buf = [0u8; 128];
        let len = req.content_len().unwrap_or(0) as usize;
        let len = len.min(128);

        if len == 0 {
            let mut resp = req.into_status_response(400)?;
            resp.write_all(b"{\"status\":\"error\",\"message\":\"Empty body\"}")?;
            return Ok(());
        }

        req.read_exact(&mut buf[..len])?;
        let body_str = std::str::from_utf8(&buf[..len])
            .map_err(|_| anyhow::anyhow!("UTF-8 error"))?;
        
        let mut state: Option<bool> = None;
        
        for pair in body_str.split('&') {
            if let Some((key, value)) = pair.split_once('=') {
                if key == "state" {
                    state = Some(value == "on" || value == "1" || value == "true");
                }
            }
        }
        
        if let Some(state) = state {
            scheduler_clone.lock().unwrap().set_led_state(state);
            
            if let Ok(mut prod) = producer_clone.try_lock() {
                let _ = prod.enqueue(LedCommand::SetPowerState(state));
            }
            
            let mut resp = req.into_ok_response()?;
            write!(resp, "{{\"status\":\"ok\",\"state\":\"{}\"}}", 
                   if state { "on" } else { "off" })?;
        } else {
            let mut resp = req.into_status_response(400)?;
            resp.write_all(b"{\"status\":\"error\",\"message\":\"Missing state\"}")?;
        }
        
        Ok(())
    })?;

    // ========================================================================
    // Other endpoints
    // ========================================================================
    server.fn_handler::<anyhow::Error, _>("/status", esp_idf_svc::http::Method::Get, |req| {
        let mut resp = req.into_ok_response()?;
        resp.write_all(b"{\"status\":\"ok\",\"device\":\"WS2812\",\"version\":\"4.0\"}")?;
        Ok(())
    })?;

    server.fn_handler::<anyhow::Error, _>("/led", esp_idf_svc::http::Method::Options, |req| {
        let mut resp = req.into_ok_response()?;
        resp.write_all(b"")?;
        Ok(())
    })?;

    info!("✅ HTTP server ready (optimized stack usage)");
    Ok(server)
}

// ============================================================================
// Helper functions
// ============================================================================

fn parse_hex_color(s: &str) -> Result<(u8, u8, u8), ()> {
    if s.len() != 6 {
        return Err(());
    }
    
    let r = u8::from_str_radix(&s[0..2], 16).map_err(|_| ())?;
    let g = u8::from_str_radix(&s[2..4], 16).map_err(|_| ())?;
    let b = u8::from_str_radix(&s[4..6], 16).map_err(|_| ())?;
    
    Ok((r, g, b))
}

fn parse_form_pairs<'a>(body_str: &'a str) -> heapless::Vec<(&'a str, &'a str), 16> {
    let mut out: heapless::Vec<(&str, &str), 16> = heapless::Vec::new();
    for pair in body_str.split('&') {
        if let Some((k, v)) = pair.split_once('=') {
            let _ = out.push((k, v));
        }
    }
    out
}