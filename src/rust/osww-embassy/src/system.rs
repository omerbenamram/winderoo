//! System services for persistence, NTP sync, and device reset.
//!
//! This module keeps IO-heavy work out of the controller loop by reacting to
//! lightweight signals emitted by the `SystemHooks` implementation.

use embedded_storage::nor_flash::NorFlash;

use winderoo_firmware::settings::StoredSettings;
use winderoo_firmware::model::RuntimeState;

use crate::hardware::SystemHooks;
use crate::state::SystemSignals;

#[cfg(feature = "embedded")]
use crate::sntp::SntpClient;
#[cfg(feature = "embedded")]
use crate::state::StatusCache;
#[cfg(feature = "embedded")]
use crate::time::RtcClock;

const SETTINGS_HEADER_SIZE: usize = 8;

/// Errors returned by settings persistence implementations.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SettingsStoreError {
    /// A storage backend reported an IO failure.
    StorageFailure,
    /// JSON serialization failed.
    SerializationFailure,
}

/// Storage interface for persisted settings.
pub trait SettingsStore {
    /// Load settings from storage, returning defaults if none are found.
    fn load(&mut self) -> Result<StoredSettings, SettingsStoreError>;
    /// Save settings to storage.
    fn save(&mut self, settings: &StoredSettings) -> Result<(), SettingsStoreError>;
}

/// Load runtime state from a settings store, falling back to defaults on error.
pub fn load_runtime_state<S: SettingsStore>(
    store: &mut S,
    screen_equipped: bool,
) -> RuntimeState {
    let stored = store.load().unwrap_or_default();
    stored
        .to_runtime(screen_equipped)
        .unwrap_or_else(|_| StoredSettings::default().to_runtime(screen_equipped).expect("default runtime"))
}

/// Abstraction for device reset logic.
pub trait ResetControl {
    /// Trigger a device reset.
    fn reset(&mut self);
}

/// A simple in-memory settings store for tests and host simulations.
#[derive(Debug, Default)]
pub struct InMemorySettingsStore {
    settings: Option<StoredSettings>,
}

impl InMemorySettingsStore {
    /// Create an empty store.
    pub fn new() -> Self {
        Self { settings: None }
    }
}

impl SettingsStore for InMemorySettingsStore {
    fn load(&mut self) -> Result<StoredSettings, SettingsStoreError> {
        Ok(self.settings.clone().unwrap_or_default())
    }

    fn save(&mut self, settings: &StoredSettings) -> Result<(), SettingsStoreError> {
        self.settings = Some(settings.clone());
        Ok(())
    }
}

/// Settings store that writes a JSON blob into a fixed flash region.
#[derive(Debug)]
pub struct NorFlashSettingsStore<F>
where
    F: NorFlash,
{
    flash: F,
    offset: u32,
    length: usize,
}

impl<F> NorFlashSettingsStore<F>
where
    F: NorFlash,
{
    /// Magic header used to detect valid blobs.
    pub const MAGIC: [u8; 4] = *b"WDRS";

    /// Create a new flash-backed settings store.
    pub fn new(flash: F, offset: u32, length: usize) -> Self {
        Self { flash, offset, length }
    }

    fn read_header(&mut self, buf: &mut [u8; SETTINGS_HEADER_SIZE]) -> Result<(), SettingsStoreError> {
        self.flash
            .read(self.offset, buf)
            .map_err(|_| SettingsStoreError::StorageFailure)
    }

    fn write_blob(&mut self, payload: &[u8]) -> Result<(), SettingsStoreError> {
        let write_align = F::WRITE_SIZE;
        let aligned_payload_len = payload
            .len()
            .checked_add(write_align - 1)
            .map(|value| value / write_align * write_align)
            .ok_or(SettingsStoreError::SerializationFailure)?;

        if aligned_payload_len + SETTINGS_HEADER_SIZE > self.length {
            return Err(SettingsStoreError::SerializationFailure);
        }

        let mut header = [0u8; SETTINGS_HEADER_SIZE];
        header[..4].copy_from_slice(&Self::MAGIC);
        header[4..8].copy_from_slice(&(payload.len() as u32).to_le_bytes());

        self.flash
            .erase(self.offset, self.offset + self.length as u32)
            .map_err(|_| SettingsStoreError::StorageFailure)?;
        self.flash
            .write(self.offset, &header)
            .map_err(|_| SettingsStoreError::StorageFailure)?;

        let mut padded = alloc::vec![0u8; aligned_payload_len];
        padded[..payload.len()].copy_from_slice(payload);
        self.flash
            .write(self.offset + SETTINGS_HEADER_SIZE as u32, &padded)
            .map_err(|_| SettingsStoreError::StorageFailure)?;
        Ok(())
    }
}

