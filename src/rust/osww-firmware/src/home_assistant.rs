//! Home Assistant (MQTT) discovery + command protocol.
//!
//! This module is intentionally **pure and host-testable**. It contains:
//! - Topic names and discovery payload generation.
//! - Parsing incoming command topics/payloads into strongly typed actions.
//! - Applying those actions to the firmware [`Controller`] (emitting [`ControllerEvent`]s).
//!
//! The ESP32 runtime wiring (MQTT client, subscriptions, publish loops) lives under the `esp32`
//! module so we can keep this logic covered by `cargo test` on a desktop.

use crate::api::{UpdateAction, UpdateRequest};
use crate::controller::{calculate_winding_duration_secs, Controller, ControllerEvent};
use crate::hardware::RandomSource;
use crate::model::{Direction, MotorDirection, RuntimeState};
use crate::settings::StoredSettings;
use crate::time::{epoch_with_offset, TimeOfDay};

/// A Home Assistant command received over MQTT.
#[derive(Debug, Clone, PartialEq)]
pub enum HaCommand {
    Power(bool),
    Timer(bool),
    /// OLED switch uses "ON" = screen awake.
    Oled(bool),
    Start,
    Stop,
    Direction(Direction),
    Rpd(u16),
    Hour(u8),
    Minute(u8),
    CustomWindDuration(u32),
    CustomWindPause(u32),
    RotationDuration(u16),
    RtcOffset(f32),
    RtcDst(bool),
    ScreenScheduleEnabled(bool),
    ScreenScheduleStartHour(u8),
    ScreenScheduleStartMinute(u8),
    ScreenScheduleEndHour(u8),
    ScreenScheduleEndMinute(u8),
}

impl HaCommand {
    /// Topics that should be subscribed to for incoming commands.
    pub fn topics(device_id: &str) -> Vec<String> {
        vec![
            format!("winderoo/{device_id}/power/set"),
            format!("winderoo/{device_id}/timer/set"),
            format!("winderoo/{device_id}/oled/set"),
            format!("winderoo/{device_id}/start/set"),
            format!("winderoo/{device_id}/stop/set"),
            format!("winderoo/{device_id}/direction/set"),
            format!("winderoo/{device_id}/rpd/set"),
            format!("winderoo/{device_id}/hour/set"),
            format!("winderoo/{device_id}/minute/set"),
            format!("winderoo/{device_id}/custom_wind_duration/set"),
            format!("winderoo/{device_id}/custom_wind_pause/set"),
            format!("winderoo/{device_id}/rotation_duration/set"),
            format!("winderoo/{device_id}/rtc_offset/set"),
            format!("winderoo/{device_id}/rtc_dst/set"),
            format!("winderoo/{device_id}/screen_schedule_enabled/set"),
            format!("winderoo/{device_id}/screen_schedule_start_hour/set"),
            format!("winderoo/{device_id}/screen_schedule_start_minute/set"),
            format!("winderoo/{device_id}/screen_schedule_end_hour/set"),
            format!("winderoo/{device_id}/screen_schedule_end_minute/set"),
        ]
    }

