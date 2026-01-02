//! Simulated OLED display using embedded-graphics.
//!
//! This module uses the SAME rendering code as the real `Ssd1306Display` in
//! `osww-embassy/src/hardware.rs`, ensuring the simulator shows exactly what
//! the real hardware would display.

use embedded_graphics::mono_font::{ascii::FONT_6X10, MonoTextStyle};
use embedded_graphics::pixelcolor::BinaryColor;
use embedded_graphics::prelude::*;
use embedded_graphics::primitives::{PrimitiveStyle, Rectangle};
use embedded_graphics::text::{Baseline, Text};

/// Display dimensions (SSD1306 128x64)
pub const DISPLAY_WIDTH: usize = 128;
pub const DISPLAY_HEIGHT: usize = 64;

/// Status snapshot for display rendering.
///
/// This mirrors the data available to `Ssd1306Display` via `StatusCache`.
#[derive(Debug, Clone, Default)]
pub struct DisplaySnapshot {
    pub status: heapless::String<16>,
    pub rotations_per_day: u16,
    pub direction: heapless::String<8>,
    pub timer_hour: u8,
    pub timer_minutes: u8,
    pub timer_enabled: bool,
}

/// Simulated display buffer that implements `DrawTarget`.
///
/// This allows us to use the exact same embedded-graphics drawing code
/// as the real firmware, just rendering to a memory buffer instead of
/// actual I2C hardware.
pub struct SimDisplay {
    /// Frame buffer: true = pixel on, false = pixel off
    buffer: [[bool; DISPLAY_WIDTH]; DISPLAY_HEIGHT],
    /// Current title (set by draw_static)
    title: heapless::String<24>,
    /// Snapshot of runtime state for dynamic rendering
    snapshot: DisplaySnapshot,
}

impl SimDisplay {
    /// Create a new display buffer.
    pub fn new() -> Self {
        Self {
            buffer: [[false; DISPLAY_WIDTH]; DISPLAY_HEIGHT],
            title: heapless::String::new(),
            snapshot: DisplaySnapshot::default(),
        }
    }

    /// Clear the display buffer.
    pub fn clear(&mut self) {
        for row in &mut self.buffer {
            for pixel in row {
                *pixel = false;
            }
        }
    }

    /// Update the status snapshot (called from tick).
    pub fn update_snapshot(&mut self, snapshot: DisplaySnapshot) {
        self.snapshot = snapshot;
    }

    /// Draw static UI with title.
    ///
    /// This is called when `ControllerEvent::DisplayStatic` is received.
    pub fn draw_static(&mut self, title: &str) {
        self.title.clear();
        let _ = self.title.push_str(title);
        self.redraw(None);
    }

    /// Draw dynamic UI values.
    ///
    /// This is called when `ControllerEvent::DisplayDynamic` is received.
    pub fn draw_dynamic(&mut self) {
        self.redraw(None);
    }

    /// Draw a notification overlay.
    ///
    /// This is called when `ControllerEvent::DisplayNotification` is received.
    pub fn notify(&mut self, message: &str) {
        self.redraw(Some(message));
    }

    /// Internal redraw - THIS IS THE SAME CODE AS `Ssd1306Display::redraw()`
    /// from osww-embassy/src/hardware.rs
    fn redraw(&mut self, notification: Option<&str>) {
        let style = MonoTextStyle::new(&FONT_6X10, BinaryColor::On);

        // Clone data we need to avoid borrow conflicts
        let title = self.title.clone();
        let snapshot = self.snapshot.clone();

        self.clear();

        // Simple frame + title.
        let _ = Rectangle::new(Point::new(0, 0), Size::new(128, 64))
            .into_styled(PrimitiveStyle::with_stroke(BinaryColor::On, 1))
            .draw(self);

        let _ = Text::with_baseline(&title, Point::new(4, 2), style, Baseline::Top)
            .draw(self);

        if let Some(message) = notification {
            // Center-ish notification (wrapped).
            let mut y = 22;
            for chunk in message.as_bytes().chunks(18).take(3) {
                if let Ok(line) = core::str::from_utf8(chunk) {
                    let _ = Text::with_baseline(line, Point::new(4, y), style, Baseline::Top)
                        .draw(self);
                }
                y += 12;
            }
        } else {
            // Dynamic snapshot values.
            let mut line1 = heapless::String::<32>::new();
            let _ = line1.push_str(&snapshot.status);
            let _ = Text::with_baseline(&line1, Point::new(4, 18), style, Baseline::Top)
                .draw(self);

            let mut line2 = heapless::String::<32>::new();
            let _ = core::fmt::write(
                &mut line2,
                format_args!("TPD {} {}", snapshot.rotations_per_day, snapshot.direction),
            );
            let _ = Text::with_baseline(&line2, Point::new(4, 30), style, Baseline::Top)
                .draw(self);

            let mut line3 = heapless::String::<32>::new();
            let _ = core::fmt::write(
                &mut line3,
                format_args!(
                    "Timer {:02}:{:02} {}",
                    snapshot.timer_hour,
                    snapshot.timer_minutes,
                    if snapshot.timer_enabled { "ON" } else { "OFF" }
                ),
            );
            let _ = Text::with_baseline(&line3, Point::new(4, 42), style, Baseline::Top)
                .draw(self);
        }
    }

    /// Get the frame buffer as a flat array for JavaScript.
    ///
    /// Returns 128*64 = 8192 bytes, where each byte is 0 (off) or 1 (on).
    pub fn get_buffer(&self) -> Vec<u8> {
        let mut out = Vec::with_capacity(DISPLAY_WIDTH * DISPLAY_HEIGHT);
        for row in &self.buffer {
            for &pixel in row {
                out.push(if pixel { 1 } else { 0 });
            }
        }
        out
    }
}

impl Default for SimDisplay {
    fn default() -> Self {
        Self::new()
    }
}

/// Implement `DrawTarget` so embedded-graphics can draw to our buffer.
impl DrawTarget for SimDisplay {
    type Color = BinaryColor;
    type Error = core::convert::Infallible;

    fn draw_iter<I>(&mut self, pixels: I) -> Result<(), Self::Error>
    where
        I: IntoIterator<Item = Pixel<Self::Color>>,
    {
        for Pixel(coord, color) in pixels {
            let x = coord.x as usize;
            let y = coord.y as usize;
            if x < DISPLAY_WIDTH && y < DISPLAY_HEIGHT {
                self.buffer[y][x] = color == BinaryColor::On;
            }
        }
        Ok(())
    }
}

impl OriginDimensions for SimDisplay {
    fn size(&self) -> Size {
        Size::new(DISPLAY_WIDTH as u32, DISPLAY_HEIGHT as u32)
    }
}
