mod sim;

use std::time::Duration;

use dioxus::prelude::*;

use crate::sim::SimEngine;
use winderoo_firmware::model::{Direction, UpdateAction, UpdateRequest};
use winderoo_firmware::time::TimeOfDay;

fn main() {
    use dioxus::desktop::{Config, WindowBuilder};

    LaunchBuilder::new()
        .with_cfg(desktop! {
            Config::new().with_window(
                WindowBuilder::new().with_title("Winderoo Simulator")
            )
        })
        .launch(app);
}

#[derive(Debug, Clone, PartialEq)]
struct UpdateDraft {
    direction: Direction,
    rotations_per_day: String,
    action: UpdateAction,
    timer_hour: String,
    timer_minutes: String,
    timer_enabled: bool,
    screen_sleep: bool,
    screen_schedule_enabled: bool,
    screen_schedule_start: String,
    screen_schedule_end: String,
    custom_wind_duration_secs: String,
    custom_wind_pause_secs: String,
    rotation_duration_secs: String,
    rtc_gmt_offset: String,
    rtc_dst: bool,
}

impl UpdateDraft {
    fn default_values() -> Self {
        Self {
            direction: Direction::Both,
            rotations_per_day: "220".to_string(),
            action: UpdateAction::Stop,
            timer_hour: "0".to_string(),
            timer_minutes: "0".to_string(),
            timer_enabled: false,
            screen_sleep: false,
            screen_schedule_enabled: false,
            screen_schedule_start: "00:00".to_string(),
            screen_schedule_end: "00:00".to_string(),
            custom_wind_duration_secs: "180".to_string(),
            custom_wind_pause_secs: "15".to_string(),
            rotation_duration_secs: "8".to_string(),
            rtc_gmt_offset: "0".to_string(),
            rtc_dst: false,
        }
    }

    fn from_engine(engine: &SimEngine) -> Self {
        Self::from_state(&engine.runtime_state())
    }

    fn from_state(state: &winderoo_firmware::model::RuntimeState) -> Self {
        Self {
            direction: state.direction,
            rotations_per_day: state.rotations_per_day.to_string(),
            action: UpdateAction::Stop,
            timer_hour: state.timer.start_time.hour.to_string(),
            timer_minutes: state.timer.start_time.minute.to_string(),
            timer_enabled: state.timer.enabled,
            screen_sleep: state.screen.sleep,
            screen_schedule_enabled: state.screen.schedule.enabled,
            screen_schedule_start: state.screen.schedule.start.to_hh_mm(),
            screen_schedule_end: state.screen.schedule.end.to_hh_mm(),
            custom_wind_duration_secs: state.custom_wind_duration_secs.to_string(),
            custom_wind_pause_secs: state.custom_wind_pause_secs.to_string(),
            rotation_duration_secs: state.rotation_duration_secs.to_string(),
            rtc_gmt_offset: state.rtc.gmt_offset.to_string(),
            rtc_dst: state.rtc.dst,
        }
    }

    fn parse_u8(field: &str, value: &str) -> Result<u8, String> {
        value
            .trim()
            .parse::<u8>()
            .map_err(|_| format!("Invalid {field}: {value}"))
    }

    fn parse_u16(field: &str, value: &str) -> Result<u16, String> {
        value
            .trim()
            .parse::<u16>()
            .map_err(|_| format!("Invalid {field}: {value}"))
    }

    fn parse_u32(field: &str, value: &str) -> Result<u32, String> {
        value
            .trim()
            .parse::<u32>()
            .map_err(|_| format!("Invalid {field}: {value}"))
    }

    fn parse_f32(field: &str, value: &str) -> Result<f32, String> {
        value
            .trim()
            .parse::<f32>()
            .map_err(|_| format!("Invalid {field}: {value}"))
    }

    fn parse_time(field: &str, value: &str) -> Result<TimeOfDay, String> {
        TimeOfDay::parse_hh_mm(value.trim())
            .map_err(|err| format!("Invalid {field} ({value}): {err}"))
    }

