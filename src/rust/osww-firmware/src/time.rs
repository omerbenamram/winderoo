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
        let hour_str = parts.next().ok_or_else(|| TimeError::InvalidFormat(input.to_string()))?;
        let minute_str = parts.next().ok_or_else(|| TimeError::InvalidFormat(input.to_string()))?;
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
}
