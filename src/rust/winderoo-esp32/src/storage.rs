//! Flash-backed persistence helpers for the ESP32 runtime.
//!
//! This module provides:
//! - `FlashPartition`: a simple NOR flash "view" over a shared `FlashStorage`.
//! - `NorFlashWifiCredentialStore`: a credential store backed by JSON blobs in flash.
//!
//! The flash layout is intentionally small and self-contained: we reserve two
//! erasable regions (settings + Wi-Fi credentials) and store length-prefixed
//! JSON payloads in each region.

use core::cell::RefCell;

use embassy_sync::blocking_mutex::raw::CriticalSectionRawMutex;
use embassy_sync::blocking_mutex::Mutex;
use embedded_storage::nor_flash::{ErrorType, NorFlash, ReadNorFlash};
use esp_storage::{FlashStorage, FlashStorageError};

use winderoo_embassy::wifi::{CredentialStore, WifiCredentials};

/// Shared flash handle used across tasks.
pub type SharedFlash = Mutex<CriticalSectionRawMutex, RefCell<FlashStorage<'static>>>;

/// A bounded view into flash, expressed as an offset + size.
#[derive(Debug, Clone, Copy)]
pub struct FlashPartition<'a> {
    flash: &'a SharedFlash,
    base: u32,
    size: u32,
}

impl<'a> FlashPartition<'a> {
    /// Create a new flash partition view.
    pub const fn new(flash: &'a SharedFlash, base: u32, size: u32) -> Self {
        Self { flash, base, size }
    }

    fn check_bounds(&self, offset: u32, length: usize) -> Result<(), FlashStorageError> {
        let offset = offset as usize;
        let size = self.size as usize;
        if length > size || offset > size - length {
            return Err(FlashStorageError::OutOfBounds);
        }
        Ok(())
    }

    fn check_align(
        &self,
        offset: u32,
        length: usize,
        align: usize,
    ) -> Result<(), FlashStorageError> {
        let offset = offset as usize;
        if offset % align != 0 || length % align != 0 {
            return Err(FlashStorageError::NotAligned);
        }
        Ok(())
    }
}

impl ErrorType for FlashPartition<'_> {
    type Error = FlashStorageError;
}

impl ReadNorFlash for FlashPartition<'_> {
    const READ_SIZE: usize = FlashStorage::WORD_SIZE as usize;

    fn read(&mut self, offset: u32, bytes: &mut [u8]) -> Result<(), Self::Error> {
        self.check_align(offset, bytes.len(), Self::READ_SIZE)?;
        self.check_bounds(offset, bytes.len())?;

        self.flash.lock(|flash| {
            let mut flash = flash.borrow_mut();
            ReadNorFlash::read(&mut *flash, self.base + offset, bytes)
        })
    }

    fn capacity(&self) -> usize {
        self.size as usize
    }
}

impl NorFlash for FlashPartition<'_> {
    const WRITE_SIZE: usize = FlashStorage::WORD_SIZE as usize;
    const ERASE_SIZE: usize = FlashStorage::SECTOR_SIZE as usize;

    fn erase(&mut self, from: u32, to: u32) -> Result<(), Self::Error> {
        let len = to.checked_sub(from).ok_or(FlashStorageError::OutOfBounds)? as usize;
        self.check_align(from, len, Self::ERASE_SIZE)?;
        self.check_bounds(from, len)?;

        self.flash.lock(|flash| {
            let mut flash = flash.borrow_mut();
            NorFlash::erase(&mut *flash, self.base + from, self.base + to)
        })
    }

    fn write(&mut self, offset: u32, bytes: &[u8]) -> Result<(), Self::Error> {
        self.check_align(offset, bytes.len(), Self::WRITE_SIZE)?;
        self.check_bounds(offset, bytes.len())?;

        self.flash.lock(|flash| {
            let mut flash = flash.borrow_mut();
            NorFlash::write(&mut *flash, self.base + offset, bytes)
        })
    }
}

const HEADER_SIZE: usize = 8;

