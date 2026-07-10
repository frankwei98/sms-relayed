use std::path::Path;
use std::sync::Arc;
use std::time::{Duration, Instant};

use anyhow::Result;
use inquire::{Confirm, Text};
use time::OffsetDateTime;

use crate::api::auth::SessionStore;
use crate::api::ApiState;
use crate::config::AppConfig;
use crate::dbus::{self, SendTarget};
use crate::delivery::run_delivery_worker;
use crate::events::{AppEvent, EventBus};
use crate::message::{Message, MessageDirection, MessageSource, MessageStatus};
use crate::modem::ModemService;
use crate::runner::{build_http_client, RealProcessRunner};
use crate::storage::{MessageStore, NewMessage};

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
            async move {
                let mut profile_keys: Vec<String> = cfg
                    .enabled_profiles()
                    .unwrap_or_default()
                    .iter()
                    .map(|p| p.key())
                    .collect();
                profile_keys.sort();
                profile_keys.dedup();
                let msg = store.insert_message_with_deliveries(
                    NewMessage {
                        direction: MessageDirection::Inbound,
                        phone_number: sms.phone_number,
                        body: sms.body,
                        timestamp: sms.timestamp,
                        status: MessageStatus::Received,
                        source: MessageSource::Modem,
                        modem_sms_path: Some(sms.modem_sms_path),
                        read_at: None,
                        error: None,
                    },
                    &profile_keys,
                )?;
                events.send(AppEvent::MessageCreated(msg));
                Ok(())
            }
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
            Ok(Err(e)) => log::error!("retention failed: {}", e),
            Err(e) => log::error!("retention task failed: {}", e),
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
