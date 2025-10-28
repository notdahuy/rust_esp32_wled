use esp_idf_hal::peripheral::Peripheral;
use esp_idf_svc::{
    eventloop::EspSystemEventLoop,
    nvs::{EspNvs, EspNvsPartition, NvsDefault},
    timer::EspTimerService,
    wifi::{AsyncWifi, AuthMethod, ClientConfiguration, Configuration, EspWifi},
};

use esp_idf_svc::timer::Task;
use log::{info, warn, error};
use anyhow::{Result, Context, bail};
use std::sync::{Arc, Mutex};

// Constants cho NVS storage
const NVS_NAMESPACE: &str = "wifi_config";
const NVS_SSID_KEY: &str = "ssid";
const NVS_PASSWORD_KEY: &str = "password";
const NVS_CONFIGURED_KEY: &str = "configured";

// Fallback AP configuration
const AP_SSID: &str = "ESP32-AP";
const AP_PASSWORD: &str = "12345678";

// WiFi constraints
const MIN_SSID_LEN: usize = 1;
const MAX_SSID_LEN: usize = 32;
const MIN_PASSWORD_LEN: usize = 8;
const MAX_PASSWORD_LEN: usize = 64;

/// Cấu trúc lưu trữ thông tin WiFi
#[derive(Debug, Clone)]
pub struct WiFiCredentials {
    pub ssid: String,
    pub password: String,
}

impl WiFiCredentials {
    /// Tạo credentials mới với validation
    pub fn new(ssid: String, password: String) -> Result<Self> {
        let creds = Self { ssid, password };
        creds.validate()?;
        Ok(creds)
    }

    /// Validate credentials trước khi sử dụng
    pub fn validate(&self) -> Result<()> {
        if self.ssid.is_empty() || self.ssid.len() > MAX_SSID_LEN {
            bail!("SSID phải có từ {}-{} ký tự", MIN_SSID_LEN, MAX_SSID_LEN);
        }
        
        if self.password.len() < MIN_PASSWORD_LEN || self.password.len() > MAX_PASSWORD_LEN {
            bail!("Password phải có từ {}-{} ký tự", MIN_PASSWORD_LEN, MAX_PASSWORD_LEN);
        }
        
        // Kiểm tra SSID chỉ chứa các ký tự hợp lệ (UTF-8)
        if !self.ssid.is_ascii() && self.ssid.chars().any(|c| c.is_control()) {
            bail!("SSID chứa ký tự không hợp lệ");
        }
        
        Ok(())
    }
}

/// WiFi Manager với provisioning support
pub struct WiFiManager {
    wifi: AsyncWifi<EspWifi<'static>>,
    nvs: Arc<Mutex<EspNvsPartition<NvsDefault>>>,
}

impl WiFiManager {
    /// Khởi tạo WiFi Manager
    pub fn new(
        modem: impl Peripheral<P = esp_idf_hal::modem::Modem> + 'static,
        sysloop: EspSystemEventLoop,
        nvs: EspNvsPartition<NvsDefault>,
        timer_service: EspTimerService<Task>,
    ) -> Result<Self> {
        let wifi = AsyncWifi::wrap(
            EspWifi::new(modem, sysloop.clone(), Some(nvs.clone()))
                .context("Không thể khởi tạo WiFi driver")?,
            sysloop,
            timer_service,
        )
        .context("Không thể wrap AsyncWifi")?;

        Ok(Self {
            wifi,
            nvs: Arc::new(Mutex::new(nvs)),
        })
    }

    /// Bắt đầu quá trình provisioning hoặc kết nối
    pub async fn start(&mut self) -> Result<()> {
        info!("Khởi động WiFi Manager...");
        
        // Kiểm tra xem đã có cấu hình WiFi chưa
        match self.load_credentials() {
            Ok(credentials) => {
                info!("Tìm thấy cấu hình WiFi đã lưu");
                info!("SSID: {}", credentials.ssid);
                
                match self.connect_to_wifi(&credentials).await {
                    Ok(_) => {
                        info!("✓ Kết nối WiFi thành công!");
                        self.print_ip_info()?;
                        Ok(())
                    }
                    Err(e) => {
                        error!("✗ Không thể kết nối với WiFi đã lưu: {:#}", e);
                        warn!("Chuyển sang chế độ provisioning...");
                        self.start_provisioning_mode().await
                    }
                }
            }
            Err(e) => {
                info!("Chưa có cấu hình WiFi: {}", e);
                info!("Khởi động chế độ provisioning...");
                self.start_provisioning_mode().await
            }
        }
    }

