//! Async firmware task loop for embassy-based runtimes.
//!
//! The `FirmwareTask` binds a `Controller` to a `TimeSource` and dispatches
//! emitted `ControllerEvent`s using an `EventDispatcher`. The loop itself is
//! fully deterministic aside from the `RandomSource` supplied to the controller.

use embassy_time::{Duration, Ticker};

use winderoo_firmware::controller::Controller;
use winderoo_firmware::hardware::RandomSource;
use winderoo_firmware::time::TimeOfDay;

use crate::hardware::{DisplayControl, EventDispatcher, LedControl, MotorControl, SystemHooks};

/// Provides time information for the firmware loop.
pub trait TimeSource {
    /// Return the current epoch in seconds.
    fn now_epoch(&mut self) -> u64;
    /// Return the current time of day.
    fn now_time_of_day(&mut self) -> TimeOfDay;
}

/// Async firmware loop wrapper for embassy-based applications.
#[derive(Debug)]
pub struct FirmwareTask<R, M, L, D, S, T, C>
where
    R: RandomSource,
    M: MotorControl,
    L: LedControl,
    D: DisplayControl,
    S: SystemHooks,
    T: embedded_hal::delay::DelayNs,
    C: TimeSource,
{
    controller: Controller<R>,
    dispatcher: EventDispatcher<M, L, D, S, T>,
    time_source: C,
    tick_interval: Duration,
}

impl<R, M, L, D, S, T, C> FirmwareTask<R, M, L, D, S, T, C>
where
    R: RandomSource,
    M: MotorControl,
    L: LedControl,
    D: DisplayControl,
    S: SystemHooks,
    T: embedded_hal::delay::DelayNs,
    C: TimeSource,
{
    /// Create a new firmware task wrapper.
    pub fn new(
        controller: Controller<R>,
        dispatcher: EventDispatcher<M, L, D, S, T>,
        time_source: C,
        tick_interval: Duration,
    ) -> Self {
        Self {
            controller,
            dispatcher,
            time_source,
            tick_interval,
        }
    }

    /// Run the firmware loop forever at the configured tick interval.
    pub async fn run(mut self) -> ! {
        let mut ticker = Ticker::every(self.tick_interval);
        loop {
            let now_epoch = self.time_source.now_epoch();
            let now_time = self.time_source.now_time_of_day();
            let events = self.controller.tick(now_epoch, now_time);
            for event in events {
                self.dispatcher.handle_event(event);
            }
            ticker.next().await;
        }
    }

    /// Access the underlying controller mutably.
    pub fn controller_mut(&mut self) -> &mut Controller<R> {
        &mut self.controller
    }

    /// Access the underlying dispatcher mutably.
    pub fn dispatcher_mut(&mut self) -> &mut EventDispatcher<M, L, D, S, T> {
        &mut self.dispatcher
    }

    /// Access the time source mutably.
    pub fn time_source_mut(&mut self) -> &mut C {
        &mut self.time_source
    }
}
