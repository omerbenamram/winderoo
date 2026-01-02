//! ESP32 filesystem-backed storage helpers.
//!
//! This module owns:
//! - Loading/saving `settings.json` from LittleFS.
//! - Resolving static web assets from the mounted filesystem (including optional `.gz`).
//!
//! NOTE: This code is only compiled for the ESP32 build (`feature = "esp32"`).

use crate::settings::StoredSettings;
use std::fs;
use std::path::{Path, PathBuf};

use super::Esp32Error;

pub(super) struct Storage {
    root: PathBuf,
    settings_path: PathBuf,
}

impl Storage {
    pub(super) fn new(root: &str, settings_file: &str) -> Self {
        let root = PathBuf::from(root);
        let settings_path = root.join(settings_file);
        Self {
            root,
            settings_path,
        }
    }

    pub(super) fn load_or_init(&self) -> Result<StoredSettings, Esp32Error> {
        if let Ok(contents) = fs::read_to_string(&self.settings_path) {
            if let Ok(settings) = serde_json::from_str::<StoredSettings>(&contents) {
                return Ok(settings);
            }
        }
        let settings = StoredSettings::default();
        self.save(&settings)?;
        Ok(settings)
    }

    pub(super) fn save(&self, settings: &StoredSettings) -> Result<(), Esp32Error> {
        let json = serde_json::to_string_pretty(settings)?;
        fs::write(&self.settings_path, json)?;
        Ok(())
    }

    pub(super) fn flush(&self) -> Result<(), Esp32Error> {
        Ok(())
    }

    pub(super) fn resolve_asset(&self, uri: &str) -> Option<StaticAsset> {
        let mut path = uri.split('?').next().unwrap_or("").trim_start_matches('/');
        if path.is_empty() {
            path = "index.html";
        }
        if path.contains("..") {
            return None;
        }

        let candidate = self.root.join(path);
        if let Some(asset) = StaticAsset::from_path(&candidate, false) {
            return Some(asset);
        }

        let gz_candidate = PathBuf::from(format!("{}.gz", candidate.display()));
        if let Some(asset) = StaticAsset::from_path(&gz_candidate, true) {
            return Some(asset);
        }

        None
    }
}

pub(super) struct StaticAsset {
    pub(super) bytes: Vec<u8>,
    pub(super) content_type: &'static str,
    pub(super) cache_control: &'static str,
    pub(super) content_encoding: &'static str,
}

impl StaticAsset {
    fn from_path(path: &Path, gzipped: bool) -> Option<Self> {
        let bytes = fs::read(path).ok()?;
        let ext = if gzipped {
            path.file_stem()
                .and_then(|s| Path::new(s).extension())
                .and_then(|s| s.to_str())
                .unwrap_or("")
        } else {
            path.extension().and_then(|s| s.to_str()).unwrap_or("")
        };
        let content_type = match ext {
            "html" | "htm" => "text/html",
            "css" => "text/css",
            "js" => "application/javascript",
            "json" => "application/json",
            "ico" => "image/x-icon",
            "svg" => "image/svg+xml",
            "png" => "image/png",
            "jpg" | "jpeg" => "image/jpeg",
            "gz" => "application/octet-stream",
            _ => "application/octet-stream",
        };
        let cache_control = if ext == "js" || ext == "css" {
            "max-age=31536000"
        } else {
            "no-cache"
        };
        let content_encoding = if gzipped { "gzip" } else { "" };
        Some(Self {
            bytes,
            content_type,
            cache_control,
            content_encoding,
        })
    }
}