    /// Lưu thông tin WiFi vào NVS
    pub fn save_credentials(&self, credentials: &WiFiCredentials) -> Result<()> {
        // Validate trước khi lưu
        credentials.validate()
            .context("Credentials không hợp lệ")?;
        
        let nvs_partition = self.nvs.lock()
            .map_err(|e| anyhow::anyhow!("Không thể lock NVS partition: {}", e))?;
       
        let mut nvs_handle = EspNvs::new(nvs_partition.clone(), NVS_NAMESPACE, true)
            .context("Không thể mở NVS namespace để ghi")?;
    
        nvs_handle.set_str(NVS_SSID_KEY, &credentials.ssid)
            .context("Không thể lưu SSID")?;
        
        nvs_handle.set_str(NVS_PASSWORD_KEY, &credentials.password)
            .context("Không thể lưu password")?;
        
        nvs_handle.set_u8(NVS_CONFIGURED_KEY, 1)
            .context("Không thể đánh dấu đã cấu hình")?;

        info!("✓ Đã lưu thông tin WiFi vào flash");
        Ok(())
    }

    /// Đọc thông tin WiFi từ NVS
    pub fn load_credentials(&self) -> Result<WiFiCredentials> {
        let nvs_partition = self.nvs.lock()
            .map_err(|e| anyhow::anyhow!("Không thể lock NVS partition: {}", e))?;
        
        let nvs_handle = EspNvs::new(nvs_partition.clone(), NVS_NAMESPACE, false)
            .context("Không thể mở NVS namespace để đọc")?;

        // Kiểm tra xem đã được cấu hình chưa
        let configured = nvs_handle.get_u8(NVS_CONFIGURED_KEY)
            .context("Lỗi khi đọc trạng thái cấu hình")?
            .context("Chưa có cấu hình WiFi")?;

        if configured != 1 {
            bail!("WiFi chưa được cấu hình");
        }

        // Đọc SSID
        let mut ssid_buf = [0u8; MAX_SSID_LEN + 1];
        let ssid = nvs_handle.get_str(NVS_SSID_KEY, &mut ssid_buf)
            .context("Lỗi khi đọc SSID")?
            .context("Không tìm thấy SSID")?
            .to_string();

        // Đọc Password
        let mut password_buf = [0u8; MAX_PASSWORD_LEN + 1];
        let password = nvs_handle.get_str(NVS_PASSWORD_KEY, &mut password_buf)
            .context("Lỗi khi đọc password")?
            .context("Không tìm thấy password")?
            .to_string();

        let credentials = WiFiCredentials { ssid, password };
        
        // Validate sau khi đọc
        credentials.validate()
            .context("Credentials đã lưu không hợp lệ")?;

        Ok(credentials)
    }

    /// Xóa thông tin WiFi đã lưu
    pub fn clear_credentials(&self) -> Result<()> {
        let nvs_partition = self.nvs.lock()
            .map_err(|e| anyhow::anyhow!("Không thể lock NVS partition: {}", e))?;
        
        let mut nvs_handle = EspNvs::new(nvs_partition.clone(), NVS_NAMESPACE, true)
            .context("Không thể mở NVS namespace để xóa")?;

        // Xóa từng key - không cần check return value
        let _ = nvs_handle.remove(NVS_SSID_KEY);
        let _ = nvs_handle.remove(NVS_PASSWORD_KEY);
        let _ = nvs_handle.remove(NVS_CONFIGURED_KEY);

        info!("✓ Đã xóa thông tin WiFi khỏi flash");
        Ok(())
    }

    /// Khởi động chế độ provisioning (Access Point)
    async fn start_provisioning_mode(&mut self) -> Result<()> {
        info!("Đang khởi động Access Point để provisioning...");

        let ap_config = Configuration::AccessPoint(
            esp_idf_svc::wifi::AccessPointConfiguration {
                ssid: AP_SSID.try_into()
                    .map_err(|_| anyhow::anyhow!("AP SSID không hợp lệ"))?,
                password: AP_PASSWORD.try_into()
                    .map_err(|_| anyhow::anyhow!("AP Password không hợp lệ"))?,
                channel: 1,
                auth_method: AuthMethod::WPA2Personal,
                max_connections: 4,
                ssid_hidden: false,
                ..Default::default()
            },
        );

        self.wifi.set_configuration(&ap_config)
            .context("Không thể cấu hình Access Point")?;
        
        self.wifi.start().await
            .context("Không thể khởi động Access Point")?;
        
        self.wifi.wait_netif_up().await
            .context("Network interface không sẵn sàng")?;

        info!("Access Point đã sẵn sàng!");
        info!("  SSID:     {}", AP_SSID);
        info!("  Password: {}", AP_PASSWORD);
        self.print_ap_info()?;

        Ok(())
    }

