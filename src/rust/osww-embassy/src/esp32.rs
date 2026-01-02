//! ESP32-specific adapters for Wi-Fi and reset control.
//!
//! These helpers connect the generic `WifiControl` and `ResetControl` traits to
//! the `esp-wifi` and `esp-hal` crates when the `esp32` feature is enabled.

#[cfg(feature = "esp32")]
use core::future::Future;

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
        esp_hal::reset::software_reset();
    }
}

/// Wi-Fi control wrapper backed by `esp-wifi`.
#[cfg(feature = "esp32")]
#[derive(Debug)]
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
    type ConnectFuture<'b> = impl Future<Output = Result<(), WifiError>> + 'b where Self: 'b;
    type StartApFuture<'b> = impl Future<Output = Result<(), WifiError>> + 'b where Self: 'b;
    type DisconnectFuture<'b> = impl Future<Output = Result<(), WifiError>> + 'b where Self: 'b;

    fn connect<'b>(&'b mut self, credentials: &'b WifiCredentials) -> Self::ConnectFuture<'b> {
        async move {
            if matches!(self.controller.is_started(), Ok(true)) {
                let _ = self.controller.stop_async().await;
            }
            let mut config = ClientConfig::default()
                .with_ssid(credentials.ssid.clone())
                .with_password(credentials.password.clone());
            if credentials.password.is_empty() {
                config = config.with_auth_method(AuthMethod::None);
            }
            self.controller
                .set_config(&ModeConfig::Client(config))
                .map_err(|_| WifiError::ConnectionFailed)?;
            self.controller
                .start_async()
                .await
                .map_err(|_| WifiError::ConnectionFailed)?;
            self.controller
                .connect_async()
                .await
                .map_err(|_| WifiError::ConnectionFailed)?;
            Ok(())
        }
    }

    fn start_ap<'b>(&'b mut self, credentials: &'b WifiCredentials) -> Self::StartApFuture<'b> {
        async move {
            if matches!(self.controller.is_started(), Ok(true)) {
                let _ = self.controller.stop_async().await;
            }
            let mut config = AccessPointConfig::default()
                .with_ssid(credentials.ssid.clone())
                .with_password(credentials.password.clone());
            if !credentials.password.is_empty() {
                config = config.with_auth_method(AuthMethod::Wpa2Personal);
            }
            self.controller
                .set_config(&ModeConfig::AccessPoint(config))
                .map_err(|_| WifiError::ProvisioningFailed)?;
            self.controller
                .start_async()
                .await
                .map_err(|_| WifiError::ProvisioningFailed)?;
            Ok(())
        }
    }

    fn disconnect<'b>(&'b mut self) -> Self::DisconnectFuture<'b> {
        async move {
            self.controller
                .disconnect_async()
                .await
                .map_err(|_| WifiError::ConnectionFailed)?;
            Ok(())
        }
    }

    fn is_connected(&self) -> bool {
        self.controller.is_connected().unwrap_or(false)
    }

    fn rssi_db(&self) -> i32 {
        self.controller.rssi().unwrap_or(-100)
    }
}
