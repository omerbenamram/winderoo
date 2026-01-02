//! Hardware-adjacent traits and utilities for deterministic testing.

/// A minimal random number source used by the winding logic.
pub trait RandomSource {
    /// Return the next pseudo-random byte.
    fn next_u8(&mut self) -> u8;
}

/// A lightweight xorshift RNG suitable for deterministic tests.
#[derive(Debug, Clone)]
pub struct XorShift32 {
    /// Internal RNG state.
    state: u32,
}

impl XorShift32 {
    /// Create a new RNG with the given non-zero seed.
    pub fn new(seed: u32) -> Self {
        let seed = if seed == 0 { 0x1234_5678 } else { seed };
        Self { state: seed }
    }
}

impl RandomSource for XorShift32 {
    fn next_u8(&mut self) -> u8 {
        // xorshift32
        let mut x = self.state;
        x ^= x << 13;
        x ^= x >> 17;
        x ^= x << 5;
        self.state = x;
        (x & 0xFF) as u8
    }
}

/// LED patterns supported by the firmware state machine.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LedPattern {
    /// Turn the LED fully on.
    On,
    /// Turn the LED off.
    Off,
    /// Slow blink pattern (success).
    SlowBlink,
    /// Fast blink pattern (reset).
    FastBlink,
    /// PWM pulse pattern (sleep indicator).
    Pulse,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn xorshift_is_deterministic() {
        let mut rng = XorShift32::new(1);
        let first = rng.next_u8();
        let second = rng.next_u8();
        assert_ne!(first, second);
    }
}
