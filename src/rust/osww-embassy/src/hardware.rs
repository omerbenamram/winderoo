//! Embedded-hal adapters for Winderoo IO.
//!
//! This module is intentionally small and focused on bridging controller events
//! to hardware drivers. The controller logic remains in `winderoo-firmware` and
//! is fully testable on the host.

use embedded_hal::delay::DelayNs;
use embedded_hal::digital::OutputPin;
use embedded_hal::pwm::PwmPin;
use winderoo_firmware::controller::ControllerEvent;
use winderoo_firmware::hardware::LedPattern;
use winderoo_firmware::model::{MotorDirection, SettingsSnapshot};

/// Motor driver trait implemented by concrete hardware drivers.
pub trait MotorControl {
    /// Start the motor in the given direction.
    fn start(&mut self, direction: MotorDirection);
    /// Stop the motor.
    fn stop(&mut self);
}

/// LED driver trait for applying status patterns.
pub trait LedControl {
    /// Apply an LED pattern using the provided delay implementation.
    fn apply_pattern<D: DelayNs>(&mut self, pattern: LedPattern, delay: &mut D);
}

/// Display driver trait for status rendering.
pub trait DisplayControl {
    /// Clear the display.
    fn clear(&mut self);
    /// Draw the static UI chrome with a title.
    fn draw_static(&mut self, title: &str);
    /// Draw the dynamic UI values.
    fn draw_dynamic(&mut self);
    /// Render a notification banner.
    fn notify(&mut self, message: &str);
}

/// System hooks for persistence, time sync, and resets.
pub trait SystemHooks {
    /// Persist the latest settings snapshot.
    fn persist_settings(&mut self, snapshot: &SettingsSnapshot);
    /// Trigger a time sync (NTP/RTC).
    fn sync_time(&mut self);
    /// Restart the device.
    fn restart(&mut self);
}

/// A no-op display implementation for builds without a screen.
#[derive(Debug, Default)]
pub struct NoopDisplay;

impl DisplayControl for NoopDisplay {
    fn clear(&mut self) {}
    fn draw_static(&mut self, _title: &str) {}
    fn draw_dynamic(&mut self) {}
    fn notify(&mut self, _message: &str) {}
}

/// A no-op LED implementation for builds without a status LED.
#[derive(Debug, Default)]
pub struct NoopLed;

impl LedControl for NoopLed {
    fn apply_pattern<D: DelayNs>(&mut self, _pattern: LedPattern, _delay: &mut D) {}
}

/// Motor driver for an H-bridge with two direction pins.
#[derive(Debug)]
pub struct MotorDriver<PIN_A, PIN_B> {
    pin_a: PIN_A,
    pin_b: PIN_B,
}

impl<PIN_A, PIN_B> MotorDriver<PIN_A, PIN_B>
where
    PIN_A: OutputPin,
    PIN_B: OutputPin,
{
    /// Create a new motor driver using the provided GPIO pins.
    pub fn new(pin_a: PIN_A, pin_b: PIN_B) -> Self {
        Self { pin_a, pin_b }
    }

    fn drive_clockwise(&mut self) {
        let _ = self.pin_a.set_high();
        let _ = self.pin_b.set_low();
    }

    fn drive_counter_clockwise(&mut self) {
        let _ = self.pin_a.set_low();
        let _ = self.pin_b.set_high();
    }

    fn drive_stop(&mut self) {
        let _ = self.pin_a.set_low();
        let _ = self.pin_b.set_low();
    }
}

impl<PIN_A, PIN_B> MotorControl for MotorDriver<PIN_A, PIN_B>
where
    PIN_A: OutputPin,
    PIN_B: OutputPin,
{
    fn start(&mut self, direction: MotorDirection) {
        match direction {
            MotorDirection::Clockwise => self.drive_clockwise(),
            MotorDirection::CounterClockwise => self.drive_counter_clockwise(),
        }
    }

    fn stop(&mut self) {
        self.drive_stop();
    }
}

