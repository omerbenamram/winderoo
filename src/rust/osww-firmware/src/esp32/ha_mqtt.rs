//! ESP32 Home Assistant MQTT integration.
//!
//! This module owns the ESP-IDF MQTT client wiring:
//! - Connect to a broker (credentials via env / build-time env).
//! - Publish Home Assistant discovery config (retained).
//! - Publish periodic state updates.
//! - Subscribe to command topics and apply them to the firmware controller.
//!
//! The *protocol* (topic names, discovery JSON, parsing/apply logic) lives in
//! `crate::home_assistant` so it can be tested on a host.

#![cfg(feature = "home-assistant")]

use crate::home_assistant::{self, HaCommand};
use embedded_svc::mqtt::client::QoS;
use esp_idf_svc::mqtt::client::{EspMqttClient, EventPayload, MqttClientConfiguration};
use log::warn;
use std::sync::{mpsc, Arc, Mutex};

use super::events::apply_events;
use super::{current_epoch, Esp32Error, Hardware, SharedState, Storage};

const HA_BROKER_ENV: Option<&str> = option_env!("HOME_ASSISTANT_BROKER");
const HA_USERNAME_ENV: Option<&str> = option_env!("HOME_ASSISTANT_USERNAME");
const HA_PASSWORD_ENV: Option<&str> = option_env!("HOME_ASSISTANT_PASSWORD");

pub(super) struct HomeAssistant {
    client: EspMqttClient<'static>,
    command_rx: mpsc::Receiver<HaCommand>,
    device_id: String,
    api_version: String,
}

impl HomeAssistant {
    pub(super) fn try_new(
        shared: Arc<Mutex<SharedState>>,
        api_version: &str,
    ) -> Result<Option<Self>, Esp32Error> {
        let broker = env_or_build("HOME_ASSISTANT_BROKER", HA_BROKER_ENV).unwrap_or_default();
        if broker.is_empty() {
            warn!("HOME_ASSISTANT_BROKER not set; skipping MQTT");
            return Ok(None);
        }

        let username = env_or_build("HOME_ASSISTANT_USERNAME", HA_USERNAME_ENV);
        let password = env_or_build("HOME_ASSISTANT_PASSWORD", HA_PASSWORD_ENV);
        let device_id = format!("winderoo-{}", mac_suffix());

        let (command_tx, command_rx) = mpsc::channel();
        let mut config = MqttClientConfiguration::default();
        config.client_id = Some("winderoo");
        config.username = username.as_deref();
        config.password = password.as_deref();

        let url = format!("mqtt://{}", broker);
        let tx = command_tx.clone();
        let client = EspMqttClient::new_cb(&url, &config, move |event| {
            if let EventPayload::Received {
                topic: Some(topic),
                data,
                ..
            } = event.payload()
            {
                if let Some(command) = HaCommand::parse(topic, data) {
                    let _ = tx.send(command);
                }
            }
        })?;

        let mut ha = Self {
            client,
            command_rx,
            device_id,
            api_version: api_version.to_string(),
        };

        ha.publish_config()?;
        ha.subscribe_commands()?;

        // Immediately publish a first state snapshot (helps HA pick up initial values).
        let _ = ha.publish_state(&shared);

        Ok(Some(ha))
    }

    fn subscribe_commands(&mut self) -> Result<(), Esp32Error> {
        let topics = HaCommand::topics(&self.device_id);
        for topic in topics {
            let _ = self.client.subscribe(&topic, QoS::AtMostOnce);
        }
        Ok(())
    }

    fn publish_config(&mut self) -> Result<(), Esp32Error> {
        let config_messages = home_assistant::config_messages(&self.device_id, &self.api_version);
        for (topic, payload) in config_messages {
            let _ = self
                .client
                .publish(&topic, QoS::AtMostOnce, true, payload.as_bytes());
        }
        Ok(())
    }

    pub(super) fn publish_state(
        &mut self,
        shared: &Arc<Mutex<SharedState>>,
    ) -> Result<(), Esp32Error> {
        let guard = shared.lock().map_err(|_| Esp32Error::Lock)?;
        let state_messages = home_assistant::state_messages(
            &self.device_id,
            &guard.controller.state,
            guard.rssi,
            current_epoch(),
        );
        for (topic, payload) in state_messages {
            let _ = self
                .client
                .publish(&topic, QoS::AtMostOnce, false, payload.as_bytes());
        }
        Ok(())
    }

    pub(super) fn drain_commands(
        &mut self,
        shared: &Arc<Mutex<SharedState>>,
        hardware: &Arc<Mutex<Hardware>>,
        storage: &Arc<Storage>,
    ) -> Result<(), Esp32Error> {
        while let Ok(cmd) = self.command_rx.try_recv() {
            let now = current_epoch();
            let events = {
                let mut guard = shared.lock().map_err(|_| Esp32Error::Lock)?;
                cmd.apply(&mut guard.controller, now)
            };
            apply_events(shared, hardware, storage, events)?;
        }
        Ok(())
    }
}

fn env_or_build(key: &str, build_value: Option<&'static str>) -> Option<String> {
    std::env::var(key)
        .ok()
        .or_else(|| build_value.map(|value| value.to_string()))
}

fn mac_suffix() -> String {
    unsafe {
        let mut mac = [0u8; 6];
        esp_idf_sys::esp_read_mac(
            mac.as_mut_ptr(),
            esp_idf_sys::esp_mac_type_t_ESP_MAC_WIFI_STA,
        );
        format!("{:02X}{:02X}{:02X}", mac[3], mac[4], mac[5])
    }
}
