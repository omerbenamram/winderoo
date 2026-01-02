//! Async firmware task loop for embassy-based runtimes.
//!
//! The `FirmwareTask` binds a `Controller` to a `TimeSource` and dispatches
//! emitted `ControllerEvent`s using an `EventDispatcher`. The loop itself is
//! fully deterministic aside from the `RandomSource` supplied to the controller.

use alloc::vec;

#[cfg(feature = "embedded")]
use embassy_time::{Duration, Ticker, Timer};

use winderoo_firmware::controller::ControllerEvent;
use winderoo_firmware::hardware::LedPattern;
#[cfg(feature = "embedded")]
use winderoo_firmware::controller::Controller;
#[cfg(feature = "embedded")]
use winderoo_firmware::hardware::RandomSource;
use winderoo_firmware::model::UpdateRequest;
use winderoo_firmware::time::TimeOfDay;

#[cfg(feature = "embedded")]
use crate::hardware::{DisplayControl, EventDispatcher, LedControl, MotorControl, SystemHooks};
#[cfg(feature = "embedded")]
use crate::state::{StatusCache, WifiStatus};

/// Provides time information for the firmware loop.
pub trait TimeSource {
    /// Return the current epoch in seconds.
    fn now_epoch(&mut self) -> u64;
    /// Return the current time of day.
    fn now_time_of_day(&mut self) -> TimeOfDay;
}

/// Async firmware loop wrapper for embassy-based applications.
#[cfg(feature = "embedded")]
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

#[cfg(feature = "embedded")]
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
                match event {
                    winderoo_firmware::controller::ControllerEvent::PauseSeconds(seconds) => {
                        Timer::after(Duration::from_secs(seconds as u64)).await;
                    }
                    other => self.dispatcher.handle_event(other),
                }
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

/// Commands sent to the runtime controller task.
#[derive(Debug, Clone, PartialEq)]
pub enum RuntimeCommand {
    /// Apply a power toggle.
    ApplyPower(bool),
    /// Apply a timer enabled flag.
    ApplyTimer(bool),
    /// Apply a full update payload.
    ApplyUpdate(UpdateRequest),
    /// Wi‑Fi provisioning succeeded (new credentials connected).
    ///
    /// Mirrors the Arduino WiFiManager flow: show success indication and reboot.
    ProvisioningSuccess,
    /// Trigger a device reset.
    Reset,
}

/// Events emitted when Wi‑Fi provisioning succeeds.
///
/// Kept as a helper so we can unit test the exact UX bundle on the host.
pub fn provisioning_success_events() -> alloc::vec::Vec<ControllerEvent> {
    vec![
        ControllerEvent::DisplayNotification(alloc::string::String::from("Connected to WiFi")),
        ControllerEvent::Led(LedPattern::SlowBlink),
        ControllerEvent::RestartDevice,
    ]
}

/// Channel type for runtime commands.
pub type RuntimeCommandChannel<const N: usize> = embassy_sync::channel::Channel<
    embassy_sync::blocking_mutex::raw::CriticalSectionRawMutex,
    RuntimeCommand,
    N,
>;

/// Controller task that owns the firmware state and dispatches events.
#[cfg(feature = "embedded")]
#[derive(Debug)]
pub struct ControllerTask<'a, R, M, L, D, S, T, C, const N: usize>
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
    status_cache: &'a StatusCache,
    wifi_status: &'a WifiStatus,
    last_wifi_connected: bool,
    commands: embassy_sync::channel::Receiver<
        'a,
        embassy_sync::blocking_mutex::raw::CriticalSectionRawMutex,
        RuntimeCommand,
        N,
    >,
    api_version: &'a str,
    tick_interval: Duration,
}