/// PWM LED driver that can render the same patterns as the Arduino firmware.
#[derive(Debug)]
pub struct LedPwmDriver<PWM>
where
    PWM: PwmPin<Duty = u16>,
{
    pwm: PWM,
}

impl<PWM> LedPwmDriver<PWM>
where
    PWM: PwmPin<Duty = u16>,
{
    /// Create a new LED driver for the provided PWM channel.
    pub fn new(mut pwm: PWM) -> Self {
        pwm.enable();
        Self { pwm }
    }

    fn set_duty_fraction(&mut self, numerator: u16, denominator: u16) {
        let max = self.pwm.get_max_duty();
        let duty = ((max as u32) * (numerator as u32) / (denominator as u32)) as u16;
        self.pwm.set_duty(duty);
    }

    fn ramp(&mut self, delay: &mut impl DelayNs, step_delay_ms: u32) {
        for i in 0..=255u16 {
            self.set_duty_fraction(i, 255);
            delay.delay_ms(step_delay_ms);
        }
        for i in (0..=255u16).rev() {
            self.set_duty_fraction(i, 255);
            delay.delay_ms(step_delay_ms);
        }
    }
}

impl<PWM> LedControl for LedPwmDriver<PWM>
where
    PWM: PwmPin<Duty = u16>,
{
    fn apply_pattern<D: DelayNs>(&mut self, pattern: LedPattern, delay: &mut D) {
        match pattern {
            LedPattern::Off => {
                self.pwm.set_duty(0);
            }
            LedPattern::Pulse => {
                self.ramp(delay, 7);
            }
            LedPattern::SlowBlink => {
                for _ in 0..3 {
                    self.ramp(delay, 7);
                    delay.delay_ms(150);
                }
            }
            LedPattern::FastBlink => {
                for _ in 0..12 {
                    self.ramp(delay, 2);
                    delay.delay_ms(50);
                }
            }
        }
    }
}

/// Dispatches controller events to hardware drivers.
#[derive(Debug)]
pub struct EventDispatcher<M, L, D, S, T>
where
    M: MotorControl,
    L: LedControl,
    D: DisplayControl,
    S: SystemHooks,
    T: DelayNs,
{
    motor: M,
    led: L,
    display: D,
    system: S,
    delay: T,
}

impl<M, L, D, S, T> EventDispatcher<M, L, D, S, T>
where
    M: MotorControl,
    L: LedControl,
    D: DisplayControl,
    S: SystemHooks,
    T: DelayNs,
{
    /// Create a new dispatcher with the provided hardware drivers.
    pub fn new(motor: M, led: L, display: D, system: S, delay: T) -> Self {
        Self {
            motor,
            led,
            display,
            system,
            delay,
        }
    }

    /// Handle a single controller event.
    pub fn handle_event(&mut self, event: ControllerEvent) {
        match event {
            ControllerEvent::MotorStart(direction) => self.motor.start(direction),
            ControllerEvent::MotorStop => self.motor.stop(),
            ControllerEvent::PauseSeconds(seconds) => {
                self.delay.delay_ms(seconds.saturating_mul(1000));
            }
            ControllerEvent::DisplayClear => self.display.clear(),
            ControllerEvent::DisplayStatic { title } => self.display.draw_static(title.as_str()),
            ControllerEvent::DisplayDynamic => self.display.draw_dynamic(),
            ControllerEvent::DisplayNotification(message) => self.display.notify(message.as_str()),
            ControllerEvent::Led(pattern) => self.led.apply_pattern(pattern, &mut self.delay),
            ControllerEvent::PersistSettings(snapshot) => self.system.persist_settings(&snapshot),
            ControllerEvent::SyncTime => self.system.sync_time(),
            ControllerEvent::RestartDevice => self.system.restart(),
        }
    }

    /// Access the underlying motor driver mutably.
    pub fn motor_mut(&mut self) -> &mut M {
        &mut self.motor
    }

    /// Access the underlying display driver mutably.
    pub fn display_mut(&mut self) -> &mut D {
        &mut self.display
    }
}
