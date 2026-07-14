use std::path::Path;
use std::sync::Arc;
use std::time::{Duration, Instant};

use anyhow::Result;
use inquire::{Confirm, Text};
use time::OffsetDateTime;

use crate::api::auth::SessionStore;
use crate::api::ApiState;
use crate::config::AppConfig;
use crate::dbus::FINGERPRINT_META_KEY;
use crate::dbus::{self, ReceivedSms, SendTarget};
use crate::delivery::run_delivery_worker;
use crate::events::{AppEvent, EventBus};
use crate::message::{Message, MessageDirection, MessageSource, MessageStatus};
use crate::modem::ModemService;
use crate::runner::{build_http_client, RealProcessRunner};
use crate::storage::{InboundInsertResult, MessageStore, NewMessage};

pub async fn run_forwarding(config_path: &Path) -> Result<()> {
    let config = AppConfig::load(config_path)?;
    config.validate()?;

    let store = MessageStore::open(Path::new(&config.api.database_path))?;
    let events = EventBus::new();

    let client = Arc::new(build_http_client(&config.http));
    let shell_timeout = Duration::from_secs(config.http.shell_timeout_secs);
    let shell_runner = RealProcessRunner;
    let modem_service = ModemService::new();

    // Recover expired delivery leases
    let lease_timeout =
        OffsetDateTime::now_utc().format(&time::format_description::well_known::Rfc3339)?;
    if let Ok(count) = store.recover_expired_leases(&lease_timeout) {
        if count > 0 {
            log::info!("recovered {} expired delivery leases", count);
        }
    }

    let store_inbound = store.clone();
    let events_inbound = events.clone();
    let config_inbound = config.clone();

    let dbus_modem = modem_service.clone();
    let dbus_future = dbus::monitor_dbus_with_handler(
        &config.app.modem_path,
        &config,
        move |sms| {
            let store = store_inbound.clone();
            let events = events_inbound.clone();
            let cfg = config_inbound.clone();
            async move { process_inbound_sms(&store, &events, &sms, &cfg).await }
        },
        &dbus_modem,
        &store,
    );

    let delivery_worker = run_delivery_worker(
        store.clone(),
        config.clone(),
        client.clone(),
        Arc::new(shell_runner),
        shell_timeout,
    );
    let retention_worker = run_retention_worker(store.clone(), config.clone());

    if config.api.enabled {
        let api_state = ApiState {
            config: Arc::new(config.clone()),
            config_path: config_path.to_path_buf(),
            store: store.clone(),
            events: events.clone(),
            started_at: Instant::now(),
            sessions: SessionStore::default(),
            modem: modem_service,
        };
        tokio::select! {
            biased;
            result = crate::api::serve(api_state) => {
                log::warn!("API server exited: {:?}", result);
                result
            }
            result = dbus_future => result,
            _ = delivery_worker => {
                Err(anyhow::anyhow!("delivery worker exited unexpectedly"))
            }
            _ = retention_worker => {
                Err(anyhow::anyhow!("retention worker exited unexpectedly"))
            }
        }
    } else {
        tokio::select! {
            result = dbus_future => result,
            _ = delivery_worker => {
                Err(anyhow::anyhow!("delivery worker exited unexpectedly"))
            }
            _ = retention_worker => {
                Err(anyhow::anyhow!("retention worker exited unexpectedly"))
            }
        }
    }
}

/// Process a received modem SMS: deduplicate, persist, and emit MessageCreated
/// only for first-time insertions.
pub async fn process_inbound_sms(
    store: &MessageStore,
    events: &EventBus,
    sms: &ReceivedSms,
    cfg: &AppConfig,
) -> Result<()> {
    let mut profile_keys: Vec<String> = cfg
        .enabled_profiles()
        .unwrap_or_default()
        .iter()
        .map(|p| p.key())
        .collect();
    profile_keys.sort();
    profile_keys.dedup();
    let fingerprint = store.get_meta(FINGERPRINT_META_KEY).unwrap_or_default();
    let msg = NewMessage::modem_inbound(
        &sms.phone_number,
        &sms.body,
        &sms.timestamp,
        &sms.modem_sms_path,
        &fingerprint,
    );
    match store.insert_inbound_message_with_deliveries(msg, &profile_keys)? {
        InboundInsertResult::Inserted(m) => {
            events.send(AppEvent::MessageCreated(m));
        }
        InboundInsertResult::Duplicate(_) => {
            log::debug!("duplicate inbound message suppressed");
        }
    }
    Ok(())
}

