# P1 CLI and Installer Redesign Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Build the P1 OpenWrt-first CLI, TOML config, multi-profile forwarding model, and POSIX installer described in `docs/superpowers/specs/2026-07-08-p1-cli-installer-design.md`.

**Architecture:** Migrate in compile-safe layers. First add new CLI/config types while keeping old `Channel` and flat `Config` available for existing modules. Then wire config commands and wizard. Then migrate runtime, D-Bus, SMS code extraction, and all forwarders in one task because those signatures are coupled. Finish by deleting the legacy setup path and adding the installer/docs.

**Tech Stack:** Rust 2021, Tokio, clap, serde, toml, inquire, is-terminal, zbus, reqwest, POSIX `sh`, OpenWrt procd, systemd.

## Global Constraints

- Default config path is `/etc/sms-relayed/config.toml`.
- Do not implement compatibility with old `config.txt` as a user-facing format.
- Keep old internal structs only as transitional compile bridges; remove them before final verification.
- Use `inquire` for the interactive setup wizard; do not use a full-screen TUI.
- `sms-relayed run` must be non-interactive.
- `sms-relayed` with no subcommand may enter the wizard only when stdin and stdout are TTYs.
- OpenWrt/procd is the priority service target; systemd is supported as a secondary target.
- The installer must be POSIX `sh`, with no Bash-only syntax.
- The official installer command is `curl -fsSL https://raw.githubusercontent.com/frankwei98/sms-relayed/main/install.sh | sh`.
- Service runs as root in P1.
- P1 supports existing channel types: Bark, Telegram, PushPlus, WeCom, DingTalk, Shell.
- P1 does not implement API/frontend, SMS history storage, or web password protection.
- `armv7l` is best-effort asset lookup only; `aarch64` is primary.
- Every task below must end with a compiling workspace unless the step explicitly says it is the failing-test step.

---

## File Structure

- Modify `Cargo.toml`: add `toml`, `inquire`, and `is-terminal`.
- Modify `src/cli.rs`: add the new subcommand CLI while preserving transitional `Channel` for old modules.
- Modify `src/config.rs`: add typed TOML config while preserving transitional old `Config` until runtime migration is complete.
- Create `src/wizard.rs`: `inquire` prompt flow that returns `AppConfig`.
- Create `src/runtime.rs`: validates config, starts forwarding, and implements interactive send flow.
- Modify `src/dbus.rs`: accept `modem_path`, `ChannelProfile`, and multi-storage filters.
- Modify `src/forward/mod.rs` and `src/forward/*.rs`: dispatch by typed `ChannelProfile`.
- Modify `src/smscode.rs`: read code keywords from typed `AppConfig`.
- Remove `src/setup.rs`, `src/mode.rs`, and `mod web;` from P1 runtime. Keep `web.rs` on disk for P2 if desired, but do not compile it from `main.rs` in P1.
- Create `install.sh`: POSIX installer with release asset resolution, OpenWrt service, systemd service, and safe non-TTY behavior.
- Modify `README.md`: update quick start, installer, config, and service docs.

---

### Task 1: Dependencies and Compile-Safe CLI Skeleton

**Files:**
- Modify: `Cargo.toml`
- Modify: `src/cli.rs`
- Modify: `src/main.rs`

**Interfaces:**
- Produces: `cli::Args`, `cli::Command`, `cli::ConfigCommand`, `cli::DEFAULT_CONFIG_PATH`
- Preserves transitional bridge: `cli::Channel` for current `dbus.rs` and `forward/*`
- Removes from runtime entry: old numeric setup path, old `RunMode`, and old `preprocess_args`

- [ ] **Step 1: Add dependencies**

Add to `Cargo.toml`:

```toml
toml = "0.8"
inquire = "0.7"
is-terminal = "0.4"
```

- [ ] **Step 2: Replace `src/cli.rs` with new CLI plus transitional channel bridge**

```rust
use std::path::PathBuf;

use clap::{Parser, Subcommand};

pub const DEFAULT_CONFIG_PATH: &str = "/etc/sms-relayed/config.toml";

#[derive(Parser, Debug)]
#[command(name = "sms-relayed", version, about = "SMS relay for ModemManager devices")]
pub struct Args {
    #[arg(long = "config", global = true, default_value = DEFAULT_CONFIG_PATH)]
    pub config: PathBuf,

    #[command(subcommand)]
    pub command: Option<Command>,
}

#[derive(Subcommand, Debug, Clone, PartialEq, Eq)]
pub enum Command {
    Setup,
    Run,
    Send,
    Config {
        #[command(subcommand)]
        command: ConfigCommand,
    },
}

#[derive(Subcommand, Debug, Clone, PartialEq, Eq)]
pub enum ConfigCommand {
    Check,
    Show,
}

// Transitional bridge for modules migrated in Task 5.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Channel {
    PushPlus,
    WeCom,
    Telegram,
    DingTalk,
    Bark,
    Shell,
}

impl Channel {
    pub fn name(&self) -> &str {
        match self {
            Channel::PushPlus => "PushPlus",
            Channel::WeCom => "WeCom",
            Channel::Telegram => "Telegram",
            Channel::DingTalk => "DingTalk",
            Channel::Bark => "Bark",
            Channel::Shell => "Shell",
        }
    }
}
```

- [ ] **Step 3: Replace `src/main.rs` with the new dispatcher shell**

Do not import `setup`, `mode`, or `web`. Those are outside the P1 runtime.

