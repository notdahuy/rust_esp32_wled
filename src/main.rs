use crate::audio::AudioData;
use crate::http::LedCommand;
use controller::LedController;
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
use esp_idf_sys::esp_timer_get_time;
use heapless::spsc::{Consumer, Queue};
use log::info;
use ntp::NtpManager;
use scheduler::LedScheduler;
use smart_leds::RGB8;
use std::{
    sync::{Arc, Mutex},
    thread,
};
use wifi::WifiManager;
use ws2812_esp32_rmt_driver::Ws2812Esp32RmtDriver;

mod audio;
mod controller;
mod effects;
mod http;
mod ntp;
mod scheduler;
mod wifi;

// Hàng đợi command giao tiếp giữa HTTP và LED task
static mut Q: Queue<LedCommand, 8> = Queue::new();

// Task điều khiển LED (chạy trên Core 1)
fn led_task(
    channel: esp_idf_hal::rmt::CHANNEL0,
    pin: esp_idf_hal::gpio::Gpio18,
    mut consumer: Consumer<'static, LedCommand>,
    audio_data: Arc<Mutex<audio::AudioData>>,
    scheduler: Arc<Mutex<LedScheduler>>,
    ntp: Arc<NtpManager>,
) -> Result<(), anyhow::Error> {
    // Khởi tạo driver LED WS2812
    let ws2812 = Ws2812Esp32RmtDriver::new(channel, pin)?;
    let mut controller = LedController::new(ws2812, 144);
    controller.set_audio_data(audio_data);

    info!("LED task started with scheduler support");

    let mut last_schedule_check = 0u64;
    let mut last_ntp_warning = 0u64;

    loop {
        let now_us = unsafe { esp_timer_get_time() } as u64;

        // Kiểm tra lịch trình mỗi giây (1.000.000 us)
        if now_us.wrapping_sub(last_schedule_check) >= 1_000_000 {
            last_schedule_check = now_us;

            // Chỉ xử lý lịch nếu NTP đã đồng bộ thời gian
            if ntp.is_synced() {
                if let Ok(time_info) = ntp.get_time() {
                    let current_time = scheduler::TimeOfDay::new(time_info.hour, time_info.minute);

                    if let Ok(current_time) = current_time {
                        // Khóa scheduler để kiểm tra
                        if let Ok(mut sched) = scheduler.try_lock() {
                            if let Some(preset) =
                                sched.check_and_execute(current_time, time_info.weekday)
                            {
                                // Áp dụng preset từ lịch trình
                                controller.set_effect(preset.effect);

                                if let Some(color) = preset.color {
                                    controller.set_color(color);
                                }

                                if let Some(brightness) = preset.brightness {
                                    controller.set_brightness(brightness);
                                }

                                if let Some(speed) = preset.speed {
                                    controller.set_speed(speed);
                                }

                                info!(
                                    "Schedule applied at {:02}:{:02}",
                                    time_info.hour, time_info.minute
                                );
                            }
                        }
                    }
                }
            } else {
                // Cảnh báo nếu NTP chưa sync (mỗi 30 giây)
                if now_us.wrapping_sub(last_ntp_warning) >= 30_000_000 {
                    log::warn!("NTP not synced yet, schedules won't work");
                    last_ntp_warning = now_us;
                }
            }
        }

        // Xử lý tất cả commands từ hàng đợi (HTTP gửi xuống)
        let mut has_command = false;
        while let Some(cmd) = consumer.dequeue() {
            has_command = true;
            match cmd {
                LedCommand::SetEffect(e) => controller.set_effect(e),
                LedCommand::SetBrightness(b) => controller.set_brightness(b),
                LedCommand::SetColor(r, g, b) => controller.set_color(RGB8 { r, g, b }),
                LedCommand::SetSpeed(s) => controller.set_speed(s),
                LedCommand::SetPowerState(on) => {
                    if on {
                        controller.set_brightness(1.0);
                    } else {
                        controller.set_brightness(0.0);
                    }
                }
            }
        }

        // Cập nhật hiệu ứng LED nếu có lệnh mới hoặc đến hạn update
        if has_command || controller.needs_update(now_us) {
            controller.update(now_us);
        }

        // Delay động dựa trên hiệu ứng hiện tại
        let delay_ms = controller.get_delay_ms(now_us);
        FreeRtos::delay_ms(delay_ms);
    }
}

// Task xử lý âm thanh (chạy trên Core 0)
fn audio_task(
    i2s: esp_idf_hal::i2s::I2S0,
    sck: esp_idf_hal::gpio::Gpio33,
    ws: esp_idf_hal::gpio::Gpio25,
    sd: esp_idf_hal::gpio::Gpio32,
    audio_data: Arc<Mutex<audio::AudioData>>,
) -> Result<(), anyhow::Error> {
    // Hàm xử lý blocking đọc dữ liệu từ I2S microphone
    audio::audio_processing_blocking(i2s, sck, ws, sd, audio_data)?;
    Ok(())
}

