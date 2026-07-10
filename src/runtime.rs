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

    let store_inbound = store.clone();
    let events_inbound = events.clone();
    let client_inbound = client.clone();
    let modem_inbound = modem_service.clone();

    let dbus_future = dbus::monitor_dbus_with_handler(
        &config.app.modem_path,
        &config,
        move |sms| {
            let store = store_inbound.clone();
            let events = events_inbound.clone();
            async move {
                let msg = store.insert_message(NewMessage {
                    direction: MessageDirection::Inbound,
                    phone_number: sms.phone_number,
                    body: sms.body,
                    timestamp: sms.timestamp,
                    status: MessageStatus::Received,
                    source: MessageSource::Modem,
                    modem_sms_path: Some(sms.modem_sms_path),
                    read_at: None,
                    error: None,
                })?;
                events.send(AppEvent::MessageCreated(msg));
                Ok(())
            }
        },
        &client_inbound,
        &shell_runner,
        shell_timeout,
        &modem_inbound,
        &store,
    );

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
            result = crate::api::serve(api_state) => result,
            result = dbus_future => result,
        }
    } else {
        dbus_future.await
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