    /// Parse an MQTT topic + payload into a typed command.
    pub fn parse(topic: &str, payload: &[u8]) -> Option<Self> {
        let payload = core::str::from_utf8(payload).ok()?.trim();
        if topic.ends_with("/power/set") {
            return Some(Self::Power(parse_bool(payload)));
        }
        if topic.ends_with("/timer/set") {
            return Some(Self::Timer(parse_bool(payload)));
        }
        if topic.ends_with("/oled/set") {
            return Some(Self::Oled(parse_bool(payload)));
        }
        if topic.ends_with("/start/set") {
            return Some(Self::Start);
        }
        if topic.ends_with("/stop/set") {
            return Some(Self::Stop);
        }
        if topic.ends_with("/direction/set") {
            let direction = Direction::from_api(payload)?;
            return Some(Self::Direction(direction));
        }
        if topic.ends_with("/rpd/set") {
            return payload.parse().ok().map(Self::Rpd);
        }
        if topic.ends_with("/hour/set") {
            return payload.parse().ok().map(Self::Hour);
        }
        if topic.ends_with("/minute/set") {
            return payload.parse().ok().map(Self::Minute);
        }
        if topic.ends_with("/custom_wind_duration/set") {
            return payload.parse().ok().map(Self::CustomWindDuration);
        }
        if topic.ends_with("/custom_wind_pause/set") {
            return payload.parse().ok().map(Self::CustomWindPause);
        }
        if topic.ends_with("/rotation_duration/set") {
            return payload.parse().ok().map(Self::RotationDuration);
        }
        if topic.ends_with("/rtc_offset/set") {
            return payload.parse().ok().map(Self::RtcOffset);
        }
        if topic.ends_with("/rtc_dst/set") {
            return Some(Self::RtcDst(parse_bool(payload)));
        }
        if topic.ends_with("/screen_schedule_enabled/set") {
            return Some(Self::ScreenScheduleEnabled(parse_bool(payload)));
        }
        if topic.ends_with("/screen_schedule_start_hour/set") {
            return payload.parse().ok().map(Self::ScreenScheduleStartHour);
        }
        if topic.ends_with("/screen_schedule_start_minute/set") {
            return payload.parse().ok().map(Self::ScreenScheduleStartMinute);
        }
        if topic.ends_with("/screen_schedule_end_hour/set") {
            return payload.parse().ok().map(Self::ScreenScheduleEndHour);
        }
        if topic.ends_with("/screen_schedule_end_minute/set") {
            return payload.parse().ok().map(Self::ScreenScheduleEndMinute);
        }
        None
    }