```rust
mod cli;
mod config;
mod dbus;
mod forward;
mod smscode;
mod util;

use anyhow::Result;
use clap::Parser;
use cli::{Args, Command, ConfigCommand};
use is_terminal::IsTerminal;

#[tokio::main]
async fn main() -> Result<()> {
    env_logger::init();
    let args = Args::parse();

    match args.command {
        None => {
            if std::io::stdin().is_terminal() && std::io::stdout().is_terminal() {
                println!("setup wizard is added in Task 4");
                Ok(())
            } else {
                Err(anyhow::anyhow!(
                    "no subcommand in non-interactive mode; run `sms-relayed run --config {}` or `sms-relayed setup`",
                    args.config.display()
                ))
            }
        }
        Some(Command::Setup) => {
            println!("setup wizard is added in Task 4");
            Ok(())
        }
        Some(Command::Run) => Err(anyhow::anyhow!("runtime is connected in Task 5")),
        Some(Command::Send) => Err(anyhow::anyhow!("send is connected in Task 5")),
        Some(Command::Config { command }) => match command {
            ConfigCommand::Check => Err(anyhow::anyhow!("config check is connected in Task 3")),
            ConfigCommand::Show => Err(anyhow::anyhow!("config show is connected in Task 3")),
        },
    }
}
```

- [ ] **Step 4: Verify**

Run:

```bash
cargo check
cargo run -- --help
cargo run -- config --help
```

Expected: `cargo check` passes; help output lists `setup`, `run`, `send`, and `config`.

- [ ] **Step 5: Commit**

```bash
git add Cargo.toml Cargo.lock src/cli.rs src/main.rs
git commit -m "feat: add p1 cli command skeleton"
```

---

### Task 2: Typed TOML Config Beside Legacy Config

**Files:**
- Modify: `src/config.rs`

**Interfaces:**
- Produces: `AppConfig`, `AppSection`, `SmsSection`, `ForwardSection`, `ChannelsSection`
- Produces: `BarkConfig`, `TelegramConfig`, `PushPlusConfig`, `WeComConfig`, `DingTalkConfig`, `ShellConfig`
- Produces: `ChannelType`, `ProfileRef`, `ChannelProfile`
- Produces: `AppConfig::load`, `save_secure`, `validate`, `enabled_profiles`, `redacted_summary`
- Preserves transitional bridge: old `Config` struct and methods for modules migrated in Task 5

- [ ] **Step 1: Add tests to `src/config.rs` without deleting old tests yet**

Append these tests inside the existing `#[cfg(test)] mod tests` block:

```rust
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
    cfg.channels.bark.insert("personal".to_string(), BarkConfig::default());
    let err = cfg.validate().unwrap_err().to_string();
    assert!(err.contains("channels.bark.personal.key"));
}

#[test]
fn validates_multiple_profiles_of_same_type() {
    let mut cfg = AppConfig::default();
    cfg.forward.enabled = vec!["bark.personal".to_string(), "bark.ops".to_string()];
    cfg.channels.bark.insert("personal".to_string(), BarkConfig {
        server_url: "https://api.day.app".to_string(),
        key: "personal-key".to_string(),
    });
    cfg.channels.bark.insert("ops".to_string(), BarkConfig {
        server_url: "https://api.day.app".to_string(),
        key: "ops-key".to_string(),
    });
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
```

- [ ] **Step 2: Run tests and verify failure**

Run:

```bash
cargo test config::
```

Expected: fails because new typed config does not exist.

- [ ] **Step 3: Add typed config types and defaults**

Add these types above the legacy `Config` struct:

```rust
use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

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

#[derive(Debug, Clone, Serialize, Deserialize, Default, PartialEq, Eq)]
pub struct ApiSection {
    pub enabled: bool,
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
    Bark { name: String, config: BarkConfig },
    Telegram { name: String, config: TelegramConfig },
    PushPlus { name: String, config: PushPlusConfig },
    WeCom { name: String, config: WeComConfig },
    DingTalk { name: String, config: DingTalkConfig },
    Shell { name: String, config: ShellConfig },
}
```

Add `Default for AppConfig`, `ProfileRef::parse`, `ChannelProfile::redacted_line`, and helper `redact` exactly as follows:

```rust
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
        Ok(Self { channel_type, name: name.to_string() })
    }
}

impl ChannelProfile {
    pub fn redacted_line(&self) -> String {
        match self {
            ChannelProfile::Bark { name, config } => format!("bark.{} key={}", name, redact(&config.key)),
            ChannelProfile::Telegram { name, config } => format!("telegram.{} bot_token={} chat_id={}", name, redact(&config.bot_token), config.chat_id),
            ChannelProfile::PushPlus { name, config } => format!("pushplus.{} token={}", name, redact(&config.token)),
            ChannelProfile::WeCom { name, config } => format!("wecom.{} corp_id={} secret={}", name, config.corp_id, redact(&config.secret)),
            ChannelProfile::DingTalk { name, config } => format!("dingtalk.{} access_token={} secret={}", name, redact(&config.access_token), redact(&config.secret)),
            ChannelProfile::Shell { name, config } => format!("shell.{} path={}", name, config.path),
        }
    }
}

fn redact(secret: &str) -> String {
    if secret.chars().count() <= 8 {
        "****".to_string()
    } else {
        let prefix: String = secret.chars().take(4).collect();
        let suffix: String = secret.chars().rev().take(4).collect::<String>().chars().rev().collect();
        format!("{}...{}", prefix, suffix)
    }
}
```

- [ ] **Step 4: Add typed config load/save/validation methods**

