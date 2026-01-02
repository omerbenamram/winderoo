//! Shared state caches used across async embassy tasks.
//!
//! The controller task owns the authoritative firmware state, while other tasks
//! (HTTP, NTP, Wi-Fi) need read-only access to snapshots. This module provides
//! lightweight, lock-protected caches and system signals for cross-task
//! coordination without leaking IO into the core logic.

use core::cell::RefCell;
use core::sync::atomic::{AtomicBool, AtomicI32, Ordering};

#[cfg(feature = "embedded")]
use embassy_sync::blocking_mutex::raw::CriticalSectionRawMutex as RawMutex;
#[cfg(not(feature = "embedded"))]
use embassy_sync::blocking_mutex::raw::NoopRawMutex as RawMutex;
use embassy_sync::blocking_mutex::Mutex;

use winderoo_firmware::model::{SettingsSnapshot, StatusSnapshot};

/// Thread-safe cache of the latest status snapshot for HTTP and system tasks.
#[derive(Debug)]
pub struct StatusCache {
    inner: Mutex<RawMutex, RefCell<StatusSnapshot>>,
}

impl StatusCache {
    /// Create a new cache seeded with the provided snapshot.
    pub fn new(initial: StatusSnapshot) -> Self {
        Self {
            inner: Mutex::new(RefCell::new(initial)),
        }
    }

    /// Replace the stored snapshot with a new value.
    pub fn update(&self, snapshot: StatusSnapshot) {
        self.inner.lock(|current| {
            *current.borrow_mut() = snapshot;
        });
    }

    /// Fetch a clone of the latest snapshot.
    pub fn snapshot(&self) -> StatusSnapshot {
        self.inner.lock(|current| current.borrow().clone())
    }
}

/// Shared Wi-Fi status used by the controller for API responses.
#[derive(Debug)]
pub struct WifiStatus {
    connected: AtomicBool,
    rssi_db: AtomicI32,
}

impl WifiStatus {
    /// Create a Wi-Fi status block with sensible defaults.
    pub fn new() -> Self {
        Self {
            connected: AtomicBool::new(false),
            rssi_db: AtomicI32::new(-100),
        }
    }

    /// Update the connection flag.
    pub fn set_connected(&self, connected: bool) {
        self.connected.store(connected, Ordering::Release);
    }

    /// Update the current RSSI reading in dB.
    pub fn set_rssi_db(&self, rssi_db: i32) {
        self.rssi_db.store(rssi_db, Ordering::Release);
    }

    /// Read the current connection flag.
    pub fn is_connected(&self) -> bool {
        self.connected.load(Ordering::Acquire)
    }

    /// Read the current RSSI reading in dB.
    pub fn rssi_db(&self) -> i32 {
        self.rssi_db.load(Ordering::Acquire)
    }
}

/// Cross-task signals for persistence, time sync, and restarts.
#[derive(Debug)]
pub struct SystemSignals {
    persist: Mutex<RawMutex, RefCell<Option<SettingsSnapshot>>>,
    sync_requested: AtomicBool,
    restart_requested: AtomicBool,
}

#[cfg(test)]
mod tests {
    use super::*;
    use winderoo_firmware::model::{Direction, SettingsSnapshot, WinderStatus};
    use winderoo_firmware::time::TimeOfDay;

    #[test]
    fn status_cache_updates() {
        let snapshot = StatusSnapshot::from_state(
            &winderoo_firmware::model::RuntimeState {
                status: WinderStatus::Stopped,
                rotations_per_day: 220,
                direction: Direction::Both,
                motor_direction: winderoo_firmware::model::MotorDirection::Clockwise,
                timer: winderoo_firmware::model::TimerConfig {
                    enabled: false,
                    start_time: TimeOfDay::new(0, 0).unwrap(),
                },
                winder_enabled: true,
                custom_wind_duration_secs: 180,
                custom_wind_pause_secs: 15,
                rotation_duration_secs: 8,
                rtc: winderoo_firmware::model::RtcConfig {
                    gmt_offset: 0.0,
                    dst: false,
                },
                screen: winderoo_firmware::model::ScreenState {
                    equipped: false,
                    sleep: false,
                    schedule: winderoo_firmware::model::ScreenSchedule {
                        enabled: false,
                        start: TimeOfDay::new(0, 0).unwrap(),
                        end: TimeOfDay::new(0, 0).unwrap(),
                    },
                },
                routine: winderoo_firmware::model::RoutineState::idle(),
                cycle_progress: 0.0,
            },
            0,
            -55,
            "test",
        );
        let cache = StatusCache::new(snapshot.clone());
        assert_eq!(cache.snapshot(), snapshot);
    }

    #[test]
    fn system_signals_track_requests() {
        let signals = SystemSignals::new();
        let snapshot = SettingsSnapshot {
            status: WinderStatus::Stopped,
            rotations_per_day: 220,
            winder_enabled: true,
            timer_enabled: false,
            timer_hour: 0,
            timer_minutes: 0,
            direction: Direction::Both,
            custom_wind_duration_secs: 180,
            custom_wind_pause_secs: 15,
            rotation_duration_secs: 8,
            gmt_offset: 0.0,
            dst: false,
            screen_schedule_enabled: false,
            screen_schedule_start: TimeOfDay::new(0, 0).unwrap(),
            screen_schedule_end: TimeOfDay::new(0, 0).unwrap(),
            screen_sleep: false,
        };
        signals.request_persist(snapshot.clone());
        assert_eq!(signals.take_persist(), Some(snapshot));
        signals.request_sync();
        assert!(signals.take_sync());
        signals.request_restart();
        assert!(signals.take_restart());
    }
}

impl SystemSignals {
    /// Create a new, idle signal set.
    pub fn new() -> Self {
        Self {
            persist: Mutex::new(RefCell::new(None)),
            sync_requested: AtomicBool::new(false),
            restart_requested: AtomicBool::new(false),
        }
    }

    /// Stash a settings snapshot for persistence.
    pub fn request_persist(&self, snapshot: SettingsSnapshot) {
        self.persist.lock(|slot| {
            *slot.borrow_mut() = Some(snapshot);
        });
    }

    /// Take the pending snapshot, if any.
    pub fn take_persist(&self) -> Option<SettingsSnapshot> {
        self.persist.lock(|slot| slot.borrow_mut().take())
    }

    /// Mark an NTP sync request.
    pub fn request_sync(&self) {
        self.sync_requested.store(true, Ordering::Release);
    }

    /// Check and clear the sync request flag.
    pub fn take_sync(&self) -> bool {
        self.sync_requested.swap(false, Ordering::AcqRel)
    }

    /// Mark a restart request.
    pub fn request_restart(&self) {
        self.restart_requested.store(true, Ordering::Release);
    }

    /// Check and clear the restart request flag.
    pub fn take_restart(&self) -> bool {
        self.restart_requested.swap(false, Ordering::AcqRel)
    }
}
