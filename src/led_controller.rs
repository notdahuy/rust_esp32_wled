use std::sync::{Arc, Mutex};

use esp_idf_hal::delay::Ets;
use log::warn;
use smart_leds::RGB8;
use ws2812_esp32_rmt_driver::Ws2812Esp32RmtDriver;

pub struct LedController<'a> {
    driver: Arc<Mutex<Ws2812Esp32RmtDriver<'a>>>,
    num_leds: usize,
    brightness: f32,
}

impl<'a> LedController<'a> {
    pub fn new(driver: Ws2812Esp32RmtDriver<'a>, num_leds: usize) -> Self {
        Self {
            driver: Arc::new(Mutex::new(driver)),
            num_leds,
            brightness: 1.0,
        }
    }

    pub fn set_brightness(&mut self, level: f32) {
        self.brightness = level.clamp(0.0, 1.0);
    }

    pub fn update_frame(&self, frame: &[RGB8]) {
        
        if frame.len() != self.num_leds {
            warn!("Frame size ({}) doesn't match LED count ({})", frame.len(), self.num_leds);
            return;
        }

        let mut pixel_bytes = Vec::with_capacity(self.num_leds * 3);
        for pixel in frame {
            let scaled = RGB8 {
                r: ((pixel.r as f32) * self.brightness) as u8,
                g: ((pixel.g as f32) * self.brightness) as u8,
                b: ((pixel.b as f32) * self.brightness) as u8,
            };
            pixel_bytes.extend_from_slice(&[scaled.g, scaled.r, scaled.b]);
        }

        // KHÔNG TẮT INTERRUPT - chỉ dùng priority cao
        match self.driver.lock() {
            Ok(mut driver) => {
                if let Err(e) = driver.write_blocking(pixel_bytes.iter().cloned()) {
                    warn!("LED write error: {:?}", e);
                }
            }
            Err(_) => warn!("Failed to acquire LED driver lock (Mutex poisoned)")
        }

        Ets::delay_us(300);
    }

    fn driver(&self) -> Arc<Mutex<Ws2812Esp32RmtDriver<'a>>> {
        Arc::clone(&self.driver)
    }

    pub fn count(&self) -> usize {
        self.num_leds
    }
}