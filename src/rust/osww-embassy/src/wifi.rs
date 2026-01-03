//! Wi-Fi provisioning and connection management for ESP32.
//!
//! The controller is kept agnostic to networking. This module implements a
//! small async state machine that mirrors the behavior of the Arduino
//! WiFiManager flow: try saved credentials first, fall back to SoftAP
//! provisioning, and allow credentials to be updated via HTTP.

use alloc::string::String;
use core::future::Future;

use embassy_sync::blocking_mutex::raw::CriticalSectionRawMutex;
use embassy_sync::channel::{Channel, Receiver, Sender};

#[cfg(feature = "embedded")]
use log::debug;
use log::{info, warn};

use crate::state::WifiStatus;
#[cfg(feature = "embedded")]
use crate::tasks::RuntimeCommand;

/// Wi-Fi credentials provided by the user.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WifiCredentials {
    /// Network SSID.
    pub ssid: String,
    /// Network password.
    pub password: String,
}

impl WifiCredentials {
    /// Create a new credentials payload.
    pub fn new<S: Into<String>, P: Into<String>>(ssid: S, password: P) -> Self {
        Self {
            ssid: ssid.into(),
            password: password.into(),
        }
    }
}

/// Commands that can be sent to the Wi-Fi manager task.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum WifiCommand {
    /// Store new credentials and attempt to connect.
    SetCredentials(WifiCredentials),
    /// Forget saved credentials and re-enter provisioning mode.
    ForgetCredentials,
    /// Force a reconnect using the last saved credentials.
    ForceReconnect,
}

/// Result type used by Wi-Fi control implementations.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum WifiError {
    /// Connection or configuration failed.
    ConnectionFailed,
    /// SoftAP provisioning failed.
    ProvisioningFailed,
}

/// Storage backend for Wi-Fi credentials.
pub trait CredentialStore {
    /// Load credentials, if any are stored.
    fn load(&mut self) -> Option<WifiCredentials>;
    /// Save credentials to storage.
    fn save(&mut self, credentials: &WifiCredentials);
    /// Clear any saved credentials.
    fn clear(&mut self);
}

/// In-memory credential store for tests and host simulations.
#[derive(Debug, Default)]
pub struct InMemoryCredentialStore {
    credentials: Option<WifiCredentials>,
}

impl InMemoryCredentialStore {
    /// Create an empty credential store.
    pub fn new() -> Self {
        Self { credentials: None }
    }
}

impl CredentialStore for InMemoryCredentialStore {
    fn load(&mut self) -> Option<WifiCredentials> {
        self.credentials.clone()
    }

    fn save(&mut self, credentials: &WifiCredentials) {
        self.credentials = Some(credentials.clone());
    }

    fn clear(&mut self) {
        self.credentials = None;
    }
}

/// Async Wi-Fi control interface (implemented by esp-wifi wrappers).
pub trait WifiControl {
    /// Future returned by [`WifiControl::connect`].
    type ConnectFuture<'a>: Future<Output = Result<(), WifiError>>
    where
        Self: 'a;
    /// Future returned by [`WifiControl::start_ap`].
    type StartApFuture<'a>: Future<Output = Result<(), WifiError>>
    where
        Self: 'a;
    /// Future returned by [`WifiControl::disconnect`].
    type DisconnectFuture<'a>: Future<Output = Result<(), WifiError>>
    where
        Self: 'a;

    /// Attempt to connect using the provided credentials.
    fn connect<'a>(&'a mut self, credentials: &'a WifiCredentials) -> Self::ConnectFuture<'a>;
    /// Start a SoftAP for provisioning.
    fn start_ap<'a>(&'a mut self, credentials: &'a WifiCredentials) -> Self::StartApFuture<'a>;
    /// Disconnect from the current network.
    fn disconnect<'a>(&'a mut self) -> Self::DisconnectFuture<'a>;
    /// Return true if the station is connected.
    fn is_connected(&self) -> bool;
    /// Return the current RSSI in dB.
    fn rssi_db(&self) -> i32;
}

