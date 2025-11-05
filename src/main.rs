
use heapless::spsc::{Queue, Consumer};
use esp_idf_hal::{
    cpu::{self, Core},
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
use crate::{audio::AudioData, http::LedCommand};

mod wifi;
mod controller;
mod http;
mod audio;

static mut Q: Queue<LedCommand, 8> = Queue::new();

fn led_task(
    channel: esp_idf_hal::rmt::CHANNEL0,
    pin: esp_idf_hal::gpio::Gpio18,
    mut consumer: Consumer<'static, LedCommand>, 
    audio_proc: Arc<AudioData>,
) -> Result<(), anyhow::Error> {
    // RMT on core 1
    let ws2812 = Ws2812Esp32RmtDriver::new(channel, pin)?;
    let mut controller = LedController::new(ws2812, 144);
    info!("RMT driver initialized on core {:?}", esp_idf_svc::hal::cpu::core());

    controller.set_brightness(0.5);
    controller.set_effect(controller::EffectType::Rainbow);

    loop {
        // Xử lý commands từ HTTP
        
        if let Some(cmd) = consumer.dequeue() {
            match cmd {
                http::LedCommand::SetEffect(effect) => {
                    info!("Received effect command: {:?}", effect);
                    controller.set_effect(effect);
                }
                http::LedCommand::SetBrightness(brightness) => {
                    info!("Received brightness command: {}", brightness);
                    controller.set_brightness(brightness);
                }
                http::LedCommand::SetColor(r, g, b) => {
                    info!("Received color command: R:{} G:{} B:{}", r, g, b);
                    controller.set_color(RGB8 { r, g, b });
                }
                http::LedCommand::SetSpeed(speed) => {
                    info!("Received speed command: {}", speed);
                    controller.set_speed(speed);
                }
            }
        }
        controller.update(Some(&audio_proc));
        FreeRtos::delay_ms(1);
    }
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

    let cpu_cores = cpu::CORES;
    info!("Core counts {} cores", cpu_cores);
    info!("Main thread running on core {:?}", esp_idf_svc::hal::cpu::core());

    // // Start I2S audio processing
    info!("Initializing I2S audio processor (INMP441)...");
    ThreadSpawnConfiguration {
        name: Some(b"audio-task\0"),
        stack_size: 8192,
        pin_to_core: Some(Core::Core0),
        priority: 15,
        ..Default::default()
    }.set()?;

    let audio_data_arc = audio::start_i2s_audio_task(
        i2s,
        sck_pin,
        ws_pin,
        sd_pin,
    )?;
    // let audio_data_arc = Arc::new(audio::AudioData::default());
    let audio_data_clone = audio_data_arc.clone();
    info!("Audio processor initialized and pinned to {:?}", esp_idf_svc::hal::cpu::core());

    let (producer, consumer) = unsafe { Q.split() };
    let producer = Arc::new(Mutex::new(producer));

    // Start HTTP server
    let _server = http::start_http_server(producer.clone())?;
    info!("HTTP server started successfully");

    // Thread spawn config for Core 1
    ThreadSpawnConfiguration {
        name: Some(b"led-task\0"),
        stack_size: 8192,
        pin_to_core: Some(Core::Core1),
        priority: 20,
        ..Default::default()
    }.set()?;

    // Spawn LED thread on core 1

    thread::spawn(move || {
        if let Err(e) = led_task(channel, led_pin, consumer, audio_data_clone) {
            log::error!("LED task error: {:?}", e);
        }
    });

    info!("LED task spawned on Core 1");

    // Keep main thread alive
    loop {
        FreeRtos::delay_ms(1000);
    }
}