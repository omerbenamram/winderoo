//! Optional Home Assistant integration (MQTT autodiscovery) using `embassy-ha`.
//!
//! This mirrors the Arduino firmware's `HOME_ASSISTANT_ENABLED` path, but uses a smaller
//! set of entities (bounded by `embassy-ha`'s fixed entity limit).

#![cfg(feature = "home-assistant")]

use embassy_time::Duration;
use winderoo_embassy::state::StatusCache;
use winderoo_embassy::tasks::RuntimeCommand;

use winderoo_firmware::model::{Direction, StatusSnapshot, UpdateAction, UpdateRequest, WinderStatus};
use winderoo_firmware::time::TimeOfDay;

pub type RuntimeSender = embassy_sync::channel::Sender<
    'static,
    embassy_sync::blocking_mutex::raw::NoopRawMutex,
    RuntimeCommand,
    { crate::RUNTIME_QUEUE_DEPTH },
>;

fn base_update(snapshot: &StatusSnapshot) -> UpdateRequest {
    let action = match snapshot.status {
        WinderStatus::Winding => UpdateAction::Start,
        _ => UpdateAction::Stop,
    };

    UpdateRequest {
        direction: snapshot.direction,
        rotations_per_day: snapshot.rotations_per_day,
        action,
        hour: snapshot.timer_hour,
        minutes: snapshot.timer_minutes,
        timer_enabled: snapshot.timer_enabled,
        screen_sleep: snapshot.screen_sleep,
        screen_schedule_enabled: None,
        screen_schedule_start: None,
        screen_schedule_end: None,
        custom_wind_duration_secs: snapshot.custom_wind_duration_secs,
        custom_wind_pause_secs: snapshot.custom_wind_pause_secs,
        rotation_duration_secs: snapshot.rotation_duration_secs,
        rtc_gmt_offset: snapshot.gmt_offset,
        rtc_dst: snapshot.dst,
    }
}

fn dir_to_number(dir: Direction) -> f32 {
    match dir {
        Direction::CounterClockwise => 0.0,
        Direction::Both => 1.0,
        Direction::Clockwise => 2.0,
    }
}

fn number_to_dir(value: f32) -> Direction {
    match value.round() as i32 {
        0 => Direction::CounterClockwise,
        2 => Direction::Clockwise,
        _ => Direction::Both,
    }
}

fn clamp_u16(value: f32, min: u16, max: u16) -> u16 {
    let v = value.round() as i32;
    let min = min as i32;
    let max = max as i32;
    v.clamp(min, max) as u16
}

fn minutes_to_time(total_minutes: u16) -> TimeOfDay {
    let hour = (total_minutes / 60) as u8;
    let minute = (total_minutes % 60) as u8;
    TimeOfDay::new(hour, minute).unwrap_or(TimeOfDay { hour: 0, minute: 0 })
}

#[embassy_executor::task]
pub async fn ha_run_task(
    stack: embassy_net::Stack<'static>,
    mut device: embassy_ha::Device<'static>,
    broker: &'static str,
) -> ! {
    // Avoid DNS queries before DHCP has an address.
    stack.wait_config_up().await;
    embassy_ha::connect_and_run(stack, device, broker).await
}

#[embassy_executor::task]
pub async fn ha_power_task(
    status_cache: &'static StatusCache,
    runtime: RuntimeSender,
    mut power: embassy_ha::Switch<'static>,
) -> ! {
    use embassy_ha::BinaryState;

    loop {
        let snapshot = status_cache.snapshot();
        power.set(if snapshot.winder_enabled {
            BinaryState::On
        } else {
            BinaryState::Off
        });

        if let Ok(cmd) = embassy_time::with_timeout(Duration::from_secs(1), power.wait()).await {
            let enabled = matches!(cmd, BinaryState::On);
            let _ = runtime.send(RuntimeCommand::ApplyPower(enabled)).await;
        }
    }
}

#[embassy_executor::task]
pub async fn ha_timer_enabled_task(
    status_cache: &'static StatusCache,
    runtime: RuntimeSender,
    mut timer: embassy_ha::Switch<'static>,
) -> ! {
    use embassy_ha::BinaryState;

    loop {
        let snapshot = status_cache.snapshot();
        timer.set(if snapshot.timer_enabled {
            BinaryState::On
        } else {
            BinaryState::Off
        });

        if let Ok(cmd) = embassy_time::with_timeout(Duration::from_secs(1), timer.wait()).await {
            let enabled = matches!(cmd, BinaryState::On);
            let _ = runtime.send(RuntimeCommand::ApplyTimer(enabled)).await;
        }
    }
}

#[embassy_executor::task]
pub async fn ha_start_button_task(
    status_cache: &'static StatusCache,
    runtime: RuntimeSender,
    mut start: embassy_ha::Button<'static>,
) -> ! {
    loop {
        start.pressed().await;
        let snapshot = status_cache.snapshot();
        let mut update = base_update(&snapshot);
        update.action = UpdateAction::Start;
        let _ = runtime.send(RuntimeCommand::ApplyUpdate(update)).await;
    }
}

