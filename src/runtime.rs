use std::path::Path;

use anyhow::Result;
use inquire::{Confirm, Text};

use crate::config::AppConfig;
use crate::dbus::{self, SendTarget};

pub async fn run_forwarding(config_path: &Path) -> Result<()> {
    let config = AppConfig::load(config_path)?;
    config.validate()?;
    let profiles = config.enabled_profiles()?;
    dbus::monitor_dbus(&config.app.modem_path, &profiles, &config).await
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
    dbus::send_sms(
        &connection,
        &config.app.modem_path,
        tel_number.trim(),
        sms_text.trim(),
        SendTarget::Command,
    )
    .await
}