/// Default SoftAP credentials used during provisioning.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProvisioningConfig {
    /// SSID of the provisioning network.
    pub ssid: String,
    /// Password for the provisioning network.
    pub password: String,
}

impl ProvisioningConfig {
    /// Create a new provisioning configuration.
    pub fn new<S: Into<String>, P: Into<String>>(ssid: S, password: P) -> Self {
        Self {
            ssid: ssid.into(),
            password: password.into(),
        }
    }
}

/// Channel type used to send Wi-Fi commands.
pub type WifiCommandChannel<const N: usize> = Channel<CriticalSectionRawMutex, WifiCommand, N>;

/// Manages Wi-Fi connectivity and provisioning behavior.
#[derive(Debug)]
#[cfg_attr(not(feature = "embedded"), allow(dead_code))]
pub struct WifiManager<'a, C, S, const N: usize>
where
    C: WifiControl,
    S: CredentialStore,
{
    control: C,
    store: S,
    status: &'a WifiStatus,
    commands: Receiver<'a, CriticalSectionRawMutex, WifiCommand, N>,
    #[cfg(feature = "embedded")]
    runtime_sender: Option<
        embassy_sync::channel::Sender<
            'a,
            embassy_sync::blocking_mutex::raw::CriticalSectionRawMutex,
            RuntimeCommand,
            N,
        >,
    >,
    provisioning: ProvisioningConfig,
    reconnect_interval_secs: u64,
    /// True when we've switched the radio into SoftAP provisioning mode.
    ///
    /// Note: `WifiControl::is_connected()` represents STA connectivity, so it will typically be
    /// false in AP mode. We track this separately to avoid repeatedly restarting the AP.
    in_provisioning: bool,
}

