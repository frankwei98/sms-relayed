use std::path::Path;

use anyhow::{bail, Result};
use inquire::{Confirm, MultiSelect, Password, Select, Text};

use crate::config::{
    AppConfig, BarkConfig, DingTalkConfig, PushPlusConfig, ShellConfig, TelegramConfig, WeComConfig,
};

pub fn run_setup_wizard(existing: Option<AppConfig>) -> Result<Option<AppConfig>> {
    let mut cfg = match existing {
        Some(existing_config) => {
            match Select::new(
                "Existing config found",
                vec![
                    "Keep existing",
                    "Edit guided",
                    "Replace from scratch",
                    "Cancel",
                ],
            )
            .prompt()?
            {
                "Keep existing" => return Ok(None),
                "Edit guided" => existing_config,
                "Replace from scratch" => AppConfig::default(),
                "Cancel" => bail!("setup cancelled"),
                _ => unreachable!("fixed Select options"),
            }
        }
        None => AppConfig::default(),
    };

    Select::new("Runtime target", vec!["SMS forwarding service"])
        .with_help_message("API and frontend are planned for P2.")
        .prompt()?;

    cfg.api.enabled = Confirm::new("Enable Web API and frontend?")
        .with_default(true)
        .with_help_message("Provides a browser console for SMS history and config.")
        .prompt()?;
    if cfg.api.enabled {
        cfg.api.password = Password::new("Web password")
            .with_help_message("Stored in config.toml; file permissions are restricted.")
            .prompt()?;
        cfg.api.bind = Text::new("Web bind address")
            .with_default(&cfg.api.bind)
            .prompt()?;
        cfg.api.port = Text::new("Web port")
            .with_default(&cfg.api.port.to_string())
            .prompt()?
            .parse()?;
        cfg.api.enable_ipv6 = Confirm::new("Enable IPv6 listener?")
            .with_default(cfg.api.enable_ipv6)
            .prompt()?;
        cfg.api.database_path = Text::new("SQLite database path")
            .with_default(&cfg.api.database_path)
            .prompt()?;
    }

    cfg.app.device_name = Text::new("Device display name")
        .with_default(&cfg.app.device_name)
        .with_help_message("Use *Host*Name* to read hostname dynamically.")
        .prompt()?;

    cfg.app.modem_path = Text::new("ModemManager modem path")
        .with_default(&cfg.app.modem_path)
        .prompt()?;

    let selected = MultiSelect::new(
        "Push channels",
        vec!["Bark", "Telegram", "PushPlus", "WeCom", "DingTalk", "Shell"],
    )
    .prompt()?;

    for item in selected {
        add_profiles_for_channel(&mut cfg, item)?;
    }

    cfg.sms.ignore_storage.clear();
    loop {
        let storage = Text::new("Ignore SMS storage type")
            .with_default("sm")
            .with_help_message("Common values: sm, me, mt, sr, bm, ta. Use all for no filtering.")
            .prompt()?;
        cfg.sms.ignore_storage.push(storage);
        if !Confirm::new("Add another ignored storage type?")
            .with_default(false)
            .prompt()?
        {
            break;
        }
    }

    Ok(Some(cfg))
}

pub fn write_setup_config(config: &AppConfig, path: &Path) -> Result<()> {
    config.validate()?;
    config.save_secure(path)
}

fn add_profiles_for_channel(cfg: &mut AppConfig, label: &str) -> Result<()> {
    loop {
        let default_name = if profile_count(cfg, label) == 0 {
            "default"
        } else {
            "extra"
        };
        let name = Text::new(&format!("{} profile name", label))
            .with_default(default_name)
            .prompt()?;
        match label {
            "Bark" => {
                let server_url = Text::new("Bark server URL")
                    .with_default("https://api.day.app")
                    .prompt()?;
                let key = Password::new("Bark key").without_confirmation().prompt()?;
                cfg.channels
                    .bark
                    .insert(name.clone(), BarkConfig { server_url, key });
                cfg.forward.enabled.push(format!("bark.{}", name));
            }
            "Telegram" => {
                let bot_token = Password::new("Telegram bot token")
                    .without_confirmation()
                    .prompt()?;
                let chat_id = Text::new("Telegram chat id").prompt()?;
                let api_base = Text::new("Telegram API base")
                    .with_default("https://api.telegram.org")
                    .prompt()?;
                cfg.channels.telegram.insert(
                    name.clone(),
                    TelegramConfig {
                        bot_token,
                        chat_id,
                        api_base,
                    },
                );
                cfg.forward.enabled.push(format!("telegram.{}", name));
            }
            "PushPlus" => {
                let token = Password::new("PushPlus token")
                    .without_confirmation()
                    .prompt()?;
                cfg.channels
                    .pushplus
                    .insert(name.clone(), PushPlusConfig { token });
                cfg.forward.enabled.push(format!("pushplus.{}", name));
            }
            "WeCom" => {
                let corp_id = Text::new("WeCom corp id").prompt()?;
                let agent_id = Text::new("WeCom agent id").prompt()?;
                let secret = Password::new("WeCom app secret")
                    .without_confirmation()
                    .prompt()?;
                cfg.channels.wecom.insert(
                    name.clone(),
                    WeComConfig {
                        corp_id,
                        agent_id,
                        secret,
                        to_user: "@all".to_string(),
                    },
                );
                cfg.forward.enabled.push(format!("wecom.{}", name));
            }
            "DingTalk" => {
                let access_token = Password::new("DingTalk access token")
                    .without_confirmation()
                    .prompt()?;
                let secret = Password::new("DingTalk signing secret")
                    .without_confirmation()
                    .prompt()?;
                cfg.channels.dingtalk.insert(
                    name.clone(),
                    DingTalkConfig {
                        access_token,
                        secret,
                    },
                );
                cfg.forward.enabled.push(format!("dingtalk.{}", name));
            }
            "Shell" => {
                let path = Text::new("Shell script path").prompt()?;
                cfg.channels
                    .shell
                    .insert(name.clone(), ShellConfig { path });
                cfg.forward.enabled.push(format!("shell.{}", name));
            }
            other => anyhow::bail!("unknown wizard channel: {}", other),
        }
        if !Confirm::new(&format!("Add another {} profile?", label))
            .with_default(false)
            .prompt()?
        {
            return Ok(());
        }
    }
}

fn profile_count(cfg: &AppConfig, label: &str) -> usize {
    match label {
        "Bark" => cfg.channels.bark.len(),
        "Telegram" => cfg.channels.telegram.len(),
        "PushPlus" => cfg.channels.pushplus.len(),
        "WeCom" => cfg.channels.wecom.len(),
        "DingTalk" => cfg.channels.dingtalk.len(),
        "Shell" => cfg.channels.shell.len(),
        _ => 0,
    }
}
