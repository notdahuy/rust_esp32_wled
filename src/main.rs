use esp_idf_hal::{
    cpu::{self, Core},
    delay::FreeRtos,
    peripherals::Peripherals,
    prelude::*,
    
    task::thread::ThreadSpawnConfiguration,
};
use esp_idf_svc::{
    eventloop::EspSystemEventLoop,
    log::EspLogger,
    nvs::EspDefaultNvsPartition,
    wifi::EspWifi,
    timer::EspTaskTimerService,
};
use log::info;
use smart_leds::{SmartLedsWrite, RGB, RGB8};
use led_controller::LedController;
use ws2812_esp32_rmt_driver::Ws2812Esp32RmtDriver;

use std::{any, sync::{Arc, Mutex}, thread};

mod wifi;
mod led_controller;

fn led_task(channel: esp_idf_hal::rmt::CHANNEL0, pin: esp_idf_hal::gpio::Gpio18) -> Result<(), anyhow::Error> {
    info!("LED task started on core {:?}", esp_idf_svc::hal::cpu::core());
    
    // Khởi tạo RMT driver trên Core 1
    let ws2812 = Ws2812Esp32RmtDriver::new(channel, pin).unwrap();
    let mut controller = LedController::new(ws2812, 30);
    
    info!("RMT driver initialized on core {:?}", esp_idf_svc::hal::cpu::core());
    
    let num_leds = controller.count();
    
    // Tạo frame màu đỏ cho tất cả LED
    let red_frame: Vec<RGB8> = vec![RGB8 { r: 255, g: 255, b: 0 }; num_leds];
    // Tạo frame tắt LED (đen)
    let off_frame: Vec<RGB8> = vec![RGB8 { r: 0, g: 0, b: 0 }; num_leds];
    
    info!("Starting blink loop with {} LEDs", num_leds);
    
    // Vòng lặp blink liên tục
    loop {
        // Bật LED (đỏ)
        controller.set_brightness(0.1);
        controller.update_frame(&red_frame);
        FreeRtos::delay_ms(500);
        
        // Tắt LED
        controller.update_frame(&off_frame);
        FreeRtos::delay_ms(500);
    }
}

fn main() -> anyhow::Result<()> {
    // --- ESP setup ---
    esp_idf_sys::link_patches();
    EspLogger::initialize_default();

    let peripherals = Peripherals::take().unwrap();

    
    let sysloop = EspSystemEventLoop::take().unwrap();
    let timer_service = EspTaskTimerService::new().unwrap();
    let nvs = EspDefaultNvsPartition::take().unwrap();
    let _wifi = wifi::wifi(peripherals.modem, sysloop, Some(nvs), timer_service)?;

    // Lấy channel và pin để tạo RMT driver
    let channel = peripherals.rmt.channel0;
    let pin = peripherals.pins.gpio18;

    let cpu_cores = cpu::CORES;
    info!("Core counts {} cores", cpu_cores);
    info!("Main thread running on core {:?}", esp_idf_svc::hal::cpu::core());

    // Cấu hình thread trên Core 1
    ThreadSpawnConfiguration {
        name: Some(b"led-task\0"),
        stack_size: 4096,
        pin_to_core: Some(Core::Core1), 
        ..Default::default()
    }.set()?;
    
    // Spawn thread và khởi tạo RMT bên trong thread Core 1
    thread::spawn(move || {
        if let Err(e) = led_task(channel, pin) {
            log::error!("LED task error: {:?}", e);
        }
    });

    loop {
        FreeRtos::delay_ms(1000);
    }
}