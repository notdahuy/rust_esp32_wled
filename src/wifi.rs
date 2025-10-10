use esp_idf_hal::peripheral::Peripheral;
use esp_idf_svc::{
    eventloop::EspSystemEventLoop, nvs::{EspNvsPartition, NvsDefault}, ping::EspPing, timer::EspTimerService, wifi::{AsyncWifi, AuthMethod, ClientConfiguration, Configuration, EspWifi}
};
use esp_idf_svc::timer::Task;
use log::info;
use anyhow::Result;

const SSID: &str = "MY CHAU";
const PASSWORD: &str = "0908814847";

#[allow(dead_code)]
pub fn wifi(
    modem: impl Peripheral<P = esp_idf_hal::modem::Modem> + 'static,
    sysloop: EspSystemEventLoop,
    nvs: Option<EspNvsPartition<NvsDefault>>,
    timer_service: EspTimerService<Task>,
) -> Result<AsyncWifi<EspWifi<'static>>> {
    use futures::executor::block_on;

    let mut wifi = AsyncWifi::wrap(
        EspWifi::new(modem, sysloop.clone(), nvs)?,
        sysloop,
        timer_service.clone(),
    )?;



    block_on(connect_to_wifi(&mut wifi))?;

    let ip_info = wifi.wifi().sta_netif().get_ip_info()?;

    info!("Thông tin Wifi DHCP: {:?}", ip_info);
    
    EspPing::default().ping(ip_info.ip, &esp_idf_svc::ping::Configuration::default())?;
    Ok(wifi)
}


// Hàm để khởi động Access Point
async fn start_access_point(wifi: &mut AsyncWifi<EspWifi<'static>>) -> anyhow::Result<()> {
    let ap_configuration: Configuration = Configuration::AccessPoint(esp_idf_svc::wifi::AccessPointConfiguration {
        ssid: "ESP32AP".try_into().unwrap(),
        password: "21078481".try_into().unwrap(),
        channel: 1,
        auth_method: AuthMethod::WPA2Personal,
        max_connections: 4,
        ssid_hidden: false,
        ..Default::default()
    });

    wifi.set_configuration(&ap_configuration)?;
    info!("Wi-Fi configuration set to Access Point mode.");

    wifi.start().await?;
    info!("Wi-Fi started as an Access Point.");

    wifi.wait_netif_up().await?;
    info!("Access Point network interface is up.");

    Ok(())
}

// Hàm để kết nối với mạng Wi-Fi    
async fn connect_to_wifi(wifi: &mut AsyncWifi<EspWifi<'static>>) -> anyhow::Result<()> {
    let sta_configuration: Configuration = Configuration::Client(ClientConfiguration {
        ssid: SSID.try_into().unwrap(),
        password: PASSWORD.try_into().unwrap(),
        ..Default::default()
    });

    wifi.set_configuration(&sta_configuration)?;
    info!("Wi-Fi configuration set to Station mode.");

    wifi.start().await?;
    info!("Wi-Fi started as a Station.");

    wifi.connect().await?;
    info!("Connected to Wi-Fi network: {}", SSID);

    wifi.wait_netif_up().await?;
    info!("Station network interface is up.");

    Ok(())
}