fn main() -> anyhow::Result<()> {
    // Khởi tạo các thành phần hệ thống cơ bản
    esp_idf_sys::link_patches();
    EspLogger::initialize_default();

    info!("=== ESP32 LED Controller v4.0 ===");

    let peripherals = Peripherals::take().unwrap();
    let sysloop = EspSystemEventLoop::take().unwrap();
    let timer_service = EspTaskTimerService::new().unwrap();
    let nvs = EspDefaultNvsPartition::take().unwrap();

    // Khởi tạo NTP Manager với timezone Việt Nam
    let ntp_manager = Arc::new(NtpManager::new(ntp::timezones::VIETNAM)?);

    // Khởi tạo Scheduler quản lý lịch trình
    let scheduler = Arc::new(Mutex::new(LedScheduler::new()));

    // Khởi tạo Wifi Manager
    let wifi_manager = Arc::new(WifiManager::new(
        peripherals.modem,
        sysloop,
        nvs.clone(),
        timer_service,
    )?);

    // Tạo luồng riêng để đợi WiFi và bắt đầu sync NTP
    let ntp_clone = ntp_manager.clone();
    let wifi_clone = wifi_manager.clone();
    thread::spawn(move || {
        info!("Waiting for WiFi connection before starting NTP...");

        // Đợi WiFi connect (tối đa 60 giây)
        for i in 0..60 {
            if wifi_clone.get_status().connected {
                info!("WiFi connected, starting NTP sync");

                if let Err(e) = ntp_clone.start_sync() {
                    log::error!("Failed to start NTP: {:?}", e);
                } else {
                    // Đợi sync xong rồi log thời gian
                    std::thread::sleep(std::time::Duration::from_secs(5));

                    if ntp_clone.is_synced() {
                        if let Ok(time) = ntp_clone.get_time() {
                            info!("Current time: {} ({})", time.format(), time.weekday_name());
                        }
                    }
                }
                break;
            }

            if i % 10 == 0 && i > 0 {
                info!("   Still waiting for WiFi... ({}s)", i);
            }

            std::thread::sleep(std::time::Duration::from_secs(1));
        }
    });

    // Cấu hình chân GPIO cho LED
    let channel = peripherals.rmt.channel0;
    let led_pin = peripherals.pins.gpio18;

    // Cấu hình chân GPIO cho I2S Microphone (INMP441)
    let i2s = peripherals.i2s0;
    let sck_pin = peripherals.pins.gpio33;
    let ws_pin = peripherals.pins.gpio25;
    let sd_pin = peripherals.pins.gpio32;

    // Tách hàng đợi (Queue) thành producer và consumer
    let (producer, consumer) = unsafe { Q.split() };
    let producer = Arc::new(Mutex::new(producer));

    // Chia sẻ dữ liệu Audio giữa các task
    let audio_data = Arc::new(Mutex::new(audio::AudioData::default()));
    let audio_data_for_led = audio_data.clone();
    let audio_data_for_audio = audio_data.clone();

    // Clone các Arc resource cho task con
    let scheduler_for_led = scheduler.clone();
    let scheduler_for_http = scheduler.clone();
    let ntp_for_led = ntp_manager.clone();

    // Khởi động HTTP Server
    let _server =
        http::start_http_server(producer.clone(), wifi_manager.clone(), scheduler_for_http)?;
    info!("HTTP server started");

    // Cấu hình và chạy LED Task trên Core 1
    ThreadSpawnConfiguration {
        name: Some(b"led-task\0"),
        stack_size: 16384,
        pin_to_core: Some(Core::Core1),
        priority: 24,
        ..Default::default()
    }
    .set()?;

    thread::spawn(move || {
        if let Err(e) = led_task(
            channel,
            led_pin,
            consumer,
            audio_data_for_led,
            scheduler_for_led,
            ntp_for_led,
        ) {
            log::error!("LED task error: {:?}", e);
        }
    });

    info!("LED task spawned on Core 1");

    // Cấu hình và chạy Audio Task trên Core 0
    ThreadSpawnConfiguration {
        name: Some(b"audio-task\0"),
        stack_size: 12288,
        pin_to_core: Some(Core::Core0),
        priority: 15,
        ..Default::default()
    }
    .set()?;

    thread::spawn(move || {
        if let Err(e) = audio_task(i2s, sck_pin, ws_pin, sd_pin, audio_data_for_audio) {
            log::error!("Audio task error: {:?}", e);
        }
    });

    loop {
        // Delay để tránh chiếm dụng CPU, giữ main thread chạy ngầm
        FreeRtos::delay_ms(10000);
    }
}