Add methods without deleting legacy `Config`:

```rust
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
        if !self.app.modem_path.starts_with("/org/freedesktop/ModemManager1/Modem/") {
            bail!("app.modem_path must be a ModemManager modem object path");
        }
        for reference in &self.forward.enabled {
            let parsed = ProfileRef::parse(reference)?;
            self.profile_for_ref(&parsed)?;
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
                let cfg = self.channels.bark.get(&reference.name)
                    .ok_or_else(|| anyhow::anyhow!("enabled profile bark.{} does not exist", reference.name))?;
                require("channels.bark", &reference.name, "server_url", &cfg.server_url)?;
                require("channels.bark", &reference.name, "key", &cfg.key)?;
                Ok(ChannelProfile::Bark { name: reference.name.clone(), config: cfg.clone() })
            }
            ChannelType::Telegram => {
                let cfg = self.channels.telegram.get(&reference.name)
                    .ok_or_else(|| anyhow::anyhow!("enabled profile telegram.{} does not exist", reference.name))?;
                require("channels.telegram", &reference.name, "bot_token", &cfg.bot_token)?;
                require("channels.telegram", &reference.name, "chat_id", &cfg.chat_id)?;
                Ok(ChannelProfile::Telegram { name: reference.name.clone(), config: cfg.clone() })
            }
            ChannelType::PushPlus => {
                let cfg = self.channels.pushplus.get(&reference.name)
                    .ok_or_else(|| anyhow::anyhow!("enabled profile pushplus.{} does not exist", reference.name))?;
                require("channels.pushplus", &reference.name, "token", &cfg.token)?;
                Ok(ChannelProfile::PushPlus { name: reference.name.clone(), config: cfg.clone() })
            }
            ChannelType::WeCom => {
                let cfg = self.channels.wecom.get(&reference.name)
                    .ok_or_else(|| anyhow::anyhow!("enabled profile wecom.{} does not exist", reference.name))?;
                require("channels.wecom", &reference.name, "corp_id", &cfg.corp_id)?;
                require("channels.wecom", &reference.name, "agent_id", &cfg.agent_id)?;
                require("channels.wecom", &reference.name, "secret", &cfg.secret)?;
                Ok(ChannelProfile::WeCom { name: reference.name.clone(), config: cfg.clone() })
            }
            ChannelType::DingTalk => {
                let cfg = self.channels.dingtalk.get(&reference.name)
                    .ok_or_else(|| anyhow::anyhow!("enabled profile dingtalk.{} does not exist", reference.name))?;
                require("channels.dingtalk", &reference.name, "access_token", &cfg.access_token)?;
                require("channels.dingtalk", &reference.name, "secret", &cfg.secret)?;
                Ok(ChannelProfile::DingTalk { name: reference.name.clone(), config: cfg.clone() })
            }
            ChannelType::Shell => {
                let cfg = self.channels.shell.get(&reference.name)
                    .ok_or_else(|| anyhow::anyhow!("enabled profile shell.{} does not exist", reference.name))?;
                require("channels.shell", &reference.name, "path", &cfg.path)?;
                Ok(ChannelProfile::Shell { name: reference.name.clone(), config: cfg.clone() })
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
```

- [ ] **Step 5: Verify**

Run:

```bash
cargo test config::
cargo check
```

Expected: tests and check pass. Existing modules still compile because legacy `Config` remains.

- [ ] **Step 6: Commit**

```bash
git add Cargo.toml Cargo.lock src/config.rs
git commit -m "feat: add typed toml config"
```

---

### Task 3: Config CLI Commands

**Files:**
- Modify: `src/main.rs`

**Interfaces:**
- Consumes: `AppConfig::load`, `AppConfig::validate`, `AppConfig::redacted_summary`
- Produces: working `sms-relayed config check` and `sms-relayed config show`

- [ ] **Step 1: Wire config commands in `src/main.rs`**

Replace the `ConfigCommand` match arms:

```rust
Some(Command::Config { command }) => match command {
    ConfigCommand::Check => {
        let cfg = config::AppConfig::load(&args.config)?;
        cfg.validate()?;
        println!("config ok: {}", args.config.display());
        Ok(())
    }
    ConfigCommand::Show => {
        let cfg = config::AppConfig::load(&args.config)?;
        cfg.validate()?;
        println!("{}", cfg.redacted_summary());
        Ok(())
    }
},
```

- [ ] **Step 2: Verify valid and invalid config behavior**

Run:

```bash
tmp="$(mktemp -d)"
cat > "$tmp/config.toml" <<'EOF'
[app]
device_name = "test-device"
modem_path = "/org/freedesktop/ModemManager1/Modem/0"

[sms]
ignore_storage = ["sm"]
code_keywords = ["验证码", "verification", "code"]

[forward]
enabled = ["bark.personal", "bark.ops"]

[channels.bark.personal]
server_url = "https://api.day.app"
key = "abcdef1234567890"

[channels.bark.ops]
server_url = "https://api.day.app"
key = "opsabcdef123456"
EOF
cargo run -- --config "$tmp/config.toml" config check
cargo run -- --config "$tmp/config.toml" config show
```

Expected: check prints `config ok`; show prints both `bark.personal` and `bark.ops`; full keys are not printed.

Run:

```bash
cat > "$tmp/bad.toml" <<'EOF'
[app]
device_name = "test-device"
modem_path = "/org/freedesktop/ModemManager1/Modem/0"

[sms]
ignore_storage = ["sm"]
code_keywords = ["验证码"]

[forward]
enabled = ["bark.missing"]
EOF
cargo run -- --config "$tmp/bad.toml" config check
```