    fn to_update_request(&self) -> Result<UpdateRequest, String> {
        let rotations_per_day = Self::parse_u16("TPD", &self.rotations_per_day)?;
        let hour = Self::parse_u8("timer hour", &self.timer_hour)?;
        let minutes = Self::parse_u8("timer minutes", &self.timer_minutes)?;

        let _ = TimeOfDay::new(hour, minutes).map_err(|err| format!("Invalid timer time: {err}"))?;

        Ok(UpdateRequest {
            direction: self.direction,
            rotations_per_day,
            action: self.action,
            hour,
            minutes,
            timer_enabled: self.timer_enabled,
            screen_sleep: self.screen_sleep,
            screen_schedule_enabled: Some(self.screen_schedule_enabled),
            screen_schedule_start: Some(Self::parse_time(
                "screen schedule start",
                &self.screen_schedule_start,
            )?),
            screen_schedule_end: Some(Self::parse_time(
                "screen schedule end",
                &self.screen_schedule_end,
            )?),
            custom_wind_duration_secs: Self::parse_u32(
                "custom wind duration (seconds)",
                &self.custom_wind_duration_secs,
            )?,
            custom_wind_pause_secs: Self::parse_u32(
                "custom wind pause (seconds)",
                &self.custom_wind_pause_secs,
            )?,
            rotation_duration_secs: Self::parse_u16(
                "rotation duration (seconds)",
                &self.rotation_duration_secs,
            )?,
            rtc_gmt_offset: Self::parse_f32("RTC GMT offset", &self.rtc_gmt_offset)?,
            rtc_dst: self.rtc_dst,
        })
    }
}

impl Default for UpdateDraft {
    fn default() -> Self {
        Self::default_values()
    }
}

