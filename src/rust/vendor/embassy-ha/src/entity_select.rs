use crate::{
    CommandPolicy, Entity, EntityCommonConfig, EntityConfig, SelectCommand, SelectState,
    constants,
};

/// Configuration for a select entity.
///
/// See [`CommandPolicy`] for details on how commands are handled.
#[derive(Debug)]
pub struct SelectConfig {
    pub common: EntityCommonConfig,
    pub options: &'static [&'static str],
    pub command_policy: CommandPolicy,
}

impl Default for SelectConfig {
    fn default() -> Self {
        Self {
            common: EntityCommonConfig::default(),
            options: &[],
            command_policy: CommandPolicy::default(),
        }
    }
}

impl SelectConfig {
    pub(crate) fn populate(&self, config: &mut EntityConfig) {
        self.common.populate(config);
        config.domain = constants::HA_DOMAIN_SELECT;
        config.options = Some(self.options);
    }
}

pub struct Select<'a>(Entity<'a>);

impl<'a> Select<'a> {
    pub(crate) fn new(entity: Entity<'a>) -> Self {
        Self(entity)
    }

    pub fn state(&self) -> Option<u8> {
        self.0.with_data(|data| {
            let storage = data.storage.as_select_mut();
            storage.state.as_ref().map(|s| s.index)
        })
    }

    pub fn command(&self) -> Option<u8> {
        self.0.with_data(|data| {
            let storage = data.storage.as_select_mut();
            storage.command.as_ref().map(|s| s.index)
        })
    }

    pub fn set_state_index(&mut self, index: u8) {
        let publish = self.0.with_data(|data| {
            let Some(options) = data.config.options else {
                return false;
            };
            if (index as usize) >= options.len() {
                return false;
            }

            let storage = data.storage.as_select_mut();
            let timestamp = embassy_time::Instant::now();
            let publish = match &storage.command {
                Some(command) => command.index != index,
                None => true,
            };
            storage.state = Some(SelectState { index, timestamp });
            storage.command = Some(SelectCommand { index, timestamp });
            publish
        });

        if publish {
            self.0.queue_publish();
        }
    }

    pub async fn wait(&mut self) -> u8 {
        loop {
            self.0.wait_command().await;
            if let Some(index) = self.command() {
                return index;
            }
        }
    }
}