Expected: exits non-zero and includes `enabled profile bark.missing does not exist`.

- [ ] **Step 3: Commit**

```bash
git add src/main.rs
git commit -m "feat: wire config cli commands"
```

---

### Task 4: Interactive Setup Wizard

**Files:**
- Create: `src/wizard.rs`
- Modify: `src/main.rs`

**Interfaces:**
- Consumes: `AppConfig` and channel config structs
- Produces: `wizard::run_setup_wizard(existing: Option<AppConfig>) -> anyhow::Result<AppConfig>`
- Produces: `wizard::write_setup_config(config: &AppConfig, path: &Path) -> anyhow::Result<()>`

- [ ] **Step 1: Create `src/wizard.rs`**

Use `inquire` prompts. The write function must validate before writing.

```rust
use std::path::Path;

use anyhow::Result;
use inquire::{Confirm, MultiSelect, Password, Select, Text};

use crate::config::{
    AppConfig, BarkConfig, DingTalkConfig, PushPlusConfig, ShellConfig, TelegramConfig,
    WeComConfig,
};

pub fn run_setup_wizard(existing: Option<AppConfig>) -> Result<AppConfig> {
    let mut cfg = existing.unwrap_or_default();

    Select::new("Runtime target", vec!["SMS forwarding service"])
        .with_help_message("API and frontend are planned for P2.")
        .prompt()?;

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

    cfg.sms.ignore_storage = vec![
        Text::new("Ignore SMS storage type")
            .with_default("sm")
            .prompt()?,
    ];

    Ok(cfg)
}

pub fn write_setup_config(config: &AppConfig, path: &Path) -> Result<()> {
    config.validate()?;
    config.save_secure(path)
}
```

Add `add_profiles_for_channel` with these exact mappings:

```rust
fn add_profiles_for_channel(cfg: &mut AppConfig, label: &str) -> Result<()> {
    loop {
        let default_name = if profile_count(cfg, label) == 0 { "default" } else { "extra" };
        let name = Text::new(&format!("{} profile name", label))
            .with_default(default_name)
            .prompt()?;
        match label {
            "Bark" => {
                let server_url = Text::new("Bark server URL")
                    .with_default("https://api.day.app")
                    .prompt()?;
                let key = Password::new("Bark key").without_confirmation().prompt()?;
                cfg.channels.bark.insert(name.clone(), BarkConfig { server_url, key });
                cfg.forward.enabled.push(format!("bark.{}", name));
            }
            "Telegram" => {
                let bot_token = Password::new("Telegram bot token").without_confirmation().prompt()?;
                let chat_id = Text::new("Telegram chat id").prompt()?;
                let api_base = Text::new("Telegram API base")
                    .with_default("https://api.telegram.org")
                    .prompt()?;
                cfg.channels.telegram.insert(name.clone(), TelegramConfig { bot_token, chat_id, api_base });
                cfg.forward.enabled.push(format!("telegram.{}", name));
            }
            "PushPlus" => {
                let token = Password::new("PushPlus token").without_confirmation().prompt()?;
                cfg.channels.pushplus.insert(name.clone(), PushPlusConfig { token });
                cfg.forward.enabled.push(format!("pushplus.{}", name));
            }
            "WeCom" => {
                let corp_id = Text::new("WeCom corp id").prompt()?;
                let agent_id = Text::new("WeCom agent id").prompt()?;
                let secret = Password::new("WeCom app secret").without_confirmation().prompt()?;
                cfg.channels.wecom.insert(name.clone(), WeComConfig {
                    corp_id,
                    agent_id,
                    secret,
                    to_user: "@all".to_string(),
                });
                cfg.forward.enabled.push(format!("wecom.{}", name));
            }
            "DingTalk" => {
                let access_token = Password::new("DingTalk access token").without_confirmation().prompt()?;
                let secret = Password::new("DingTalk signing secret").without_confirmation().prompt()?;
                cfg.channels.dingtalk.insert(name.clone(), DingTalkConfig { access_token, secret });
                cfg.forward.enabled.push(format!("dingtalk.{}", name));
            }
            "Shell" => {
                let path = Text::new("Shell script path").prompt()?;
                cfg.channels.shell.insert(name.clone(), ShellConfig { path });
                cfg.forward.enabled.push(format!("shell.{}", name));
            }
            other => anyhow::bail!("unknown wizard channel: {}", other),
        }
        if !Confirm::new(&format!("Add another {} profile?", label))
            .with_default(false)
            .prompt()? {
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
```

- [ ] **Step 2: Wire setup commands**

Add `mod wizard;` to `src/main.rs`. Replace `None` and `Setup` arms with:

```rust
None => {
    if std::io::stdin().is_terminal() && std::io::stdout().is_terminal() {
        let existing = config::AppConfig::load(&args.config).ok();
        let cfg = wizard::run_setup_wizard(existing)?;
        wizard::write_setup_config(&cfg, &args.config)?;
        println!("config written: {}", args.config.display());
        Ok(())
    } else {
        Err(anyhow::anyhow!(
            "no subcommand in non-interactive mode; run `sms-relayed run --config {}` or `sms-relayed setup`",
            args.config.display()
        ))
    }
}
Some(Command::Setup) => {
    let existing = config::AppConfig::load(&args.config).ok();
    let cfg = wizard::run_setup_wizard(existing)?;
    wizard::write_setup_config(&cfg, &args.config)?;
    println!("config written: {}", args.config.display());
    Ok(())
}
```

