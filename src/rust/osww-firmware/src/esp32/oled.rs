//! SSD1306 OLED display implementation (ESP32 + ESP-IDF).
//!
//! This module owns the low-level drawing implementation for the optional OLED screen.
//! High-level state logic (what to render and when) lives in `controller.rs`.
//!
//! Notes:
//! - Keep the drawing layout aligned with the legacy Arduino firmware where possible.
//! - Prefer keeping any pure calculations in `crate::oled_ui` so they stay unit-testable on host.

#![cfg(feature = "oled")]

use crate::model::RuntimeState;
use crate::oled_ui::{progress_width, wifi_bars_for_rssi};
use embedded_graphics::mono_font::ascii::{FONT_10X20, FONT_6X10};
use embedded_graphics::mono_font::MonoTextStyleBuilder;
use embedded_graphics::pixelcolor::BinaryColor;
use embedded_graphics::prelude::*;
use embedded_graphics::primitives::{Line, PrimitiveStyle, Rectangle, Triangle};
use embedded_graphics::text::{Alignment, Text};
use esp_idf_hal::i2c::I2cDriver;
use ssd1306::prelude::{DisplayConfig, DisplayRotation, DisplaySize128x64, I2CInterface};
use ssd1306::{I2CDisplayInterface, Ssd1306};
use std::time::Duration;

use super::Esp32Error;

const OLED_ADDR: u8 = 0x3C;

pub(super) struct OledDisplay {
    display: Ssd1306<
        I2CInterface<I2cDriver<'static>>,
        DisplaySize128x64,
        ssd1306::mode::BufferedGraphicsMode<DisplaySize128x64>,
    >,
}

impl OledDisplay {
    pub(super) fn new(i2c: I2cDriver<'static>) -> Result<Self, Esp32Error> {
        let interface = I2CDisplayInterface::new_custom_address(i2c, OLED_ADDR);
        let mut display = Ssd1306::new(interface, DisplaySize128x64, DisplayRotation::Rotate0)
            .into_buffered_graphics_mode();
        display.init()?;
        display.set_invert(false)?;
        display.flush()?;
        Ok(Self { display })
    }

    pub(super) fn clear(&mut self) -> Result<(), Esp32Error> {
        self.display.clear(BinaryColor::Off)?;
        self.display.flush()?;
        Ok(())
    }

    pub(super) fn draw_static(&mut self, title: &str) -> Result<(), Esp32Error> {
        self.display.clear(BinaryColor::Off)?;

        let text_style = MonoTextStyleBuilder::new()
            .font(&FONT_6X10)
            .text_color(BinaryColor::On)
            .build();

        Text::with_alignment(title, Point::new(64, 3), text_style, Alignment::Center)
            .draw(&mut self.display)?;

        Line::new(Point::new(0, 14), Point::new(127, 14))
            .into_styled(PrimitiveStyle::with_stroke(BinaryColor::On, 1))
            .draw(&mut self.display)?;
        Line::new(Point::new(64, 14), Point::new(64, 50))
            .into_styled(PrimitiveStyle::with_stroke(BinaryColor::On, 1))
            .draw(&mut self.display)?;
        Line::new(Point::new(0, 50), Point::new(127, 50))
            .into_styled(PrimitiveStyle::with_stroke(BinaryColor::On, 1))
            .draw(&mut self.display)?;

        Text::new("TPD", Point::new(4, 18), text_style).draw(&mut self.display)?;
        Text::new("DIR", Point::new(71, 18), text_style).draw(&mut self.display)?;

        self.display.flush()?;
        Ok(())
    }

    pub(super) fn draw_dynamic(&mut self, state: &RuntimeState, rssi: i32) -> Result<(), Esp32Error> {
        let small_style = MonoTextStyleBuilder::new()
            .font(&FONT_6X10)
            .text_color(BinaryColor::On)
            .build();
        let large_style = MonoTextStyleBuilder::new()
            .font(&FONT_10X20)
            .text_color(BinaryColor::On)
            .build();

        Rectangle::new(Point::new(8, 25), Size::new(54, 25))
            .into_styled(PrimitiveStyle::with_fill(BinaryColor::Off))
            .draw(&mut self.display)?;
        Text::new(
            &state.rotations_per_day.to_string(),
            Point::new(8, 30),
            large_style,
        )
        .draw(&mut self.display)?;

        Rectangle::new(Point::new(66, 25), Size::new(62, 25))
            .into_styled(PrimitiveStyle::with_fill(BinaryColor::Off))
            .draw(&mut self.display)?;
        Text::new(
            state.direction.as_api_str(),
            Point::new(74, 30),
            large_style,
        )
        .draw(&mut self.display)?;

        self.draw_progress_bar(state.cycle_progress)?;
        self.draw_wifi_status(rssi, small_style)?;
        self.draw_timer_status(state, small_style)?;

        self.display.flush()?;
        Ok(())
    }

