//! Embedded-hal adapters for Winderoo IO.
//!
//! This module is intentionally small and focused on bridging controller events
//! to hardware drivers. The controller logic remains in `winderoo-firmware` and
//! is fully testable on the host.

use embedded_hal::delay::DelayNs;
use embedded_hal::digital::OutputPin;
use embedded_hal::pwm::SetDutyCycle;
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

/// SSD1306 OLED display driver (128x64) using I2C + embedded-graphics.
///
/// This is an optional driver, enabled with the `oled` crate feature.
/// It keeps a tiny amount of UI state (current title) and redraws the screen on demand.
#[cfg(feature = "oled")]
pub struct Ssd1306Display<I2C> {
    display: ssd1306::Ssd1306<
        ssd1306::prelude::I2CInterface<I2C>,
        ssd1306::prelude::DisplaySize128x64,
        ssd1306::mode::BufferedGraphicsMode<ssd1306::prelude::DisplaySize128x64>,
    >,
    status_cache: &'static crate::state::StatusCache,
    title: heapless::String<24>,
}

#[cfg(feature = "oled")]
impl<I2C> Ssd1306Display<I2C>
where
    I2C: embedded_hal::i2c::I2c,
{
    /// Create and initialize the display.
    pub fn new(
        i2c: I2C,
        status_cache: &'static crate::state::StatusCache,
        invert: bool,
        rotate_180: bool,
    ) -> Self {
        use ssd1306::prelude::*;

        let interface = ssd1306::I2CDisplayInterface::new(i2c);
        let rotation = if rotate_180 {
            DisplayRotation::Rotate180
        } else {
            DisplayRotation::Rotate0
        };

        let mut display =
            ssd1306::Ssd1306::new(interface, DisplaySize128x64, rotation).into_buffered_graphics_mode();
        let _ = display.init();
        let _ = display.set_invert(invert);
        let _ = display.flush();

        Self {
            display,
            status_cache,
            title: heapless::String::new(),
        }
    }

    fn redraw(&mut self, notification: Option<&str>) {
        use embedded_graphics::mono_font::{ascii::FONT_6X10, MonoTextStyle};
        use embedded_graphics::pixelcolor::BinaryColor;
        use embedded_graphics::prelude::*;
        use embedded_graphics::primitives::{PrimitiveStyle, Rectangle};
        use embedded_graphics::text::{Baseline, Text};

        let style = MonoTextStyle::new(&FONT_6X10, BinaryColor::On);

        self.display.clear_buffer();

        // Simple frame + title.
        let _ = Rectangle::new(Point::new(0, 0), Size::new(128, 64))
            .into_styled(PrimitiveStyle::with_stroke(BinaryColor::On, 1))
            .draw(&mut self.display);

        let _ = Text::with_baseline(&self.title, Point::new(4, 2), style, Baseline::Top)
            .draw(&mut self.display);

        if let Some(message) = notification {
            // Center-ish notification (wrapped).
            let mut y = 22;
            for chunk in message.as_bytes().chunks(18).take(3) {
                if let Ok(line) = core::str::from_utf8(chunk) {
                    let _ = Text::with_baseline(line, Point::new(4, y), style, Baseline::Top)
                        .draw(&mut self.display);
                }
                y += 12;
            }
        } else {
            // Dynamic snapshot values.
            let snapshot = self.status_cache.snapshot();
            let mut line1 = heapless::String::<32>::new();
            let _ = line1.push_str(snapshot.status.as_str());
            let _ = Text::with_baseline(&line1, Point::new(4, 18), style, Baseline::Top)
                .draw(&mut self.display);

            let mut line2 = heapless::String::<32>::new();
            let _ = core::fmt::write(
                &mut line2,
                format_args!("TPD {} {:?}", snapshot.rotations_per_day, snapshot.direction),
            );
            let _ = Text::with_baseline(&line2, Point::new(4, 30), style, Baseline::Top)
                .draw(&mut self.display);

            let mut line3 = heapless::String::<32>::new();
            let _ = core::fmt::write(
                &mut line3,
                format_args!("Timer {:02}:{:02} {}", snapshot.timer_hour, snapshot.timer_minutes, if snapshot.timer_enabled { "ON" } else { "OFF" }),
            );
            let _ = Text::with_baseline(&line3, Point::new(4, 42), style, Baseline::Top)
                .draw(&mut self.display);
        }

        let _ = self.display.flush();
    }
}

#[cfg(feature = "oled")]
impl<I2C> DisplayControl for Ssd1306Display<I2C>
where
    I2C: embedded_hal::i2c::I2c,
{
    fn clear(&mut self) {
        self.display.clear_buffer();
        let _ = self.display.flush();
    }

    fn draw_static(&mut self, title: &str) {
        self.title.clear();
        let _ = self.title.push_str(title);
        self.redraw(None);
    }

    fn draw_dynamic(&mut self) {
        self.redraw(None);
    }

    fn notify(&mut self, message: &str) {
        self.redraw(Some(message));
    }
}

/// A no-op LED implementation for builds without a status LED.
#[derive(Debug, Default)]
pub struct NoopLed;

impl LedControl for NoopLed {
    fn apply_pattern<D: DelayNs>(&mut self, _pattern: LedPattern, _delay: &mut D) {}
}

/// Motor driver for an H-bridge with two direction pins.
#[derive(Debug)]
pub struct MotorDriver<PinA, PinB> {
    pin_a: PinA,
    pin_b: PinB,
}

impl<PinA, PinB> MotorDriver<PinA, PinB>
where
    PinA: OutputPin,
    PinB: OutputPin,
{
    /// Create a new motor driver using the provided GPIO pins.
    pub fn new(pin_a: PinA, pin_b: PinB) -> Self {
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

impl<PinA, PinB> MotorControl for MotorDriver<PinA, PinB>
where
    PinA: OutputPin,
    PinB: OutputPin,
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
    PWM: SetDutyCycle,
{
    pwm: PWM,
}

impl<PWM> LedPwmDriver<PWM>
where
    PWM: SetDutyCycle,
{
    /// Create a new LED driver for the provided PWM channel.
    pub fn new(pwm: PWM) -> Self {
        Self { pwm }
    }

    fn set_duty_fraction(&mut self, numerator: u16, denominator: u16) {
        let _ = self.pwm.set_duty_cycle_fraction(numerator, denominator);
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
    PWM: SetDutyCycle,
{
    fn apply_pattern<D: DelayNs>(&mut self, pattern: LedPattern, delay: &mut D) {
        match pattern {
            LedPattern::Off => {
                let _ = self.pwm.set_duty_cycle_fully_off();
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
