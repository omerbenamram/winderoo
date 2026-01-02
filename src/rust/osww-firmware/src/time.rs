//! Time-related helpers for the firmware domain model.

use core::fmt;
use thiserror::Error;

/// Errors returned when parsing or constructing a [`TimeOfDay`].
#[derive(Debug, Error, Clone, PartialEq, Eq)]
pub enum TimeError {
    /// Hour was outside the 0-23 range.
    #[error("hour out of range: {0}")]
    InvalidHour(u8),
    /// Minute was outside the 0-59 range.
    #[error("minute out of range: {0}")]
    InvalidMinute(u8),
    /// The input string was not in the expected `HH:MM` format.
    #[error("invalid time format: {0}")]
    InvalidFormat(String),
}

/// A 24-hour clock time without a date or timezone.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TimeOfDay {
    /// Hour in 24-hour form (0-23).
    pub hour: u8,
    /// Minute (0-59).
    pub minute: u8,
}

impl TimeOfDay {
    /// Construct a new [`TimeOfDay`] after validating the hour and minute ranges.
    pub fn new(hour: u8, minute: u8) -> Result<Self, TimeError> {
        if hour > 23 {
            return Err(TimeError::InvalidHour(hour));
        }
        if minute > 59 {
            return Err(TimeError::InvalidMinute(minute));
        }
        Ok(Self { hour, minute })
    }

    /// Parse a `HH:MM` string into a [`TimeOfDay`].
    pub fn parse_hh_mm(input: &str) -> Result<Self, TimeError> {
        let mut parts = input.split(':');
        let hour_str = parts
            .next()
            .ok_or_else(|| TimeError::InvalidFormat(input.to_string()))?;
        let minute_str = parts
            .next()
            .ok_or_else(|| TimeError::InvalidFormat(input.to_string()))?;
        if parts.next().is_some() {
            return Err(TimeError::InvalidFormat(input.to_string()));
        }
        let hour: u8 = hour_str
            .parse()
            .map_err(|_| TimeError::InvalidFormat(input.to_string()))?;
        let minute: u8 = minute_str
            .parse()
            .map_err(|_| TimeError::InvalidFormat(input.to_string()))?;
        Self::new(hour, minute)
    }

    /// Render the time as a zero-padded `HH:MM` string.
    pub fn to_hh_mm(&self) -> String {
        format!("{:02}:{:02}", self.hour, self.minute)
    }

    /// Total minutes since midnight.
    pub fn total_minutes(&self) -> u16 {
        (self.hour as u16) * 60 + (self.minute as u16)
    }
}

/// Convert a UTC epoch into a local [`TimeOfDay`] using a GMT offset and DST flag.
pub fn time_of_day_from_epoch(epoch: u64, gmt_offset: f32, dst: bool) -> TimeOfDay {
    let adjusted = (epoch as i128) + (offset_seconds(gmt_offset, dst) as i128);
    let seconds_in_day = ((adjusted % 86_400) + 86_400) % 86_400;
    let hour = (seconds_in_day / 3600) as u8;
    let minute = ((seconds_in_day % 3600) / 60) as u8;
    TimeOfDay::new(hour, minute).unwrap_or(TimeOfDay { hour: 0, minute: 0 })
}

/// Convert a UTC epoch into a "local epoch" by applying the configured GMT offset and DST flag.
///
/// This mirrors the legacy Arduino firmware behavior where the "RTC epoch" is effectively
/// timezone-shifted (UTC + offset), and clients format it in UTC to display local wall clock time.
///
/// Notes:
/// - `epoch == 0` is treated as "unset" and returned unchanged.
/// - The returned value is clamped at 0 to avoid underflow for extreme inputs.
pub fn epoch_with_offset(epoch: u64, gmt_offset: f32, dst: bool) -> u64 {
    if epoch == 0 {
        return 0;
    }
    let adjusted = (epoch as i128) + (offset_seconds(gmt_offset, dst) as i128);
    if adjusted <= 0 {
        0
    } else {
        adjusted as u64
    }
}

fn offset_seconds(gmt_offset: f32, dst: bool) -> i64 {
    let mut offset_secs = (gmt_offset * 3600.0).round() as i64;
    if dst {
        offset_secs += 3600;
    }
    offset_secs
}

impl fmt::Display for TimeOfDay {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.to_hh_mm())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_valid_time() {
        let time = TimeOfDay::parse_hh_mm("09:05").expect("valid time");
        assert_eq!(time.hour, 9);
        assert_eq!(time.minute, 5);
        assert_eq!(time.to_hh_mm(), "09:05");
    }

    #[test]
    fn reject_invalid_time() {
        assert!(TimeOfDay::parse_hh_mm("24:00").is_err());
        assert!(TimeOfDay::parse_hh_mm("10:60").is_err());
        assert!(TimeOfDay::parse_hh_mm("10").is_err());
        assert!(TimeOfDay::parse_hh_mm("10:00:00").is_err());
    }

    #[test]
    fn total_minutes() {
        let time = TimeOfDay::new(2, 30).unwrap();
        assert_eq!(time.total_minutes(), 150);
    }

    #[test]
    fn epoch_to_time_of_day_with_offset() {
        // 01:30 UTC, offset +2 => 03:30
        let time = time_of_day_from_epoch(90 * 60, 2.0, false);
        assert_eq!(time, TimeOfDay::new(3, 30).unwrap());
    }

    #[test]
    fn epoch_to_time_of_day_with_dst() {
        // 23:30 UTC, offset -5 with DST => 19:30
        let epoch = 23 * 3600 + 30 * 60;
        let time = time_of_day_from_epoch(epoch, -5.0, true);
        assert_eq!(time, TimeOfDay::new(19, 30).unwrap());
    }

    #[test]
    fn epoch_with_offset_shifts_seconds() {
        // 01:00 UTC, offset +2 => local epoch corresponds to 03:00 UTC.
        let epoch = 3600;
        assert_eq!(epoch_with_offset(epoch, 2.0, false), 10_800);
    }

    #[test]
    fn epoch_with_offset_preserves_zero() {
        assert_eq!(epoch_with_offset(0, 5.0, true), 0);
    }
}
