use crate::types::{LedState, LED_COUNT};
use smart_leds::RGB8;
use ws2812_esp32_rmt_driver::Ws2812Esp32RmtDriver;
use std::sync::{Arc, Mutex};
use std::time::Duration;


/// Update LED với trạng thái hiện tại
pub fn update_leds(driver: &Arc<Mutex<Ws2812Esp32RmtDriver>>, state: &LedState) -> Result<(), anyhow::Error> {
    let final_color = state.get_static_color();
    let pixels: Vec<RGB8> = vec![final_color; LED_COUNT];
    let pixel_bytes: Vec<u8> = pixels.iter()
        .flat_map(|pixel| [pixel.g, pixel.r, pixel.b])
        .collect();
        
    let mut driver = driver.lock().unwrap();
    for _ in 0..2 {
        let _ = driver.write_blocking(pixel_bytes.iter().cloned());
        std::thread::sleep(Duration::from_micros(300));
    }
    Ok(())
}

pub fn turn_off_leds(driver: &Arc<Mutex<Ws2812Esp32RmtDriver>>) -> Result<(), anyhow::Error> {
    let pixels: Vec<RGB8> = vec![RGB8 { r: 0, g: 0, b: 0 }; LED_COUNT];
    let pixel_bytes: Vec<u8> = pixels.iter()
        .flat_map(|pixel| [pixel.g, pixel.r, pixel.b])
        .collect();
        
    let mut driver = driver.lock().unwrap();
    for _ in 0..3 {
        if let Ok(_) = driver.write_blocking(pixel_bytes.iter().cloned()) {
            break;
        }
        std::thread::sleep(Duration::from_millis(5));
    }
    Ok(())
}