fn app() -> Element {
    let mut engine = use_signal(SimEngine::default);

    let mut running = use_signal(|| false);
    let mut steps_per_frame = use_signal(|| 1u32);
    let mut draft = use_signal(UpdateDraft::default);
    let mut draft_error = use_signal(|| Option::<String>::None);

    // One-time init: load a reasonable draft from the engine state.
    {
        let mut did_init = use_signal(|| false);
        use_effect(move || {
            if !did_init() {
                let next = engine.with(UpdateDraft::from_engine);
                draft.set(next);
                did_init.set(true);
            }
        });
    }

    // Run-loop: step the simulation while "running" is on.
    use_future(move || async move {
        loop {
            if running() {
                let steps = steps_per_frame().max(1);
                engine.with_mut(|e| {
                    for _ in 0..steps {
                        e.step_tick();
                    }
                });
            }
            tokio::time::sleep(Duration::from_millis(16)).await;
        }
    });

    let view = engine.with(|e| {
        (
            e.now_ms(),
            e.now_epoch(),
            e.now_time_of_day().to_hh_mm(),
            e.motor().clone(),
            e.led().clone(),
            e.display().clone(),
            e.system().clone(),
            e.runtime_state(),
            e.status_snapshot(),
            e.log()
                .iter()
                .rev()
                .take(300)
                .cloned()
                .collect::<Vec<_>>(),
            e.config().tick_interval_ms,
        )
    });

    let (
        now_ms,
        now_epoch,
        now_tod,
        motor,
        led,
        display,
        system,
        state,
        status,
        recent_log,
        tick_interval_ms,
    ) = view;

    let motor_dir = format!("{:?}", motor.direction);
    let led_pattern = format!("{:?}", led.last_pattern);

    rsx! {
        style { {APP_CSS} }

        div { class: "app",
            header { class: "topbar",
                div { class: "title",
                    h1 { "Winderoo Simulator" }
                    div { class: "subtitle",
                        "tick={tick_interval_ms}ms · t={now_ms}ms · epoch={now_epoch} · tod={now_tod}"
                    }
                }

                div { class: "controls",
                    button {
                        class: if running() { "btn primary" } else { "btn" },
                        onclick: move |_| running.toggle(),
                        if running() { "Running" } else { "Paused" }
                    }
                    button {
                        class: "btn",
                        onclick: move |_| engine.with_mut(|e| e.step_tick()),
                        "Step tick"
                    }
                    button {
                        class: "btn",
                        onclick: move |_| engine.with_mut(|e| e.clear_log()),
                        "Clear log"
                    }
                    button {
                        class: "btn danger",
                        onclick: move |_| {
                            running.set(false);
                            engine.set(SimEngine::default());
                            draft_error.set(None);
                            // Reload draft from the new engine state.
                            let next = engine.with(UpdateDraft::from_engine);
                            draft.set(next);
                        },
                        "Reset sim"
                    }
                }
            }

            main { class: "grid",
                section { class: "card",
                    h2 { "Device" }

                    div { class: "row",
                        label { "Power" }
                        input {
                            r#type: "checkbox",
                            checked: state.winder_enabled,
                            onclick: move |_| {
                                let next = !state.winder_enabled;
                                engine.with_mut(|e| e.apply_power(next));
                            }
                        }
                        span { class: "muted", "{state.winder_enabled}" }
                    }

                    div { class: "row",
                        label { "Timer enabled" }
                        input {
                            r#type: "checkbox",
                            checked: state.timer.enabled,
                            onclick: move |_| {
                                let next = !state.timer.enabled;
                                engine.with_mut(|e| e.apply_timer_enabled(next));
                            }
                        }
                        span { class: "muted", "{state.timer.enabled} @ {state.timer.start_time.to_hh_mm()}" }
                    }

                    div { class: "row",
                        label { "Routine" }
                        span { "{state.status_str()}" }
                        span { class: "muted", "progress={state.cycle_progress}" }
                    }

                    div { class: "row",
                        label { "Motor" }
                        span { "{motor.running}" }
                        span { class: "muted", "dir={motor_dir} starts={motor.starts} stops={motor.stops}" }
                    }

                    div { class: "row",
                        label { "LED" }
                        span { "{led_pattern}" }
                        span { class: "muted", "count={led.pattern_count}" }
                    }

                    div { class: "row",
                        label { "Display" }
                        span { "{display.title.clone().unwrap_or_else(|| \"(no title)\".to_string())}" }
                        span { class: "muted",
                            "notif={display.last_notification.clone().unwrap_or_else(|| \"(none)\".to_string())} · dyn={display.dynamic_renders} · clears={display.clears}"
                        }
                    }

                    div { class: "row",
                        label { "System" }
                        span { "sync={system.sync_time_requested} restart={system.restart_requests}" }
                        span { class: "muted", "persisted={system.last_persisted.is_some()}" }
                    }

                    div { class: "row",
                        label { "Status snapshot" }
                        span { class: "muted",
                            "rssi={status.rssi_db} api={status.api_version} est_finish={status.estimated_routine_finish_epoch}"
                        }
                    }
                }

                section { class: "card",
                    h2 { "Update payload" }
                    p { class: "muted",
                        "Edits here go through the same `apply_update` path as `/api/update`."
                    }

                    if let Some(err) = draft_error() {
                        div { class: "error", "{err}" }
                    }

                    div { class: "form",
                        div { class: "row",
                            label { "Action" }
                            select {
                                value: match draft().action {
                                    UpdateAction::Start => "START",
                                    UpdateAction::Stop => "STOP",
                                },
                                onchange: move |evt| {
                                    let v = evt.value();
                                    draft.with_mut(|d| {
                                        d.action = if v == "START" { UpdateAction::Start } else { UpdateAction::Stop };
                                    });
                                },
                                option { value: "START", "START" }
                                option { value: "STOP", "STOP" }
                            }
                        }

                        div { class: "row",
                            label { "Direction" }
                            select {
                                value: match draft().direction {
                                    Direction::Clockwise => "CW",
                                    Direction::CounterClockwise => "CCW",
                                    Direction::Both => "BOTH",
                                },
                                onchange: move |evt| {
                                    let v = evt.value();
                                    draft.with_mut(|d| {
                                        d.direction = match v.as_str() {
                                            "CW" => Direction::Clockwise,
                                            "CCW" => Direction::CounterClockwise,
                                            _ => Direction::Both,
                                        };
                                    });
                                },
                                option { value: "BOTH", "BOTH" }
                                option { value: "CCW", "CCW" }
                                option { value: "CW", "CW" }
                            }
                        }

                        div { class: "row",
                            label { "TPD" }
                            input {
                                class: "input",
                                value: "{draft().rotations_per_day}",
                                oninput: move |evt| draft.with_mut(|d| d.rotations_per_day = evt.value()),
                            }
                        }

                        div { class: "row",
                            label { "Timer enabled" }
                            input {
                                r#type: "checkbox",
                                checked: draft().timer_enabled,
                                onclick: move |_| draft.with_mut(|d| d.timer_enabled = !d.timer_enabled),
                            }
                        }

                        div { class: "row",
                            label { "Timer time" }
                            input {
                                class: "input small",
                                value: "{draft().timer_hour}",
                                oninput: move |evt| draft.with_mut(|d| d.timer_hour = evt.value()),
                            }
                            span { class: "muted", ":" }
                            input {
                                class: "input small",
                                value: "{draft().timer_minutes}",
                                oninput: move |evt| draft.with_mut(|d| d.timer_minutes = evt.value()),
                            }
                        }

                        div { class: "row",
                            label { "Screen sleep" }
                            input {
                                r#type: "checkbox",
                                checked: draft().screen_sleep,
                                onclick: move |_| draft.with_mut(|d| d.screen_sleep = !d.screen_sleep),
                            }
                        }

                        div { class: "row",
                            label { "Screen schedule enabled" }
                            input {
                                r#type: "checkbox",
                                checked: draft().screen_schedule_enabled,
                                onclick: move |_| draft.with_mut(|d| d.screen_schedule_enabled = !d.screen_schedule_enabled),
                            }
                        }

                        div { class: "row",
                            label { "Screen schedule" }
                            input {
                                class: "input",
                                value: "{draft().screen_schedule_start}",
                                oninput: move |evt| draft.with_mut(|d| d.screen_schedule_start = evt.value()),
                            }
                            span { class: "muted", "→" }
                            input {
                                class: "input",
                                value: "{draft().screen_schedule_end}",
                                oninput: move |evt| draft.with_mut(|d| d.screen_schedule_end = evt.value()),
                            }
                        }

                        div { class: "row",
                            label { "Custom wind duration (s)" }
                            input {
                                class: "input",
                                value: "{draft().custom_wind_duration_secs}",
                                oninput: move |evt| draft.with_mut(|d| d.custom_wind_duration_secs = evt.value()),
                            }
                        }
                        div { class: "row",
                            label { "Custom wind pause (s)" }
                            input {
                                class: "input",
                                value: "{draft().custom_wind_pause_secs}",
                                oninput: move |evt| draft.with_mut(|d| d.custom_wind_pause_secs = evt.value()),
                            }
                        }
                        div { class: "row",
                            label { "Rotation duration (s)" }
                            input {
                                class: "input",
                                value: "{draft().rotation_duration_secs}",
                                oninput: move |evt| draft.with_mut(|d| d.rotation_duration_secs = evt.value()),
                            }
                        }

                        div { class: "row",
                            label { "RTC GMT offset" }
                            input {
                                class: "input",
                                value: "{draft().rtc_gmt_offset}",
                                oninput: move |evt| draft.with_mut(|d| d.rtc_gmt_offset = evt.value()),
                            }
                        }

                        div { class: "row",
                            label { "RTC DST" }
                            input {
                                r#type: "checkbox",
                                checked: draft().rtc_dst,
                                onclick: move |_| draft.with_mut(|d| d.rtc_dst = !d.rtc_dst),
                            }
                        }

                        div { class: "row actions",
                            button {
                                class: "btn",
                                onclick: move |_| {
                                    let next = engine.with(UpdateDraft::from_engine);
                                    draft.set(next);
                                    draft_error.set(None);
                                },
                                "Load from state"
                            }
                            button {
                                class: "btn primary",
                                onclick: move |_| {
                                    let res = draft.with(|d| d.to_update_request());
                                    match res {
                                        Ok(update) => {
                                            engine.with_mut(|e| e.apply_update(update));
                                            draft_error.set(None);
                                        }
                                        Err(err) => draft_error.set(Some(err)),
                                    }
                                },
                                "Apply update"
                            }

                            button {
                                class: "btn",
                                onclick: move |_| engine.with_mut(|e| e.apply_update(e.update_from_state_with_action(UpdateAction::Start))),
                                "Quick START"
                            }
                            button {
                                class: "btn",
                                onclick: move |_| engine.with_mut(|e| e.apply_update(e.update_from_state_with_action(UpdateAction::Stop))),
                                "Quick STOP"
                            }
                            button {
                                class: "btn danger",
                                onclick: move |_| engine.with_mut(|e| e.request_reset()),
                                "Reset device"
                            }
                        }
                    }
                }

                section { class: "card log",
                    h2 { "Event log (latest first)" }

                    div { class: "row",
                        label { "Steps/frame" }
                        input {
                            class: "input small",
                            value: "{steps_per_frame()}",
                            oninput: move |evt| {
                                let v = evt.value().parse::<u32>().unwrap_or(1);
                                steps_per_frame.set(v.max(1));
                            }
                        }
                        span { class: "muted", "≈ sim_speed={tick_interval_ms}ms × steps/frame" }
                    }

                    pre {
                        for entry in &recent_log {
                            div { "{entry.t_ms:>8}ms  {entry.message}" }
                        }
                        if recent_log.is_empty() {
                            div { class: "muted", "(no events yet)" }
                        }
                    }
                }
            }
        }
    }
}