#[embassy_executor::task]
pub async fn ha_stop_button_task(
    status_cache: &'static StatusCache,
    runtime: RuntimeSender,
    mut stop: embassy_ha::Button<'static>,
) -> ! {
    loop {
        stop.pressed().await;
        let snapshot = status_cache.snapshot();
        let mut update = base_update(&snapshot);
        update.action = UpdateAction::Stop;
        let _ = runtime.send(RuntimeCommand::ApplyUpdate(update)).await;
    }
}

#[embassy_executor::task]
pub async fn ha_oled_task(
    status_cache: &'static StatusCache,
    runtime: RuntimeSender,
    mut oled: embassy_ha::Switch<'static>,
) -> ! {
    use embassy_ha::BinaryState;

    loop {
        let snapshot = status_cache.snapshot();
        oled.set(if snapshot.screen_sleep {
            BinaryState::Off
        } else {
            BinaryState::On
        });

        if let Ok(cmd) = embassy_time::with_timeout(Duration::from_secs(1), oled.wait()).await {
            let on = matches!(cmd, BinaryState::On);
            let snapshot = status_cache.snapshot();
            let mut update = base_update(&snapshot);
            update.screen_sleep = !on;
            let _ = runtime.send(RuntimeCommand::ApplyUpdate(update)).await;
        }
    }
}

#[embassy_executor::task]
pub async fn ha_rpd_task(
    status_cache: &'static StatusCache,
    runtime: RuntimeSender,
    mut rpd: embassy_ha::Number<'static>,
) -> ! {
    loop {
        let snapshot = status_cache.snapshot();
        rpd.publish(snapshot.rotations_per_day as f32);

        if let Ok(cmd) = embassy_time::with_timeout(Duration::from_secs(1), rpd.wait()).await {
            // Round to nearest 10 and clamp to 100..960 (Arduino defaults).
            let mut v = clamp_u16(cmd, 100, 960);
            v = (v / 10) * 10;

            let snapshot = status_cache.snapshot();
            let mut update = base_update(&snapshot);
            update.rotations_per_day = v;
            let _ = runtime.send(RuntimeCommand::ApplyUpdate(update)).await;
        }
    }
}

#[embassy_executor::task]
pub async fn ha_direction_task(
    status_cache: &'static StatusCache,
    runtime: RuntimeSender,
    mut direction: embassy_ha::Number<'static>,
) -> ! {
    loop {
        let snapshot = status_cache.snapshot();
        direction.publish(dir_to_number(snapshot.direction));

        if let Ok(cmd) = embassy_time::with_timeout(Duration::from_secs(1), direction.wait()).await {
            let snapshot = status_cache.snapshot();
            let mut update = base_update(&snapshot);
            update.direction = number_to_dir(cmd);
            let _ = runtime.send(RuntimeCommand::ApplyUpdate(update)).await;
        }
    }
}

#[embassy_executor::task]
pub async fn ha_timer_start_minutes_task(
    status_cache: &'static StatusCache,
    runtime: RuntimeSender,
    mut timer_start: embassy_ha::Number<'static>,
) -> ! {
    loop {
        let snapshot = status_cache.snapshot();
        let current = (snapshot.timer_hour as u16) * 60 + (snapshot.timer_minutes as u16);
        timer_start.publish(current as f32);

        if let Ok(cmd) = embassy_time::with_timeout(Duration::from_secs(1), timer_start.wait()).await {
            let minutes = clamp_u16(cmd, 0, 1439);
            let time = minutes_to_time(minutes);
            let snapshot = status_cache.snapshot();
            let mut update = base_update(&snapshot);
            update.hour = time.hour;
            update.minutes = time.minute;
            let _ = runtime.send(RuntimeCommand::ApplyUpdate(update)).await;
        }
    }
}

#[embassy_executor::task]
pub async fn ha_custom_wind_duration_task(
    status_cache: &'static StatusCache,
    runtime: RuntimeSender,
    mut seconds: embassy_ha::Number<'static>,
) -> ! {
    loop {
        let snapshot = status_cache.snapshot();
        seconds.publish(snapshot.custom_wind_duration_secs as f32);

        if let Ok(cmd) = embassy_time::with_timeout(Duration::from_secs(1), seconds.wait()).await {
            let value = cmd.max(0.0) as u32;
            let snapshot = status_cache.snapshot();
            let mut update = base_update(&snapshot);
            update.custom_wind_duration_secs = value;
            let _ = runtime.send(RuntimeCommand::ApplyUpdate(update)).await;
        }
    }
}

#[embassy_executor::task]
pub async fn ha_custom_wind_pause_task(
    status_cache: &'static StatusCache,
    runtime: RuntimeSender,
    mut seconds: embassy_ha::Number<'static>,
) -> ! {
    loop {
        let snapshot = status_cache.snapshot();
        seconds.publish(snapshot.custom_wind_pause_secs as f32);

        if let Ok(cmd) = embassy_time::with_timeout(Duration::from_secs(1), seconds.wait()).await {
            let value = cmd.max(0.0) as u32;
            let snapshot = status_cache.snapshot();
            let mut update = base_update(&snapshot);
            update.custom_wind_pause_secs = value;
            let _ = runtime.send(RuntimeCommand::ApplyUpdate(update)).await;
        }
    }
}

