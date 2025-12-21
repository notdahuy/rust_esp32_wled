use esp_idf_hal::{peripheral::Peripheral, delay::FreeRtos};
use esp_idf_svc::{
    eventloop::EspSystemEventLoop,
    nvs::{EspNvsPartition, NvsDefault, EspNvs},
    wifi::{
        AccessPointConfiguration, AuthMethod, ClientConfiguration, Configuration, EspWifi,
    },
    timer::EspTaskTimerService,
};
use anyhow::Result;
use log::{info, warn};
use std::sync::{Arc, Mutex};

const DEFAULT_AP_SSID: &str = "ESP32-LED";
const DEFAULT_AP_PASS: &str = "setup1234";

pub struct WifiManager {
    wifi: Arc<Mutex<EspWifi<'static>>>,
    nvs_partition: EspNvsPartition<NvsDefault>,
    current_mode: Arc<Mutex<WifiMode>>,
}

#[derive(Debug, Clone, Copy, PartialEq)]
enum WifiMode {
    Mixed,
}

impl WifiManager {
    pub fn new(
        modem: impl Peripheral<P = esp_idf_hal::modem::Modem> + 'static,
        sysloop: EspSystemEventLoop,
        nvs: EspNvsPartition<NvsDefault>,
        _timer: EspTaskTimerService,
    ) -> Result<Self> {
        let mut wifi = EspWifi::new(modem, sysloop, Some(nvs.clone()))?;
        
        info!("WiFi Manager starting...");
        
        // Generate unique AP name from MAC
        let mac = wifi.get_mac(esp_idf_svc::wifi::WifiDeviceId::Ap)?;
        let ap_name = format!("{}-{:02X}{:02X}", DEFAULT_AP_SSID, mac[4], mac[5]);
        // Try to load saved credentials
        let saved_creds = Self::load_credentials(&nvs)?;
        
        if let Some((ssid, pass)) = saved_creds {
            info!("Found saved WiFi: {}", ssid);
            // Start in Mixed mode with saved credentials
            let mixed_config = Configuration::Mixed(
                ClientConfiguration {
                    ssid: ssid.as_str().try_into().unwrap(),
                    password: pass.as_str().try_into().unwrap(),
                    auth_method: AuthMethod::WPA2Personal,
                    ..Default::default()
                },
                AccessPointConfiguration {
                    ssid: ap_name.as_str().try_into().unwrap(),
                    password: DEFAULT_AP_PASS.try_into().unwrap(),
                    channel: 6,
                    auth_method: AuthMethod::WPA2Personal,
                    max_connections: 4,
                    ssid_hidden: false,
                    ..Default::default()
                },
            );
            
            wifi.set_configuration(&mixed_config)?;
            wifi.start()?;
            match wifi.connect() {
                Ok(_) => info!("   Connection initiated"),
                Err(e) => warn!("   Connection failed: {:?}", e),
            }
            
        } else {
            // Start in Mixed mode with empty STA
            let mixed_config = Configuration::Mixed(
                ClientConfiguration::default(),
                AccessPointConfiguration {
                    ssid: ap_name.as_str().try_into().unwrap(),
                    password: DEFAULT_AP_PASS.try_into().unwrap(),
                    channel: 6,
                    auth_method: AuthMethod::WPA2Personal,
                    max_connections: 4,
                    ssid_hidden: false,
                    ..Default::default()
                },
            );
            
            wifi.set_configuration(&mixed_config)?;
            wifi.start()?;
        }
        FreeRtos::delay_ms(1000);
        
        // Log AP info
        if let Ok(ap_info) = wifi.ap_netif().get_ip_info() {
            info!("AP Mode ready");
            info!("SSID: {}", ap_name);
            info!("Password: {}", DEFAULT_AP_PASS);
            info!("Setup URL: http://{}", ap_info.ip);
        }
    
        // Check and log STA status
        if wifi.is_connected().unwrap_or(false) {
            if let Ok(sta_info) = wifi.sta_netif().get_ip_info() {
                info!("✅ STA connected - IP: {}", sta_info.ip);
            }
        }
        
        Ok(Self {
            wifi: Arc::new(Mutex::new(wifi)),
            nvs_partition: nvs,
            current_mode: Arc::new(Mutex::new(WifiMode::Mixed)),
        })
    }