async fn run_retention_worker(store: MessageStore, config: AppConfig) {
    const RETENTION_INTERVAL: Duration = Duration::from_secs(6 * 60 * 60);
    if !config.retention.enabled {
        std::future::pending::<()>().await;
        return;
    }

    loop {
        let store = store.clone();
        let max_age_days = config.retention.max_age_days;
        let batch_size = config.retention.batch_size;
        match tokio::task::spawn_blocking(move || store.run_retention(max_age_days, batch_size))
            .await
        {
            Ok(Ok(deleted)) if deleted > 0 => {
                log::info!("retention deleted {} messages", deleted);
            }
            Ok(Ok(_)) => {}
            Ok(Err(e)) => {
                log::error!("retention failed: {}", e);
                crate::monitoring::capture_failure("retention", "retention.cleanup_failed");
            }
            Err(e) => {
                log::error!("retention task failed: {}", e);
                crate::monitoring::capture_failure("retention", "retention.task_failed");
            }
        }
        tokio::time::sleep(RETENTION_INTERVAL).await;
    }
}

pub async fn send_interactive(config_path: &Path) -> Result<()> {
    let config = AppConfig::load(config_path)?;
    config.validate()?;
    let tel_number = Text::new("Recipient number").prompt()?;
    let sms_text = Text::new("SMS text").prompt()?;
    if !Confirm::new("Send SMS now?").with_default(false).prompt()? {
        println!("send cancelled");
        return Ok(());
    }
    let connection = zbus::Connection::system().await?;
    let store = MessageStore::open(Path::new(&config.api.database_path))?;
    send_and_persist(
        &store,
        &connection,
        &config.app.modem_path,
        tel_number.trim(),
        sms_text.trim(),
        MessageSource::Cli,
        SendTarget::Cli,
    )
    .await?;
    Ok(())
}

async fn send_and_persist(
    store: &MessageStore,
    connection: &zbus::Connection,
    modem_path: &str,
    phone_number: &str,
    body: &str,
    source: MessageSource,
    target: SendTarget,
) -> anyhow::Result<Message> {
    let now = OffsetDateTime::now_utc().format(&time::format_description::well_known::Rfc3339)?;
    let msg = store.insert_message(NewMessage {
        direction: MessageDirection::Outbound,
        phone_number: phone_number.to_string(),
        body: body.to_string(),
        timestamp: now.clone(),
        status: MessageStatus::Sending,
        source,
        modem_sms_path: None,
        read_at: Some(now),
        error: None,
        inbound_dedupe_key: None,
    })?;
    match dbus::send_sms(connection, modem_path, phone_number, body, target).await {
        Ok(_outcome) => {
            let updated = store.update_status(msg.id, MessageStatus::Sent, None)?;
            Ok(updated)
        }
        Err(err) => {
            let updated =
                store.update_status(msg.id, MessageStatus::Failed, Some(err.to_string()))?;
            Ok(updated)
        }
    }
}

#[cfg(test)]
mod tests {
    use crate::events::EventBus;
    use crate::storage::MessageStore;

    use super::*;

    #[tokio::test]
    async fn process_inbound_sms_twice_produces_one_message_and_event() {
        let store = MessageStore::open_in_memory().unwrap();
        let events = EventBus::new();
        let mut rx = events.subscribe();

        let sms = ReceivedSms {
            phone_number: "+15550000000".to_string(),
            body: "duplicate test".to_string(),
            timestamp: "2026-07-12T17:00:00Z".to_string(),
            storage: 0,
            modem_sms_path: "/org/freedesktop/ModemManager1/SMS/42".to_string(),
        };

        // Make a minimal config with an enabled Bark profile
        let mut cfg = crate::config::AppConfig::default();
        cfg.api.enabled = true;
        cfg.forward.enabled.push("bark.primary".to_string());
        cfg.channels.bark.insert(
            "primary".to_string(),
            crate::config::BarkConfig {
                server_url: "https://api.day.app".to_string(),
                key: "test".to_string(),
            },
        );

        // Ensure fingerprint so dedup key is produced
        store
            .set_meta(crate::dbus::FINGERPRINT_META_KEY, "test-fingerprint")
            .unwrap();

        // First call: inserted
        process_inbound_sms(&store, &events, &sms, &cfg)
            .await
            .unwrap();
        assert_eq!(store.count_messages().unwrap(), 1);
        assert_eq!(store.count_deliveries().unwrap(), 1);
        assert_eq!(
            rx.try_recv().ok().map(|e| e.name()),
            Some("message.created")
        );

        // Second call: duplicate, no event, no new rows
        process_inbound_sms(&store, &events, &sms, &cfg)
            .await
            .unwrap();
        assert_eq!(store.count_messages().unwrap(), 1);
        assert_eq!(store.count_deliveries().unwrap(), 1);
        assert!(rx.try_recv().is_err(), "no more events after duplicate");
    }
}
