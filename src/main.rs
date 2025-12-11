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
    sntp::{EspSntp, SyncStatus},
};
use log::info;
use smart_leds::RGB8;
use controller::LedController;
use ws2812_esp32_rmt_driver::Ws2812Esp32RmtDriver;

use std::{sync::{Arc, Mutex}, thread};
use crate::http::LedCommand;
// use crate::audio::AudioData;
use crate::scheduler::{LedScheduler, TimeOfDay};

mod wifi;
mod controller;
mod http;
mod audio;
mod effect;
mod scheduler;

static mut Q: Queue<LedCommand, 8> = Queue::new();

struct TimeCache {
    last_hour: u8,
    last_minute: u8,
    last_check_ms: u64,
}

impl TimeCache {
    fn new() -> Self {
        Self {
            last_hour: 255,
            last_minute: 255,
            last_check_ms: 0,
        }
    }
    
    /// Check scheduler every 5 seconds
    fn should_check_scheduler(&mut self) -> bool {
        let now_ms = unsafe { esp_idf_sys::esp_timer_get_time() } as u64 / 1000;
        
        // Only check every 5000ms (schedule precision is 1 minute anyway)
        if now_ms - self.last_check_ms >= 5000 {
            self.last_check_ms = now_ms;
            true
        } else {
            false
        }
    }
    
    fn get_current_time_cached(&mut self) -> Option<(u8, u8, u8)> {
        unsafe {
            let mut now: esp_idf_sys::time_t = 0;
            esp_idf_sys::time(&mut now);
            
            let mut timeinfo: esp_idf_sys::tm = std::mem::zeroed();
            esp_idf_sys::localtime_r(&now, &mut timeinfo);
            
            let hour = timeinfo.tm_hour as u8;
            let minute = timeinfo.tm_min as u8;
            let day = timeinfo.tm_wday as u8;
            let adjusted_day = if day == 0 { 6 } else { day - 1 };
            
            // Only trigger if minute changed
            if hour != self.last_hour || minute != self.last_minute {
                self.last_hour = hour;
                self.last_minute = minute;
                Some((hour, minute, adjusted_day))
            } else {
                None
            }
        }
    }
}

fn led_task(
    channel: esp_idf_hal::rmt::CHANNEL0,
    pin: esp_idf_hal::gpio::Gpio18,
    mut consumer: Consumer<'static, LedCommand>, 
    audio_data: Arc<Mutex<audio::AudioData>>,
    scheduler: Arc<Mutex<LedScheduler>>,
) -> Result<(), anyhow::Error> {
    // RMT on core 1
    let ws2812 = Ws2812Esp32RmtDriver::new(channel, pin)?;
    let mut controller = LedController::new(ws2812, 144);
    controller.set_audio_data(audio_data);
    info!("✅ RMT driver initialized on core {:?}", esp_idf_svc::hal::cpu::core());

    // ✅ Time caching to reduce syscall overhead
    let mut time_cache = TimeCache::new();

    info!("🎬 LED task loop starting - Controller handles FPS internally");

    loop {
        let mut commands_processed = 0;
        while let Some(cmd) = consumer.dequeue() {
            match cmd {
                http::LedCommand::SetEffect(effect) => {
                    controller.power_on();
                    controller.set_effect(effect);
                }
                http::LedCommand::SetBrightness(brightness) => {
                    controller.power_on();
                    controller.set_brightness(brightness);
                }
                http::LedCommand::SetColor(r, g, b) => {
                    controller.power_on();
                    controller.set_color(RGB8 { r, g, b });
                }
                http::LedCommand::SetSpeed(speed) => {
                    controller.power_on();
                    controller.set_speed(speed);
                }
                http::LedCommand::SetPowerState(state) => {
                    if state {
                        controller.power_on();
                    } else {
                        controller.power_off();
                    }
                }
            }
            
            commands_processed += 1;
            // ✅ Limit commands per iteration to avoid blocking
            if commands_processed >= 4 {
                break;
            }
        }

        if time_cache.should_check_scheduler() {
            if let Some((hour, minute, day)) = time_cache.get_current_time_cached() {
                // Time changed! Check scheduler
                if let Ok(mut sched) = scheduler.try_lock() {
                    if let Ok(current_time) = TimeOfDay::new(hour, minute) {
                        if let Some(action) = sched.check_and_execute(current_time, day) {
                            info!("⏰ Schedule triggered at {:02}:{:02}!", hour, minute);
                            
                            if action.power_on {
                                controller.power_on();
                                
                                if let Some(effect) = action.effect {
                                    controller.set_effect(effect);
                                }
                                if let Some(color) = action.color {
                                    controller.set_color(color);
                                }
                                if let Some(brightness) = action.brightness {
                                    controller.set_brightness(brightness);
                                }
                                if let Some(speed) = action.speed {
                                    controller.set_speed(speed);
                                }
                            } else {
                                controller.power_off();
                            }
                        }
                    }
                }
            }
        }

        controller.update();
        FreeRtos::delay_ms(1); 
    }
}

fn audio_task(
    i2s: esp_idf_hal::i2s::I2S0,
    sck: esp_idf_hal::gpio::Gpio33,
    ws: esp_idf_hal::gpio::Gpio25,
    sd: esp_idf_hal::gpio::Gpio32,
    audio_data: Arc<Mutex<audio::AudioData>>,
) -> Result<(), anyhow::Error> {
    info!("Audio task started on core {:?}", esp_idf_svc::hal::cpu::core());
    
    // Use blocking version for FreeRTOS thread
    audio::audio_processing_blocking(i2s, sck, ws, sd, audio_data)?;
    
    Ok(())
}

