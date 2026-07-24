use std::path::Path;
use std::sync::Arc;
use std::time::{Duration, Instant};

use anyhow::Result;
use inquire::{Confirm, Text};
use time::OffsetDateTime;

use crate::api::auth::SessionStore;
use crate::api::ApiState;
use crate::config::AppConfig;
use crate::dbus::{self, ReceivedSms};
use crate::delivery::{run_delivery_worker, DeliveryWakeup};
use crate::events::EventBus;
use crate::message::MessageSource;
use crate::messaging::{Messaging, ReceiveMessage, SendMessage};
use crate::modem::ModemService;
use crate::persistence::Store;
use crate::runner::{build_http_client, RealProcessRunner};

pub async fn run_forwarding(config_path: &Path) -> Result<()> {
    let config = AppConfig::load(config_path)?;
    config.validate()?;

    let store = Store::open(Path::new(&config.api.database_path))?;
    let events = EventBus::new();
    let delivery_wakeup = DeliveryWakeup::new();
    let sms_sender = Arc::new(dbus::SystemSmsSender::new());
    let messaging = Messaging::new(
        store.clone(),
        events.clone(),
        delivery_wakeup.clone(),
        sms_sender.clone(),
    );

    let client = Arc::new(build_http_client(&config.http));
    let shell_timeout = Duration::from_secs(config.http.shell_timeout_secs);
    let shell_runner = RealProcessRunner;
    let modem_service = ModemService::new();

    // Recover expired delivery leases
    let lease_timeout =
        OffsetDateTime::now_utc().format(&time::format_description::well_known::Rfc3339)?;
    let recovered = store.recover_expired_leases(lease_timeout).await?;
    if recovered > 0 {
        log::info!("recovered {} expired delivery leases", recovered);
    }

    let messaging_inbound = messaging.clone();
    let config_inbound = config.clone();

    let dbus_modem = modem_service.clone();
    let dbus_future = dbus::monitor_dbus_with_handler(
        &config.app.modem_path,
        &config,
        move |sms| {
            let messaging = messaging_inbound.clone();
            let cfg = config_inbound.clone();
            async move { process_inbound_sms(&messaging, sms, &cfg).await }
        },
        &dbus_modem,
        &store,
    );

    let delivery_worker = run_delivery_worker(
        store.delivery_store(),
        config.clone(),
        client.clone(),
        Arc::new(shell_runner),
        shell_timeout,
        delivery_wakeup.clone(),
    );
    let retention_worker = run_retention_worker(store.clone(), config.clone());

    if config.api.enabled {
        let api_state = ApiState {
            config: Arc::new(config.clone()),
            config_path: config_path.to_path_buf(),
            store: store.clone(),
            events: events.clone(),
            delivery_wakeup: delivery_wakeup.clone(),
            started_at: Instant::now(),
            sessions: SessionStore::default(),
            modem: modem_service,
            sms_sender: sms_sender.clone(),
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
async fn process_inbound_sms(
    messaging: &Messaging,
    sms: ReceivedSms,
    cfg: &AppConfig,
) -> Result<()> {
    let profile_keys: Vec<String> = cfg
        .enabled_profiles()
        .unwrap_or_default()
        .iter()
        .map(|p| p.key())
        .collect();
    messaging
        .receive(ReceiveMessage { sms, profile_keys })
        .await?;
    Ok(())
}

async fn run_retention_worker(store: Store, config: AppConfig) {
    const RETENTION_INTERVAL: Duration = Duration::from_secs(6 * 60 * 60);
    if !config.retention.enabled {
        std::future::pending::<()>().await;
        return;
    }

    loop {
        let max_age_days = config.retention.max_age_days;
        let batch_size = config.retention.batch_size;
        match store.run_retention(max_age_days, batch_size).await {
            Ok(deleted) if deleted > 0 => {
                log::info!("retention deleted {} messages", deleted);
            }
            Ok(_) => {}
            Err(e) => {
                log::error!("retention failed: {}", e);
                crate::monitoring::capture_failure("retention", "retention.cleanup_failed");
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
    let messaging = Messaging::new(
        Store::open(Path::new(&config.api.database_path))?,
        EventBus::new(),
        DeliveryWakeup::new(),
        Arc::new(dbus::SystemSmsSender::new()),
    );
    messaging
        .send(SendMessage {
            modem_path: config.app.modem_path,
            phone_number: tel_number.trim().to_string(),
            body: sms_text.trim().to_string(),
            source: MessageSource::Cli,
        })
        .await?;
    Ok(())
}