- [ ] **Step 3: Verify non-TTY behavior and compile**

Run:

```bash
cargo run -- < /dev/null
cargo check
```

Expected: first command exits non-zero and includes `no subcommand in non-interactive mode`; `cargo check` passes.

- [ ] **Step 4: Manual OpenWrt console note**

Record in the task handoff whether wizard prompts were tested through SSH, serial console, or not tested yet. If not tested on the Qualcomm 410 board, leave a note for Task 8/9 manual validation rather than changing code blindly.

- [ ] **Step 5: Commit**

```bash
git add src/main.rs src/wizard.rs
git commit -m "feat: add interactive setup wizard"
```

---

### Task 5: Runtime, D-Bus, SMS Code, and Forwarding Migration

**Files:**
- Create: `src/runtime.rs`
- Modify: `src/main.rs`
- Modify: `src/dbus.rs`
- Modify: `src/smscode.rs`
- Modify: `src/forward/mod.rs`
- Modify: `src/forward/bark.rs`
- Modify: `src/forward/telegram.rs`
- Modify: `src/forward/pushplus.rs`
- Modify: `src/forward/wecom.rs`
- Modify: `src/forward/dingtalk.rs`
- Modify: `src/forward/shell.rs`

**Interfaces:**
- Produces: `runtime::run_forwarding(config_path: &Path) -> anyhow::Result<()>`
- Produces: `runtime::send_interactive(config_path: &Path) -> anyhow::Result<()>`
- Produces: `dbus::SendTarget`, introduced so command-send and future P2 API-send can share D-Bus code without stringly typed targets.
- Produces: `dbus::monitor_dbus(modem_path: &str, profiles: &[ChannelProfile], config: &AppConfig) -> anyhow::Result<()>`
- Produces: `dbus::send_sms(connection: &zbus::Connection, modem_path: &str, tel_number: &str, sms_text: &str, target: SendTarget) -> anyhow::Result<()>`
- Produces: `forward::forward_sms(profiles: &[ChannelProfile], tel_number: &str, sms_text: &str, sms_date: &str, config: &AppConfig) -> anyhow::Result<()>`

- [ ] **Step 1: Update SMS code extraction to typed config**

Change `src/smscode.rs` imports and keyword access:

```rust
use crate::config::AppConfig;

pub fn has_verification_keyword(sms_content: &mut String, config: &AppConfig) -> bool {
    for keyword in &config.sms.code_keywords {
        if sms_content.contains(keyword) {
            let replacement = format!("{} ", keyword);
            *sms_content = sms_content.replacen(keyword, &replacement, 1);
            return true;
        }
    }
    false
}

pub fn get_sms_code_str(sms_text: &str, config: &AppConfig) -> (String, String, String) {
    let mut content = sms_text.trim().to_string();
    if has_verification_keyword(&mut content, config) {
        let code = extract_code(&content);
        let code_from = extract_code_source(&content);
        let code_str = if code.is_empty() {
            String::new()
        } else if code_from.is_empty() {
            format!("验证码 {}", code)
        } else {
            format!("{} 验证码 {}", code_from, code)
        };
        (code_str, code, code_from)
    } else {
        (String::new(), String::new(), String::new())
    }
}
```

- [ ] **Step 2: Change each forwarder to typed profile config**

Use these signatures:

```rust
// bark.rs
pub async fn send(
    tel_number: &str,
    sms_text: &str,
    sms_date: &str,
    device_name: &str,
    profile: &BarkConfig,
    app_config: &AppConfig,
) -> Result<()>

// telegram.rs
pub async fn send(
    tel_number: &str,
    sms_text: &str,
    sms_date: &str,
    device_name: &str,
    profile: &TelegramConfig,
    app_config: &AppConfig,
) -> Result<()>
```

Apply the same parameter order to PushPlus, WeCom, DingTalk, and Shell. Replace flat `config.get_or_empty(...)` reads with fields from `profile`. Keep SMS-code calls using `smscode::get_sms_code_str(sms_text, app_config)`.

- [ ] **Step 3: Replace `src/forward/mod.rs` dispatcher**

```rust
pub mod bark;
pub mod dingtalk;
pub mod pushplus;
pub mod shell;
pub mod telegram;
pub mod wecom;

use anyhow::Result;
use log::error;

use crate::config::{AppConfig, ChannelProfile};
use crate::util;

pub async fn forward_sms(
    profiles: &[ChannelProfile],
    tel_number: &str,
    sms_text: &str,
    sms_date: &str,
    config: &AppConfig,
) -> Result<()> {
    let device_name = resolve_device_name(config);
    let sms_date = sms_date.replace('T', " ");
    println!("发信电话:{}\n时间:{}\n短信内容:{}", tel_number, sms_date, sms_text);

    let mut failures = 0usize;
    for profile in profiles {
        let result = match profile {
            ChannelProfile::PushPlus { config: profile_config, .. } => {
                pushplus::send(tel_number, sms_text, &sms_date, &device_name, profile_config, config).await
            }
            ChannelProfile::WeCom { config: profile_config, .. } => {
                wecom::send(tel_number, sms_text, &sms_date, &device_name, profile_config, config).await
            }
            ChannelProfile::Telegram { config: profile_config, .. } => {
                telegram::send(tel_number, sms_text, &sms_date, &device_name, profile_config, config).await
            }
            ChannelProfile::DingTalk { config: profile_config, .. } => {
                dingtalk::send(tel_number, sms_text, &sms_date, &device_name, profile_config, config).await
            }
            ChannelProfile::Bark { config: profile_config, .. } => {
                bark::send(tel_number, sms_text, &sms_date, &device_name, profile_config, config).await
            }
            ChannelProfile::Shell { config: profile_config, .. } => {
                shell::send(tel_number, sms_text, &sms_date, &device_name, profile_config, config).await
            }
        };
        if let Err(e) = result {
            failures += 1;
            error!("profile forward failed: {}", e);
        }
    }

    if failures == profiles.len() && !profiles.is_empty() {
        error!("all forwarding profiles failed for this SMS");
    }
    Ok(())
}

fn resolve_device_name(config: &AppConfig) -> String {
    let name = config.app.device_name.as_str();
    if name == "*Host*Name*" || name.is_empty() {
        util::hostname()
    } else {
        name.to_string()
    }
}
```