    /// Apply this command to the controller, returning firmware events to enact.
    pub fn apply<R: RandomSource>(
        self,
        controller: &mut Controller<R>,
        now_epoch: u64,
    ) -> Vec<ControllerEvent> {
        match self {
            HaCommand::Power(enabled) => controller.apply_power(enabled),
            HaCommand::Timer(enabled) => controller.apply_timer_enabled(enabled),
            HaCommand::Oled(enabled) => {
                controller.state.screen.sleep = !enabled;
                vec![ControllerEvent::PersistSettings(Box::new(
                    StoredSettings::from_runtime(&controller.state),
                ))]
            }
            HaCommand::Start => {
                if !controller.state.routine.running {
                    controller
                        .apply_update(build_update(controller, UpdateAction::Start), now_epoch)
                } else {
                    Vec::new()
                }
            }
            HaCommand::Stop => {
                controller.apply_update(build_update(controller, UpdateAction::Stop), now_epoch)
            }
            HaCommand::Direction(direction) => {
                controller.state.direction = direction;
                controller.state.motor_direction = match direction {
                    Direction::Clockwise => MotorDirection::Clockwise,
                    Direction::CounterClockwise => MotorDirection::CounterClockwise,
                    Direction::Both => controller.state.motor_direction,
                };
                vec![ControllerEvent::PersistSettings(Box::new(
                    StoredSettings::from_runtime(&controller.state),
                ))]
            }
            HaCommand::Rpd(rpd) => {
                controller.state.rotations_per_day = rpd;
                if controller.state.routine.running {
                    controller.state.routine.estimated_finish_epoch =
                        now_epoch + calculate_winding_duration_secs(&controller.state);
                }
                vec![ControllerEvent::PersistSettings(Box::new(
                    StoredSettings::from_runtime(&controller.state),
                ))]
            }
            HaCommand::Hour(hour) => {
                if let Ok(time) = TimeOfDay::new(hour, controller.state.timer.start_time.minute) {
                    controller.state.timer.start_time = time;
                }
                vec![ControllerEvent::PersistSettings(Box::new(
                    StoredSettings::from_runtime(&controller.state),
                ))]
            }
            HaCommand::Minute(minute) => {
                if let Ok(time) = TimeOfDay::new(controller.state.timer.start_time.hour, minute) {
                    controller.state.timer.start_time = time;
                }
                vec![ControllerEvent::PersistSettings(Box::new(
                    StoredSettings::from_runtime(&controller.state),
                ))]
            }
            HaCommand::CustomWindDuration(duration) => {
                controller.state.custom_wind_duration_secs = duration;
                if controller.state.routine.running {
                    controller.state.routine.estimated_finish_epoch =
                        now_epoch + calculate_winding_duration_secs(&controller.state);
                }
                vec![ControllerEvent::PersistSettings(Box::new(
                    StoredSettings::from_runtime(&controller.state),
                ))]
            }
            HaCommand::CustomWindPause(duration) => {
                controller.state.custom_wind_pause_secs = duration;
                if controller.state.routine.running {
                    controller.state.routine.estimated_finish_epoch =
                        now_epoch + calculate_winding_duration_secs(&controller.state);
                }
                vec![ControllerEvent::PersistSettings(Box::new(
                    StoredSettings::from_runtime(&controller.state),
                ))]
            }
            HaCommand::RotationDuration(duration) => {
                controller.state.rotation_duration_secs = duration;
                if controller.state.routine.running {
                    controller.state.routine.estimated_finish_epoch =
                        now_epoch + calculate_winding_duration_secs(&controller.state);
                }
                vec![ControllerEvent::PersistSettings(Box::new(
                    StoredSettings::from_runtime(&controller.state),
                ))]
            }
            HaCommand::RtcOffset(offset) => {
                controller.state.rtc.gmt_offset = offset;
                vec![
                    ControllerEvent::SyncTime,
                    ControllerEvent::PersistSettings(Box::new(StoredSettings::from_runtime(
                        &controller.state,
                    ))),
                ]
            }
            HaCommand::RtcDst(dst) => {
                controller.state.rtc.dst = dst;
                vec![
                    ControllerEvent::SyncTime,
                    ControllerEvent::PersistSettings(Box::new(StoredSettings::from_runtime(
                        &controller.state,
                    ))),
                ]
            }
            HaCommand::ScreenScheduleEnabled(enabled) => {
                controller.state.screen.schedule.enabled = enabled;
                vec![ControllerEvent::PersistSettings(Box::new(
                    StoredSettings::from_runtime(&controller.state),
                ))]
            }
            HaCommand::ScreenScheduleStartHour(hour) => {
                if let Ok(time) =
                    TimeOfDay::new(hour, controller.state.screen.schedule.start.minute)
                {
                    controller.state.screen.schedule.start = time;
                }
                vec![ControllerEvent::PersistSettings(Box::new(
                    StoredSettings::from_runtime(&controller.state),
                ))]
            }
            HaCommand::ScreenScheduleStartMinute(minute) => {
                if let Ok(time) =
                    TimeOfDay::new(controller.state.screen.schedule.start.hour, minute)
                {
                    controller.state.screen.schedule.start = time;
                }
                vec![ControllerEvent::PersistSettings(Box::new(
                    StoredSettings::from_runtime(&controller.state),
                ))]
            }
            HaCommand::ScreenScheduleEndHour(hour) => {
                if let Ok(time) = TimeOfDay::new(hour, controller.state.screen.schedule.end.minute)
                {
                    controller.state.screen.schedule.end = time;
                }
                vec![ControllerEvent::PersistSettings(Box::new(
                    StoredSettings::from_runtime(&controller.state),
                ))]
            }
            HaCommand::ScreenScheduleEndMinute(minute) => {
                if let Ok(time) = TimeOfDay::new(controller.state.screen.schedule.end.hour, minute)
                {
                    controller.state.screen.schedule.end = time;
                }
                vec![ControllerEvent::PersistSettings(Box::new(
                    StoredSettings::from_runtime(&controller.state),
                ))]
            }
        }
    }
}