fn get_current_datetime() -> (u8, u8, u8, u8) {
    unsafe {
        let mut now: esp_idf_sys::time_t = 0;
        esp_idf_sys::time(&mut now);
        
        let mut timeinfo: esp_idf_sys::tm = std::mem::zeroed();
        esp_idf_sys::localtime_r(&now, &mut timeinfo);
        
        let hour = timeinfo.tm_hour as u8;
        let minute = timeinfo.tm_min as u8;
        let second = timeinfo.tm_sec as u8;
        let day = timeinfo.tm_wday as u8;
        let adjusted_day = if day == 0 { 6 } else { day - 1 };
        
        (hour, minute, second, adjusted_day)
    }
}

fn setup_sntp() -> anyhow::Result<EspSntp<'static>> {
    info!("🕒 Setting up SNTP...");
    
    use esp_idf_svc::sntp::{SntpConf, OperatingMode};
    
    let sntp_conf = SntpConf {
        servers: ["pool.ntp.org"],
        operating_mode: OperatingMode::Poll,
        sync_mode: esp_idf_svc::sntp::SyncMode::Smooth,
    };
    
    let sntp = EspSntp::new(&sntp_conf)?;
    
    info!("Waiting for SNTP sync...");
    let mut retries = 300;
    while sntp.get_sync_status() != SyncStatus::Completed && retries > 0 {
        thread::sleep(std::time::Duration::from_millis(100));
        retries -= 1;
        
        if retries % 50 == 0 {
            info!("   Still waiting... ({} seconds left)", retries / 10);
        }
    }
    
    if sntp.get_sync_status() == SyncStatus::Completed {
        info!("✅ SNTP synchronized!");
    } else {
        log::warn!("SNTP sync timeout after 30s");
        log::warn!("Time may be incorrect until sync succeeds");
    }
    
    // Set timezone
    unsafe {
        esp_idf_sys::setenv(
            b"TZ\0".as_ptr() as *const u8,
            b"UTC-7\0".as_ptr() as *const u8,
            1,
        );
        esp_idf_sys::tzset();
    }
    
    let (h, m, s, d) = get_current_datetime();
    info!("Current time: {:02}:{:02}:{:02}, Day: {} (0=Mon, 6=Sun)", h, m, s, d);
    
    // Validate time
    unsafe {
        let mut now: esp_idf_sys::time_t = 0;
        esp_idf_sys::time(&mut now);
        
        if now < 1577836800 {
            log::warn!("⚠️  System time looks wrong (before 2020)");
        }
    }
    
    Ok(sntp)
}

fn main() -> anyhow::Result<()> {
    esp_idf_sys::link_patches();
    EspLogger::initialize_default();

    let peripherals = Peripherals::take().unwrap();
    let sysloop = EspSystemEventLoop::take().unwrap();
    let timer_service = EspTaskTimerService::new().unwrap();
    let nvs = EspDefaultNvsPartition::take().unwrap();
    
    // Connect WiFi
    info!("Connecting to WiFi...");
    let _wifi = wifi::wifi(peripherals.modem, sysloop, Some(nvs), timer_service)?;
    info!("WiFi connected");

    // Wait for WiFi stability
    info!("⏳ Waiting for WiFi to stabilize...");
    FreeRtos::delay_ms(2000);

    // Setup SNTP
    let _sntp = setup_sntp()?;

    // Get pins
    let channel = peripherals.rmt.channel0;
    let led_pin = peripherals.pins.gpio18;
    let i2s = peripherals.i2s0;
    let sck_pin = peripherals.pins.gpio33;
    let ws_pin = peripherals.pins.gpio25;
    let sd_pin = peripherals.pins.gpio32;

    // Setup command queue
    let (producer, consumer) = unsafe { Q.split() };
    let producer = Arc::new(Mutex::new(producer));

    // Setup shared data
    let audio_data = Arc::new(Mutex::new(audio::AudioData::default()));
    let audio_data_for_led = audio_data.clone();
    let audio_data_for_audio = audio_data.clone();

    let scheduler = Arc::new(Mutex::new(LedScheduler::new()));
    let scheduler_for_http = scheduler.clone();
    let scheduler_for_led = scheduler.clone();

    // Start HTTP server
    info!("🌐 Starting HTTP server...");
    let _server = http::start_http_server(producer.clone(), scheduler_for_http)?;
    info!("✅ HTTP server started");

    
    // LED Task - Core 1
    ThreadSpawnConfiguration {
        name: Some(b"led-task\0"),
        stack_size: 12288,  // 12KB
        pin_to_core: Some(Core::Core1),
        priority: 20,
        ..Default::default()
    }.set()?;

    thread::spawn(move || {
        if let Err(e) = led_task(channel, led_pin, consumer, audio_data_for_led, scheduler_for_led) {
            log::error!("LED task error: {:?}", e);
        }
    });

    // Audio Task - Core 0
    ThreadSpawnConfiguration {
        name: Some(b"audio-task\0"),
        stack_size: 16384,  // 16KB
        pin_to_core: Some(Core::Core0),
        priority: 15,
        ..Default::default()
    }.set()?;

    thread::spawn(move || {
        if let Err(e) = audio_task(i2s, sck_pin, ws_pin, sd_pin, audio_data_for_audio) {
            log::error!("Audio task error: {:?}", e);
        }
    });

    loop {
        FreeRtos::delay_ms(1000);
    }
}