const APP_CSS: &str = r#"
.app {
  font-family: ui-sans-serif, system-ui, -apple-system, Segoe UI, Roboto, Helvetica, Arial, "Apple Color Emoji", "Segoe UI Emoji";
  color: #0f172a;
  background: #f8fafc;
  min-height: 100vh;
}
.topbar {
  display: flex;
  justify-content: space-between;
  align-items: flex-start;
  padding: 16px 18px;
  border-bottom: 1px solid #e2e8f0;
  background: #ffffff;
}
.title h1 {
  margin: 0;
  font-size: 18px;
  font-weight: 700;
}
.subtitle {
  margin-top: 6px;
  font-size: 12px;
  color: #475569;
}
.controls {
  display: flex;
  gap: 8px;
  align-items: center;
}
.grid {
  display: grid;
  grid-template-columns: 420px 560px 1fr;
  gap: 14px;
  padding: 14px;
}
.card {
  background: #ffffff;
  border: 1px solid #e2e8f0;
  border-radius: 12px;
  padding: 14px;
  box-shadow: 0 1px 2px rgba(15, 23, 42, 0.06);
}
.card h2 {
  margin: 0 0 10px 0;
  font-size: 14px;
  font-weight: 700;
}
.muted { color: #64748b; font-size: 12px; }
.row {
  display: grid;
  grid-template-columns: 170px 1fr auto;
  gap: 8px;
  align-items: center;
  padding: 6px 0;
  border-bottom: 1px dashed #eef2f7;
}
.row:last-child { border-bottom: 0; }
.row label { font-size: 12px; color: #334155; }
.form .row { grid-template-columns: 170px 1fr; }
.actions { grid-template-columns: 1fr; display: flex; flex-wrap: wrap; gap: 8px; padding-top: 10px; }
.btn {
  border: 1px solid #cbd5e1;
  background: #ffffff;
  color: #0f172a;
  font-size: 12px;
  padding: 8px 10px;
  border-radius: 10px;
  cursor: pointer;
}
.btn:hover { background: #f1f5f9; }
.btn.primary { background: #2563eb; border-color: #2563eb; color: white; }
.btn.primary:hover { background: #1d4ed8; }
.btn.danger { background: #ef4444; border-color: #ef4444; color: white; }
.btn.danger:hover { background: #dc2626; }
.input, select {
  width: 100%;
  border: 1px solid #cbd5e1;
  border-radius: 10px;
  padding: 7px 10px;
  font-size: 12px;
  background: white;
}
.input.small { max-width: 90px; }
.error {
  background: #fee2e2;
  border: 1px solid #fecaca;
  color: #991b1b;
  padding: 8px 10px;
  border-radius: 10px;
  font-size: 12px;
  margin-bottom: 10px;
}
.log pre {
  margin: 10px 0 0 0;
  padding: 10px;
  background: #0b1220;
  color: #e2e8f0;
  border-radius: 12px;
  border: 1px solid #111827;
  max-height: calc(100vh - 220px);
  overflow: auto;
  font-family: ui-monospace, SFMono-Regular, Menlo, Monaco, Consolas, "Liberation Mono", "Courier New", monospace;
  font-size: 12px;
  line-height: 1.35;
}
"#;
