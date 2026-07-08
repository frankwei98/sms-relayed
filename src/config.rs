use std::collections::BTreeMap;
use std::fs;
use std::io::Write;
use std::path::Path;

use anyhow::{bail, Context, Result};
use serde::{Deserialize, Serialize};

// ---------------------------------------------------------------------------
// Typed TOML config (new P1 model)
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct AppConfig {
    pub app: AppSection,
    pub sms: SmsSection,
    pub forward: ForwardSection,
    #[serde(default)]
    pub channels: ChannelsSection,
    #[serde(default)]
    pub api: ApiSection,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct AppSection {
    pub device_name: String,
    pub modem_path: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct SmsSection {
    pub ignore_storage: Vec<String>,
    pub code_keywords: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default, PartialEq, Eq)]
pub struct ForwardSection {
    pub enabled: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ApiSection {
    #[serde(default = "default_api_enabled")]
    pub enabled: bool,
    #[serde(default = "default_api_bind")]
    pub bind: String,
    #[serde(default = "default_api_port")]
    pub port: u16,
    #[serde(default)]
    pub enable_ipv6: bool,
    #[serde(default)]
    pub password: String,
    #[serde(default = "default_database_path")]
    pub database_path: String,
}

fn default_api_enabled() -> bool {
    true
}

fn default_api_bind() -> String {
    "0.0.0.0".to_string()
}

fn default_api_port() -> u16 {
    8080
}

fn default_database_path() -> String {
    "/etc/sms-relayed/sms-relayed.sqlite".to_string()
}

impl Default for ApiSection {
    fn default() -> Self {
        Self {
            enabled: default_api_enabled(),
            bind: default_api_bind(),
            port: default_api_port(),
            enable_ipv6: false,
            password: String::new(),
            database_path: default_database_path(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, Default, PartialEq, Eq)]
pub struct ChannelsSection {
    #[serde(default)]
    pub bark: BTreeMap<String, BarkConfig>,
    #[serde(default)]
    pub telegram: BTreeMap<String, TelegramConfig>,
    #[serde(default)]
    pub pushplus: BTreeMap<String, PushPlusConfig>,
    #[serde(default)]
    pub wecom: BTreeMap<String, WeComConfig>,
    #[serde(default)]
    pub dingtalk: BTreeMap<String, DingTalkConfig>,
    #[serde(default)]
    pub shell: BTreeMap<String, ShellConfig>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default, PartialEq, Eq)]
pub struct BarkConfig {
    pub server_url: String,
    pub key: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct TelegramConfig {
    pub bot_token: String,
    pub chat_id: String,
    #[serde(default = "default_telegram_api_base")]
    pub api_base: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default, PartialEq, Eq)]
pub struct PushPlusConfig {
    pub token: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct WeComConfig {
    pub corp_id: String,
    pub agent_id: String,
    pub secret: String,
    #[serde(default = "default_wecom_to_user")]
    pub to_user: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default, PartialEq, Eq)]
pub struct DingTalkConfig {
    pub access_token: String,
    pub secret: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default, PartialEq, Eq)]
pub struct ShellConfig {
    pub path: String,
}

fn default_telegram_api_base() -> String {
    "https://api.telegram.org".to_string()
}

fn default_wecom_to_user() -> String {
    "@all".to_string()
}

impl Default for TelegramConfig {
    fn default() -> Self {
        Self {
            bot_token: String::new(),
            chat_id: String::new(),
            api_base: default_telegram_api_base(),
        }
    }
}

impl Default for WeComConfig {
    fn default() -> Self {
        Self {
            corp_id: String::new(),
            agent_id: String::new(),
            secret: String::new(),
            to_user: default_wecom_to_user(),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ChannelType {
    Bark,
    Telegram,
    PushPlus,
    WeCom,
    DingTalk,
    Shell,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProfileRef {
    pub channel_type: ChannelType,
    pub name: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ChannelProfile {
    Bark {
        name: String,
        config: BarkConfig,
    },
    Telegram {
        name: String,
        config: TelegramConfig,
    },
    PushPlus {
        name: String,
        config: PushPlusConfig,
    },
    WeCom {
        name: String,
        config: WeComConfig,
    },
    DingTalk {
        name: String,
        config: DingTalkConfig,
    },
    Shell {
        name: String,
        config: ShellConfig,
    },
}

impl Default for AppConfig {
    fn default() -> Self {
        Self {
            app: AppSection {
                device_name: "*Host*Name*".to_string(),
                modem_path: "/org/freedesktop/ModemManager1/Modem/0".to_string(),
            },
            sms: SmsSection {
                ignore_storage: vec!["sm".to_string()],
                code_keywords: vec![
                    "验证码".to_string(),
                    "verification".to_string(),
                    "code".to_string(),
                    "인증".to_string(),
                    "代码".to_string(),
                    "随机码".to_string(),
                ],
            },
            forward: ForwardSection::default(),
            channels: ChannelsSection::default(),
            api: ApiSection::default(),
        }
    }
}

impl ProfileRef {
    pub fn parse(input: &str) -> Result<Self> {
        let (channel, name) = input
            .split_once('.')
            .ok_or_else(|| anyhow::anyhow!("profile reference must be type.name: {}", input))?;
        let channel_type = match channel {
            "bark" => ChannelType::Bark,
            "telegram" => ChannelType::Telegram,
            "pushplus" => ChannelType::PushPlus,
            "wecom" => ChannelType::WeCom,
            "dingtalk" => ChannelType::DingTalk,
            "shell" => ChannelType::Shell,
            other => bail!("unknown channel type: {}", other),
        };
        if name.trim().is_empty() {
            bail!("profile name is required: {}", input);
        }
        Ok(Self {
            channel_type,
            name: name.to_string(),
        })
    }
}

impl ChannelProfile {
    pub fn redacted_line(&self) -> String {
        match self {
            ChannelProfile::Bark { name, config } => {
                format!("bark.{} key={}", name, redact(&config.key))
            }
            ChannelProfile::Telegram { name, config } => {
                format!(
                    "telegram.{} bot_token={} chat_id={}",
                    name,
                    redact(&config.bot_token),
                    config.chat_id
                )
            }
            ChannelProfile::PushPlus { name, config } => {
                format!("pushplus.{} token={}", name, redact(&config.token))
            }
            ChannelProfile::WeCom { name, config } => {
                format!(
                    "wecom.{} corp_id={} secret={}",
                    name,
                    config.corp_id,
                    redact(&config.secret)
                )
            }
            ChannelProfile::DingTalk { name, config } => {
                format!(
                    "dingtalk.{} access_token={} secret={}",
                    name,
                    redact(&config.access_token),
                    redact(&config.secret)
                )
            }
            ChannelProfile::Shell { name, config } => {
                format!("shell.{} path={}", name, config.path)
            }
        }
    }
}

fn redact(secret: &str) -> String {
    if secret.chars().count() <= 8 {
        "****".to_string()
    } else {
        let prefix: String = secret.chars().take(4).collect();
        let suffix: String = secret
            .chars()
            .rev()
            .take(4)
            .collect::<String>()
            .chars()
            .rev()
            .collect();
        format!("{}...{}", prefix, suffix)
    }
}

impl AppConfig {
    pub fn load(path: &Path) -> Result<Self> {
        let content = fs::read_to_string(path)
            .with_context(|| format!("failed to read config {}", path.display()))?;
        toml::from_str(&content)
            .with_context(|| format!("failed to parse config {}", path.display()))
    }

    pub fn save_secure(&self, path: &Path) -> Result<()> {
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent)
                .with_context(|| format!("failed to create {}", parent.display()))?;
            #[cfg(unix)]
            {
                use std::os::unix::fs::PermissionsExt;
                fs::set_permissions(parent, fs::Permissions::from_mode(0o700))?;
            }
        }
        let content = toml::to_string_pretty(self)?;
        let mut file = fs::File::create(path)
            .with_context(|| format!("failed to write config {}", path.display()))?;
        file.write_all(content.as_bytes())?;
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            fs::set_permissions(path, fs::Permissions::from_mode(0o600))?;
        }
        Ok(())
    }

    pub fn validate(&self) -> Result<()> {
        if !self
            .app
            .modem_path
            .starts_with("/org/freedesktop/ModemManager1/Modem/")
        {
            bail!("app.modem_path must be a ModemManager modem object path");
        }
        for reference in &self.forward.enabled {
            let parsed = ProfileRef::parse(reference)?;
            self.profile_for_ref(&parsed)?;
        }
        if self.api.enabled {
            if self.api.password.trim().is_empty() {
                bail!("api.password is required when api.enabled is true");
            }
            if self.api.bind.trim().is_empty() {
                bail!("api.bind is required when api.enabled is true");
            }
            if self.api.port == 0 {
                bail!("api.port must be between 1 and 65535");
            }
            if self.api.database_path.trim().is_empty() {
                bail!("api.database_path is required when api.enabled is true");
            }
        }
        Ok(())
    }

    pub fn enabled_profiles(&self) -> Result<Vec<ChannelProfile>> {
        self.forward
            .enabled
            .iter()
            .map(|reference| ProfileRef::parse(reference).and_then(|r| self.profile_for_ref(&r)))
            .collect()
    }

    pub fn redacted_summary(&self) -> String {
        let mut out = format!(
            "device_name: {}\nmodem_path: {}\n",
            self.app.device_name, self.app.modem_path
        );
        for profile in self.enabled_profiles().unwrap_or_default() {
            out.push_str(&format!("{}\n", profile.redacted_line()));
        }
        out
    }

    fn profile_for_ref(&self, reference: &ProfileRef) -> Result<ChannelProfile> {
        match reference.channel_type {
            ChannelType::Bark => {
                let cfg = self.channels.bark.get(&reference.name).ok_or_else(|| {
                    anyhow::anyhow!("enabled profile bark.{} does not exist", reference.name)
                })?;
                require(
                    "channels.bark",
                    &reference.name,
                    "server_url",
                    &cfg.server_url,
                )?;
                require("channels.bark", &reference.name, "key", &cfg.key)?;
                Ok(ChannelProfile::Bark {
                    name: reference.name.clone(),
                    config: cfg.clone(),
                })
            }
            ChannelType::Telegram => {
                let cfg = self.channels.telegram.get(&reference.name).ok_or_else(|| {
                    anyhow::anyhow!("enabled profile telegram.{} does not exist", reference.name)
                })?;
                require(
                    "channels.telegram",
                    &reference.name,
                    "bot_token",
                    &cfg.bot_token,
                )?;
                require(
                    "channels.telegram",
                    &reference.name,
                    "chat_id",
                    &cfg.chat_id,
                )?;
                Ok(ChannelProfile::Telegram {
                    name: reference.name.clone(),
                    config: cfg.clone(),
                })
            }
            ChannelType::PushPlus => {
                let cfg = self.channels.pushplus.get(&reference.name).ok_or_else(|| {
                    anyhow::anyhow!("enabled profile pushplus.{} does not exist", reference.name)
                })?;
                require("channels.pushplus", &reference.name, "token", &cfg.token)?;
                Ok(ChannelProfile::PushPlus {
                    name: reference.name.clone(),
                    config: cfg.clone(),
                })
            }
            ChannelType::WeCom => {
                let cfg = self.channels.wecom.get(&reference.name).ok_or_else(|| {
                    anyhow::anyhow!("enabled profile wecom.{} does not exist", reference.name)
                })?;
                require("channels.wecom", &reference.name, "corp_id", &cfg.corp_id)?;
                require("channels.wecom", &reference.name, "agent_id", &cfg.agent_id)?;
                require("channels.wecom", &reference.name, "secret", &cfg.secret)?;
                Ok(ChannelProfile::WeCom {
                    name: reference.name.clone(),
                    config: cfg.clone(),
                })
            }
            ChannelType::DingTalk => {
                let cfg = self.channels.dingtalk.get(&reference.name).ok_or_else(|| {
                    anyhow::anyhow!("enabled profile dingtalk.{} does not exist", reference.name)
                })?;
                require(
                    "channels.dingtalk",
                    &reference.name,
                    "access_token",
                    &cfg.access_token,
                )?;
                require("channels.dingtalk", &reference.name, "secret", &cfg.secret)?;
                Ok(ChannelProfile::DingTalk {
                    name: reference.name.clone(),
                    config: cfg.clone(),
                })
            }
            ChannelType::Shell => {
                let cfg = self.channels.shell.get(&reference.name).ok_or_else(|| {
                    anyhow::anyhow!("enabled profile shell.{} does not exist", reference.name)
                })?;
                require("channels.shell", &reference.name, "path", &cfg.path)?;
                Ok(ChannelProfile::Shell {
                    name: reference.name.clone(),
                    config: cfg.clone(),
                })
            }
        }
    }
}

fn require(section: &str, name: &str, field: &str, value: &str) -> Result<()> {
    if value.trim().is_empty() {
        bail!("{}.{}.{} is required", section, name, field);
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_app_config_has_expected_modem_and_sms_defaults() {
        let cfg = AppConfig::default();
        assert_eq!(cfg.app.modem_path, "/org/freedesktop/ModemManager1/Modem/0");
        assert_eq!(cfg.sms.ignore_storage, vec!["sm"]);
        assert!(cfg.sms.code_keywords.contains(&"验证码".to_string()));
    }

    #[test]
    fn parses_profile_refs() {
        let r = ProfileRef::parse("bark.personal").unwrap();
        assert_eq!(r.channel_type, ChannelType::Bark);
        assert_eq!(r.name, "personal");
    }

    #[test]
    fn validates_enabled_profile_exists() {
        let mut cfg = AppConfig::default();
        cfg.forward.enabled = vec!["bark.personal".to_string()];
        let err = cfg.validate().unwrap_err().to_string();
        assert!(err.contains("bark.personal"));
    }

    #[test]
    fn validates_required_bark_fields() {
        let mut cfg = AppConfig::default();
        cfg.forward.enabled = vec!["bark.personal".to_string()];
        cfg.channels.bark.insert(
            "personal".to_string(),
            BarkConfig {
                server_url: "https://api.day.app".to_string(),
                key: String::new(),
            },
        );
        let err = cfg.validate().unwrap_err().to_string();
        assert!(err.contains("channels.bark.personal.key"));
    }

    #[test]
    fn validates_multiple_profiles_of_same_type() {
        let mut cfg = AppConfig::default();
        cfg.forward.enabled = vec!["bark.personal".to_string(), "bark.ops".to_string()];
        cfg.channels.bark.insert(
            "personal".to_string(),
            BarkConfig {
                server_url: "https://api.day.app".to_string(),
                key: "personal-key".to_string(),
            },
        );
        cfg.channels.bark.insert(
            "ops".to_string(),
            BarkConfig {
                server_url: "https://api.day.app".to_string(),
                key: "ops-key".to_string(),
            },
        );
        assert_eq!(cfg.enabled_profiles().unwrap().len(), 2);
    }

    #[test]
    fn redacts_secret_values() {
        let mut cfg = AppConfig::default();
        cfg.channels.telegram.insert(
            "main".to_string(),
            TelegramConfig {
                bot_token: "1234567890abcdef".to_string(),
                chat_id: "42".to_string(),
                api_base: "https://api.telegram.org".to_string(),
            },
        );
        cfg.forward.enabled = vec!["telegram.main".to_string()];
        let summary = cfg.redacted_summary();
        assert!(summary.contains("1234...cdef"));
        assert!(!summary.contains("1234567890abcdef"));
    }

    #[test]
    fn default_api_config_matches_p2_defaults() {
        let cfg = AppConfig::default();
        assert!(cfg.api.enabled);
        assert_eq!(cfg.api.bind, "0.0.0.0");
        assert_eq!(cfg.api.port, 8080);
        assert!(!cfg.api.enable_ipv6);
        assert_eq!(cfg.api.database_path, "/etc/sms-relayed/sms-relayed.sqlite");
    }

    #[test]
    fn enabled_api_requires_password() {
        let cfg = AppConfig::default();
        let err = cfg.validate().unwrap_err().to_string();
        assert!(err.contains("api.password"));
    }

    #[test]
    fn api_config_accepts_password() {
        let mut cfg = AppConfig::default();
        cfg.api.password = "secret".to_string();
        assert!(cfg.validate().is_ok());
    }

    #[test]
    fn disabled_api_does_not_require_password() {
        let mut cfg = AppConfig::default();
        cfg.api.enabled = false;
        cfg.api.password.clear();
        assert!(cfg.validate().is_ok());
    }
}