- [ ] **Step 4: Parameterize D-Bus**

In `src/dbus.rs`, remove dependency on `cli::Channel` and flat `Config`. Add:

```rust
use crate::config::{AppConfig, ChannelProfile};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SendTarget {
    Command,
    Api,
}
```

Change monitor/send signatures:

```rust
pub async fn monitor_dbus(
    modem_path: &str,
    profiles: &[ChannelProfile],
    config: &AppConfig,
) -> Result<()>

pub async fn send_sms(
    connection: &Connection,
    modem_path: &str,
    tel_number: &str,
    sms_text: &str,
    target: SendTarget,
) -> Result<()>
```

Use `modem_path` in the signal match rule, `Messaging.Create`, and `Messaging.Delete`.

Define storage filtering as:

```rust
fn should_ignore_storage(storage: u32, filters: &[StorageType]) -> bool {
    filters
        .iter()
        .any(|filter| !matches!(filter, StorageType::All) && filter.should_ignore(storage))
}
```

This preserves current semantics: `StorageType::All` means "do not filter anything". In a multi-value list, `all` is ignored rather than causing every message to be filtered.

- [ ] **Step 5: Create runtime module**

Create `src/runtime.rs`:

```rust
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
```

- [ ] **Step 6: Wire runtime commands and stop compiling P2 web module**

Add `mod runtime;` to `src/main.rs`. Replace:

```rust
Some(Command::Run) => runtime::run_forwarding(&args.config).await,
Some(Command::Send) => runtime::send_interactive(&args.config).await,
```

Ensure `mod web;` is not present in `src/main.rs`. `web.rs` can remain on disk for P2, but it must not be compiled in P1.

- [ ] **Step 7: Verify**

Run:

```bash
cargo check
cargo test
```

Expected: both pass. If this task fails, do not add compatibility shims in `dbus` or `forward`; finish the typed migration in this same task.

- [ ] **Step 8: Commit**

```bash
git add src/main.rs src/runtime.rs src/dbus.rs src/smscode.rs src/forward
git commit -m "feat: migrate runtime to typed profiles"
```

---

### Task 6: Remove Legacy Setup and Flat Config

**Files:**
- Modify: `src/config.rs`
- Modify: `src/cli.rs`
- Delete: `src/setup.rs`
- Delete: `src/mode.rs`

**Interfaces:**
- Removes: old flat `Config`
- Removes: transitional `cli::Channel`
- Removes: old stdin numeric setup flow

- [ ] **Step 1: Confirm no legacy references remain**

Run:

```bash
rg "crate::config::Config|\\bConfig::|crate::cli::Channel|\\bChannel::|setup::|mode::|mod setup|mod mode" src
```

Expected: only hits are inside `src/config.rs` for the old `Config` declaration and inside `src/cli.rs` for the transitional `Channel` declaration.

- [ ] **Step 2: Delete old config implementation and transitional channel bridge**

Remove from `src/config.rs`:

- `CONFIG_KEYS`
- old `pub struct Config`
- old `impl Config`
- old flat-config tests

Remove from `src/cli.rs`:

- transitional `pub enum Channel`
- `impl Channel`

- [ ] **Step 3: Delete old setup files**

Run:

```bash
git rm src/setup.rs src/mode.rs
```

- [ ] **Step 4: Verify**

Run:

```bash
cargo check
cargo test
```

Expected: both pass.

- [ ] **Step 5: Commit**

```bash
git add src/config.rs src/cli.rs
git add -u src/setup.rs src/mode.rs
git commit -m "refactor: remove legacy setup and config"
```

---

### Task 7: POSIX OpenWrt-First Installer

**Files:**
- Create: `install.sh`

**Interfaces:**
- Produces: POSIX `sh` installer
- Produces: release asset lookup without `jq`
- Produces: OpenWrt init script and systemd service
- Produces: safe behavior when non-TTY or config is missing

- [ ] **Step 1: Create `install.sh` header and environment**

```sh
#!/bin/sh
set -eu

REPO="frankwei98/sms-relayed"
VERSION="${SMS_RELAYED_VERSION:-latest}"
ROOT="${SMS_RELAYED_ROOT:-}"
BIN_DIR="${SMS_RELAYED_BIN_DIR:-/usr/bin}"
CONFIG_DIR="${SMS_RELAYED_CONFIG_DIR:-/etc/sms-relayed}"
START_SERVICE="${SMS_RELAYED_START:-1}"
CONFIG_ONLY="${SMS_RELAYED_CONFIG_ONLY:-0}"

target_path() {
  printf '%s%s\n' "$ROOT" "$1"
}

log() { printf '%s\n' "$*"; }
warn() { printf 'warning: %s\n' "$*" >&2; }
die() { printf 'error: %s\n' "$*" >&2; exit 1; }
have() { command -v "$1" >/dev/null 2>&1; }
```