    pub fn save_credentials(&self, ssid: &str, password: &str) -> Result<()> {
        let mut nvs_store = EspNvs::new(self.nvs_partition.clone(), "wifi", true)?;
        nvs_store.set_str("ssid", ssid)?;
        nvs_store.set_str("pass", password)?;
        Ok(())
    }

    fn load_credentials(nvs: &EspNvsPartition<NvsDefault>) -> Result<Option<(String, String)>> {
        // Try to open NVS namespace (read-only mode)
        let nvs_store = match EspNvs::new(nvs.clone(), "wifi", false) {
            Ok(store) => store,
            Err(_) => {
                info!("   No wifi namespace in NVS");
                return Ok(None);
            }
        };
        
        // Allocate buffers
        let mut ssid_buf = [0u8; 33]; 
        let mut pass_buf = [0u8; 65]; 
        
        // Read SSID
        let ssid = match nvs_store.get_str("ssid", &mut ssid_buf) {
            Ok(Some(s)) => {
                info!("   Found SSID: {}", s);
                s.to_string()
            }
            Ok(None) => {
                info!("   No SSID key found");
                return Ok(None);
            }
            Err(e) => {
                warn!("   Error reading SSID: {:?}", e);
                return Ok(None);
            }
        };
        
        // Read password
        let pass = match nvs_store.get_str("pass", &mut pass_buf) {
            Ok(Some(p)) => p.to_string(),
            Ok(None) => {
                warn!("   SSID found but no password");
                return Ok(None);
            }
            Err(e) => {
                warn!("   Error reading password: {:?}", e);
                return Ok(None);
            }
        };
        
        Ok(Some((ssid, pass)))
    }

    pub fn reconnect_saved(&self) -> Result<()> {
        info!("Reconnecting with saved credentials...");
        
        // Load credentials from NVS
        let saved_creds = Self::load_credentials(&self.nvs_partition)?;
        
        let (ssid, password) = match saved_creds {
            Some(creds) => creds,
            None => {
                warn!("No saved credentials to reconnect");
                return Ok(());
            }
        };
        
        // Get MAC for AP name
        let wifi = self.wifi.lock().unwrap();
        let mac = wifi.get_mac(esp_idf_svc::wifi::WifiDeviceId::Ap)?;
        let ap_name = format!("{}-{:02X}{:02X}", DEFAULT_AP_SSID, mac[4], mac[5]);
        drop(wifi);
        
        let mut wifi = self.wifi.lock().unwrap();
        
        let _ = wifi.disconnect();
        let _ = wifi.stop();
        
        let mixed_config = Configuration::Mixed(
            ClientConfiguration {
                ssid: ssid.as_str().try_into().unwrap(),
                password: password.as_str().try_into().unwrap(),
                auth_method: AuthMethod::WPA2Personal,
                ..Default::default()
            },
            AccessPointConfiguration {
                ssid: ap_name.as_str().try_into().unwrap(),
                password: DEFAULT_AP_PASS.try_into().unwrap(),
                channel: 6,
                auth_method: AuthMethod::WPA2Personal,
                max_connections: 4,
                ssid_hidden: false,
                ..Default::default()
            },
        );
        
        wifi.set_configuration(&mixed_config)?;
        wifi.start()?;
        
        info!("Connecting to '{}'...", ssid);
        wifi.connect()?;
        
        info!("Waiting for IP from DHCP...");
        
        let max_attempts = 40;
        let mut got_ip = false;
        
        for attempt in 0..max_attempts {
            if wifi.is_connected().unwrap_or(false) {
                if let Ok(ip_info) = wifi.sta_netif().get_ip_info() {
                    let ip = ip_info.ip.to_string();
                    if ip != "0.0.0.0" {
                        info!("✅ Got IP: {}", ip);
                        got_ip = true;
                        break;
                    }
                }
            }
            
            if attempt % 4 == 0 {
                info!("   Still waiting... ({}s)", attempt / 2);
            } 
            FreeRtos::delay_ms(500);
        }
        
        if !got_ip {
            warn!("⚠️  Timeout getting IP, continuing anyway");
        }
        
        Ok(())
    }
    
    
    pub fn scan_networks(&self) -> Result<Vec<WifiScanResult>> {
        info!("Scanning WiFi networks...");
        
        let mut wifi = self.wifi.lock().unwrap();
        
        // Scan directly - works in Mixed mode
        let scan_results = match wifi.scan() {
            Ok(results) => {
                info!("Scan completed successfully");
                results
            }
            Err(e) => {
                warn!("   Scan error: {:?}", e);
                return Err(anyhow::anyhow!("Scan failed: {:?}", e));
            }
        };
        
        let networks: Vec<WifiScanResult> = scan_results
            .into_iter()
            .filter(|ap| {
                // Filter out empty SSIDs and hidden networks
                !ap.ssid.is_empty() && ap.ssid.len() > 0
            })
            .map(|ap| WifiScanResult {
                ssid: ap.ssid.to_string(),
                rssi: ap.signal_strength,
                auth: format!("{:?}", ap.auth_method),
            })
            .collect();
        
        info!("Found {} networks", networks.len());
        
        // Log first few networks for debugging
        for (i, net) in networks.iter().take(5).enumerate() {
            info!("   {}. {} ({}dBm, {})", i + 1, net.ssid, net.rssi, net.auth);
        }
        
        Ok(networks)
    }
    
