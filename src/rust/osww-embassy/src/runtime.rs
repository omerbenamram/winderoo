//! Runtime glue for embedding the controller into an embassy application.
//!
//! This module is deliberately small: it takes a controller and an event
//! dispatcher and provides a single `tick` entrypoint that can be called from
//! an async task or a main loop. All IO remains in the dispatcher, while the
//! controller owns the firmware state machine.

use winderoo_firmware::controller::Controller;
use winderoo_firmware::hardware::RandomSource;
use winderoo_firmware::time::TimeOfDay;

use crate::hardware::EventDispatcher;

/// A minimal runtime wrapper that bridges controller events to hardware IO.
#[derive(Debug)]
pub struct Runtime<R, M, L, D, S, T>
where
    R: RandomSource,
    M: crate::hardware::MotorControl,
    L: crate::hardware::LedControl,
    D: crate::hardware::DisplayControl,
    S: crate::hardware::SystemHooks,
    T: embedded_hal::delay::DelayNs,
{
    controller: Controller<R>,
    dispatcher: EventDispatcher<M, L, D, S, T>,
}

impl<R, M, L, D, S, T> Runtime<R, M, L, D, S, T>
where
    R: RandomSource,
    M: crate::hardware::MotorControl,
    L: crate::hardware::LedControl,
    D: crate::hardware::DisplayControl,
    S: crate::hardware::SystemHooks,
    T: embedded_hal::delay::DelayNs,
{
    /// Create a new runtime wrapper from a controller and dispatcher.
    pub fn new(controller: Controller<R>, dispatcher: EventDispatcher<M, L, D, S, T>) -> Self {
        Self {
            controller,
            dispatcher,
        }
    }

    /// Run one controller tick and immediately dispatch all IO events.
    pub fn tick(&mut self, now_epoch: u64, now_time: TimeOfDay) {
        let events = self.controller.tick(now_epoch, now_time);
        for event in events {
            self.dispatcher.handle_event(event);
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
}
