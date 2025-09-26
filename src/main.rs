use embedded_svc::http::Headers;
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
use std::sync::{Arc, Mutex};


// Import cÃ¡c module con
use types::*;
use led_controller::{update_leds, turn_off_leds};

mod wifi;
mod types;
mod led_controller;

fn main() -> anyhow::Result<()> {
    esp_idf_sys::link_patches();
    EspLogger::initialize_default();

    info!("Khá»Ÿi táº¡o ESP32 LED Controller");

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

    // Thiết lập các endpoint
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
    server.fn_handler("/set", Method::Post, move |mut req| -> Result<(), anyhow::Error> {
        let content_length = req.content_len()
            .ok_or_else(|| anyhow::anyhow!("Content-Length header missing"))?;

        let mut buffer = vec![0u8; content_length.try_into().unwrap()];
        req.read_exact(&mut buffer)?;

        let color_req: ColorRequest = serde_json::from_slice(&buffer)
            .map_err(|e| anyhow::anyhow!("Invalid JSON: {}", e))?;
        
        // Update màu
        {
            let mut state = current_color_set.lock().unwrap();
            state.r = color_req.r;
            state.g = color_req.g;
            state.b = color_req.b;

            // Update led và brightness hiện tại
            update_leds(&ws_set, &state)?;
        }

        let mut resp = req.into_ok_response().unwrap();
        let msg = format!("Set color: R={}, G={}, B={}", color_req.r, color_req.g, color_req.b);
        resp.write(msg.as_bytes())?;
        Ok(())
    })?;

    // Endpoint chỉnh độ sáng   
    let ws_brightness = Arc::clone(&ws_driver);
    let current_color_brightness = Arc::clone(&current_color);
    server.fn_handler("/brightness", Method::Post, move |mut req| -> Result<(), anyhow::Error> {
        let content_length = req.content_len()
            .ok_or_else(|| anyhow::anyhow!("Content-Length header missing"))?;

        let mut buffer = vec![0u8; content_length.try_into().unwrap()];
        req.read_exact(&mut buffer)?;

        let brightness_req: BrightnessRequest = serde_json::from_slice(&buffer)
            .map_err(|e| anyhow::anyhow!("Invalid JSON: {}", e))?;
        
        // Validate brightness range (0-100%)
        let brightness = if brightness_req.percent > 100 { 100 } else { brightness_req.percent };
        
        // Update brightness 
        {
            let mut state = current_color_brightness.lock().unwrap();
            state.brightness = brightness;
            
            // Update LED 
            update_leds(&ws_brightness, &state)?;
        }

        let mut resp = req.into_ok_response().unwrap();
        let msg = format!("Set brightness: {}%", brightness);
        resp.write(msg.as_bytes())?;
        Ok(())
    })?;

    loop {
        std::thread::park();
    }
}