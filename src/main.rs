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

use std::{sync::{mpsc, Arc}, thread};
use controller::EffectType;

mod wifi;
mod controller;
mod http;
mod audio;

fn led_task(
    channel: esp_idf_hal::rmt::CHANNEL0,
    pin: esp_idf_hal::gpio::Gpio18,
    rx: mpsc::Receiver<http::LedCommand>,
    audio_proc: Arc<audio::AudioProcessor>,
) -> Result<(), anyhow::Error> {
    // RMT on core 1
    let ws2812 = Ws2812Esp32RmtDriver::new(channel, pin)?;
    let mut controller = LedController::new(ws2812, 144);

    info!("RMT driver initialized on core {:?}", esp_idf_svc::hal::cpu::core());

    // Set audio processor
    controller.set_audio_processor(audio_proc);

    // Set default effect
    controller.set_effect(EffectType::Off);

    loop {
        // Xử lý commands từ HTTP
        match rx.try_recv() {
            Ok(cmd) => {
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
            Err(mpsc::TryRecvError::Empty) => {
                // No command, continue normal operation
            }
            Err(mpsc::TryRecvError::Disconnected) => {
                log::error!("Channel disconnected");
                break;
            }
        }

        controller.update();
        FreeRtos::delay_ms(1);
    }

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
    let sck_pin = peripherals.pins.gpio32;
    let ws_pin = peripherals.pins.gpio25;
    let sd_pin = peripherals.pins.gpio33;

    let cpu_cores = cpu::CORES;
    info!("Core counts {} cores", cpu_cores);
    info!("Main thread running on core {:?}", esp_idf_svc::hal::cpu::core());

    // Start I2S audio processing
    info!("Initializing I2S audio processor (INMP441)...");
    let audio_processor = audio::start_i2s_audio_task(
        i2s,
        sck_pin,
        ws_pin,
        sd_pin,
    )?;
    info!("Audio processor initialized");

    // Create channel for led control
    let (tx, rx) = mpsc::channel::<http::LedCommand>();

    // Start HTTP server
    let _server = http::start_http_server(tx)?;
    info!("HTTP server started successfully");

    // Thread spawn config for Core 1
    ThreadSpawnConfiguration {
        name: Some(b"led-task\0"),
        stack_size: 8192,
        pin_to_core: Some(Core::Core1),
        ..Default::default()
    }.set()?;

    // Spawn LED thread on core 1
    let audio_proc_clone = audio_processor.clone();
    thread::spawn(move || {
        if let Err(e) = led_task(channel, led_pin, rx, audio_proc_clone) {
            log::error!("LED task error: {:?}", e);
        }
    });

    info!("LED task spawned on Core 1");

    // Keep main thread alive
    loop {
        FreeRtos::delay_ms(1000);
    }
}