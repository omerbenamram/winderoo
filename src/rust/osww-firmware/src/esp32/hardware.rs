//! ESP32 hardware drivers (motor, LED, button, optional OLED).
//!
//! This module is ESP-IDF/ESP32 specific and is compiled only as part of the `esp32` build.

use crate::hardware::LedPattern;
use crate::model::{MotorDirection, RuntimeState};
#[cfg(not(feature = "pwm-motor"))]
use esp_idf_hal::gpio::Output;
use esp_idf_hal::gpio::{Input, PinDriver, Pull};
use esp_idf_hal::i2c::{I2cConfig, I2cDriver};
use esp_idf_hal::ledc::{config::TimerConfig, LedcDriver, LedcTimerDriver};
use esp_idf_hal::prelude::*;
use std::thread;
use std::time::Duration;

#[cfg(feature = "oled")]
use super::oled::OledDisplay;
use super::Esp32Error;

type MotorPinA = esp_idf_hal::gpio::Gpio25;
type MotorPinB = esp_idf_hal::gpio::Gpio26;
type ButtonPin = esp_idf_hal::gpio::Gpio13;

pub(super) struct Hardware {
    motor: MotorControl,
    led: LedControl,
    button: PinDriver<'static, ButtonPin, Input>,
    #[cfg(feature = "oled")]
    display: Option<OledDisplay>,
}

impl Hardware {
    pub(super) fn new(
        pins: esp_idf_hal::gpio::Pins,
        ledc: esp_idf_hal::ledc::LEDC,
        i2c0: esp_idf_hal::i2c::I2C0,
    ) -> Result<Self, Esp32Error> {
        let pins = pins;
        let ledc = ledc;
        let _i2c0 = i2c0;

        let esp_idf_hal::ledc::LEDC {
            timer0,
            timer1,
            channel0,
            channel1,
            channel2,
            ..
        } = ledc;

        #[cfg(not(feature = "pwm-motor"))]
        let _ = (timer1, channel1, channel2);

        let led_timer =
            LedcTimerDriver::new(timer0, &TimerConfig::default().frequency(5.kHz().into()))?;
        let led_driver = LedcDriver::new(channel0, &led_timer, pins.gpio2)?;
        let led = LedControl::new(led_timer, led_driver);

        let motor = MotorControl::new(
            pins.gpio25,
            pins.gpio26,
            #[cfg(feature = "pwm-motor")]
            timer1,
            #[cfg(feature = "pwm-motor")]
            channel1,
            #[cfg(feature = "pwm-motor")]
            channel2,
        )?;

        let mut button = PinDriver::input(pins.gpio13)?;
        button.set_pull(Pull::Down)?;

        #[cfg(feature = "oled")]
        let display = {
            let config = I2cConfig::new().baudrate(400.kHz().into());
            let i2c = I2cDriver::new(_i2c0, pins.gpio21, pins.gpio22, &config)?;
            Some(OledDisplay::new(i2c)?)
        };

        Ok(Self {
            motor,
            led,
            button,
            #[cfg(feature = "oled")]
            display,
        })
    }

    pub(super) fn motor_start(&mut self, direction: MotorDirection) -> Result<(), Esp32Error> {
        self.motor.start(direction)
    }

    pub(super) fn motor_stop(&mut self) -> Result<(), Esp32Error> {
        self.motor.stop()
    }

    pub(super) fn led_trigger(&mut self, pattern: LedPattern) -> Result<(), Esp32Error> {
        self.led.trigger(pattern)
    }

    pub(super) fn button_is_high(&self) -> bool {
        self.button.is_high()
    }

    pub(super) fn display_clear(&mut self) -> Result<(), Esp32Error> {
        #[cfg(feature = "oled")]
        if let Some(display) = &mut self.display {
            display.clear()?;
        }
        Ok(())
    }

    pub(super) fn display_static(&mut self, title: &str) -> Result<(), Esp32Error> {
        #[cfg(feature = "oled")]
        if let Some(display) = &mut self.display {
            display.draw_static(title)?;
        }
        Ok(())
    }

    pub(super) fn display_dynamic(
        &mut self,
        state: &RuntimeState,
        rssi: i32,
    ) -> Result<(), Esp32Error> {
        #[cfg(feature = "oled")]
        if let Some(display) = &mut self.display {
            if !state.screen.sleep {
                display.draw_dynamic(state, rssi)?;
            }
        }
        Ok(())
    }

    pub(super) fn display_notification(&mut self, message: &str) -> Result<(), Esp32Error> {
        #[cfg(feature = "oled")]
        if let Some(display) = &mut self.display {
            display.draw_notification(message)?;
        }
        Ok(())
    }
}

struct MotorControl {
    #[cfg(not(feature = "pwm-motor"))]
    pin_a: PinDriver<'static, MotorPinA, Output>,
    #[cfg(not(feature = "pwm-motor"))]
    pin_b: PinDriver<'static, MotorPinB, Output>,
    #[cfg(feature = "pwm-motor")]
    pwm: MotorPwm,
}

impl MotorControl {
    fn new(
        pin_a: MotorPinA,
        pin_b: MotorPinB,
        #[cfg(feature = "pwm-motor")] timer1: esp_idf_hal::ledc::TIMER1,
        #[cfg(feature = "pwm-motor")] channel1: esp_idf_hal::ledc::CHANNEL1,
        #[cfg(feature = "pwm-motor")] channel2: esp_idf_hal::ledc::CHANNEL2,
    ) -> Result<Self, Esp32Error> {
        #[cfg(not(feature = "pwm-motor"))]
        let pin_a = PinDriver::output(pin_a)?;
        #[cfg(not(feature = "pwm-motor"))]
        let pin_b = PinDriver::output(pin_b)?;

        #[cfg(feature = "pwm-motor")]
        let pwm = MotorPwm::new(timer1, channel1, channel2, pin_a, pin_b)?;

        #[cfg(feature = "pwm-motor")]
        {
            Ok(Self { pwm })
        }

        #[cfg(not(feature = "pwm-motor"))]
        {
            Ok(Self { pin_a, pin_b })
        }
    }

