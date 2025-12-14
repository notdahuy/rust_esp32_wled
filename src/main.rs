
use esp_idf_sys::esp_timer_get_time;
use heapless::spsc::{Queue, Consumer};
use esp_idf_hal::{
    cpu::Core,
    delay::FreeRtos,
    peripherals::Peripherals,
    task::thread::ThreadSpawnConfiguration,
};
use esp_idf_svc::{
    eventloop::EspSystemEventLoop,
    log::EspLogger,
    nvs::EspDefaultNvsPartition,
    timer::EspTaskTimerService,
};
use log::info;
use smart_leds::RGB8;
use controller::LedController;
use ws2812_esp32_rmt_driver::Ws2812Esp32RmtDriver;

use std::{sync::{Arc, Mutex, RwLock}, thread};
use crate::http::LedCommand;
use crate::audio::AudioData;

mod wifi;
mod controller;
mod http;
mod audio;
mod effects;

static mut Q: Queue<LedCommand, 8> = Queue::new();

fn led_task(
    channel: esp_idf_hal::rmt::CHANNEL0,
    pin: esp_idf_hal::gpio::Gpio18,
    mut consumer: Consumer<'static, LedCommand>, 
    audio_data: Arc<Mutex<audio::AudioData>>,
) -> Result<(), anyhow::Error> {
    let ws2812 = Ws2812Esp32RmtDriver::new(channel, pin)?;
    let mut controller = LedController::new(ws2812, 144);
    controller.set_audio_data(audio_data);

    info!("LED task started");

    loop {
        let now_us = unsafe { esp_timer_get_time() } as u64;
        
        // Bước 1: Xử lý TẤT CẢ commands
        let mut has_command = false;
        while let Some(cmd) = consumer.dequeue() {
            has_command = true;
            match cmd {
                LedCommand::SetEffect(e) => controller.set_effect(e),
                LedCommand::SetBrightness(b) => controller.set_brightness(b),
                LedCommand::SetColor(r, g, b) => controller.set_color(RGB8 { r, g, b }),
                LedCommand::SetSpeed(s) => controller.set_speed(s),
            }
        }
        
        // Bước 2: Update nếu có command HOẶC đến lúc update
        if has_command || controller.needs_update(now_us) {
            controller.update(now_us);
        }
        
        // Bước 3: Delay thông minh
        let delay_ms = controller.get_delay_ms(now_us);
        FreeRtos::delay_ms(delay_ms);
    }
}

fn audio_task(
    i2s: esp_idf_hal::i2s::I2S0,
    sck: esp_idf_hal::gpio::Gpio33,
    ws: esp_idf_hal::gpio::Gpio25,
    sd: esp_idf_hal::gpio::Gpio32,
    audio_data: Arc<Mutex<audio::AudioData>>,
) -> Result<(), anyhow::Error> {
    audio::audio_processing_blocking(i2s, sck, ws, sd, audio_data)?;
    Ok(())
}

fn main() -> anyhow::Result<()> {
    esp_idf_sys::link_patches();
    EspLogger::initialize_default();

    let peripherals = Peripherals::take().unwrap();

    let sysloop = EspSystemEventLoop::take().unwrap();
    let timer_service = EspTaskTimerService::new().unwrap();
    let nvs = EspDefaultNvsPartition::take().unwrap();
    let _wifi = wifi::wifi(peripherals.modem, sysloop, Some(nvs), timer_service)?;

    // Get pins for LED strip
    let channel = peripherals.rmt.channel0;
    let led_pin = peripherals.pins.gpio18;

    // Get pins for I2S microphone (INMP441)
    let i2s = peripherals.i2s0;
    let sck_pin = peripherals.pins.gpio33;
    let ws_pin = peripherals.pins.gpio25;
    let sd_pin = peripherals.pins.gpio32;

    let (producer, consumer) = unsafe { Q.split() };
    let producer = Arc::new(Mutex::new(producer));

    let audio_data = Arc::new(Mutex::new(audio::AudioData::default()));
    let audio_data_for_led = audio_data.clone();   // Clone cho LED task
    let audio_data_for_audio = audio_data.clone(); // Clone cho audio task

    // Start HTTP server
    let _server = http::start_http_server(producer.clone())?;
    info!("HTTP server started successfully");

    // Thread spawn config for Core 1
    ThreadSpawnConfiguration {
        name: Some(b"led-task\0"),
        stack_size: 16384,
        pin_to_core: Some(Core::Core1),
        priority: 20,
        ..Default::default()
    }.set()?;

    // Spawn LED thread on core 1

    thread::spawn(move || {
        if let Err(e) = led_task(channel, led_pin, consumer, audio_data_for_led) {
            log::error!("LED task error: {:?}", e);
        }
    });

    info!("LED task spawned on Core 1");

    ThreadSpawnConfiguration {
            name: Some(b"audio-task\0"),
            stack_size: 12288,  
            pin_to_core: Some(Core::Core0),
            priority: 15,  
            ..Default::default()
        }.set()?;

    thread::spawn(move || {
        if let Err(e) = audio_task(i2s, sck_pin, ws_pin, sd_pin, audio_data_for_audio) {
            log::error!("Audio task error: {:?}", e);
        }
    });

    // Keep main thread alive
    loop {
        FreeRtos::delay_ms(1000);
    }
}