#[cfg_attr(not(feature = "embedded"), allow(dead_code))]
impl<'a, C, S, const N: usize> WifiManager<'a, C, S, N>
where
    C: WifiControl,
    S: CredentialStore,
{
    /// Create a new Wi-Fi manager task.
    pub fn new(
        control: C,
        store: S,
        status: &'a WifiStatus,
        commands: Receiver<'a, CriticalSectionRawMutex, WifiCommand, N>,
        #[cfg(feature = "embedded")] runtime_sender: Option<
            embassy_sync::channel::Sender<
                'a,
                embassy_sync::blocking_mutex::raw::CriticalSectionRawMutex,
                RuntimeCommand,
                N,
            >,
        >,
        provisioning: ProvisioningConfig,
        reconnect_interval_secs: u64,
    ) -> Self {
        Self {
            control,
            store,
            status,
            commands,
            #[cfg(feature = "embedded")]
            runtime_sender,
            provisioning,
            reconnect_interval_secs,
            in_provisioning: false,
        }
    }

    async fn connect_with_saved(&mut self) -> Result<(), WifiError> {
        match self.store.load() {
            Some(creds) => {
                info!(
                    "wifi: connecting using saved credentials (ssid='{}')",
                    creds.ssid
                );
                match self.control.connect(&creds).await {
                    Ok(()) => {
                        info!("wifi: connected to '{}'", creds.ssid);
                        self.in_provisioning = false;
                        Ok(())
                    }
                    Err(err) => {
                        warn!("wifi: connect to '{}' failed: {:?}", creds.ssid, err);
                        Err(err)
                    }
                }
            }
            None => {
                warn!("wifi: no saved credentials");
                Err(WifiError::ConnectionFailed)
            }
        }
    }

    async fn enter_provisioning(&mut self) -> Result<(), WifiError> {
        let creds = WifiCredentials::new(
            self.provisioning.ssid.clone(),
            self.provisioning.password.clone(),
        );
        info!("wifi: starting provisioning AP (ssid='{}')", creds.ssid);
        match self.control.start_ap(&creds).await {
            Ok(()) => {
                self.in_provisioning = true;
                Ok(())
            }
            Err(err) => {
                warn!("wifi: failed to start provisioning AP: {:?}", err);
                Err(err)
            }
        }
    }

    fn refresh_status(&self) {
        self.status.set_connected(self.control.is_connected());
        self.status.set_rssi_db(self.control.rssi_db());
    }

    /// Run the Wi-Fi manager loop forever.
    #[cfg(feature = "embedded")]
    pub async fn run(mut self) -> ! {
        use embassy_futures::select::{select, Either};
        use embassy_time::{Duration, Timer};

        info!(
            "wifi: task starting (reconnect_interval={}s, provisioning_ssid='{}')",
            self.reconnect_interval_secs, self.provisioning.ssid
        );

        if self.connect_with_saved().await.is_err() {
            warn!("wifi: entering provisioning mode");
            let _ = self.enter_provisioning().await;
        }

        loop {
            self.refresh_status();

            match select(
                self.commands.receive(),
                Timer::after(Duration::from_secs(self.reconnect_interval_secs)),
            )
            .await
            {
                Either::First(cmd) => match cmd {
                    WifiCommand::SetCredentials(credentials) => {
                        info!(
                            "wifi: received new credentials (ssid='{}', password_len={})",
                            credentials.ssid,
                            credentials.password.len()
                        );
                        self.store.save(&credentials);
                        debug!("wifi: disconnecting before reconnect");
                        let _ = self.control.disconnect().await;
                        if self.control.connect(&credentials).await.is_err() {
                            warn!("wifi: connect failed; returning to provisioning mode");
                            let _ = self.enter_provisioning().await;
                        } else {
                            info!("wifi: connected using new credentials; requesting reboot UX");
                            // Mirror WiFiManager UX: after successful provisioning, reboot.
                            // We ask the controller to render the success LED pattern first.
                            if let Some(sender) = &self.runtime_sender {
                                // embassy_sync::channel::Sender::send() is infallible (waits for space)
                                sender.send(RuntimeCommand::ProvisioningSuccess).await;
                                info!("wifi: notified controller of provisioning success");
                            } else {
                                warn!(
                                    "wifi: provisioning success but no runtime sender configured"
                                );
                            }
                        }
                    }
                    WifiCommand::ForgetCredentials => {
                        warn!("wifi: forgetting saved credentials; entering provisioning");
                        self.store.clear();
                        let _ = self.control.disconnect().await;
                        let _ = self.enter_provisioning().await;
                    }
                    WifiCommand::ForceReconnect => {
                        info!("wifi: forced reconnect requested");
                        let _ = self.control.disconnect().await;
                        if self.connect_with_saved().await.is_err() {
                            let _ = self.enter_provisioning().await;
                        }
                    }
                },
                Either::Second(_) => {
                    // Only attempt STA reconnects when we're not already in provisioning mode.
                    // In AP mode, `is_connected()` stays false and we'd otherwise restart the AP
                    // every interval (making the SSID harder to discover/reliable to use).
                    if !self.control.is_connected() && !self.in_provisioning {
                        info!("wifi: disconnected; attempting periodic reconnect");
                        if self.connect_with_saved().await.is_err() {
                            let _ = self.enter_provisioning().await;
                        }
                    } else if self.in_provisioning {
                        debug!("wifi: skipping reconnect (in provisioning mode)");
                    }
                }
            }
        }
    }
}

/// Convenience helper for sending Wi-Fi commands.
#[derive(Debug, Clone)]
pub struct WifiCommandSender<'a, const N: usize> {
    sender: Sender<'a, CriticalSectionRawMutex, WifiCommand, N>,
}

impl<'a, const N: usize> WifiCommandSender<'a, N> {
    /// Create a new sender wrapper.
    pub fn new(sender: Sender<'a, CriticalSectionRawMutex, WifiCommand, N>) -> Self {
        Self { sender }
    }

    /// Send a Wi-Fi command asynchronously.
    pub async fn send(&self, command: WifiCommand) {
        self.sender.send(command).await;
    }
}
