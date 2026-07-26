use std::collections::BTreeMap;
use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};

use anyhow::{bail, Context, Result};
use base64::{engine::general_purpose::URL_SAFE_NO_PAD, Engine as _};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

// ---------------------------------------------------------------------------
// Typed TOML config (new P1 model)
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct AppConfig {
    pub app: AppSection,
    pub sms: SmsSection,
    pub forward: ForwardSection,
    #[serde(default)]
    pub delivery: DeliverySection,
    #[serde(default)]
    pub channels: ChannelsSection,
    #[serde(default)]
    pub api: ApiSection,
    #[serde(default)]
    pub http: HttpSection,
    #[serde(default)]
    pub retention: RetentionSection,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct RetentionSection {
    #[serde(default)]
    pub enabled: bool,
    #[serde(default = "default_retention_max_age_days")]
    pub max_age_days: u64,
    #[serde(default = "default_retention_batch_size")]
    pub batch_size: u32,
}

fn default_retention_max_age_days() -> u64 {
    90
}

fn default_retention_batch_size() -> u32 {
    500
}

impl Default for RetentionSection {
    fn default() -> Self {
        Self {
            enabled: false,
            max_age_days: default_retention_max_age_days(),
            batch_size: default_retention_batch_size(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct HttpSection {
    #[serde(default = "default_http_connect_timeout")]
    pub connect_timeout_secs: u64,
    #[serde(default = "default_http_request_timeout")]
    pub request_timeout_secs: u64,
    #[serde(default = "default_shell_timeout")]
    pub shell_timeout_secs: u64,
}

fn default_http_connect_timeout() -> u64 {
    10
}

fn default_http_request_timeout() -> u64 {
    30
}

fn default_shell_timeout() -> u64 {
    30
}

impl Default for HttpSection {
    fn default() -> Self {
        Self {
            connect_timeout_secs: default_http_connect_timeout(),
            request_timeout_secs: default_http_request_timeout(),
            shell_timeout_secs: default_shell_timeout(),
        }
    }
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
pub struct DeliverySection {
    #[serde(default = "default_delivery_concurrency")]
    pub concurrency: usize,
}

fn default_delivery_concurrency() -> usize {
    2
}

impl Default for DeliverySection {
    fn default() -> Self {
        Self {
            concurrency: default_delivery_concurrency(),
        }
    }
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
    false
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
    pub wecom: BTreeMap<String, WeComConfig>,
    #[serde(default)]
    pub dingtalk: BTreeMap<String, DingTalkConfig>,
    #[serde(default)]
    pub lark: BTreeMap<String, LarkConfig>,
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
pub struct LarkConfig {
    pub webhook_url: String,
    #[serde(default)]
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
    WeCom,
    DingTalk,
    Lark,
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
    WeCom {
        name: String,
        config: WeComConfig,
    },
    DingTalk {
        name: String,
        config: DingTalkConfig,
    },
    Lark {
        name: String,
        config: LarkConfig,
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
            delivery: DeliverySection::default(),
            channels: ChannelsSection::default(),
            api: ApiSection::default(),
            http: HttpSection::default(),
            retention: RetentionSection::default(),
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
            "wecom" => ChannelType::WeCom,
            "dingtalk" => ChannelType::DingTalk,
            "lark" => ChannelType::Lark,
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
    pub fn key(&self) -> String {
        match self {
            ChannelProfile::Bark { name, .. } => format!("bark.{}", name),
            ChannelProfile::Telegram { name, .. } => format!("telegram.{}", name),
            ChannelProfile::WeCom { name, .. } => format!("wecom.{}", name),
            ChannelProfile::DingTalk { name, .. } => format!("dingtalk.{}", name),
            ChannelProfile::Lark { name, .. } => format!("lark.{}", name),
            ChannelProfile::Shell { name, .. } => format!("shell.{}", name),
        }
    }

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
            ChannelProfile::Lark { name, config } => {
                format!(
                    "lark.{} webhook_url={} secret={}",
                    name,
                    redact(&config.webhook_url),
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

pub fn config_revision(content: &str) -> String {
    URL_SAFE_NO_PAD.encode(Sha256::digest(content.as_bytes()))
}

pub(crate) struct PreparedConfigWrite {
    temporary: Option<PathBuf>,
    destination: PathBuf,
    parent: PathBuf,
}

impl PreparedConfigWrite {
    pub(crate) fn commit(mut self) -> Result<()> {
        #[cfg(test)]
        if should_fail_config_commit(&self.destination) {
            bail!("injected config commit failure");
        }
        let temporary = self
            .temporary
            .as_ref()
            .expect("prepared config write has a temporary file");
        fs::rename(temporary, &self.destination).with_context(|| {
            format!(
                "failed to replace config {}",
                self.destination.as_path().display()
            )
        })?;
        self.temporary = None;
        if let Err(error) = sync_config_parent(&self.destination, &self.parent) {
            log::warn!(
                "config {} was replaced but failed to sync parent directory {}: {}",
                self.destination.display(),
                self.parent.display(),
                error
            );
        }
        Ok(())
    }
}

impl Drop for PreparedConfigWrite {
    fn drop(&mut self) {
        if let Some(temporary) = self.temporary.take() {
            let _ = fs::remove_file(temporary);
        }
    }
}

#[cfg(test)]
static PREPARE_CONFIG_WRITE_FAILURE_PATH: std::sync::Mutex<Option<PathBuf>> =
    std::sync::Mutex::new(None);

#[cfg(test)]
static CONFIG_COMMIT_FAILURE_PATH: std::sync::Mutex<Option<PathBuf>> = std::sync::Mutex::new(None);

#[cfg(test)]
static CONFIG_PARENT_SYNC_FAILURE_PATH: std::sync::Mutex<Option<PathBuf>> =
    std::sync::Mutex::new(None);

#[cfg(test)]
pub(crate) fn fail_next_prepare_config_write_for(path: &Path) {
    *PREPARE_CONFIG_WRITE_FAILURE_PATH.lock().unwrap() = Some(path.to_path_buf());
}

#[cfg(test)]
pub(crate) fn fail_next_config_commit_for(path: &Path) {
    *CONFIG_COMMIT_FAILURE_PATH.lock().unwrap() = Some(path.to_path_buf());
}

#[cfg(test)]
pub(crate) fn fail_next_config_parent_sync_for(path: &Path) {
    *CONFIG_PARENT_SYNC_FAILURE_PATH.lock().unwrap() = Some(path.to_path_buf());
}

#[cfg(test)]
fn should_fail_prepare_config_write(path: &Path) -> bool {
    let mut failure_path = PREPARE_CONFIG_WRITE_FAILURE_PATH.lock().unwrap();
    if failure_path.as_deref() == Some(path) {
        failure_path.take();
        true
    } else {
        false
    }
}

#[cfg(test)]
fn should_fail_config_commit(path: &Path) -> bool {
    let mut failure_path = CONFIG_COMMIT_FAILURE_PATH.lock().unwrap();
    if failure_path.as_deref() == Some(path) {
        failure_path.take();
        true
    } else {
        false
    }
}

fn sync_config_parent(_destination: &Path, parent: &Path) -> Result<()> {
    #[cfg(test)]
    {
        let mut failure_path = CONFIG_PARENT_SYNC_FAILURE_PATH.lock().unwrap();
        if failure_path.as_deref() == Some(_destination) {
            failure_path.take();
            bail!("injected config parent sync failure");
        }
    }
    #[cfg(unix)]
    fs::File::open(parent)?.sync_all()?;
    Ok(())
}

impl AppConfig {
    pub fn load(path: &Path) -> Result<Self> {
        let content = fs::read_to_string(path)
            .with_context(|| format!("failed to read config {}", path.display()))?;
        toml::from_str(&content)
            .with_context(|| format!("failed to parse config {}", path.display()))
    }

    pub fn canonical_toml(&self) -> Result<String> {
        toml::to_string_pretty(self).context("failed to serialize config")
    }

    pub fn save_secure(&self, path: &Path) -> Result<()> {
        self.prepare_secure_write(path)?.commit()
    }

    pub(crate) fn prepare_secure_write(&self, path: &Path) -> Result<PreparedConfigWrite> {
        let parent = path
            .parent()
            .filter(|parent| !parent.as_os_str().is_empty())
            .unwrap_or_else(|| Path::new("."));
        #[cfg(unix)]
        {
            use std::os::unix::fs::DirBuilderExt;

            let mut builder = fs::DirBuilder::new();
            builder.recursive(true).mode(0o700);
            builder
                .create(parent)
                .with_context(|| format!("failed to create {}", parent.display()))?;
        }
        #[cfg(not(unix))]
        fs::create_dir_all(parent)
            .with_context(|| format!("failed to create {}", parent.display()))?;
        #[cfg(unix)]
        secure_config_parent(parent)?;

        let content = self.canonical_toml()?;
        let file_name = path
            .file_name()
            .and_then(|name| name.to_str())
            .unwrap_or("config.toml");
        let temporary = parent.join(format!(".{file_name}.{}.tmp", uuid::Uuid::new_v4()));

        #[cfg(test)]
        if should_fail_prepare_config_write(path) {
            bail!("injected config prepare failure");
        }

        let result = (|| -> Result<()> {
            let mut options = fs::OpenOptions::new();
            options.write(true).create_new(true);
            #[cfg(unix)]
            {
                use std::os::unix::fs::OpenOptionsExt;
                options.mode(0o600);
            }
            let mut file = options
                .open(&temporary)
                .with_context(|| format!("failed to write config {}", path.display()))?;
            file.write_all(content.as_bytes())?;
            file.sync_all()?;
            Ok(())
        })();

        if let Err(error) = result {
            let _ = fs::remove_file(&temporary);
            return Err(error);
        }
        Ok(PreparedConfigWrite {
            temporary: Some(temporary),
            destination: path.to_path_buf(),
            parent: parent.to_path_buf(),
        })
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
        if !(1..=16).contains(&self.delivery.concurrency) {
            bail!("delivery.concurrency must be between 1 and 16");
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
        if self.http.connect_timeout_secs == 0
            || self.http.request_timeout_secs == 0
            || self.http.shell_timeout_secs == 0
        {
            bail!("http and shell timeouts must be greater than zero");
        }
        if self.http.connect_timeout_secs > self.http.request_timeout_secs {
            bail!("http.connect_timeout_secs must not exceed request_timeout_secs");
        }
        if self.retention.enabled
            && (self.retention.max_age_days == 0 || self.retention.batch_size == 0)
        {
            bail!("enabled retention requires positive max_age_days and batch_size");
        }
        Ok(())
    }

    pub fn configured_profile_keys(&self) -> Vec<String> {
        let mut keys = Vec::new();
        keys.extend(self.channels.bark.keys().map(|name| format!("bark.{name}")));
        keys.extend(
            self.channels
                .telegram
                .keys()
                .map(|name| format!("telegram.{name}")),
        );
        keys.extend(
            self.channels
                .wecom
                .keys()
                .map(|name| format!("wecom.{name}")),
        );
        keys.extend(
            self.channels
                .dingtalk
                .keys()
                .map(|name| format!("dingtalk.{name}")),
        );
        keys.extend(self.channels.lark.keys().map(|name| format!("lark.{name}")));
        keys.extend(
            self.channels
                .shell
                .keys()
                .map(|name| format!("shell.{name}")),
        );
        keys
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
            ChannelType::Lark => {
                let cfg = self.channels.lark.get(&reference.name).ok_or_else(|| {
                    anyhow::anyhow!("enabled profile lark.{} does not exist", reference.name)
                })?;
                require(
                    "channels.lark",
                    &reference.name,
                    "webhook_url",
                    &cfg.webhook_url,
                )?;
                Ok(ChannelProfile::Lark {
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

#[cfg(unix)]
fn secure_config_parent(parent: &Path) -> Result<()> {
    use std::os::unix::fs::PermissionsExt;

    if !should_secure_config_parent(parent) {
        return Ok(());
    }
    let canonical = fs::canonicalize(parent)
        .with_context(|| format!("failed to resolve config directory {}", parent.display()))?;
    if !should_secure_config_parent(&canonical) {
        return Ok(());
    }
    fs::set_permissions(&canonical, fs::Permissions::from_mode(0o700))
        .with_context(|| format!("failed to secure config directory {}", canonical.display()))
}

#[cfg(unix)]
fn should_secure_config_parent(parent: &Path) -> bool {
    Path::new(crate::cli::DEFAULT_CONFIG_PATH)
        .parent()
        .is_some_and(|default_parent| parent == default_parent)
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

    #[cfg(unix)]
    #[test]
    fn save_secure_atomically_writes_a_private_file_without_chmodding_custom_parent() {
        use std::os::unix::fs::PermissionsExt;

        let test_root =
            std::env::temp_dir().join(format!("sms-relayed-config-test-{}", uuid::Uuid::new_v4()));
        let directory = test_root.join("sms-relayed");
        fs::create_dir(&test_root).unwrap();
        fs::create_dir(&directory).unwrap();
        fs::set_permissions(&directory, fs::Permissions::from_mode(0o755)).unwrap();
        let path = directory.join("config.toml");
        let mut cfg = AppConfig::default();
        cfg.api.password = "private".to_string();

        cfg.save_secure(&path).unwrap();

        assert_eq!(AppConfig::load(&path).unwrap(), cfg);
        assert_eq!(
            fs::metadata(&path).unwrap().permissions().mode() & 0o777,
            0o600
        );
        assert_eq!(
            fs::metadata(&directory).unwrap().permissions().mode() & 0o777,
            0o755
        );
        let names = fs::read_dir(&directory)
            .unwrap()
            .map(|entry| entry.unwrap().file_name())
            .collect::<Vec<_>>();
        assert_eq!(names, vec![std::ffi::OsString::from("config.toml")]);
        fs::remove_dir_all(test_root).unwrap();
    }

    #[cfg(unix)]
    #[test]
    fn shared_or_relative_config_parents_are_not_secured() {
        assert!(!should_secure_config_parent(Path::new(".")));
        assert!(!should_secure_config_parent(Path::new("/")));
        assert!(!should_secure_config_parent(
            &fs::canonicalize(std::env::temp_dir()).unwrap()
        ));
        assert!(!should_secure_config_parent(
            &std::env::temp_dir().join("shared")
        ));
        assert!(!should_secure_config_parent(Path::new(
            "/srv/team/sms-relayed"
        )));
        assert!(should_secure_config_parent(Path::new("/etc/sms-relayed")));
    }

    #[test]
    fn default_app_config_has_expected_modem_and_sms_defaults() {
        let cfg = AppConfig::default();
        assert_eq!(cfg.app.modem_path, "/org/freedesktop/ModemManager1/Modem/0");
        assert_eq!(cfg.sms.ignore_storage, vec!["sm"]);
        assert!(cfg.sms.code_keywords.contains(&"验证码".to_string()));
        assert_eq!(cfg.delivery.concurrency, 2);
    }

    #[test]
    fn legacy_config_uses_default_delivery_concurrency() {
        let serialized = toml::to_string(&AppConfig::default()).unwrap();
        let without_delivery = serialized
            .lines()
            .take_while(|line| *line != "[delivery]")
            .collect::<Vec<_>>()
            .join("\n");

        let cfg: AppConfig = toml::from_str(&without_delivery).unwrap();

        assert_eq!(cfg.delivery.concurrency, 2);
    }

    #[test]
    fn delivery_concurrency_must_be_between_one_and_sixteen() {
        let mut cfg = AppConfig::default();
        cfg.delivery.concurrency = 0;
        assert!(cfg
            .validate()
            .unwrap_err()
            .to_string()
            .contains("delivery.concurrency"));

        cfg.delivery.concurrency = 17;
        assert!(cfg
            .validate()
            .unwrap_err()
            .to_string()
            .contains("delivery.concurrency"));

        cfg.delivery.concurrency = 16;
        assert!(cfg.validate().is_ok());
    }

    #[test]
    fn parses_profile_refs() {
        let r = ProfileRef::parse("bark.personal").unwrap();
        assert_eq!(r.channel_type, ChannelType::Bark);
        assert_eq!(r.name, "personal");
    }

    #[test]
    fn validates_lark_profile_and_redacts_webhook_credentials() {
        let mut cfg = AppConfig::default();
        cfg.channels.lark.insert(
            "alerts".to_string(),
            LarkConfig {
                webhook_url: "https://open.larksuite.com/open-apis/bot/v2/hook/1234567890abcdef"
                    .to_string(),
                secret: "1234567890secret".to_string(),
            },
        );
        cfg.forward.enabled = vec!["lark.alerts".to_string()];

        assert!(cfg.validate().is_ok());
        assert_eq!(
            cfg.enabled_profiles().unwrap()[0].key(),
            "lark.alerts".to_string()
        );
        let summary = cfg.redacted_summary();
        assert!(!summary.contains("1234567890abcdef"));
        assert!(!summary.contains("1234567890secret"));
    }

    #[test]
    fn enabled_lark_profile_requires_a_webhook_url() {
        let mut cfg = AppConfig::default();
        cfg.channels.lark.insert(
            "alerts".to_string(),
            LarkConfig {
                webhook_url: String::new(),
                secret: String::new(),
            },
        );
        cfg.forward.enabled = vec!["lark.alerts".to_string()];

        let error = cfg.validate().unwrap_err().to_string();
        assert!(error.contains("channels.lark.alerts.webhook_url"));
    }

    #[test]
    fn rejects_removed_pushplus_profile_refs() {
        let err = ProfileRef::parse("pushplus.default")
            .unwrap_err()
            .to_string();
        assert!(err.contains("unknown channel type: pushplus"));
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
        assert!(!cfg.api.enabled);
        assert_eq!(cfg.api.bind, "0.0.0.0");
        assert_eq!(cfg.api.port, 8080);
        assert!(!cfg.api.enable_ipv6);
        assert_eq!(cfg.api.database_path, "/etc/sms-relayed/sms-relayed.sqlite");
    }

    #[test]
    fn enabled_api_requires_password() {
        let mut cfg = AppConfig::default();
        cfg.api.enabled = true;
        let err = cfg.validate().unwrap_err().to_string();
        assert!(err.contains("api.password"));
    }

    #[test]
    fn api_config_accepts_password() {
        let mut cfg = AppConfig::default();
        cfg.api.enabled = true;
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