/// Flash-backed Wi-Fi credential store.
#[derive(Debug)]
pub struct NorFlashWifiCredentialStore<F>
where
    F: NorFlash,
{
    flash: F,
    offset: u32,
    length: usize,
}

impl<F> NorFlashWifiCredentialStore<F>
where
    F: NorFlash,
{
    /// Magic header used to detect valid blobs.
    pub const MAGIC: [u8; 4] = *b"WDRW";

    /// Create a new flash-backed credential store.
    pub fn new(flash: F, offset: u32, length: usize) -> Self {
        Self {
            flash,
            offset,
            length,
        }
    }

    fn read_header(&mut self, buf: &mut [u8; HEADER_SIZE]) -> Result<(), FlashStorageError>
    where
        F::Error: Into<FlashStorageError>,
    {
        ReadNorFlash::read(&mut self.flash, self.offset, buf).map_err(|e| e.into())
    }

    fn write_blob(&mut self, payload: &[u8]) -> Result<(), FlashStorageError>
    where
        F::Error: Into<FlashStorageError>,
    {
        let write_align = F::WRITE_SIZE;
        let aligned_len = payload
            .len()
            .checked_add(write_align - 1)
            .map(|value| value / write_align * write_align)
            .ok_or(FlashStorageError::OutOfBounds)?;

        if aligned_len + HEADER_SIZE > self.length {
            return Err(FlashStorageError::OutOfBounds);
        }

        let mut header = [0u8; HEADER_SIZE];
        header[..4].copy_from_slice(&Self::MAGIC);
        header[4..8].copy_from_slice(&(payload.len() as u32).to_le_bytes());

        NorFlash::erase(
            &mut self.flash,
            self.offset,
            self.offset + self.length as u32,
        )
        .map_err(|e| e.into())?;
        NorFlash::write(&mut self.flash, self.offset, &header).map_err(|e| e.into())?;

        let mut padded = alloc::vec![0u8; aligned_len];
        padded[..payload.len()].copy_from_slice(payload);
        NorFlash::write(&mut self.flash, self.offset + HEADER_SIZE as u32, &padded)
            .map_err(|e| e.into())?;

        Ok(())
    }
}

#[derive(Debug, serde::Serialize, serde::Deserialize)]
struct StoredWifiCredentials {
    ssid: alloc::string::String,
    password: alloc::string::String,
}

impl<F> CredentialStore for NorFlashWifiCredentialStore<F>
where
    F: NorFlash,
    F::Error: Into<FlashStorageError>,
{
    fn load(&mut self) -> Option<WifiCredentials> {
        let mut header = [0u8; HEADER_SIZE];
        if self.read_header(&mut header).is_err() {
            return None;
        }
        if header[..4] != Self::MAGIC {
            return None;
        }

        let len = u32::from_le_bytes([header[4], header[5], header[6], header[7]]) as usize;
        if len == 0 || len + HEADER_SIZE > self.length {
            return None;
        }

        let read_align = <F as ReadNorFlash>::READ_SIZE;
        let aligned_len = len
            .checked_add(read_align - 1)
            .map(|value| value / read_align * read_align)?;

        let mut payload = alloc::vec![0u8; aligned_len];
        if ReadNorFlash::read(
            &mut self.flash,
            self.offset + HEADER_SIZE as u32,
            &mut payload,
        )
        .is_err()
        {
            return None;
        }

        let json = core::str::from_utf8(&payload[..len]).ok()?;
        let parsed: StoredWifiCredentials = serde_json::from_str(json).ok()?;
        Some(WifiCredentials::new(parsed.ssid, parsed.password))
    }

    fn save(&mut self, credentials: &WifiCredentials) {
        let payload = StoredWifiCredentials {
            ssid: credentials.ssid.clone(),
            password: credentials.password.clone(),
        };
        if let Ok(json) = serde_json::to_vec(&payload) {
            let _ = self.write_blob(&json);
        }
    }

    fn clear(&mut self) {
        let _ = NorFlash::erase(
            &mut self.flash,
            self.offset,
            self.offset + self.length as u32,
        );
    }
}
