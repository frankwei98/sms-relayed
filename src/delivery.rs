//! Delivery facade: production assembly plus the producer wakeup handle.

use std::sync::Arc;
use std::time::Duration;

use anyhow::Result;
use tokio::sync::Notify;

use crate::config::AppConfig;
use crate::persistence::Store;

mod dispatcher;
mod worker;

pub(crate) use worker::{DeliverySettings, DeliveryWorker};

impl DeliverySettings {
    pub(crate) fn from_app_config(config: &AppConfig) -> Self {
        Self {
            concurrency: config.delivery.concurrency,
            channel_timeout: Duration::from_secs(config.http.request_timeout_secs),
        }
    }
}

#[derive(Clone, Default)]
pub(crate) struct DeliveryWakeup {
    notify: Arc<Notify>,
}

impl DeliveryWakeup {
    pub(crate) fn new() -> Self {
        Self::default()
    }

    pub(crate) fn notify(&self) {
        self.notify.notify_one();
    }

    #[cfg(not(test))]
    async fn wait(&self) {
        self.notify.notified().await;
    }

    #[cfg(test)]
    pub(crate) async fn wait(&self) {
        self.notify.notified().await;
    }
}

impl DeliveryWorker {
    pub(crate) fn new(
        store: Store,
        settings: DeliverySettings,
        forwarding_config: AppConfig,
        client: Arc<reqwest::Client>,
        webhook_client: Arc<reqwest::Client>,
        wakeup: DeliveryWakeup,
    ) -> Result<Self> {
        let dispatcher = Arc::new(dispatcher::ProductionDispatcher::new(
            forwarding_config,
            client,
            webhook_client,
        )?);
        Ok(Self::with_dispatcher(store, settings, dispatcher, wakeup))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn settings_extract_only_worker_scheduling_inputs() {
        let mut config = AppConfig::default();
        config.delivery.concurrency = 7;
        config.http.request_timeout_secs = 11;

        let settings = DeliverySettings::from_app_config(&config);

        assert_eq!(settings.concurrency, 7);
        assert_eq!(settings.channel_timeout, Duration::from_secs(11));
    }
}