/// Build a full `UpdateRequest` equivalent to the REST API payload.
pub fn build_update<R: RandomSource>(
    controller: &Controller<R>,
    action: UpdateAction,
) -> UpdateRequest {
    UpdateRequest {
        direction: controller.state.direction,
        rotations_per_day: controller.state.rotations_per_day,
        action,
        hour: controller.state.timer.start_time.hour,
        minutes: controller.state.timer.start_time.minute,
        timer_enabled: controller.state.timer.enabled,
        screen_sleep: controller.state.screen.sleep,
        screen_schedule_enabled: Some(controller.state.screen.schedule.enabled),
        screen_schedule_start: Some(controller.state.screen.schedule.start),
        screen_schedule_end: Some(controller.state.screen.schedule.end),
        custom_wind_duration_secs: controller.state.custom_wind_duration_secs,
        custom_wind_pause_secs: controller.state.custom_wind_pause_secs,
        rotation_duration_secs: controller.state.rotation_duration_secs,
        rtc_gmt_offset: controller.state.rtc.gmt_offset,
        rtc_dst: controller.state.rtc.dst,
    }
}

fn parse_bool(value: &str) -> bool {
    matches!(value, "1" | "true" | "True" | "TRUE" | "on" | "ON")
}

/// Home Assistant discovery config messages (topic, JSON payload).
pub fn config_messages(device_id: &str, api_version: &str) -> Vec<(String, String)> {
    let base = format!("winderoo/{device_id}");
    let device = format!(
        "{{\"identifiers\":[\"{device_id}\"],\"name\":\"Winderoo\",\"model\":\"Winderoo\",\"manufacturer\":\"mwood77\",\"sw_version\":\"{api_version}\"}}"
    );
    vec![
        (
            format!("homeassistant/switch/{device_id}_power/config"),
            format!("{{\"name\":\"Power\",\"state_topic\":\"{base}/power\",\"command_topic\":\"{base}/power/set\",\"icon\":\"mdi:power\",\"device\":{device}}}"),
        ),
        (
            format!("homeassistant/switch/{device_id}_timer/config"),
            format!("{{\"name\":\"Timer Enabled\",\"state_topic\":\"{base}/timer\",\"command_topic\":\"{base}/timer/set\",\"icon\":\"mdi:timer\",\"device\":{device}}}"),
        ),
        (
            format!("homeassistant/switch/{device_id}_oled/config"),
            format!("{{\"name\":\"OLED\",\"state_topic\":\"{base}/oled\",\"command_topic\":\"{base}/oled/set\",\"icon\":\"mdi:overscan\",\"device\":{device}}}"),
        ),
        (
            format!("homeassistant/button/{device_id}_start/config"),
            format!("{{\"name\":\"Start\",\"command_topic\":\"{base}/start/set\",\"device\":{device}}}"),
        ),
        (
            format!("homeassistant/button/{device_id}_stop/config"),
            format!("{{\"name\":\"Stop\",\"command_topic\":\"{base}/stop/set\",\"device\":{device}}}"),
        ),
        (
            format!("homeassistant/number/{device_id}_rpd/config"),
            format!("{{\"name\":\"Rotations Per Day\",\"state_topic\":\"{base}/rpd\",\"command_topic\":\"{base}/rpd/set\",\"min\":100,\"max\":960,\"step\":10,\"icon\":\"mdi:rotate-3d-variant\",\"device\":{device}}}"),
        ),
        (
            format!("homeassistant/select/{device_id}_direction/config"),
            format!("{{\"name\":\"Direction\",\"state_topic\":\"{base}/direction\",\"command_topic\":\"{base}/direction/set\",\"options\":[\"CCW\",\"BOTH\",\"CW\"],\"icon\":\"mdi:arrow-left-right\",\"device\":{device}}}"),
        ),
        (
            format!("homeassistant/select/{device_id}_hour/config"),
            format!("{{\"name\":\"Hour\",\"state_topic\":\"{base}/hour\",\"command_topic\":\"{base}/hour/set\",\"options\":[\"00\",\"01\",\"02\",\"03\",\"04\",\"05\",\"06\",\"07\",\"08\",\"09\",\"10\",\"11\",\"12\",\"13\",\"14\",\"15\",\"16\",\"17\",\"18\",\"19\",\"20\",\"21\",\"22\",\"23\"],\"icon\":\"mdi:timer-sand-full\",\"device\":{device}}}"),
        ),
        (
            format!("homeassistant/select/{device_id}_minute/config"),
            format!("{{\"name\":\"Minutes\",\"state_topic\":\"{base}/minute\",\"command_topic\":\"{base}/minute/set\",\"options\":[\"00\",\"10\",\"20\",\"30\",\"40\",\"50\"],\"icon\":\"mdi:timer-sand-empty\",\"device\":{device}}}"),
        ),
        (
            format!("homeassistant/sensor/{device_id}_status/config"),
            format!("{{\"name\":\"Status\",\"state_topic\":\"{base}/status\",\"icon\":\"mdi:information\",\"device\":{device}}}"),
        ),
        (
            format!("homeassistant/sensor/{device_id}_rssi/config"),
            format!("{{\"name\":\"WiFi RSSI\",\"state_topic\":\"{base}/rssi\",\"unit_of_measurement\":\"dBm\",\"device\":{device}}}"),
        ),
        (
            format!("homeassistant/number/{device_id}_custom_wind_duration/config"),
            format!("{{\"name\":\"Time to Rotate\",\"state_topic\":\"{base}/custom_wind_duration\",\"command_topic\":\"{base}/custom_wind_duration/set\",\"min\":100,\"max\":960,\"step\":10,\"icon\":\"mdi:play-circle-outline\",\"device\":{device}}}"),
        ),
        (
            format!("homeassistant/number/{device_id}_custom_wind_pause/config"),
            format!("{{\"name\":\"Time to Pause\",\"state_topic\":\"{base}/custom_wind_pause\",\"command_topic\":\"{base}/custom_wind_pause/set\",\"min\":10,\"max\":900,\"step\":5,\"icon\":\"mdi:pause-circle-outline\",\"device\":{device}}}"),
        ),
        (
            format!("homeassistant/number/{device_id}_rotation_duration/config"),
            format!("{{\"name\":\"Rotation Duration\",\"state_topic\":\"{base}/rotation_duration\",\"command_topic\":\"{base}/rotation_duration/set\",\"min\":1,\"max\":16,\"step\":1,\"icon\":\"mdi:arrow-u-down-right\",\"device\":{device}}}"),
        ),
        (
            format!("homeassistant/select/{device_id}_rtc_offset/config"),
            format!("{{\"name\":\"UTC Offset\",\"state_topic\":\"{base}/rtc_offset\",\"command_topic\":\"{base}/rtc_offset/set\",\"options\":[\"-12\",\"-11\",\"-10\",\"-9.5\",\"-9\",\"-8\",\"-7\",\"-6\",\"-5\",\"-4.5\",\"-4\",\"-3.5\",\"-3\",\"-2\",\"-1\",\"0\",\"1\",\"2\",\"3\",\"3.5\",\"4\",\"4.5\",\"5\",\"5.5\",\"5.75\",\"6\",\"6.5\",\"7\",\"8\",\"8.75\",\"9\",\"9.5\",\"10\",\"10.5\",\"11\",\"11.5\",\"12\",\"12.75\",\"13\",\"14\"],\"icon\":\"mdi:clock-time-eight-outline\",\"device\":{device}}}"),
        ),
        (
            format!("homeassistant/switch/{device_id}_rtc_dst/config"),
            format!("{{\"name\":\"DST\",\"state_topic\":\"{base}/rtc_dst\",\"command_topic\":\"{base}/rtc_dst/set\",\"icon\":\"mdi:clock-time-four-outline\",\"device\":{device}}}"),
        ),
        (
            format!("homeassistant/switch/{device_id}_screen_schedule/config"),
            format!("{{\"name\":\"Screen Schedule Enabled\",\"state_topic\":\"{base}/screen_schedule_enabled\",\"command_topic\":\"{base}/screen_schedule_enabled/set\",\"icon\":\"mdi:calendar-clock\",\"device\":{device}}}"),
        ),
        (
            format!("homeassistant/select/{device_id}_screen_schedule_start_hour/config"),
            format!("{{\"name\":\"Screen Schedule Start Hour\",\"state_topic\":\"{base}/screen_schedule_start_hour\",\"command_topic\":\"{base}/screen_schedule_start_hour/set\",\"options\":[\"00\",\"01\",\"02\",\"03\",\"04\",\"05\",\"06\",\"07\",\"08\",\"09\",\"10\",\"11\",\"12\",\"13\",\"14\",\"15\",\"16\",\"17\",\"18\",\"19\",\"20\",\"21\",\"22\",\"23\"],\"icon\":\"mdi:clock-outline\",\"device\":{device}}}"),
        ),
        (
            format!("homeassistant/select/{device_id}_screen_schedule_start_minute/config"),
            format!("{{\"name\":\"Screen Schedule Start Minute\",\"state_topic\":\"{base}/screen_schedule_start_minute\",\"command_topic\":\"{base}/screen_schedule_start_minute/set\",\"options\":[\"00\",\"10\",\"20\",\"30\",\"40\",\"50\"],\"icon\":\"mdi:clock-outline\",\"device\":{device}}}"),
        ),
        (
            format!("homeassistant/select/{device_id}_screen_schedule_end_hour/config"),
            format!("{{\"name\":\"Screen Schedule End Hour\",\"state_topic\":\"{base}/screen_schedule_end_hour\",\"command_topic\":\"{base}/screen_schedule_end_hour/set\",\"options\":[\"00\",\"01\",\"02\",\"03\",\"04\",\"05\",\"06\",\"07\",\"08\",\"09\",\"10\",\"11\",\"12\",\"13\",\"14\",\"15\",\"16\",\"17\",\"18\",\"19\",\"20\",\"21\",\"22\",\"23\"],\"icon\":\"mdi:clock-outline\",\"device\":{device}}}"),
        ),
        (
            format!("homeassistant/select/{device_id}_screen_schedule_end_minute/config"),
            format!("{{\"name\":\"Screen Schedule End Minute\",\"state_topic\":\"{base}/screen_schedule_end_minute\",\"command_topic\":\"{base}/screen_schedule_end_minute/set\",\"options\":[\"00\",\"10\",\"20\",\"30\",\"40\",\"50\"],\"icon\":\"mdi:clock-outline\",\"device\":{device}}}"),
        ),
        (
            format!("homeassistant/sensor/{device_id}_rtc_epoch/config"),
            format!("{{\"name\":\"RTC Epoch Time\",\"state_topic\":\"{base}/rtc_epoch\",\"icon\":\"mdi:clock-time-nine-outline\",\"device\":{device}}}"),
        ),
    ]
}

