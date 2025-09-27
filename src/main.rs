use embedded_svc::{http::Headers};
use esp_idf_hal::io::Read;
use esp_idf_hal::peripherals::Peripherals;
use esp_idf_svc::http::server::EspHttpServer;
use esp_idf_svc::http::Method;
use esp_idf_svc::eventloop::EspSystemEventLoop;
use esp_idf_sys as _;
use esp_idf_svc::log::EspLogger;
use esp_idf_svc::nvs::EspDefaultNvsPartition;
use esp_idf_svc::timer::EspTaskTimerService;
use esp_idf_svc::http::server::Configuration;
use log::*;
use ws2812_esp32_rmt_driver::Ws2812Esp32RmtDriver;
use std::sync::{Arc, Mutex, Condvar};
use std::time::Duration;

// Import các module con
use types::*;
use led_controller::{update_leds, turn_off_leds};

mod wifi;
mod types;
mod led_controller;

fn main() -> anyhow::Result<()> {
    esp_idf_sys::link_patches();
    EspLogger::initialize_default();

    info!("Khởi tạo ESP32 LED Controller");

    let peripherals = Peripherals::take().unwrap();
    let sysloop = EspSystemEventLoop::take().unwrap();
    let timer_service = EspTaskTimerService::new().unwrap();

    let _wifi = wifi::wifi(
        peripherals.modem, 
        sysloop, 
        Some(EspDefaultNvsPartition::take().unwrap()), 
        timer_service
    )?;

    let channel = peripherals.rmt.channel0;
    let led_pin = peripherals.pins.gpio18;
    let ws = Ws2812Esp32RmtDriver::new(channel, led_pin)?;
    let ws_driver = Arc::new(Mutex::new(ws));

    let mut server = EspHttpServer::new(&Configuration::default())?;
    let current_color = Arc::new(Mutex::new(LedState::default()));

    // Pair Condvar để wake up thread hiệu ứng
    let effect_signal = Arc::new((Mutex::new(false), Condvar::new()));

    // Bật LED
    let ws_on = Arc::clone(&ws_driver);
    let current_color_on = Arc::clone(&current_color);
    server.fn_handler("/on", Method::Get, move |req| -> Result<(), anyhow::Error> {
        let state = current_color_on.lock().unwrap();
        update_leds(&ws_on, &state)?;
        
        let mut resp = req.into_ok_response().unwrap();
        resp.write(b"LED ON")?;
        Ok(())
    })?;

    // Tắt LED
    let ws_off = Arc::clone(&ws_driver);
    server.fn_handler("/off", Method::Get, move |req| -> Result<(), anyhow::Error> {
        turn_off_leds(&ws_off)?;
        
        let mut resp = req.into_ok_response().unwrap();
        resp.write(b"LED OFF")?;
        Ok(())
    })?;

    // Set màu LED
    let ws_set = Arc::clone(&ws_driver);
    let current_color_set = Arc::clone(&current_color);
    let effect_signal_set = Arc::clone(&effect_signal);
    server.fn_handler("/set", Method::Post, move |mut req| -> Result<(), anyhow::Error> {
        let content_length = req.content_len()
            .ok_or_else(|| anyhow::anyhow!("Content-Length header missing"))?;

        let mut buffer = vec![0u8; content_length.try_into().unwrap()];
        req.read_exact(&mut buffer)?;

        let color_req: ColorRequest = serde_json::from_slice(&buffer)
            .map_err(|e| anyhow::anyhow!("Invalid JSON: {}", e))?;
        
        {
            let mut state = current_color_set.lock().unwrap();
            state.r = color_req.r;
            state.g = color_req.g;
            state.b = color_req.b;
            update_leds(&ws_set, &state)?;
        }

        // Báo cho thread hiệu ứng biết có thay đổi
        let (lock, cvar) = &*effect_signal_set;
        let mut changed = lock.lock().unwrap();
        *changed = true;
        cvar.notify_one();

        let mut resp = req.into_ok_response().unwrap();
        let msg = format!("Set color: R={}, G={}, B={}", color_req.r, color_req.g, color_req.b);
        resp.write(msg.as_bytes())?;
        Ok(())
    })?;

    // Endpoint chỉnh độ sáng
    let ws_brightness = Arc::clone(&ws_driver);
    let current_color_brightness = Arc::clone(&current_color);
    let effect_signal_brightness = Arc::clone(&effect_signal);
    server.fn_handler("/brightness", Method::Post, move |mut req| -> Result<(), anyhow::Error> {
        let content_length = req.content_len()
            .ok_or_else(|| anyhow::anyhow!("Content-Length header missing"))?;

        let mut buffer = vec![0u8; content_length.try_into().unwrap()];
        req.read_exact(&mut buffer)?;

        let brightness_req: BrightnessRequest = serde_json::from_slice(&buffer)
            .map_err(|e| anyhow::anyhow!("Invalid JSON: {}", e))?;
        
        let brightness = if brightness_req.percent > 100 { 100 } else { brightness_req.percent };
        
        {
            let mut state = current_color_brightness.lock().unwrap();
            state.brightness = brightness;
            update_leds(&ws_brightness, &state)?;
        }

        // Báo cho thread hiệu ứng biết có thay đổi
        let (lock, cvar) = &*effect_signal_brightness;
        let mut changed = lock.lock().unwrap();
        *changed = true;
        cvar.notify_one();

        let mut resp = req.into_ok_response().unwrap();
        let msg = format!("Set brightness: {}%", brightness);
        resp.write(msg.as_bytes())?;
        Ok(())
    })?;

    // Endpoint hiệu ứng
    let ws_effect = Arc::clone(&ws_driver);
    let current_color_effect = Arc::clone(&current_color);
    let effect_signal_effect = Arc::clone(&effect_signal);
    server.fn_handler("/effect", Method::Post, move |mut req| -> Result<(), anyhow::Error> {
        let content_length = req.content_len()
            .ok_or_else(|| anyhow::anyhow!("Content-Length header missing"))?;

        let mut buffer = vec![0u8; content_length.try_into().unwrap()];
        req.read_exact(&mut buffer)?;

        let effect_req: EffectRequest = serde_json::from_slice(&buffer)
            .map_err(|e| anyhow::anyhow!("Invalid JSON: {}", e))?;
        
        let effect = match effect_req.effect_type.as_str() {
            "static" => EffectType::Static,
            "rainbow" => EffectType::Rainbow,
            "breathing" => EffectType::Breathing,
            "colorwipe" => EffectType::ColorWipe,
            _ => return Err(anyhow::anyhow!("Unknown effect type"))
        };

        {
            let mut state = current_color_effect.lock().unwrap();
            state.effect = effect;
            state.is_running = true;
        }

        // Báo cho thread hiệu ứng biết có thay đổi
        let (lock, cvar) = &*effect_signal_effect;
        let mut changed = lock.lock().unwrap();
        *changed = true;
        cvar.notify_one();

        let mut resp = req.into_ok_response().unwrap();
        resp.write(format!("Effect set to: {}", effect_req.effect_type).as_bytes())?;
        Ok(())
    })?;

    // Start effect update thread
    let ws_effects = Arc::clone(&ws_driver);
    let state_effects = Arc::clone(&current_color);
    let effect_signal_loop = Arc::clone(&effect_signal);
    std::thread::spawn(move || {
        let (lock, cvar) = &*effect_signal_loop;
        loop {
            // Chờ tín hiệu hoặc timeout 50ms
            let mut changed = lock.lock().unwrap();
            let _ = cvar.wait_timeout(changed, Duration::from_millis(20)).unwrap();

            if let Ok(mut state) = state_effects.lock() {
                if state.is_running {
                    let colors = state.get_effect_colors();
                    let pixel_bytes: Vec<u8> = colors.iter()
                        .flat_map(|p| [p.g, p.r, p.b])
                        .collect();
                    
                    if let Ok(mut driver) = ws_effects.lock() {
                        let _ = driver.write_blocking(pixel_bytes.iter().cloned());
                    }
                }
            }
        }
    });

    loop {
        std::thread::sleep(Duration::from_secs(1));
    }
}
