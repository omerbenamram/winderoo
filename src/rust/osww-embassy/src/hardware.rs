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
    /// Toggle a small "saving" indicator (best-effort; implementations may ignore it).
    fn set_saving(&mut self, saving: bool);
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
    fn set_saving(&mut self, _saving: bool) {}
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
    saving_frames: u8,
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
            saving_frames: 0,
        }
    }

    fn redraw(&mut self, notification: Option<&str>) {
        use embedded_graphics::mono_font::{
            ascii::{FONT_10X20, FONT_6X10},
            MonoTextStyle,
        };
        use embedded_graphics::pixelcolor::BinaryColor;
        use embedded_graphics::prelude::*;
        use embedded_graphics::primitives::{Circle, Line, PrimitiveStyle, Rectangle, Triangle};
        use embedded_graphics::text::{Baseline, Text};

        const WIDTH: i32 = 128;

        // Mirrors the Arduino layout:
        // - Header banner: 0..14
        // - Static box: lines at y=14 and y=50, vertical divider at x=64
        // - Dynamic values: big text in the middle
        // - Status row: wifi icon + timer at y≈54
        const HEADER_H: i32 = 14;
        const MIDLINE_Y: i32 = 50;
        const DIVIDER_X: i32 = 64;

        let style_small = MonoTextStyle::new(&FONT_6X10, BinaryColor::On);
        let style_big = MonoTextStyle::new(&FONT_10X20, BinaryColor::On);

        fn center_x(font: &embedded_graphics::mono_font::MonoFont, text: &str, width: i32) -> i32 {
            let char_w = font.character_size.width as i32;
            let w = (text.as_bytes().len() as i32) * char_w;
            ((width - w).max(0)) / 2
        }

        self.display.clear_buffer();

        // Header: title (centered) + underline.
        let title_x = center_x(&FONT_6X10, self.title.as_str(), WIDTH);
        let _ = Text::with_baseline(
            self.title.as_str(),
            Point::new(title_x, 3),
            style_small,
            Baseline::Top,
        )
        .draw(&mut self.display);
        let _ = Line::new(Point::new(0, HEADER_H), Point::new(WIDTH - 1, HEADER_H))
            .into_styled(PrimitiveStyle::with_stroke(BinaryColor::On, 1))
            .draw(&mut self.display);

        // Static grid.
        let _ = Line::new(Point::new(0, MIDLINE_Y), Point::new(WIDTH - 1, MIDLINE_Y))
            .into_styled(PrimitiveStyle::with_stroke(BinaryColor::On, 1))
            .draw(&mut self.display);
        let _ = Line::new(
            Point::new(DIVIDER_X, HEADER_H),
            Point::new(DIVIDER_X, MIDLINE_Y),
        )
        .into_styled(PrimitiveStyle::with_stroke(BinaryColor::On, 1))
        .draw(&mut self.display);

        // Labels.
        let _ = Text::with_baseline("TPD", Point::new(4, 18), style_small, Baseline::Top)
            .draw(&mut self.display);
        let _ = Text::with_baseline("DIR", Point::new(71, 18), style_small, Baseline::Top)
            .draw(&mut self.display);

        if let Some(message) = notification {
            // Notification banner: filled header bar + centered text (wrapped-ish).
            let _ = Rectangle::new(Point::new(0, 0), Size::new(WIDTH as u32, HEADER_H as u32))
                .into_styled(PrimitiveStyle::with_fill(BinaryColor::On))
                .draw(&mut self.display);

            // Text in "off" (inverted).
            let style_inv = MonoTextStyle::new(&FONT_6X10, BinaryColor::Off);

            // Keep it simple: up to 3 lines, roughly centered.
            let mut y = 2;
            for chunk in message.as_bytes().chunks(18).take(3) {
                if let Ok(line) = core::str::from_utf8(chunk) {
                    let x = center_x(&FONT_6X10, line, WIDTH);
                    let _ = Text::with_baseline(line, Point::new(x, y), style_inv, Baseline::Top)
                        .draw(&mut self.display);
                }
                y += 10;
            }
        } else {
            // Dynamic snapshot values.
            let snapshot = self.status_cache.snapshot();

            // Left big number: rotations per day.
            let mut tpd = heapless::String::<8>::new();
            let _ = core::fmt::write(&mut tpd, format_args!("{}", snapshot.rotations_per_day));
            let tpd_x = 8;
            let _ =
                Text::with_baseline(&tpd, Point::new(tpd_x, 30), style_big, Baseline::Top)
                    .draw(&mut self.display);

            // Right big direction.
            let dir = snapshot.direction.as_api_str();
            // Aim to center within the right panel.
            let dir_x = DIVIDER_X + 10;
            let _ = Text::with_baseline(dir, Point::new(dir_x, 30), style_big, Baseline::Top)
                .draw(&mut self.display);

            // Progress bar (derived from epochs, since API parity doesn't expose cycleProgress).
            let start = snapshot.start_time_epoch;
            let now = snapshot.current_time_epoch;
            let end = snapshot.estimated_routine_finish_epoch;
            let ratio = if end > start && now >= start {
                let elapsed = (now - start) as f32;
                let total = (end - start) as f32;
                (elapsed / total).clamp(0.0, 1.0)
            } else {
                0.0
            };
            let bar_w = (ratio * (WIDTH as f32)) as i32;
            if bar_w > 0 {
                let _ = Rectangle::new(
                    Point::new(0, MIDLINE_Y),
                    Size::new(bar_w as u32, 2),
                )
                .into_styled(PrimitiveStyle::with_fill(BinaryColor::On))
                .draw(&mut self.display);
            }

            // Wi‑Fi icon + bars (Arduino thresholds).
            // Triangle + mast.
            let _ = Triangle::new(
                Point::new(4, 54),
                Point::new(10, 54),
                Point::new(7, 58),
            )
            .into_styled(PrimitiveStyle::with_stroke(BinaryColor::On, 1))
            .draw(&mut self.display);
            let _ = Line::new(Point::new(7, 58), Point::new(7, 62))
                .into_styled(PrimitiveStyle::with_stroke(BinaryColor::On, 1))
                .draw(&mut self.display);

            let bars = if snapshot.rssi_db > -50 {
                4
            } else if snapshot.rssi_db > -60 {
                3
            } else if snapshot.rssi_db > -70 {
                2
            } else {
                1
            };

            // Bars area: x=14.., y=55..63
            for i in 0..bars {
                let x = 14 + i * 4;
                let h = 2 + i * 2;
                let y = 63 - h;
                let _ = Rectangle::new(Point::new(x, y), Size::new(2, h as u32))
                    .into_styled(PrimitiveStyle::with_fill(BinaryColor::On))
                    .draw(&mut self.display);
            }

            // Timer badge (right aligned-ish).
            if snapshot.timer_enabled {
                let mut timer = heapless::String::<16>::new();
                let _ = core::fmt::write(
                    &mut timer,
                    format_args!("TIMER {:02}:{:02}", snapshot.timer_hour, snapshot.timer_minutes),
                );
                // Rough right-align using fixed font width.
                let x = (WIDTH - (timer.as_bytes().len() as i32) * (FONT_6X10.character_size.width as i32) - 2)
                    .max(DIVIDER_X + 2);
                let _ = Text::with_baseline(&timer, Point::new(x, 56), style_small, Baseline::Top)
                    .draw(&mut self.display);
            }
        }

        // "Saving" indicator (small circle near the top-left).
        // In the Arduino firmware this is shown during /api/update to indicate settings persistence.
        if self.saving_frames > 0 {
            let color = if notification.is_some() {
                // Notification banner is "inverted" (white background), so draw the icon in "off".
                BinaryColor::Off
            } else {
                BinaryColor::On
            };
            let _ = Circle::new(Point::new(2, 2), 4)
                .into_styled(PrimitiveStyle::with_stroke(color, 1))
                .draw(&mut self.display);
            self.saving_frames = self.saving_frames.saturating_sub(1);
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

    fn set_saving(&mut self, saving: bool) {
        // Show for a couple of frames; the controller tick redraws every ~500ms.
        self.saving_frames = if saving { 2 } else { 0 };
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

/// PWM motor driver for MX1508-style two-PWM H-bridge boards.
///
/// This mirrors the optional Arduino build that uses `ESP32MX1508`:
/// - CW: PWM on A, B off
/// - CCW: PWM on B, A off
/// - Stop: both off (coast)
#[derive(Debug)]
pub struct MotorPwmDriver<PWMA, PWMB>
where
    PWMA: SetDutyCycle,
    PWMB: SetDutyCycle,
{
    pwm_a: PWMA,
    pwm_b: PWMB,
    /// 0..=255 scale, to match Arduino expectations.
    speed: u8,
}

impl<PWMA, PWMB> MotorPwmDriver<PWMA, PWMB>
where
    PWMA: SetDutyCycle,
    PWMB: SetDutyCycle,
{
    /// Create a new PWM motor driver.
    pub fn new(pwm_a: PWMA, pwm_b: PWMB, speed: u8) -> Self {
        Self { pwm_a, pwm_b, speed }
    }

    fn set_outputs(&mut self, duty_a: u8, duty_b: u8) {
        let _ = self.pwm_a.set_duty_cycle_fraction(duty_a as u16, 255);
        let _ = self.pwm_b.set_duty_cycle_fraction(duty_b as u16, 255);
    }
}

impl<PWMA, PWMB> MotorControl for MotorPwmDriver<PWMA, PWMB>
where
    PWMA: SetDutyCycle,
    PWMB: SetDutyCycle,
{
    fn start(&mut self, direction: MotorDirection) {
        match direction {
            MotorDirection::Clockwise => self.set_outputs(self.speed, 0),
            MotorDirection::CounterClockwise => self.set_outputs(0, self.speed),
        }
    }

    fn stop(&mut self) {
        let _ = self.pwm_a.set_duty_cycle_fully_off();
        let _ = self.pwm_b.set_duty_cycle_fully_off();
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
            LedPattern::On => {
                self.set_duty_fraction(255, 255);
            }
            LedPattern::Off => {
                let _ = self.pwm.set_duty_cycle_fully_off();
            }
            LedPattern::Pulse => {
                self.ramp(delay, 7);
            }
            LedPattern::SlowBlink => {
                // Match Arduino behavior (4 slow "breaths").
                for _ in 0..4 {
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
            ControllerEvent::PersistSettings(snapshot) => {
                // Best-effort "saving" indicator on supported displays.
                self.display.set_saving(true);
                self.system.persist_settings(&snapshot);
            }
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
