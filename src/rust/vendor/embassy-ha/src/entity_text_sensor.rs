use crate::{constants, Entity, EntityCommonConfig, EntityConfig, TextSensorState};

/// Configuration for a text sensor entity.
///
/// This uses the Home Assistant `sensor` domain, but publishes a **string** state
/// (useful for status-like values such as "Winding" / "Stopped").
#[derive(Debug, Default)]
pub struct TextSensorConfig {
    pub common: EntityCommonConfig,
}

impl TextSensorConfig {
    pub(crate) fn populate(&self, config: &mut EntityConfig) {
        self.common.populate(config);
        config.domain = constants::HA_DOMAIN_SENSOR;
    }
}

pub struct TextSensor<'a>(Entity<'a>);

impl<'a> TextSensor<'a> {
    pub(crate) fn new(entity: Entity<'a>) -> Self {
        Self(entity)
    }

    pub fn publish(&mut self, value: &str) {
        let publish = self.0.with_data(|data| {
            let storage = data.storage.as_text_sensor_mut();
            let timestamp = embassy_time::Instant::now();

            let mut buffer = heapless::String::<64>::new();
            for ch in value.chars() {
                if buffer.push(ch).is_err() {
                    break;
                }
            }

            let publish = match &storage.state {
                Some(state) => state.value.as_str() != buffer.as_str(),
                None => true,
            };

            storage.state = Some(TextSensorState {
                value: buffer,
                timestamp,
            });
            publish
        });

        if publish {
            self.0.queue_publish();
        }
    }
}