    pub fn get_status(&self) -> WifiStatus {
        let wifi = self.wifi.lock().unwrap();
        
        let is_connected = wifi.is_connected().unwrap_or(false);
        
        let ip = if is_connected {
            if let Ok(ip_info) = wifi.sta_netif().get_ip_info() {
                let ip_str = ip_info.ip.to_string();
                // Only return non-zero IP
                if ip_str != "0.0.0.0" {
                    Some(ip_str)
                } else {
                    None
                }
            } else {
                None
            }
        } else {
            None
        };
        
        WifiStatus {
            connected: ip.is_some(), // Only connected if has real IP
            ip,
        }
    }
    
    pub fn clear_credentials(&self) -> Result<()> {
        info!("🗑️  Clearing saved WiFi credentials");
        
        // Open in READWRITE mode (false = writable)
        let mut nvs_store = EspNvs::new(self.nvs_partition.clone(), "wifi", true)?;
        
        // Try to remove both keys
        let ssid_removed = nvs_store.remove("ssid")?;
        let pass_removed = nvs_store.remove("pass")?;
        
        if ssid_removed || pass_removed {
            info!("Credentials cleared from NVS");
        } else {
            info!("No credentials to clear");
        }
        
        Ok(())
    }
    
    pub fn get_wifi(&self) -> Arc<Mutex<EspWifi<'static>>> {
        self.wifi.clone()
    }

    pub fn restart_ap_mode(&self) -> Result<()> {
        info!("Restarting WiFi in AP-only mode...");

        let mut wifi = self.wifi.lock().unwrap();

        // Generate AP name
        let mac = wifi.get_mac(esp_idf_svc::wifi::WifiDeviceId::Ap)?;
        let ap_name = format!("{}-{:02X}{:02X}", DEFAULT_AP_SSID, mac[4], mac[5]);

        // Stop everything
        let _ = wifi.stop();

        // AP-only configuration (NO STA)
        let ap_config = Configuration::AccessPoint(
            AccessPointConfiguration {
                ssid: ap_name.as_str().try_into().unwrap(),
                password: DEFAULT_AP_PASS.try_into().unwrap(),
                channel: 6,
                auth_method: AuthMethod::WPA2Personal,
                max_connections: 4,
                ssid_hidden: false,
                ..Default::default()
            }
        );

        wifi.set_configuration(&ap_config)?;
        wifi.start()?;

        drop(wifi); // release mutex before delay

        FreeRtos::delay_ms(800);

        // Log AP info
        let wifi = self.wifi.lock().unwrap();
        if let Ok(ap_info) = wifi.ap_netif().get_ip_info() {
            info!("AP-only mode ready");
            info!("SSID: {}", ap_name);
            info!("IP: {}", ap_info.ip);
        }

        Ok(())
    }

}

#[derive(Debug, Clone)]
pub struct WifiScanResult {
    pub ssid: String,
    pub rssi: i8,
    pub auth: String,
}

#[derive(Debug)]
pub struct WifiStatus {
    pub connected: bool,
    pub ip: Option<String>,
}