#[cfg(feature = "embedded")]
impl<'a, R, M, L, D, S, T, C, const N: usize> ControllerTask<'a, R, M, L, D, S, T, C, N>
where
    R: RandomSource,
    M: MotorControl,
    L: LedControl,
    D: DisplayControl,
    S: SystemHooks,
    T: embedded_hal::delay::DelayNs,
    C: TimeSource,
{
    /// Create a new controller task.
    pub fn new(
        controller: Controller<R>,
        dispatcher: EventDispatcher<M, L, D, S, T>,
        time_source: C,
        status_cache: &'a StatusCache,
        wifi_status: &'a WifiStatus,
        commands: embassy_sync::channel::Receiver<
            'a,
            embassy_sync::blocking_mutex::raw::CriticalSectionRawMutex,
            RuntimeCommand,
            N,
        >,
        api_version: &'a str,
        tick_interval: Duration,
    ) -> Self {
        Self {
            controller,
            dispatcher,
            time_source,
            status_cache,
            wifi_status,
            // Force one initial refresh on first tick.
            last_wifi_connected: !wifi_status.is_connected(),
            commands,
            api_version,
            tick_interval,
        }
    }

    fn update_status_cache(&self, now_epoch: u64) {
        let snapshot = self.controller.status_snapshot(
            now_epoch,
            self.wifi_status.rssi_db(),
            self.api_version,
        );
        self.status_cache.update(snapshot);
    }

    fn refresh_wifi_led(&mut self) {
        // Match Arduino behavior: LED is a coarse Wi‑Fi provisioning indicator.
        // Keep it simple + non-blocking: steady on when not connected, off when connected.
        let connected = self.wifi_status.is_connected();
        if connected == self.last_wifi_connected {
            return;
        }
        self.last_wifi_connected = connected;

        let pattern = if connected {
            LedPattern::Off
        } else {
            LedPattern::On
        };
        self.dispatcher
            .handle_event(winderoo_firmware::controller::ControllerEvent::Led(pattern));
    }

    async fn dispatch_events(
        &mut self,
        mut events: alloc::vec::Vec<winderoo_firmware::controller::ControllerEvent>,
    ) {
        use embassy_futures::select::{select, Either};

        loop {
            let mut interrupted: Option<
                alloc::vec::Vec<winderoo_firmware::controller::ControllerEvent>,
            > = None;

            for event in events.into_iter() {
                match event {
                    winderoo_firmware::controller::ControllerEvent::PauseSeconds(seconds) => {
                        // During long pauses, allow runtime commands (e.g. power off / reset) to
                        // interrupt the pause so we don't blindly resume winding after the delay.
                        match select(
                            Timer::after(Duration::from_secs(seconds as u64)),
                            self.commands.receive(),
                        )
                        .await
                        {
                            Either::First(_) => {}
                            Either::Second(command) => {
                                let now_epoch = self.time_source.now_epoch();
                                let events = match command {
                                    RuntimeCommand::ApplyPower(enabled) => {
                                        self.controller.apply_power(enabled)
                                    }
                                    RuntimeCommand::ApplyTimer(enabled) => {
                                        self.controller.apply_timer_enabled(enabled)
                                    }
                                    RuntimeCommand::ApplyUpdate(update) => {
                                        self.controller.apply_update(update, now_epoch)
                                    }
                                    RuntimeCommand::ProvisioningSuccess => provisioning_success_events(),
                                    RuntimeCommand::Reset => self.controller.request_reset(),
                                };
                                interrupted = Some(events);
                                // Abort the remaining (now stale) event sequence.
                                break;
                            }
                        }
                    }
                    other => self.dispatcher.handle_event(other),
                }
            }

            match interrupted {
                Some(new_events) => {
                    events = new_events;
                    continue;
                }
                None => break,
            }
        }
    }

    /// Run the controller task forever.
    pub async fn run(mut self) -> ! {
        use embassy_futures::select::{select, Either};

        let mut ticker = Ticker::every(self.tick_interval);
        loop {
            match select(ticker.next(), self.commands.receive()).await {
                Either::First(_) => {
                    let now_epoch = self.time_source.now_epoch();
                    let now_time = self.time_source.now_time_of_day();
                    let events = self.controller.tick(now_epoch, now_time);
                    self.dispatch_events(events).await;
                    let now_epoch = self.time_source.now_epoch();
                    self.update_status_cache(now_epoch);
                    self.refresh_wifi_led();
                }
                Either::Second(command) => {
                    let now_epoch = self.time_source.now_epoch();
                    match command {
                        RuntimeCommand::ApplyPower(enabled) => {
                            let events = self.controller.apply_power(enabled);
                            self.dispatch_events(events).await;
                        }
                        RuntimeCommand::ApplyTimer(enabled) => {
                            let events = self.controller.apply_timer_enabled(enabled);
                            self.dispatch_events(events).await;
                        }
                        RuntimeCommand::ApplyUpdate(update) => {
                            let events = self.controller.apply_update(update, now_epoch);
                            self.dispatch_events(events).await;
                        }
                        RuntimeCommand::ProvisioningSuccess => {
                            self.dispatch_events(provisioning_success_events()).await;
                        }
                        RuntimeCommand::Reset => {
                            let events = self.controller.request_reset();
                            self.dispatch_events(events).await;
                        }
                    }
                    let now_epoch = self.time_source.now_epoch();
                    self.update_status_cache(now_epoch);
                    self.refresh_wifi_led();
                }
            }
        }
    }
}

#[cfg(test)]
mod provisioning_tests {
    use super::*;

    #[test]
    fn provisioning_success_bundle_matches_expected_shape() {
        let events = provisioning_success_events();
        let mut expected = alloc::vec::Vec::new();
        expected.push(ControllerEvent::DisplayNotification(alloc::string::String::from(
            "Connected to WiFi",
        )));
        expected.push(ControllerEvent::Led(LedPattern::SlowBlink));
        expected.push(ControllerEvent::RestartDevice);
        assert_eq!(events, expected);
    }
}