impl<F> SettingsStore for NorFlashSettingsStore<F>
where
    F: NorFlash,
{
    fn load(&mut self) -> Result<StoredSettings, SettingsStoreError> {
        let mut header = [0u8; SETTINGS_HEADER_SIZE];
        self.read_header(&mut header)?;
        if header[..4] != Self::MAGIC {
            return Ok(StoredSettings::default());
        }
        let len = u32::from_le_bytes([header[4], header[5], header[6], header[7]]) as usize;
        if len == 0 || len + SETTINGS_HEADER_SIZE > self.length {
            return Ok(StoredSettings::default());
        }

        let read_align = <F as embedded_storage::nor_flash::ReadNorFlash>::READ_SIZE;
        let aligned_len = len
            .checked_add(read_align - 1)
            .map(|value| value / read_align * read_align)
            .ok_or(SettingsStoreError::StorageFailure)?;

        let mut payload = alloc::vec![0u8; aligned_len];
        self.flash
            .read(self.offset + SETTINGS_HEADER_SIZE as u32, &mut payload)
            .map_err(|_| SettingsStoreError::StorageFailure)?;
        let json = core::str::from_utf8(&payload[..len]).unwrap_or("");
        match serde_json::from_str::<StoredSettings>(json) {
            Ok(settings) => Ok(settings),
            Err(_) => Ok(StoredSettings::default()),
        }
    }

    fn save(&mut self, settings: &StoredSettings) -> Result<(), SettingsStoreError> {
        let json = serde_json::to_string(settings).map_err(|_| SettingsStoreError::SerializationFailure)?;
        self.write_blob(json.as_bytes())
    }
}

/// Signals emitted by the controller are handled via this SystemHooks adapter.
#[derive(Debug)]
pub struct SignalSystemHooks<'a> {
    signals: &'a SystemSignals,
}

impl<'a> SignalSystemHooks<'a> {
    /// Create a new adapter that writes into the provided signal set.
    pub fn new(signals: &'a SystemSignals) -> Self {
        Self { signals }
    }
}

impl<'a> SystemHooks for SignalSystemHooks<'a> {
    fn persist_settings(&mut self, snapshot: &winderoo_firmware::model::SettingsSnapshot) {
        self.signals.request_persist(snapshot.clone());
    }

    fn sync_time(&mut self) {
        self.signals.request_sync();
    }

    fn restart(&mut self) {
        self.signals.request_restart();
    }
}

/// Async system task that handles persistence and time sync requests.
#[cfg(feature = "embedded")]
#[derive(Debug)]
pub struct SystemTask<'a, S, R, N, X>
where
    S: SettingsStore,
    R: RtcClock,
    N: SntpClient,
    X: ResetControl,
{
    store: S,
    rtc: R,
    sntp: N,
    reset: X,
    status_cache: &'a StatusCache,
    signals: &'a SystemSignals,
    poll_interval_secs: u64,
}

#[cfg(feature = "embedded")]
impl<'a, S, R, N, X> SystemTask<'a, S, R, N, X>
where
    S: SettingsStore,
    R: RtcClock,
    N: SntpClient,
    X: ResetControl,
{
    /// Create a new system task for the provided services.
    pub fn new(
        store: S,
        rtc: R,
        sntp: N,
        reset: X,
        status_cache: &'a StatusCache,
        signals: &'a SystemSignals,
        poll_interval_secs: u64,
    ) -> Self {
        Self {
            store,
            rtc,
            sntp,
            reset,
            status_cache,
            signals,
            poll_interval_secs,
        }
    }

    fn compute_local_epoch(&self, utc_epoch: u64) -> u64 {
        let snapshot = self.status_cache.snapshot();
        winderoo_firmware::time::local_epoch_from_utc(utc_epoch, snapshot.gmt_offset, snapshot.dst)
    }

    /// Run the system task forever.
    pub async fn run(mut self) -> ! {
        use embassy_time::{Duration, Timer};

        loop {
            if let Some(snapshot) = self.signals.take_persist() {
                let stored = StoredSettings::from_snapshot(&snapshot);
                let _ = self.store.save(&stored);
            }

            if self.signals.take_sync() {
                if let Ok(utc_epoch) = self.sntp.sync().await {
                    let local_epoch = self.compute_local_epoch(utc_epoch);
                    self.rtc.set_epoch(local_epoch);
                }
            }

            if self.signals.take_restart() {
                self.reset.reset();
            }

            Timer::after(Duration::from_secs(self.poll_interval_secs)).await;
        }
    }
}
