//! ESP32-specific adapters for Wi-Fi and reset control.
//!
//! These helpers connect the generic `WifiControl` and `ResetControl` traits to
//! the `esp-wifi` and `esp-hal` crates when the `esp32` feature is enabled.

#[cfg(feature = "esp32")]
use alloc::boxed::Box;
#[cfg(feature = "esp32")]
use core::future::Future;
#[cfg(feature = "esp32")]
use core::pin::Pin;

#[cfg(feature = "esp32")]
use crate::system::ResetControl;

#[cfg(feature = "esp32")]
use crate::wifi::{WifiControl, WifiCredentials, WifiError};

#[cfg(feature = "esp32")]
use esp_radio::wifi::{AccessPointConfig, AuthMethod, ClientConfig, ModeConfig, WifiController};

/// ESP32 reset helper that issues a software reset.
#[cfg(feature = "esp32")]
#[derive(Debug, Default)]
pub struct EspReset;

#[cfg(feature = "esp32")]
impl ResetControl for EspReset {
    fn reset(&mut self) {
        esp_hal::system::software_reset();
    }
}

/// Wi-Fi control wrapper backed by `esp-wifi`.
#[cfg(feature = "esp32")]
pub struct EspWifiControl<'a> {
    controller: WifiController<'a>,
}

#[cfg(feature = "esp32")]
impl<'a> EspWifiControl<'a> {
    /// Wrap an `esp-radio` Wi-Fi controller.
    pub fn new(controller: WifiController<'a>) -> Self {
        Self { controller }
    }
}

#[cfg(feature = "esp32")]
impl<'a> WifiControl for EspWifiControl<'a> {
    type ConnectFuture<'b>
        = Pin<Box<dyn Future<Output = Result<(), WifiError>> + 'b>>
    where
        Self: 'b;
    type StartApFuture<'b>
        = Pin<Box<dyn Future<Output = Result<(), WifiError>> + 'b>>
    where
        Self: 'b;
    type DisconnectFuture<'b>
        = Pin<Box<dyn Future<Output = Result<(), WifiError>> + 'b>>
    where
        Self: 'b;

    fn connect<'b>(&'b mut self, credentials: &'b WifiCredentials) -> Self::ConnectFuture<'b> {
        Box::pin(async move {
            use log::{debug, info, warn};

            if matches!(self.controller.is_started(), Ok(true)) {
                debug!("esp-wifi: stopping controller before STA connect");
                let _ = self.controller.stop_async().await;
            }

            let config = ClientConfig::default()
                .with_ssid(credentials.ssid.clone())
                .with_password(credentials.password.clone())
                .with_auth_method(if credentials.password.is_empty() {
                    AuthMethod::None
                } else {
                    AuthMethod::Wpa2Personal
                });

            debug!("esp-wifi: setting STA config for '{}'", credentials.ssid);
            if let Err(err) = self.controller.set_config(&ModeConfig::Client(config)) {
                warn!("esp-wifi: set_config failed: {:?}", err);
                return Err(WifiError::ConnectionFailed);
            }
            debug!("esp-wifi: starting controller");
            if let Err(err) = self.controller.start_async().await {
                warn!("esp-wifi: start_async failed: {:?}", err);
                return Err(WifiError::ConnectionFailed);
            }

            // Quick scan to check signal strength before connecting
            debug!("esp-wifi: scanning for '{}'...", credentials.ssid);
            match self
                .controller
                .scan_with_config_async(Default::default())
                .await
            {
                Ok(networks) => {
                    info!("esp-wifi: scan found {} networks:", networks.len());
                    let target = credentials.ssid.as_str();
                    let mut found_target = false;
                    for net in networks.iter() {
                        let is_target = net.ssid.as_str() == target;
                        if is_target {
                            found_target = true;
                        }
                        // Log ALL networks so we can compare with Mac scan
                        info!(
                            "esp-wifi:   {} '{}' rssi={}dBm ch={} auth={:?}",
                            if is_target { ">>>" } else { "   " },
                            net.ssid,
                            net.signal_strength,
                            net.channel,
                            net.auth_method
                        );
                    }
                    if !found_target {
                        warn!("esp-wifi: TARGET '{}' NOT FOUND!", target);
                    }
                }
                Err(err) => {
                    warn!("esp-wifi: scan failed: {:?}", err);
                }
            }

            debug!("esp-wifi: connecting to '{}'...", credentials.ssid);
            if let Err(err) = self.controller.connect_async().await {
                warn!("esp-wifi: connect_async failed: {:?}", err);
                return Err(WifiError::ConnectionFailed);
            }
            // Log RSSI after successful connection
            let rssi = self.controller.rssi().unwrap_or(-100);
            info!("esp-wifi: connected! rssi={}dBm", rssi);
            Ok(())
        })
    }

    fn start_ap<'b>(&'b mut self, credentials: &'b WifiCredentials) -> Self::StartApFuture<'b> {
        Box::pin(async move {
            use log::{debug, warn};

            if matches!(self.controller.is_started(), Ok(true)) {
                debug!("esp-wifi: stopping controller before AP start");
                let _ = self.controller.stop_async().await;
            }
            let mut config = AccessPointConfig::default()
                .with_ssid(credentials.ssid.clone())
                .with_password(credentials.password.clone());
            // Match WiFiManager behavior: allow open provisioning networks when password is empty.
            //
            // `AccessPointConfig::default()` typically uses WPA2 auth; if we keep that default while
            // passing an empty password, AP start will fail and the SSID will never appear.
            config = if credentials.password.is_empty() {
                debug!("esp-wifi: AP mode with AuthMethod::None (open)");
                config.with_auth_method(AuthMethod::None)
            } else {
                debug!("esp-wifi: AP mode with AuthMethod::Wpa2Personal");
                config.with_auth_method(AuthMethod::Wpa2Personal)
            };
            self.controller
                .set_config(&ModeConfig::AccessPoint(config))
                .map_err(|_| WifiError::ProvisioningFailed)?;
            self.controller
                .start_async()
                .await
                .map_err(|_| WifiError::ProvisioningFailed)?;
            Ok(())
        })
    }

    fn disconnect<'b>(&'b mut self) -> Self::DisconnectFuture<'b> {
        Box::pin(async move {
            self.controller
                .disconnect_async()
                .await
                .map_err(|_| WifiError::ConnectionFailed)?;
            Ok(())
        })
    }

    fn is_connected(&self) -> bool {
        self.controller.is_connected().unwrap_or(false)
    }

    fn rssi_db(&self) -> i32 {
        self.controller.rssi().unwrap_or(-100)
    }
}