- [ ] **Step 2: Add architecture and download helpers**

```sh
detect_suffix() {
  arch="$(uname -m)"
  case "$arch" in
    aarch64|arm64) printf '%s\n' "linux-musl-aarch64" ;;
    x86_64) printf '%s\n' "linux-musl-x64" ;;
    armv7l) printf '%s\n' "linux-musl-armv7l" ;;
    *) die "unsupported architecture: $arch" ;;
  esac
}

fetch_url() {
  url="$1"
  if have curl; then
    curl -fsSL "$url"
  elif have wget; then
    wget -qO- "$url"
  else
    die "curl or wget is required"
  fi
}

download_file() {
  url="$1"
  dest="$2"
  if have curl; then
    curl -fL "$url" -o "$dest"
  elif have wget; then
    wget -qO "$dest" "$url"
  else
    die "curl or wget is required"
  fi
}

resolve_asset_url_from_json() {
  suffix="$1"
  tr ',' '\n' |
    grep -o '"browser_download_url"[[:space:]]*:[[:space:]]*"[^"]*"' |
    sed 's/.*"browser_download_url"[[:space:]]*:[[:space:]]*"//; s/"$//' |
    grep "$suffix" |
    head -n 1
}

resolve_asset_url() {
  suffix="$1"
  if [ "$VERSION" = "latest" ]; then
    api="https://api.github.com/repos/$REPO/releases/latest"
  else
    api="https://api.github.com/repos/$REPO/releases/tags/$VERSION"
  fi
  fetch_url "$api" | resolve_asset_url_from_json "$suffix"
}
```

This intentionally avoids `jq` because OpenWrt images usually do not include it.

- [ ] **Step 3: Add install and service writers**

```sh
install_binary() {
  suffix="$(detect_suffix)"
  url="$(resolve_asset_url "$suffix")"
  [ -n "$url" ] || die "no release asset found for $suffix in version $VERSION"

  real_bin_dir="$(target_path "$BIN_DIR")"
  mkdir -p "$real_bin_dir"
  tmp="${TMPDIR:-/tmp}/sms-relayed.$$"
  download_file "$url" "$tmp"
  chmod +x "$tmp"
  mv "$tmp" "$real_bin_dir/sms-relayed"
  log "installed $real_bin_dir/sms-relayed"
}

write_openwrt_service() {
  init_dir="$(target_path /etc/init.d)"
  mkdir -p "$init_dir"
  cat > "$init_dir/sms-relayed" <<EOF
#!/bin/sh /etc/rc.common
START=99
USE_PROCD=1

start_service() {
  procd_open_instance
  procd_set_param command $BIN_DIR/sms-relayed run --config $CONFIG_DIR/config.toml
  procd_set_param respawn
  procd_close_instance
}
EOF
  chmod +x "$init_dir/sms-relayed"
}

write_systemd_service() {
  systemd_dir="$(target_path /etc/systemd/system)"
  mkdir -p "$systemd_dir"
  cat > "$systemd_dir/sms-relayed.service" <<EOF
[Unit]
Description=sms-relayed
After=network-online.target ModemManager.service
Wants=network-online.target

[Service]
Type=simple
ExecStart=$BIN_DIR/sms-relayed run --config $CONFIG_DIR/config.toml
Restart=always
RestartSec=5

[Install]
WantedBy=multi-user.target
EOF
}
```

- [ ] **Step 4: Add safe setup/start control flow**

```sh
warn_environment() {
  have mmcli || warn "mmcli not found; install or enable ModemManager before expecting SMS forwarding to work"
}

run_setup_if_tty() {
  real_bin="$(target_path "$BIN_DIR")/sms-relayed"
  if [ ! -x "$real_bin" ]; then
    warn "binary not installed at $real_bin; skipping setup"
    return
  fi
  if [ -t 0 ] && [ -t 1 ]; then
    "$real_bin" setup --config "$CONFIG_DIR/config.toml"
  else
    log "non-interactive shell detected; run: $BIN_DIR/sms-relayed setup --config $CONFIG_DIR/config.toml"
  fi
}

start_service_if_ready() {
  [ "$START_SERVICE" = "1" ] || return
  [ "$CONFIG_ONLY" != "1" ] || return
  real_config="$(target_path "$CONFIG_DIR")/config.toml"
  if [ ! -f "$real_config" ]; then
    warn "config missing at $real_config; not starting service"
    return
  fi
  if [ -x "$(target_path /etc/init.d/sms-relayed)" ]; then
    if [ -z "$ROOT" ]; then
      /etc/init.d/sms-relayed enable
      /etc/init.d/sms-relayed start
    else
      log "SMS_RELAYED_ROOT is set; service file generated but not started"
    fi
  elif have systemctl && [ -z "$ROOT" ]; then
    systemctl enable --now sms-relayed
  fi
}

main() {
  warn_environment
  mkdir -p "$(target_path "$CONFIG_DIR")"
  chmod 700 "$(target_path "$CONFIG_DIR")" 2>/dev/null || true

  if [ "$CONFIG_ONLY" != "1" ]; then
    install_binary
    if [ -d "$(target_path /etc/init.d)" ] && [ -f "$(target_path /etc/rc.common)" ]; then
      write_openwrt_service
    elif have systemctl || [ -n "$ROOT" ]; then
      write_systemd_service
      if [ -z "$ROOT" ] && have systemctl; then
        systemctl daemon-reload || true
      fi
    else
      warn "no supported service manager detected"
    fi
  fi

  run_setup_if_tty
  start_service_if_ready
}

if [ "${SMS_RELAYED_TEST:-0}" != "1" ]; then
  main "$@"
fi
```