    /// Kết nối đến WiFi với credentials đã cho
    async fn connect_to_wifi(&mut self, credentials: &WiFiCredentials) -> Result<()> {
        info!("Đang kết nối đến WiFi: {}", credentials.ssid);
        
        let sta_config = Configuration::Client(ClientConfiguration {
            ssid: credentials.ssid.as_str().try_into()
                .map_err(|_| anyhow::anyhow!(
                    "SSID không hợp lệ hoặc quá dài (tối đa {} ký tự)", 
                    MAX_SSID_LEN
                ))?,
            password: credentials.password.as_str().try_into()
                .map_err(|_| anyhow::anyhow!(
                    "Password không hợp lệ hoặc quá dài (tối đa {} ký tự)", 
                    MAX_PASSWORD_LEN
                ))?,
            ..Default::default()
        });

        self.wifi.set_configuration(&sta_config)
            .context("Không thể cấu hình WiFi Station mode")?;
        
        info!("→ Cấu hình WiFi Station mode");

        self.wifi.start().await
            .context("Không thể khởi động WiFi")?;
        
        info!("→ WiFi driver đã khởi động");

        self.wifi.connect().await
            .context("Không thể kết nối đến WiFi")?;
        
        info!("→ Đã kết nối đến: {}", credentials.ssid);

        self.wifi.wait_netif_up().await
            .context("Network interface không sẵn sàng")?;
        
        info!("→ Network interface đã sẵn sàng");

        Ok(())
    }

    /// Cấu hình WiFi mới từ bên ngoài (qua HTTP/BLE)
    pub async fn provision(&mut self, credentials: WiFiCredentials) -> Result<()> {
        info!("Bắt đầu provisioning...");
        
        // Validate trước
        credentials.validate()
            .context("Credentials không hợp lệ")?;
        
        info!("Đang thử kết nối với WiFi mới: {}", credentials.ssid);
        
        // Thử kết nối
        self.connect_to_wifi(&credentials).await
            .context("Không thể kết nối với WiFi mới")?;
        
        // Nếu thành công, lưu vào NVS
        self.save_credentials(&credentials)
            .context("Không thể lưu credentials")?;
        
        info!("✓ Provisioning thành công!");
        self.print_ip_info()?;
        
        Ok(())
    }

    /// Kiểm tra trạng thái kết nối
    pub fn is_connected(&self) -> Result<bool> {
        Ok(self.wifi.is_connected()?)
    }

    /// In thông tin IP khi ở chế độ Station
    fn print_ip_info(&self) -> Result<()> {
        let ip_info = self.wifi.wifi().sta_netif().get_ip_info()
            .context("Không thể lấy thông tin IP")?;
        
        info!("IP: {:16}", ip_info.ip);
        info!("Subnet: {:16}", ip_info.subnet.mask);
        info!("Gateway:{:16}", ip_info.subnet.gateway);
        
        Ok(())
    }

    /// In thông tin IP khi ở chế độ AP
    fn print_ap_info(&self) -> Result<()> {
        let ip_info = self.wifi.wifi().ap_netif().get_ip_info()
            .context("Không thể lấy thông tin IP của AP")?;
  
        info!("IP: {:23} ", ip_info.ip);
        Ok(())
    }

    /// Lấy reference đến WiFi driver
    pub fn wifi(&self) -> &AsyncWifi<EspWifi<'static>> {
        &self.wifi
    }

    /// Lấy mutable reference đến WiFi driver
    pub fn wifi_mut(&mut self) -> &mut AsyncWifi<EspWifi<'static>> {
        &mut self.wifi
    }

    /// Lấy NVS partition (nếu cần truy cập trực tiếp)
    pub fn nvs(&self) -> Arc<Mutex<EspNvsPartition<NvsDefault>>> {
        self.nvs.clone()
    }
}

// ==== Ví dụ sử dụng ====

/// Hàm main ví dụ
pub async fn example_usage(
    modem: impl Peripheral<P = esp_idf_hal::modem::Modem> + 'static,
    sysloop: EspSystemEventLoop,
    nvs: EspNvsPartition<NvsDefault>,
    timer_service: EspTimerService<Task>,
) -> Result<()> {
    // Khởi tạo WiFi Manager
    let mut wifi_manager = WiFiManager::new(modem, sysloop, nvs, timer_service)?;

    // Khởi động - tự động kiểm tra và kết nối hoặc provisioning
    wifi_manager.start().await?;

    // Nếu bạn muốn xóa cấu hình cũ (reset)
    // wifi_manager.clear_credentials()?;

    // Kiểm tra trạng thái kết nối
    if wifi_manager.is_connected()? {
        info!("WiFi đang kết nối");
    }

    Ok(())
}

/// Hàm để cấu hình WiFi từ web server hoặc BLE
pub async fn configure_wifi_from_external(
    wifi_manager: &mut WiFiManager,
    ssid: String,
    password: String,
) -> Result<()> {
    // Tạo và validate credentials
    let credentials = WiFiCredentials::new(ssid, password)
        .context("Không thể tạo credentials")?;
    
    // Provision
    wifi_manager.provision(credentials).await?;
    
    Ok(())
}

/// Hàm để reset WiFi về factory settings
pub async fn reset_wifi_to_factory(
    wifi_manager: &mut WiFiManager,
) -> Result<()> {
    info!("Đang reset WiFi về cài đặt ban đầu...");
    
    // Xóa credentials
    wifi_manager.clear_credentials()?;
    
    // Khởi động lại provisioning mode
    wifi_manager.start().await?;
    
    Ok(())
}