//! Optional Home Assistant integration (MQTT autodiscovery) using `embassy-ha`.
//!
//! This aims to mirror the Arduino firmware's `HOME_ASSISTANT_ENABLED` entity surface for parity.

#![cfg(feature = "home-assistant")]

use embassy_time::Duration;
use winderoo_embassy::state::StatusCache;
use winderoo_embassy::tasks::RuntimeCommand;

use winderoo_firmware::model::{
    Direction, StatusSnapshot, UpdateAction, UpdateRequest, WinderStatus,
};
use winderoo_firmware::time::TimeOfDay;

pub type RuntimeSender = embassy_sync::channel::Sender<
    'static,
    embassy_sync::blocking_mutex::raw::CriticalSectionRawMutex,
    RuntimeCommand,
    { crate::RUNTIME_QUEUE_DEPTH },
>;

/// Home Assistant `select` options for direction.
pub const DIRECTION_OPTIONS: [&str; 3] = ["CCW", "BOTH", "CW"];

/// Home Assistant `select` options for hours (`00`-`23`).
pub const HOUR_OPTIONS: [&str; 24] = [
    "00", "01", "02", "03", "04", "05", "06", "07", "08", "09", "10", "11", "12", "13", "14", "15",
    "16", "17", "18", "19", "20", "21", "22", "23",
];

/// Home Assistant `select` options for minutes (10-minute steps).
pub const MINUTE_OPTIONS: [&str; 6] = ["00", "10", "20", "30", "40", "50"];

/// Home Assistant `select` options for UTC offsets.
///
/// This ordering matches the Arduino firmware (`utcOffsetValues`).
pub const UTC_OFFSET_OPTIONS: [&str; 40] = [
    "-12", "-11", "-10", "-9.5", "-9", "-8", "-7", "-6", "-5", "-4.5", "-4", "-3.5", "-3", "-2",
    "-1", "0", "1", "2", "3", "3.5", "4", "4.5", "5", "5.5", "5.75", "6", "6.5", "7", "8", "8.75",
    "9", "9.5", "10", "10.5", "11", "11.5", "12", "12.75", "13", "14",
];

const UTC_OFFSET_VALUES: [f32; 40] = [
    -12.0, -11.0, -10.0, -9.5, -9.0, -8.0, -7.0, -6.0, -5.0, -4.5, -4.0, -3.5, -3.0, -2.0, -1.0,
    0.0, 1.0, 2.0, 3.0, 3.5, 4.0, 4.5, 5.0, 5.5, 5.75, 6.0, 6.5, 7.0, 8.0, 8.75, 9.0, 9.5, 10.0,
    10.5, 11.0, 11.5, 12.0, 12.75, 13.0, 14.0,
];

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

fn round_to_i32(value: f32) -> i32 {
    if value >= 0.0 {
        (value + 0.5) as i32
    } else {
        (value - 0.5) as i32
    }
}

fn clamp_u16(value: f32, min: u16, max: u16) -> u16 {
    let v = round_to_i32(value);
    let min = min as i32;
    let max = max as i32;
    v.clamp(min, max) as u16
}

fn index_to_direction(index: u8) -> Direction {
    match index {
        0 => Direction::CounterClockwise,
        2 => Direction::Clockwise,
        _ => Direction::Both,
    }
}

fn timer_minutes_to_index(minute: u8) -> u8 {
    match minute {
        0 => 0,
        10 => 1,
        20 => 2,
        30 => 3,
        40 => 4,
        50 => 5,
        _ => 0,
    }
}

fn index_to_timer_minutes(index: u8) -> u8 {
    match index {
        0 => 0,
        1 => 10,
        2 => 20,
        3 => 30,
        4 => 40,
        5 => 50,
        _ => 0,
    }
}

fn gmt_offset_to_index(offset: f32) -> u8 {
    for (idx, value) in UTC_OFFSET_VALUES.iter().enumerate() {
        if (offset - *value).abs() < 0.01 {
            return idx as u8;
        }
    }
    0
}

