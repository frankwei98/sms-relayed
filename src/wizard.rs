use std::path::Path;

use anyhow::{bail, Result};
use inquire::{Confirm, MultiSelect, Password, Select, Text};

use crate::config::{
    AppConfig, BarkConfig, DingTalkConfig, PushPlusConfig, ShellConfig, TelegramConfig, WeComConfig,
};

const EXISTING_CONFIG_PROMPT: &str = "Existing config found";
const ENABLE_WEB_PROMPT: &str = "Enable Web API and frontend?";
const ENABLE_WEB_HELP: &str = "Serves the browser dashboard and HTTP API from this device.";
const WEB_PASSWORD_PROMPT: &str = "Web dashboard password";
const WEB_PASSWORD_HELP: &str =
    "Required when Web API is enabled. Stored in config.toml with restricted file permissions.";
const WEB_BIND_PROMPT: &str = "Web bind address";
const WEB_PORT_PROMPT: &str = "Web port";
const WEB_IPV6_PROMPT: &str = "Enable IPv6 listener?";
const WEB_DATABASE_PROMPT: &str = "SMS history database path";
const DEVICE_NAME_PROMPT: &str = "Device display name";
const DEVICE_NAME_HELP: &str = "Use *Host*Name* to show the current system hostname.";
const MODEM_PATH_PROMPT: &str = "ModemManager modem path";
const PUSH_CHANNELS_PROMPT: &str = "Push channels";
const IGNORE_STORAGE_PROMPT: &str = "Ignore SMS storage type";
const IGNORE_STORAGE_HELP: &str =
    "Common values: sm, me, mt, sr, bm, ta. Use all for no filtering.";
const ADD_STORAGE_PROMPT: &str = "Add another ignored storage type?";

pub fn run_setup_wizard(existing: Option<AppConfig>) -> Result<Option<AppConfig>> {
    let mut cfg = match existing {
        Some(existing_config) => {
            match Select::new(
                EXISTING_CONFIG_PROMPT,
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

    cfg.api.enabled = Confirm::new(ENABLE_WEB_PROMPT)
        .with_default(true)
        .with_help_message(ENABLE_WEB_HELP)
        .prompt()?;
    if cfg.api.enabled {
        cfg.api.password = Password::new(WEB_PASSWORD_PROMPT)
            .with_help_message(WEB_PASSWORD_HELP)
            .prompt()?;
        cfg.api.bind = Text::new(WEB_BIND_PROMPT)
            .with_default(&cfg.api.bind)
            .prompt()?;
        cfg.api.port = Text::new(WEB_PORT_PROMPT)
            .with_default(&cfg.api.port.to_string())
            .prompt()?
            .parse()?;
        cfg.api.enable_ipv6 = Confirm::new(WEB_IPV6_PROMPT)
            .with_default(cfg.api.enable_ipv6)
            .prompt()?;
        cfg.api.database_path = Text::new(WEB_DATABASE_PROMPT)
            .with_default(&cfg.api.database_path)
            .prompt()?;
    }

    cfg.app.device_name = Text::new(DEVICE_NAME_PROMPT)
        .with_default(&cfg.app.device_name)
        .with_help_message(DEVICE_NAME_HELP)
        .prompt()?;

    cfg.app.modem_path = Text::new(MODEM_PATH_PROMPT)
        .with_default(&cfg.app.modem_path)
        .prompt()?;

    let selected = MultiSelect::new(
        PUSH_CHANNELS_PROMPT,
        vec!["Bark", "Telegram", "PushPlus", "WeCom", "DingTalk", "Shell"],
    )
    .prompt()?;

    for item in selected {
        add_profiles_for_channel(&mut cfg, item)?;
    }

    cfg.sms.ignore_storage.clear();
    loop {
        let storage = Text::new(IGNORE_STORAGE_PROMPT)
            .with_default("sm")
            .with_help_message(IGNORE_STORAGE_HELP)
            .prompt()?;
        cfg.sms.ignore_storage.push(storage);
        if !Confirm::new(ADD_STORAGE_PROMPT)
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

#[cfg(test)]
fn setup_prompt_texts() -> &'static [&'static str] {
    &[
        EXISTING_CONFIG_PROMPT,
        ENABLE_WEB_PROMPT,
        ENABLE_WEB_HELP,
        WEB_PASSWORD_PROMPT,
        WEB_PASSWORD_HELP,
        WEB_BIND_PROMPT,
        WEB_PORT_PROMPT,
        WEB_IPV6_PROMPT,
        WEB_DATABASE_PROMPT,
        DEVICE_NAME_PROMPT,
        DEVICE_NAME_HELP,
        MODEM_PATH_PROMPT,
        PUSH_CHANNELS_PROMPT,
        IGNORE_STORAGE_PROMPT,
        IGNORE_STORAGE_HELP,
        ADD_STORAGE_PROMPT,
    ]
}

#[cfg(test)]
mod tests {
    use super::setup_prompt_texts;

    #[test]
    fn setup_prompt_texts_do_not_reference_planned_runtime_targets() {
        let all_text = setup_prompt_texts().join("\n");

        assert!(!all_text.contains("Runtime target"));
        assert!(!all_text.contains("planned for P2"));
        assert!(all_text.contains("Enable Web API and frontend?"));
        assert!(all_text.contains("Serves the browser dashboard and HTTP API"));
    }
}
