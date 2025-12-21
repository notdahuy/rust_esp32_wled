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

use std::{sync::{Arc, Mutex}, thread};
use crate::http::LedCommand;
use crate::audio::AudioData;

mod wifi;
mod controller;
mod http;
mod audio;
mod effects;
mod scheduler;
mod ntp;

use wifi::WifiManager;
use scheduler::LedScheduler;
use ntp::NtpManager;

static mut Q: Queue<LedCommand, 8> = Queue::new();

fn led_task(
    channel: esp_idf_hal::rmt::CHANNEL0,
    pin: esp_idf_hal::gpio::Gpio18,
    mut consumer: Consumer<'static, LedCommand>, 
    audio_data: Arc<Mutex<audio::AudioData>>,
    scheduler: Arc<Mutex<LedScheduler>>,
    ntp: Arc<NtpManager>,
) -> Result<(), anyhow::Error> {
    let ws2812 = Ws2812Esp32RmtDriver::new(channel, pin)?;
    let mut controller = LedController::new(ws2812, 144);
    controller.set_audio_data(audio_data);

    info!("LED task started with scheduler support");
    
    let mut last_schedule_check = 0u64;
    let mut last_ntp_warning = 0u64;
    
    loop {
        let now_us = unsafe { esp_timer_get_time() } as u64;
        
        // Kiểm tra schedule mỗi giây
        if now_us.wrapping_sub(last_schedule_check) >= 1_000_000 {
            last_schedule_check = now_us;
            
            // Lấy thời gian thực từ NTP
            if ntp.is_synced() {
                if let Ok(time_info) = ntp.get_time() {
                    let current_time = scheduler::TimeOfDay::new(
                        time_info.hour,
                        time_info.minute
                    );
                    
                    if let Ok(current_time) = current_time {
                        if let Ok(mut sched) = scheduler.try_lock() {
                            if let Some(preset) = sched.check_and_execute(
                                current_time, 
                                time_info.weekday
                            ) {
                                // Apply schedule preset
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
                                
                                info!("📅 Schedule applied at {:02}:{:02}", 
                                      time_info.hour, time_info.minute);
                            }
                        }
                    }
                }
            } else {
                // Cảnh báo nếu NTP chưa sync (mỗi 30 giây)
                if now_us.wrapping_sub(last_ntp_warning) >= 30_000_000 {
                    log::warn!("⏰ NTP not synced yet, schedules won't work");
                    last_ntp_warning = now_us;
                }
            }
        }
        
        // Xử lý TẤT CẢ commands từ queue
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
        
        // Update nếu có command HOẶC đến lúc update
        if has_command || controller.needs_update(now_us) {
            controller.update(now_us);
        }
        
        // Delay thông minh
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

    info!("=== ESP32 LED Controller v4.0 ===");

    let peripherals = Peripherals::take().unwrap();

    let sysloop = EspSystemEventLoop::take().unwrap();
    let timer_service = EspTaskTimerService::new().unwrap();
    let nvs = EspDefaultNvsPartition::take().unwrap();
    
    // Khởi tạo NTP Manager với timezone Việt Nam
    let ntp_manager = Arc::new(NtpManager::new(ntp::timezones::VIETNAM)?);
    
    // Khởi tạo LedScheduler
    let scheduler = Arc::new(Mutex::new(LedScheduler::new()));
    
    // Initialize WiFi Manager
    let wifi_manager = Arc::new(WifiManager::new(
        peripherals.modem,
        sysloop,
        nvs.clone(),
        timer_service,
    )?);

    // Start NTP sync trong background thread sau khi WiFi kết nối
    let ntp_clone = ntp_manager.clone();
    let wifi_clone = wifi_manager.clone();
    thread::spawn(move || {
        info!("⏳ Waiting for WiFi connection before starting NTP...");
        
        // Đợi WiFi connect (tối đa 60 giây)
        for i in 0..60 {
            if wifi_clone.get_status().connected {
                info!("✅ WiFi connected, starting NTP sync");
                
                if let Err(e) = ntp_clone.start_sync() {
                    log::error!("❌ Failed to start NTP: {:?}", e);
                } else {
                    // Đợi sync xong rồi log thời gian
                    std::thread::sleep(std::time::Duration::from_secs(5));
                    
                    if ntp_clone.is_synced() {
                        if let Ok(time) = ntp_clone.get_time() {
                            info!("🕐 Current time: {} ({})", 
                                  time.format(), time.weekday_name());
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
    let audio_data_for_led = audio_data.clone();
    let audio_data_for_audio = audio_data.clone();

    // Clone scheduler và NTP cho các task
    let scheduler_for_led = scheduler.clone();
    let scheduler_for_http = scheduler.clone();
    let ntp_for_led = ntp_manager.clone();

    // Start HTTP server
    let _server = http::start_http_server(
        producer.clone(), 
        wifi_manager.clone(),
        scheduler_for_http
    )?;
    info!("✅ HTTP server started");

    // Spawn LED task on Core 1
    ThreadSpawnConfiguration {
        name: Some(b"led-task\0"),
        stack_size: 16384,
        pin_to_core: Some(Core::Core1),
        priority: 24,
        ..Default::default()
    }.set()?;

    thread::spawn(move || {
        if let Err(e) = led_task(
            channel, 
            led_pin, 
            consumer, 
            audio_data_for_led,
            scheduler_for_led,
            ntp_for_led
        ) {
            log::error!("LED task error: {:?}", e);
        }
    });

    info!("✅ LED task spawned on Core 1");

    // Spawn Audio task on Core 0
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

    info!("✅ Audio task spawned on Core 0");
    
    // Log schedule info
    if let Ok(sched) = scheduler.lock() {
        info!("📅 Scheduler initialized with {} schedules", 
              sched.get_all_schedules().len());
    }
    
    info!("=== System ready ===");

    // Main thread loop
    let mut last_memory_log = 0u64;
    let mut last_time_log = 0u64;
    
    loop {
        FreeRtos::delay_ms(10000);
        
        let now_ms = unsafe { esp_timer_get_time() } as u64 / 1000;
          
        // Log current time mỗi 60 giây (nếu NTP đã sync)
        if now_ms.wrapping_sub(last_time_log) >= 60000 {
            if ntp_manager.is_synced() {
                if let Ok(time) = ntp_manager.get_time() {
                    info!("🕐 Current: {} ({})", 
                          time.format(), time.weekday_name());
                }
            }
            last_time_log = now_ms;
        }
    }
}