fn index_to_gmt_offset(index: u8) -> f32 {
    UTC_OFFSET_VALUES
        .get(index as usize)
        .copied()
        .unwrap_or(0.0)
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
pub async fn ha_direction_select_task(
    status_cache: &'static StatusCache,
    runtime: RuntimeSender,
    mut direction: embassy_ha::Select<'static>,
) -> ! {
    loop {
        let snapshot = status_cache.snapshot();
        direction.set_state_index(snapshot.direction.home_assistant_index());

        if let Ok(cmd) = embassy_time::with_timeout(Duration::from_secs(1), direction.wait()).await
        {
            let snapshot = status_cache.snapshot();
            let mut update = base_update(&snapshot);
            update.direction = index_to_direction(cmd);
            let _ = runtime.send(RuntimeCommand::ApplyUpdate(update)).await;
        }
    }
}

#[embassy_executor::task]
pub async fn ha_timer_hour_task(
    status_cache: &'static StatusCache,
    runtime: RuntimeSender,
    mut hour: embassy_ha::Select<'static>,
) -> ! {
    loop {
        let snapshot = status_cache.snapshot();
        // Hours are a direct select index (00-23).
        hour.set_state_index(snapshot.timer_hour);

        if let Ok(cmd) = embassy_time::with_timeout(Duration::from_secs(1), hour.wait()).await {
            if cmd > 23 {
                continue;
            }
            let snapshot = status_cache.snapshot();
            let mut update = base_update(&snapshot);
            update.hour = cmd;
            let _ = runtime.send(RuntimeCommand::ApplyUpdate(update)).await;
        }
    }
}

#[embassy_executor::task]
pub async fn ha_timer_minutes_task(
    status_cache: &'static StatusCache,
    runtime: RuntimeSender,
    mut minutes: embassy_ha::Select<'static>,
) -> ! {
    loop {
        let snapshot = status_cache.snapshot();
        // Minutes are limited to 10-minute steps (00/10/20/30/40/50) for Arduino parity.
        minutes.set_state_index(timer_minutes_to_index(snapshot.timer_minutes));

        if let Ok(cmd) = embassy_time::with_timeout(Duration::from_secs(1), minutes.wait()).await {
            let snapshot = status_cache.snapshot();
            let mut update = base_update(&snapshot);
            update.minutes = index_to_timer_minutes(cmd);
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
            // ArduinoHA defaults: 100..960 step 10 (seconds).
            let mut value = clamp_u16(cmd, 100, 960);
            value = (value / 10) * 10;
            let snapshot = status_cache.snapshot();
            let mut update = base_update(&snapshot);
            update.custom_wind_duration_secs = value as u32;
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
            // ArduinoHA defaults: 10..900 step 5 (seconds).
            let mut value = clamp_u16(cmd, 10, 900);
            value = (value / 5) * 5;
            let snapshot = status_cache.snapshot();
            let mut update = base_update(&snapshot);
            update.custom_wind_pause_secs = value as u32;
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
            // ArduinoHA defaults: 1..16 step 1 (seconds).
            let value = clamp_u16(cmd, 1, 16);
            let snapshot = status_cache.snapshot();
            let mut update = base_update(&snapshot);
            update.rotation_duration_secs = value;
            let _ = runtime.send(RuntimeCommand::ApplyUpdate(update)).await;
        }
    }
}

#[embassy_executor::task]
pub async fn ha_rtc_offset_select_task(
    status_cache: &'static StatusCache,
    runtime: RuntimeSender,
    mut offset: embassy_ha::Select<'static>,
) -> ! {
    loop {
        let snapshot = status_cache.snapshot();
        offset.set_state_index(gmt_offset_to_index(snapshot.gmt_offset));

        if let Ok(cmd) = embassy_time::with_timeout(Duration::from_secs(1), offset.wait()).await {
            let snapshot = status_cache.snapshot();
            let mut update = base_update(&snapshot);
            update.rtc_gmt_offset = index_to_gmt_offset(cmd);
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
        dst.set(if snapshot.dst {
            BinaryState::On
        } else {
            BinaryState::Off
        });

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
pub async fn ha_screen_schedule_start_hour_task(
    status_cache: &'static StatusCache,
    runtime: RuntimeSender,
    mut value: embassy_ha::Select<'static>,
) -> ! {
    loop {
        let snapshot = status_cache.snapshot();
        value.set_state_index(snapshot.screen_schedule_start.hour);

        if let Ok(cmd) = embassy_time::with_timeout(Duration::from_secs(1), value.wait()).await {
            let snapshot = status_cache.snapshot();
            let mut update = base_update(&snapshot);
            if cmd <= 23 {
                let current = snapshot.screen_schedule_start;
                let time = TimeOfDay::new(cmd, current.minute).unwrap_or(current);
                update.screen_schedule_start = Some(time);
            }
            let _ = runtime.send(RuntimeCommand::ApplyUpdate(update)).await;
        }
    }
}

#[embassy_executor::task]
pub async fn ha_screen_schedule_start_minute_task(
    status_cache: &'static StatusCache,
    runtime: RuntimeSender,
    mut value: embassy_ha::Select<'static>,
) -> ! {
    loop {
        let snapshot = status_cache.snapshot();
        value.set_state_index(timer_minutes_to_index(
            snapshot.screen_schedule_start.minute,
        ));

        if let Ok(cmd) = embassy_time::with_timeout(Duration::from_secs(1), value.wait()).await {
            let snapshot = status_cache.snapshot();
            let mut update = base_update(&snapshot);
            let current = snapshot.screen_schedule_start;
            let minute = index_to_timer_minutes(cmd);
            let time = TimeOfDay::new(current.hour, minute).unwrap_or(current);
            update.screen_schedule_start = Some(time);
            let _ = runtime.send(RuntimeCommand::ApplyUpdate(update)).await;
        }
    }
}

#[embassy_executor::task]
pub async fn ha_screen_schedule_end_hour_task(
    status_cache: &'static StatusCache,
    runtime: RuntimeSender,
    mut value: embassy_ha::Select<'static>,
) -> ! {
    loop {
        let snapshot = status_cache.snapshot();
        value.set_state_index(snapshot.screen_schedule_end.hour);

        if let Ok(cmd) = embassy_time::with_timeout(Duration::from_secs(1), value.wait()).await {
            let snapshot = status_cache.snapshot();
            let mut update = base_update(&snapshot);
            if cmd <= 23 {
                let current = snapshot.screen_schedule_end;
                let time = TimeOfDay::new(cmd, current.minute).unwrap_or(current);
                update.screen_schedule_end = Some(time);
            }
            let _ = runtime.send(RuntimeCommand::ApplyUpdate(update)).await;
        }
    }
}

#[embassy_executor::task]
pub async fn ha_screen_schedule_end_minute_task(
    status_cache: &'static StatusCache,
    runtime: RuntimeSender,
    mut value: embassy_ha::Select<'static>,
) -> ! {
    loop {
        let snapshot = status_cache.snapshot();
        value.set_state_index(timer_minutes_to_index(snapshot.screen_schedule_end.minute));

        if let Ok(cmd) = embassy_time::with_timeout(Duration::from_secs(1), value.wait()).await {
            let snapshot = status_cache.snapshot();
            let mut update = base_update(&snapshot);
            let current = snapshot.screen_schedule_end;
            let minute = index_to_timer_minutes(cmd);
            let time = TimeOfDay::new(current.hour, minute).unwrap_or(current);
            update.screen_schedule_end = Some(time);
            let _ = runtime.send(RuntimeCommand::ApplyUpdate(update)).await;
        }
    }
}

#[embassy_executor::task]
pub async fn ha_rssi_reception_task(
    status_cache: &'static StatusCache,
    mut sensor: embassy_ha::Sensor<'static>,
) -> ! {
    use embassy_time::{Duration, Timer};

    loop {
        let snapshot = status_cache.snapshot();
        sensor.publish(snapshot.rssi_db as f32);
        Timer::after(Duration::from_secs(1)).await;
    }
}

#[embassy_executor::task]
pub async fn ha_activity_task(
    status_cache: &'static StatusCache,
    mut sensor: embassy_ha::TextSensor<'static>,
) -> ! {
    use embassy_time::{Duration, Timer};

    loop {
        let snapshot = status_cache.snapshot();
        sensor.publish(snapshot.status.as_str());
        Timer::after(Duration::from_secs(1)).await;
    }
}

#[embassy_executor::task]
pub async fn ha_current_epoch_task(
    status_cache: &'static StatusCache,
    mut sensor: embassy_ha::TextSensor<'static>,
) -> ! {
    use embassy_time::{Duration, Timer};

    let mut buf = alloc::string::String::new();
    loop {
        let snapshot = status_cache.snapshot();
        buf.clear();
        use core::fmt::Write;
        let _ = write!(&mut buf, "{}", snapshot.current_time_epoch);
        sensor.publish(&buf);
        Timer::after(Duration::from_secs(1)).await;
    }
}
