//! RTC helpers and time conversions for embassy-based runtimes.
//!
//! The core firmware logic relies on `TimeOfDay` and epoch timestamps. This
//! module provides a minimal RTC abstraction plus helpers to convert between
//! epoch seconds and local time-of-day.

use core::cell::Cell;

use crate::tasks::TimeSource;
use embassy_sync::blocking_mutex::raw::NoopRawMutex;
use embassy_sync::blocking_mutex::Mutex;
use winderoo_firmware::time::{time_of_day_from_epoch, TimeOfDay};

/// Abstraction over a real-time clock used by the firmware.
pub trait RtcClock {
    /// Read the current epoch in seconds.
    fn now_epoch(&self) -> u64;
    /// Read the current local time-of-day.
    fn now_time_of_day(&self) -> TimeOfDay {
        time_of_day_from_epoch(self.now_epoch())
    }
    /// Set the RTC epoch in seconds.
    fn set_epoch(&mut self, epoch: u64);
}

/// Simple in-memory RTC useful for host-side tests.
#[derive(Debug)]
pub struct SoftwareRtc {
    epoch: Mutex<NoopRawMutex, Cell<u64>>,
}

impl SoftwareRtc {
    /// Create a new software RTC starting at the provided epoch.
    pub fn new(initial_epoch: u64) -> Self {
        Self {
            epoch: Mutex::new(Cell::new(initial_epoch)),
        }
    }

    /// Advance the epoch by a number of seconds.
    pub fn advance(&self, seconds: u64) {
        self.epoch
            .lock(|value| value.set(value.get().saturating_add(seconds)));
    }
}

impl RtcClock for SoftwareRtc {
    fn now_epoch(&self) -> u64 {
        self.epoch.lock(|value| value.get())
    }

    fn set_epoch(&mut self, epoch: u64) {
        self.epoch.lock(|value| value.set(epoch));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn software_rtc_updates_epoch() {
        let rtc = SoftwareRtc::new(100);
        assert_eq!(rtc.now_epoch(), 100);
        rtc.advance(20);
        assert_eq!(rtc.now_epoch(), 120);
    }
}

/// Adapter that exposes an [`RtcClock`] as a [`TimeSource`].
#[derive(Debug)]
pub struct RtcTimeSource<R>
where
    R: RtcClock,
{
    rtc: R,
}

impl<R> RtcTimeSource<R>
where
    R: RtcClock,
{
    /// Wrap an RTC clock as a time source.
    pub fn new(rtc: R) -> Self {
        Self { rtc }
    }

    /// Access the underlying RTC clock mutably.
    pub fn rtc_mut(&mut self) -> &mut R {
        &mut self.rtc
    }
}

impl<R> TimeSource for RtcTimeSource<R>
where
    R: RtcClock,
{
    fn now_epoch(&mut self) -> u64 {
        self.rtc.now_epoch()
    }

    fn now_time_of_day(&mut self) -> TimeOfDay {
        self.rtc.now_time_of_day()
    }
}