/// Home Assistant state messages (topic, payload).
pub fn state_messages(
    device_id: &str,
    state: &RuntimeState,
    rssi: i32,
    now_epoch_utc: u64,
) -> Vec<(String, String)> {
    let base = format!("winderoo/{device_id}");
    let rtc_epoch = epoch_with_offset(now_epoch_utc, state.rtc.gmt_offset, state.rtc.dst);
    vec![
        (
            format!("{base}/power"),
            if state.winder_enabled { "ON" } else { "OFF" }.to_string(),
        ),
        (
            format!("{base}/timer"),
            if state.timer.enabled { "ON" } else { "OFF" }.to_string(),
        ),
        (
            format!("{base}/oled"),
            if state.screen.sleep { "OFF" } else { "ON" }.to_string(),
        ),
        (format!("{base}/status"), state.status_str().to_string()),
        (format!("{base}/rssi"), rssi.to_string()),
        (
            format!("{base}/direction"),
            state.direction.as_api_str().to_string(),
        ),
        (format!("{base}/rpd"), state.rotations_per_day.to_string()),
        (
            format!("{base}/hour"),
            format!("{:02}", state.timer.start_time.hour),
        ),
        (
            format!("{base}/minute"),
            format!("{:02}", state.timer.start_time.minute),
        ),
        (
            format!("{base}/custom_wind_duration"),
            state.custom_wind_duration_secs.to_string(),
        ),
        (
            format!("{base}/custom_wind_pause"),
            state.custom_wind_pause_secs.to_string(),
        ),
        (
            format!("{base}/rotation_duration"),
            state.rotation_duration_secs.to_string(),
        ),
        (
            format!("{base}/rtc_offset"),
            state.rtc.gmt_offset.to_string(),
        ),
        (
            format!("{base}/rtc_dst"),
            if state.rtc.dst { "ON" } else { "OFF" }.to_string(),
        ),
        (
            format!("{base}/screen_schedule_enabled"),
            if state.screen.schedule.enabled {
                "ON"
            } else {
                "OFF"
            }
            .to_string(),
        ),
        (
            format!("{base}/screen_schedule_start_hour"),
            format!("{:02}", state.screen.schedule.start.hour),
        ),
        (
            format!("{base}/screen_schedule_start_minute"),
            format!("{:02}", state.screen.schedule.start.minute),
        ),
        (
            format!("{base}/screen_schedule_end_hour"),
            format!("{:02}", state.screen.schedule.end.hour),
        ),
        (
            format!("{base}/screen_schedule_end_minute"),
            format!("{:02}", state.screen.schedule.end.minute),
        ),
        (format!("{base}/rtc_epoch"), rtc_epoch.to_string()),
    ]
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::hardware::XorShift32;
    use crate::settings::StoredSettings;

    #[test]
    fn parse_command_variants() {
        let cmd = HaCommand::parse("winderoo/dev/power/set", b"ON").unwrap();
        assert_eq!(cmd, HaCommand::Power(true));

        let cmd = HaCommand::parse("winderoo/dev/direction/set", b"CCW").unwrap();
        assert_eq!(cmd, HaCommand::Direction(Direction::CounterClockwise));

        let cmd = HaCommand::parse("winderoo/dev/rpd/set", b"240").unwrap();
        assert_eq!(cmd, HaCommand::Rpd(240));
    }

    #[test]
    fn config_includes_version() {
        let messages = config_messages("dev", "4.0.1");
        assert!(messages.iter().any(|(topic, payload)| {
            topic == "homeassistant/switch/dev_power/config"
                && payload.contains("\"sw_version\":\"4.0.1\"")
        }));
    }

    #[test]
    fn state_rtc_epoch_is_shifted() {
        let mut state = StoredSettings::default().to_runtime(true).expect("runtime");
        state.rtc.gmt_offset = 2.0;
        state.rtc.dst = false;
        let messages = state_messages("dev", &state, -42, 3600);
        let rtc = messages
            .iter()
            .find(|(topic, _)| topic == "winderoo/dev/rtc_epoch")
            .map(|(_, payload)| payload.as_str())
            .expect("rtc_epoch");
        assert_eq!(rtc, "10800");
    }

    #[test]
    fn apply_power_emits_persist() {
        let state = StoredSettings::default().to_runtime(true).expect("runtime");
        let mut controller = Controller::new(state, XorShift32::new(1));
        let events = HaCommand::Power(false).apply(&mut controller, 1000);
        assert!(!controller.state.winder_enabled);
        assert!(events
            .iter()
            .any(|e| matches!(e, ControllerEvent::PersistSettings(_))));
    }
}