- [ ] **Step 5: Add installer asset parser fixture test**

Run:

```bash
tmp="$(mktemp -d)"
cat > "$tmp/release.json" <<'EOF'
{"assets":[{"browser_download_url":"https://example.invalid/sms-relayed-abc-linux-musl-x64"},{"browser_download_url":"https://example.invalid/sms-relayed-abc-linux-musl-aarch64"}]}
EOF
SMS_RELAYED_TEST=1 . ./install.sh
url="$(resolve_asset_url_from_json linux-musl-aarch64 < "$tmp/release.json")"
test "$url" = "https://example.invalid/sms-relayed-abc-linux-musl-aarch64"
missing="$(resolve_asset_url_from_json linux-musl-armv7l < "$tmp/release.json" || true)"
test -z "$missing"
```

Expected: all shell commands exit 0 and sourcing `install.sh` does not run `main`.

- [ ] **Step 6: Verify installer syntax and root override**

Run:

```bash
sh -n install.sh
if command -v shellcheck >/dev/null 2>&1; then shellcheck -s sh install.sh; fi
tmp="$(mktemp -d)"
mkdir -p "$tmp/etc/init.d"
touch "$tmp/etc/rc.common"
SMS_RELAYED_ROOT="$tmp" SMS_RELAYED_START=0 sh install.sh || true
test -d "$tmp/etc/sms-relayed"
```

Expected: syntax passes; ShellCheck passes if present; root override does not write to real `/etc`.

- [ ] **Step 7: Commit**

```bash
git add install.sh
git commit -m "feat: add openwrt-first installer"
```

---

### Task 8: README and Final Verification

**Files:**
- Modify: `README.md`

**Interfaces:**
- Produces: P1 user docs
- Produces: final verification evidence

- [ ] **Step 1: Update README for P1 commands**

Document:

````markdown
## Quick Start

OpenWrt first-run install:

```sh
curl -fsSL https://raw.githubusercontent.com/frankwei98/sms-relayed/main/install.sh | sh
```

Manual setup:

```sh
sudo sms-relayed setup
sudo sms-relayed config check
sudo sms-relayed run
```
````

- [ ] **Step 2: Document TOML profiles and service commands**

Include:

````markdown
Default config path:

```text
/etc/sms-relayed/config.toml
```

Profiles are enabled by `forward.enabled` entries like `bark.personal`.

OpenWrt:

```sh
/etc/init.d/sms-relayed enable
/etc/init.d/sms-relayed start
/etc/init.d/sms-relayed status
```

systemd:

```sh
systemctl enable --now sms-relayed
systemctl status sms-relayed
```
````

- [ ] **Step 3: Full verification**

Run:

```bash
cargo fmt -- --check
cargo test
cargo clippy -- -D warnings
cargo build --release
sh -n install.sh
if command -v shellcheck >/dev/null 2>&1; then shellcheck -s sh install.sh; fi
cargo run -- < /dev/null
git status --short
```

Expected:

- Rust formatting, tests, clippy, and release build pass.
- Installer syntax passes.
- ShellCheck passes if present.
- Non-TTY no-subcommand exits non-zero and includes `no subcommand in non-interactive mode`.
- `git status --short` shows only intended README changes before commit.

- [ ] **Step 4: Manual OpenWrt verification**

On the Qualcomm 410 OpenWrt board, verify through SSH first:

```sh
curl -fsSL https://raw.githubusercontent.com/frankwei98/sms-relayed/main/install.sh | sh
sms-relayed config check
/etc/init.d/sms-relayed start
logread | tail -n 50
```

If serial-console setup is used, note whether `inquire` arrow-key prompts work correctly. If serial console has raw-mode issues, do not redesign the CLI immediately; record the terminal type and failure mode first.

- [ ] **Step 5: Commit**

```bash
git add README.md
git commit -m "docs: update p1 cli installer usage"
```

---

## Self-Review Checklist

- Compile-safe ordering:
  - Task 1 preserves `Channel`.
  - Task 2 preserves old flat `Config`.
  - Task 5 migrates D-Bus, runtime, forwarders, and SMS code together because those signatures are coupled.
  - Task 6 removes transitional bridges only after migration.
- Spec coverage:
  - CLI subcommands: Tasks 1, 3, 4, 5.
  - `inquire` wizard: Task 4.
  - TOML config and multi-profile model: Task 2.
  - Configurable `modem_path`: Tasks 2 and 5.
  - OpenWrt-first installer: Task 7.
  - systemd support: Task 7.
  - P2 exclusion: `mod web;` is absent from P1 `main.rs`; API/frontend remain out of scope.
- Feedback incorporated:
  - No per-task `cargo check` gate depends on removed types before their consumers migrate.
  - GitHub release asset parsing avoids `jq` and avoids greedy full-line JSON matching.
  - Installer skips setup when binary is absent.
  - Installer skips service start when config is missing.
  - `mmcli` absence is a warning.
  - `SMS_RELAYED_ROOT` supports isolated-root installer smoke tests.
  - Storage filtering defines `all` as "do not filter anything".
  - Wizard validates config before saving.
  - ShellCheck is included when available.