#[embassy_executor::task]
pub async fn ha_rotation_duration_task(
    status_cache: &'static StatusCache,
    runtime: RuntimeSender,
    mut seconds: embassy_ha::Number<'static>,
) -> ! {
    loop {
        let snapshot = status_cache.snapshot();
        seconds.publish(snapshot.rotation_duration_secs as f32);

        if let Ok(cmd) = embassy_time::with_timeout(Duration::from_secs(1), seconds.wait()).await {
            let value = clamp_u16(cmd, 1, 60);
            let snapshot = status_cache.snapshot();
            let mut update = base_update(&snapshot);
            update.rotation_duration_secs = value;
            let _ = runtime.send(RuntimeCommand::ApplyUpdate(update)).await;
        }
    }
}

#[embassy_executor::task]
pub async fn ha_rtc_offset_task(
    status_cache: &'static StatusCache,
    runtime: RuntimeSender,
    mut offset: embassy_ha::Number<'static>,
) -> ! {
    loop {
        let snapshot = status_cache.snapshot();
        offset.publish(snapshot.gmt_offset);

        if let Ok(cmd) = embassy_time::with_timeout(Duration::from_secs(1), offset.wait()).await {
            let snapshot = status_cache.snapshot();
            let mut update = base_update(&snapshot);
            update.rtc_gmt_offset = cmd.clamp(-12.0, 14.0);
            let _ = runtime.send(RuntimeCommand::ApplyUpdate(update)).await;
        }
    }
}

#[embassy_executor::task]
pub async fn ha_rtc_dst_task(
    status_cache: &'static StatusCache,
    runtime: RuntimeSender,
    mut dst: embassy_ha::Switch<'static>,
) -> ! {
    use embassy_ha::BinaryState;

    loop {
        let snapshot = status_cache.snapshot();
        dst.set(if snapshot.dst { BinaryState::On } else { BinaryState::Off });

        if let Ok(cmd) = embassy_time::with_timeout(Duration::from_secs(1), dst.wait()).await {
            let enabled = matches!(cmd, BinaryState::On);
            let snapshot = status_cache.snapshot();
            let mut update = base_update(&snapshot);
            update.rtc_dst = enabled;
            let _ = runtime.send(RuntimeCommand::ApplyUpdate(update)).await;
        }
    }
}

#[embassy_executor::task]
pub async fn ha_screen_schedule_enabled_task(
    status_cache: &'static StatusCache,
    runtime: RuntimeSender,
    mut enabled: embassy_ha::Switch<'static>,
) -> ! {
    use embassy_ha::BinaryState;

    loop {
        let snapshot = status_cache.snapshot();
        enabled.set(if snapshot.screen_schedule_enabled {
            BinaryState::On
        } else {
            BinaryState::Off
        });

        if let Ok(cmd) = embassy_time::with_timeout(Duration::from_secs(1), enabled.wait()).await {
            let on = matches!(cmd, BinaryState::On);
            let snapshot = status_cache.snapshot();
            let mut update = base_update(&snapshot);
            update.screen_schedule_enabled = Some(on);
            let _ = runtime.send(RuntimeCommand::ApplyUpdate(update)).await;
        }
    }
}

#[embassy_executor::task]
pub async fn ha_screen_schedule_start_minutes_task(
    status_cache: &'static StatusCache,
    runtime: RuntimeSender,
    mut value: embassy_ha::Number<'static>,
) -> ! {
    loop {
        let snapshot = status_cache.snapshot();
        value.publish(snapshot.screen_schedule_start.total_minutes() as f32);

        if let Ok(cmd) = embassy_time::with_timeout(Duration::from_secs(1), value.wait()).await {
            let minutes = clamp_u16(cmd, 0, 1439);
            let time = minutes_to_time(minutes);
            let snapshot = status_cache.snapshot();
            let mut update = base_update(&snapshot);
            update.screen_schedule_start = Some(time);
            let _ = runtime.send(RuntimeCommand::ApplyUpdate(update)).await;
        }
    }
}

#[embassy_executor::task]
pub async fn ha_screen_schedule_end_minutes_task(
    status_cache: &'static StatusCache,
    runtime: RuntimeSender,
    mut value: embassy_ha::Number<'static>,
) -> ! {
    loop {
        let snapshot = status_cache.snapshot();
        value.publish(snapshot.screen_schedule_end.total_minutes() as f32);

        if let Ok(cmd) = embassy_time::with_timeout(Duration::from_secs(1), value.wait()).await {
            let minutes = clamp_u16(cmd, 0, 1439);
            let time = minutes_to_time(minutes);
            let snapshot = status_cache.snapshot();
            let mut update = base_update(&snapshot);
            update.screen_schedule_end = Some(time);
            let _ = runtime.send(RuntimeCommand::ApplyUpdate(update)).await;
        }
    }
}