    fn draw_progress_bar(&mut self, progress: f32) -> Result<(), Esp32Error> {
        Rectangle::new(Point::new(0, 50), Size::new(128, 2))
            .into_styled(PrimitiveStyle::with_fill(BinaryColor::Off))
            .draw(&mut self.display)?;
        let width = progress_width(progress, 128);
        Rectangle::new(Point::new(0, 50), Size::new(width, 2))
            .into_styled(PrimitiveStyle::with_fill(BinaryColor::On))
            .draw(&mut self.display)?;
        Ok(())
    }

    fn draw_wifi_status(
        &mut self,
        rssi: i32,
        style: embedded_graphics::mono_font::MonoTextStyle<'_, BinaryColor>,
    ) -> Result<(), Esp32Error> {
        Triangle::new(Point::new(4, 54), Point::new(10, 54), Point::new(7, 58))
            .into_styled(PrimitiveStyle::with_stroke(BinaryColor::On, 1))
            .draw(&mut self.display)?;
        Line::new(Point::new(7, 58), Point::new(7, 62))
            .into_styled(PrimitiveStyle::with_stroke(BinaryColor::On, 1))
            .draw(&mut self.display)?;

        Rectangle::new(Point::new(12, 54), Size::new(58, 10))
            .into_styled(PrimitiveStyle::with_fill(BinaryColor::Off))
            .draw(&mut self.display)?;

        let bars = wifi_bars_for_rssi(rssi);
        for i in 0..bars {
            let height = 2 + (i as i32) * 2;
            Rectangle::new(
                Point::new(14 + (i as i32) * 4, 55 + (8 - height)),
                Size::new(2, height as u32),
            )
            .into_styled(PrimitiveStyle::with_fill(BinaryColor::On))
            .draw(&mut self.display)?;
        }

        let _ = style;
        Ok(())
    }

    fn draw_timer_status(
        &mut self,
        state: &RuntimeState,
        style: embedded_graphics::mono_font::MonoTextStyle<'_, BinaryColor>,
    ) -> Result<(), Esp32Error> {
        Rectangle::new(Point::new(60, 54), Size::new(68, 13))
            .into_styled(PrimitiveStyle::with_fill(BinaryColor::Off))
            .draw(&mut self.display)?;

        if state.timer.enabled {
            let text = format!(
                "TIMER {:02}:{:02}",
                state.timer.start_time.hour, state.timer.start_time.minute
            );
            Text::new(&text, Point::new(60, 56), style).draw(&mut self.display)?;
        }
        Ok(())
    }

    pub(super) fn draw_notification(&mut self, message: &str) -> Result<(), Esp32Error> {
        Rectangle::new(Point::new(0, 0), Size::new(128, 14))
            .into_styled(PrimitiveStyle::with_fill(BinaryColor::On))
            .draw(&mut self.display)?;

        let text_style = MonoTextStyleBuilder::new()
            .font(&FONT_6X10)
            .text_color(BinaryColor::Off)
            .build();

        Text::with_alignment(message, Point::new(64, 3), text_style, Alignment::Center)
            .draw(&mut self.display)?;

        self.display.flush()?;
        std::thread::sleep(Duration::from_millis(200));

        Rectangle::new(Point::new(0, 0), Size::new(128, 14))
            .into_styled(PrimitiveStyle::with_fill(BinaryColor::Off))
            .draw(&mut self.display)?;
        let text_style = MonoTextStyleBuilder::new()
            .font(&FONT_6X10)
            .text_color(BinaryColor::On)
            .build();
        Text::with_alignment(message, Point::new(64, 3), text_style, Alignment::Center)
            .draw(&mut self.display)?;
        Line::new(Point::new(0, 14), Point::new(127, 14))
            .into_styled(PrimitiveStyle::with_stroke(BinaryColor::On, 1))
            .draw(&mut self.display)?;
        self.display.flush()?;
        Ok(())
    }
}