    fn start(&mut self, direction: MotorDirection) -> Result<(), Esp32Error> {
        #[cfg(feature = "pwm-motor")]
        {
            return self.pwm.start(direction);
        }

        #[cfg(not(feature = "pwm-motor"))]
        {
            match direction {
                MotorDirection::Clockwise => {
                    self.pin_a.set_high()?;
                    self.pin_b.set_low()?;
                }
                MotorDirection::CounterClockwise => {
                    self.pin_a.set_low()?;
                    self.pin_b.set_high()?;
                }
            }
            Ok(())
        }
    }

    fn stop(&mut self) -> Result<(), Esp32Error> {
        #[cfg(feature = "pwm-motor")]
        {
            return self.pwm.stop();
        }

        #[cfg(not(feature = "pwm-motor"))]
        {
            self.pin_a.set_low()?;
            self.pin_b.set_low()?;
            Ok(())
        }
    }
}

#[cfg(feature = "pwm-motor")]
struct MotorPwm {
    _timer: LedcTimerDriver<'static, esp_idf_hal::ledc::TIMER1>,
    driver_a: LedcDriver<'static>,
    driver_b: LedcDriver<'static>,
    speed: u32,
}

#[cfg(feature = "pwm-motor")]
impl MotorPwm {
    fn new(
        timer1: esp_idf_hal::ledc::TIMER1,
        channel1: esp_idf_hal::ledc::CHANNEL1,
        channel2: esp_idf_hal::ledc::CHANNEL2,
        pin_a: MotorPinA,
        pin_b: MotorPinB,
    ) -> Result<Self, Esp32Error> {
        let timer =
            LedcTimerDriver::new(timer1, &TimerConfig::default().frequency(1.kHz().into()))?;
        let driver_a = LedcDriver::new(channel1, &timer, pin_a)?;
        let driver_b = LedcDriver::new(channel2, &timer, pin_b)?;
        Ok(Self {
            _timer: timer,
            driver_a,
            driver_b,
            speed: 145,
        })
    }

    fn start(&mut self, direction: MotorDirection) -> Result<(), Esp32Error> {
        let max = self.driver_a.get_max_duty();
        let duty = (self.speed.min(255) as u32 * max) / 255;
        match direction {
            MotorDirection::Clockwise => {
                self.driver_a.set_duty(duty)?;
                self.driver_b.set_duty(0)?;
            }
            MotorDirection::CounterClockwise => {
                self.driver_a.set_duty(0)?;
                self.driver_b.set_duty(duty)?;
            }
        }
        Ok(())
    }

    fn stop(&mut self) -> Result<(), Esp32Error> {
        self.driver_a.set_duty(0)?;
        self.driver_b.set_duty(0)?;
        Ok(())
    }
}

struct LedControl {
    _timer: LedcTimerDriver<'static, esp_idf_hal::ledc::TIMER0>,
    driver: LedcDriver<'static>,
    max_duty: u32,
}

impl LedControl {
    fn new(
        timer: LedcTimerDriver<'static, esp_idf_hal::ledc::TIMER0>,
        driver: LedcDriver<'static>,
    ) -> Self {
        let max_duty = driver.get_max_duty();
        Self {
            _timer: timer,
            driver,
            max_duty,
        }
    }

    fn trigger(&mut self, pattern: LedPattern) -> Result<(), Esp32Error> {
        self.off()?;
        thread::sleep(Duration::from_millis(50));
        match pattern {
            LedPattern::Off => self.off(),
            LedPattern::SlowBlink => self.slow_blink(),
            LedPattern::FastBlink => self.fast_blink(),
            LedPattern::Pulse => self.pulse(),
        }
    }

    fn off(&mut self) -> Result<(), Esp32Error> {
        self.driver.set_duty(0)?;
        Ok(())
    }

    fn pulse(&mut self) -> Result<(), Esp32Error> {
        for duty in 0..=255 {
            let scaled = (duty as u32 * self.max_duty) / 255;
            self.driver.set_duty(scaled)?;
            thread::sleep(Duration::from_millis(7));
        }
        for duty in (0..=255).rev() {
            let scaled = (duty as u32 * self.max_duty) / 255;
            self.driver.set_duty(scaled)?;
            thread::sleep(Duration::from_millis(7));
        }
        Ok(())
    }

    fn slow_blink(&mut self) -> Result<(), Esp32Error> {
        for _ in 0..3 {
            self.pulse()?;
            thread::sleep(Duration::from_millis(150));
        }
        Ok(())
    }

    fn fast_blink(&mut self) -> Result<(), Esp32Error> {
        for _ in 0..12 {
            for duty in 0..=255 {
                let scaled = (duty as u32 * self.max_duty) / 255;
                self.driver.set_duty(scaled)?;
                thread::sleep(Duration::from_millis(2));
            }
            for duty in (0..=255).rev() {
                let scaled = (duty as u32 * self.max_duty) / 255;
                self.driver.set_duty(scaled)?;
                thread::sleep(Duration::from_millis(2));
            }
            thread::sleep(Duration::from_millis(50));
        }
        Ok(())
    }
}
