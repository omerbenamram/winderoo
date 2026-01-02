//! Pure helper logic used by the OLED UI implementation.
//!
//! This module is host-testable and intentionally has **no ESP-IDF dependencies**.

/// Map RSSI (dBm) to the number of bars the UI should display.
///
/// The thresholds mirror the original Arduino firmware.
pub fn wifi_bars_for_rssi(rssi: i32) -> u8 {
    if rssi > -50 {
        4
    } else if rssi > -60 {
        3
    } else if rssi > -70 {
        2
    } else {
        1
    }
}

/// Compute a progress bar width for a given fraction.
pub fn progress_width(progress: f32, total_width: u32) -> u32 {
    let clamped = progress.clamp(0.0, 1.0);
    (clamped * (total_width as f32)) as u32
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn wifi_bars_thresholds() {
        assert_eq!(wifi_bars_for_rssi(-40), 4);
        assert_eq!(wifi_bars_for_rssi(-55), 3);
        assert_eq!(wifi_bars_for_rssi(-65), 2);
        assert_eq!(wifi_bars_for_rssi(-80), 1);
    }

    #[test]
    fn progress_width_clamps() {
        assert_eq!(progress_width(-1.0, 128), 0);
        assert_eq!(progress_width(0.5, 128), 64);
        assert_eq!(progress_width(2.0, 128), 128);
    }
}
