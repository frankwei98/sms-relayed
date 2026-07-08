# P1 CLI and Installer Redesign Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Build the P1 OpenWrt-first CLI, TOML config, multi-profile forwarding model, and POSIX installer described in `docs/superpowers/specs/2026-07-08-p1-cli-installer-design.md`.

**Architecture:** Convert the current flat `config.txt` and mode flags into typed TOML config plus explicit `clap` subcommands. Keep ModemManager access and channel senders small and testable by passing typed profile config into forwarding and passing `modem_path` into D-Bus calls. Keep `install.sh` independent and POSIX `sh` compatible so it works on OpenWrt BusyBox.

**Tech Stack:** Rust 2021, Tokio, clap, serde, toml, inquire, zbus, reqwest, axum retained but API not started in P1, POSIX `sh`, OpenWrt procd, systemd.

## Global Constraints

- Default config path is `/etc/sms-relayed/config.toml`.
- Do not implement compatibility with old `config.txt`.
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

---

## File Structure

- Modify `Cargo.toml`: add `toml`, `inquire`, and `is-terminal`.
- Modify `src/main.rs`: thin async dispatcher from parsed CLI to runtime/wizard/config actions.
- Replace `src/cli.rs`: subcommand definitions and CLI constants only.
- Replace `src/config.rs`: typed TOML config structs, defaults, load/save, validation, profile resolution, and redaction.
- Create `src/wizard.rs`: `inquire` prompt flow that returns `AppConfig`.
- Create `src/runtime.rs`: validates config, starts forwarding, and implements interactive send flow.
- Modify `src/dbus.rs`: accept modem path from config for monitor/send operations.
- Replace or shrink `src/setup.rs` and `src/mode.rs`: remove old numeric stdin setup path from runtime. Delete modules if no longer referenced.
- Modify `src/forward/mod.rs` and `src/forward/*.rs`: dispatch by typed `ChannelProfile`, not flat string keys.
- Create `install.sh`: POSIX installer.
- Modify `README.md`: update quick start, installer, config, and service docs.

---

### Task 1: Dependencies and CLI Skeleton

**Files:**
- Modify: `Cargo.toml`
- Modify: `src/cli.rs`
- Modify: `src/main.rs`

**Interfaces:**
- Produces: `cli::Args`, `cli::Command`, `cli::ConfigCommand`, `cli::DEFAULT_CONFIG_PATH`
- Produces: command parsing for `setup`, `run`, `send`, `config check`, `config show`
- Later tasks consume these command enums in `main.rs` and `runtime.rs`

- [ ] **Step 1: Add dependencies**

Edit `Cargo.toml` dependencies:

```toml
toml = "0.8"
inquire = "0.7"
is-terminal = "0.4"
```

- [ ] **Step 2: Replace CLI definitions**

Replace `src/cli.rs` with:

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
```

- [ ] **Step 3: Write a compile-only initial dispatcher**

Replace `src/main.rs` with an initial dispatcher that compiles before runtime and wizard modules are connected:

```rust
mod cli;
mod config;
mod dbus;
mod forward;
mod smscode;
mod util;
mod web;

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
        Some(Command::Run) => Err(anyhow::anyhow!("runtime is connected in Task 6")),
        Some(Command::Send) => Err(anyhow::anyhow!("send is connected in Task 6")),
        Some(Command::Config { command }) => match command {
            ConfigCommand::Check => Err(anyhow::anyhow!("config check is connected in Task 3")),
            ConfigCommand::Show => Err(anyhow::anyhow!("config show is connected in Task 3")),
        },
    }
}
```

- [ ] **Step 4: Verify help output**

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

### Task 2: Typed TOML Config

**Files:**
- Modify: `src/config.rs`
- Test: `src/config.rs`

**Interfaces:**
- Consumes: `cli::DEFAULT_CONFIG_PATH`
- Produces: `config::AppConfig`
- Produces: `AppConfig::default()`
- Produces: `AppConfig::load(path: &Path) -> anyhow::Result<Self>`
- Produces: `AppConfig::save_secure(&self, path: &Path) -> anyhow::Result<()>`
- Produces: `AppConfig::validate(&self) -> anyhow::Result<()>`
- Produces: `AppConfig::redacted_summary(&self) -> String`
- Produces: `ProfileRef::parse(input: &str) -> anyhow::Result<ProfileRef>`
- Produces: `ChannelProfile` enum used by forwarding tasks

- [ ] **Step 1: Write config tests**

Replace the tests in `src/config.rs` with tests for the new model:

```rust
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_config_has_expected_modem_and_sms_defaults() {
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
    fn rejects_unknown_profile_ref_type() {
        let err = ProfileRef::parse("unknown.main").unwrap_err().to_string();
        assert!(err.contains("unknown channel type"));
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
        let summary = cfg.redacted_summary();
        assert!(summary.contains("1234...cdef"));
        assert!(!summary.contains("1234567890abcdef"));
    }
}
```

- [ ] **Step 2: Run tests and verify failure**

Run:

```bash
cargo test config::
```

Expected: fails because `AppConfig`, `ProfileRef`, and channel config types do not exist.

- [ ] **Step 3: Add config structs and validation**

Replace `src/config.rs` with typed TOML config. Include these public types:

```rust
use std::collections::BTreeMap;
use std::fs;
use std::io::Write;
use std::path::Path;

use anyhow::{bail, Context, Result};
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

Add defaults:

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
```

Add these methods after the defaults. The validation helper must return exact missing-field messages such as `channels.bark.personal.key is required`.

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

fn redact(secret: &str) -> String {
    if secret.chars().count() <= 8 {
        "****".to_string()
    } else {
        let prefix: String = secret.chars().take(4).collect();
        let suffix: String = secret.chars().rev().take(4).collect::<String>().chars().rev().collect();
        format!("{}...{}", prefix, suffix)
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
                format!("telegram.{} bot_token={} chat_id={}", name, redact(&config.bot_token), config.chat_id)
            }
            ChannelProfile::PushPlus { name, config } => {
                format!("pushplus.{} token={}", name, redact(&config.token))
            }
            ChannelProfile::WeCom { name, config } => {
                format!("wecom.{} corp_id={} secret={}", name, config.corp_id, redact(&config.secret))
            }
            ChannelProfile::DingTalk { name, config } => {
                format!("dingtalk.{} access_token={} secret={}", name, redact(&config.access_token), redact(&config.secret))
            }
            ChannelProfile::Shell { name, config } => {
                format!("shell.{} path={}", name, config.path)
            }
        }
    }
}
```

- [ ] **Step 4: Run config tests**

Run:

```bash
cargo test config::
```

Expected: all config tests pass.

- [ ] **Step 5: Commit**

```bash
git add src/config.rs Cargo.toml Cargo.lock
git commit -m "feat: add typed toml config"
```

---

### Task 3: Config CLI Commands

**Files:**
- Modify: `src/main.rs`
- Test: `src/main.rs` through command execution

**Interfaces:**
- Consumes: `AppConfig::load`, `AppConfig::validate`, `AppConfig::redacted_summary`
- Produces: working `sms-relayed config check` and `sms-relayed config show`

- [ ] **Step 1: Wire config commands in `main.rs`**

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

- [ ] **Step 2: Create a valid temp config**

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
enabled = ["bark.personal"]

[channels.bark.personal]
server_url = "https://api.day.app"
key = "abcdef1234567890"
EOF
cargo run -- --config "$tmp/config.toml" config check
cargo run -- --config "$tmp/config.toml" config show
```

Expected: first command prints `config ok`; second command prints `bark.personal` and a redacted key, not `abcdef1234567890`.

- [ ] **Step 3: Verify missing field fails**

Run:

```bash
tmp="$(mktemp -d)"
cat > "$tmp/bad.toml" <<'EOF'
[app]
device_name = "test-device"
modem_path = "/org/freedesktop/ModemManager1/Modem/0"

[sms]
ignore_storage = ["sm"]
code_keywords = ["验证码"]

[forward]
enabled = ["bark.personal"]

[channels.bark.personal]
server_url = "https://api.day.app"
key = ""
EOF
cargo run -- --config "$tmp/bad.toml" config check
```

Expected: exits non-zero and includes `channels.bark.personal.key is required`.

- [ ] **Step 4: Commit**

```bash
git add src/main.rs
git commit -m "feat: wire config cli commands"
```

---

### Task 4: Wizard Flow with `inquire`

**Files:**
- Create: `src/wizard.rs`
- Modify: `src/main.rs`

**Interfaces:**
- Consumes: `AppConfig` and channel config structs
- Produces: `wizard::run_setup_wizard(existing: Option<AppConfig>) -> anyhow::Result<AppConfig>`
- Produces: `wizard::write_setup_config(config: &AppConfig, path: &Path) -> anyhow::Result<()>`

- [ ] **Step 1: Add module and interfaces**

Add to `src/main.rs` module list:

```rust
mod wizard;
```

Create `src/wizard.rs` with:

```rust
use std::path::Path;

use anyhow::Result;
use inquire::{Confirm, MultiSelect, Password, Select, Text};

use crate::config::{
    AppConfig, BarkConfig, ChannelType, DingTalkConfig, PushPlusConfig, ShellConfig,
    TelegramConfig, WeComConfig,
};

pub fn run_setup_wizard(existing: Option<AppConfig>) -> Result<AppConfig> {
    let mut cfg = existing.unwrap_or_default();

    let target = Select::new("Runtime target", vec!["SMS forwarding service"])
        .with_help_message("API and frontend are planned for P2.")
        .prompt()?;
    if target != "SMS forwarding service" {
        unreachable!("only one runtime target is exposed in P1");
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

    cfg.sms.ignore_storage = vec![
        Text::new("Ignore SMS storage type")
            .with_default("sm")
            .prompt()?,
    ];

    Ok(cfg)
}

pub fn write_setup_config(config: &AppConfig, path: &Path) -> Result<()> {
    config.save_secure(path)
}
```

Add this helper in `src/wizard.rs`:

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

- [ ] **Step 2: Wire no-subcommand and setup**

In `src/main.rs`, replace the initial setup branches:

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

- [ ] **Step 3: Verify non-TTY behavior**

Run:

```bash
cargo run -- < /dev/null
```

Expected: exits non-zero and includes `no subcommand in non-interactive mode`.

- [ ] **Step 4: Compile wizard**

Run:

```bash
cargo check
```

Expected: passes.

- [ ] **Step 5: Commit**

```bash
git add src/main.rs src/wizard.rs
git commit -m "feat: add interactive setup wizard"
```

---

### Task 5: Forwarding Profiles

**Files:**
- Modify: `src/forward/mod.rs`
- Modify: `src/forward/bark.rs`
- Modify: `src/forward/telegram.rs`
- Modify: `src/forward/pushplus.rs`
- Modify: `src/forward/wecom.rs`
- Modify: `src/forward/dingtalk.rs`
- Modify: `src/forward/shell.rs`
- Modify: `src/smscode.rs`

**Interfaces:**
- Consumes: `ChannelProfile`
- Produces: `forward::forward_sms(profiles: &[ChannelProfile], tel_number: &str, sms_text: &str, sms_date: &str, config: &AppConfig) -> anyhow::Result<()>`
- Produces: channel senders that accept typed configs

- [ ] **Step 1: Update SMS code keyword access**

Change `src/smscode.rs` to accept `&AppConfig` and read:

```rust
let keys = config.sms.code_keywords.iter().map(String::as_str);
```

Keep public function:

```rust
pub fn get_sms_code_str(sms_text: &str, config: &AppConfig) -> (String, String, String)
```

- [ ] **Step 2: Rewrite forward dispatcher**

Replace `src/forward/mod.rs` dispatcher imports and signature with:

```rust
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

    for profile in profiles {
        let result = match profile {
            ChannelProfile::PushPlus { name: _, config: profile_config } => {
                pushplus::send(tel_number, sms_text, &sms_date, &device_name, profile_config, config).await
            }
            ChannelProfile::WeCom { name: _, config: profile_config } => {
                wecom::send(tel_number, sms_text, &sms_date, &device_name, profile_config, config).await
            }
            ChannelProfile::Telegram { name: _, config: profile_config } => {
                telegram::send(tel_number, sms_text, &sms_date, &device_name, profile_config, config).await
            }
            ChannelProfile::DingTalk { name: _, config: profile_config } => {
                dingtalk::send(tel_number, sms_text, &sms_date, &device_name, profile_config, config).await
            }
            ChannelProfile::Bark { name: _, config: profile_config } => {
                bark::send(tel_number, sms_text, &sms_date, &device_name, profile_config, config).await
            }
            ChannelProfile::Shell { name: _, config: profile_config } => {
                shell::send(tel_number, sms_text, &sms_date, &device_name, profile_config, config).await
            }
        };
        if let Err(e) = result {
            error!("profile forward failed: {}", e);
        }
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

- [ ] **Step 3: Change each sender signature**

For Bark, use:

```rust
pub async fn send(
    tel_number: &str,
    sms_text: &str,
    sms_date: &str,
    device_name: &str,
    profile: &BarkConfig,
    app_config: &AppConfig,
) -> Result<()>
```

Replace flat config access:

```rust
let bark_url = profile.server_url.trim_end_matches('/');
let bark_key = profile.key.as_str();
let (code_str, code, _) = smscode::get_sms_code_str(sms_text, app_config);
```

Apply the same pattern for Telegram, PushPlus, WeCom, DingTalk, and Shell using their typed config structs.

- [ ] **Step 4: Compile forwarding changes**

Run:

```bash
cargo check
```

Expected: passes after all old `Config` imports are removed from forwarders.

- [ ] **Step 5: Commit**

```bash
git add src/forward src/smscode.rs
git commit -m "feat: route forwarding through channel profiles"
```

---

### Task 6: Modem Path and Runtime

**Files:**
- Create: `src/runtime.rs`
- Modify: `src/main.rs`
- Modify: `src/dbus.rs`

**Interfaces:**
- Consumes: `AppConfig::enabled_profiles()`
- Produces: `runtime::run_forwarding(config_path: &Path) -> anyhow::Result<()>`
- Produces: `runtime::send_interactive(config_path: &Path) -> anyhow::Result<()>`
- Produces: `dbus::monitor_dbus(modem_path: &str, profiles: &[ChannelProfile], config: &AppConfig) -> anyhow::Result<()>`
- Produces: `dbus::send_sms(connection: &zbus::Connection, modem_path: &str, tel_number: &str, sms_text: &str, target: SendTarget) -> anyhow::Result<()>`

- [ ] **Step 1: Add `SendTarget` and parameterize D-Bus calls**

In `src/dbus.rs`, replace `MODEM_PATH` usage with a parameter. Add:

```rust
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SendTarget {
    Command,
    Api,
}
```

Change signatures:

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

Use `modem_path` in `AddMatch`, `Messaging.Create`, and `Messaging.Delete`.

- [ ] **Step 2: Support multiple ignored storage values**

Replace single `StorageType` config parsing with:

```rust
let ignored_storage: Vec<StorageType> = config
    .sms
    .ignore_storage
    .iter()
    .map(|s| StorageType::from_config(s))
    .collect();
```

Pass `&[StorageType]` into `get_sms_content` and ignore when any storage filter matches.

- [ ] **Step 3: Create runtime module**

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

- [ ] **Step 4: Wire runtime commands**

Add `mod runtime;` to `src/main.rs`. Replace `Run` and `Send` arms:

```rust
Some(Command::Run) => runtime::run_forwarding(&args.config).await,
Some(Command::Send) => runtime::send_interactive(&args.config).await,
```

- [ ] **Step 5: Compile**

Run:

```bash
cargo check
```

Expected: passes.

- [ ] **Step 6: Commit**

```bash
git add src/main.rs src/runtime.rs src/dbus.rs
git commit -m "feat: add runtime with configurable modem path"
```

---

### Task 7: Remove Old Setup/Mode Wiring and Keep Web Out of P1 Runtime

**Files:**
- Modify: `src/main.rs`
- Delete if unused: `src/setup.rs`
- Delete if unused: `src/mode.rs`
- Modify: `src/web.rs`

**Interfaces:**
- Consumes: new runtime path
- Produces: no references to old numeric setup flow
- Produces: `web.rs` either compiles with new `dbus::send_sms` signature or is feature-neutral dead code

- [ ] **Step 1: Remove old modules from `main.rs`**

Ensure these are absent from `src/main.rs`:

```rust
mod mode;
mod setup;
```

- [ ] **Step 2: Update `web.rs` to compile without enabling P1 API**

Change the API send call to pass modem path and `SendTarget::Api` if `web.rs` remains compiled:

```rust
dbus::send_sms(
    &state.dbus_connection,
    &state.modem_path,
    &params.telnum,
    &params.smstext,
    dbus::SendTarget::Api,
)
.await
```

Add `modem_path: String` to `AppState` and populate it from config. If `web.rs` still depends on old config keys, remove `mod web;` from `main.rs` for P1 because the API is outside this phase.

- [ ] **Step 3: Delete unused old files**

Run:

```bash
rg "setup::|mode::|mod setup|mod mode" src
```

If there are no hits, delete:

```bash
git rm src/setup.rs src/mode.rs
```

- [ ] **Step 4: Compile**

Run:

```bash
cargo check
```

Expected: passes without old setup/mode references.

- [ ] **Step 5: Commit**

```bash
git add src/main.rs src/web.rs
git add -u src/setup.rs src/mode.rs
git commit -m "refactor: remove legacy setup flow"
```

---

### Task 8: POSIX Installer

**Files:**
- Create: `install.sh`

**Interfaces:**
- Produces: `install.sh` with POSIX `sh` compatibility
- Produces: OpenWrt init script generation
- Produces: systemd service generation

- [ ] **Step 1: Create installer script**

Create `install.sh` with these top-level functions:

```sh
#!/bin/sh
set -eu

REPO="frankwei98/sms-relayed"
VERSION="${SMS_RELAYED_VERSION:-latest}"
BIN_DIR="${SMS_RELAYED_BIN_DIR:-/usr/bin}"
CONFIG_DIR="${SMS_RELAYED_CONFIG_DIR:-/etc/sms-relayed}"
START_SERVICE="${SMS_RELAYED_START:-1}"
CONFIG_ONLY="${SMS_RELAYED_CONFIG_ONLY:-0}"

log() { printf '%s\n' "$*"; }
die() { printf 'error: %s\n' "$*" >&2; exit 1; }
have() { command -v "$1" >/dev/null 2>&1; }
```

Implement:

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
```

Add these functions to `install.sh`:

```sh
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

resolve_asset_url() {
  suffix="$1"
  if [ "$VERSION" = "latest" ]; then
    api="https://api.github.com/repos/$REPO/releases/latest"
  else
    api="https://api.github.com/repos/$REPO/releases/tags/$VERSION"
  fi
  fetch_url "$api" | sed -n 's/.*"browser_download_url": "\(.*\)".*/\1/p' | grep "$suffix" | head -n 1
}

install_binary() {
  suffix="$(detect_suffix)"
  url="$(resolve_asset_url "$suffix")"
  [ -n "$url" ] || die "no release asset found for $suffix in version $VERSION"
  mkdir -p "$BIN_DIR"
  tmp="${TMPDIR:-/tmp}/sms-relayed.$$"
  download_file "$url" "$tmp"
  chmod +x "$tmp"
  mv "$tmp" "$BIN_DIR/sms-relayed"
  log "installed $BIN_DIR/sms-relayed"
}

write_openwrt_service() {
  cat > /etc/init.d/sms-relayed <<EOF
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
  chmod +x /etc/init.d/sms-relayed
}

write_systemd_service() {
  cat > /etc/systemd/system/sms-relayed.service <<EOF
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

run_setup_if_tty() {
  if [ -t 0 ] && [ -t 1 ]; then
    "$BIN_DIR/sms-relayed" setup --config "$CONFIG_DIR/config.toml"
  else
    log "non-interactive shell detected; run: $BIN_DIR/sms-relayed setup --config $CONFIG_DIR/config.toml"
  fi
}
```

Add `main` with this control flow:

```sh
main() {
  mkdir -p "$CONFIG_DIR"
  chmod 700 "$CONFIG_DIR" 2>/dev/null || true

  if [ "$CONFIG_ONLY" != "1" ]; then
    install_binary
    if [ -d /etc/init.d ] && [ -f /etc/rc.common ]; then
      write_openwrt_service
    elif have systemctl; then
      write_systemd_service
      systemctl daemon-reload || true
    else
      log "no supported service manager detected"
    fi
  fi

  run_setup_if_tty

  if [ "$START_SERVICE" = "1" ] && [ "$CONFIG_ONLY" != "1" ]; then
    if [ -x /etc/init.d/sms-relayed ]; then
      /etc/init.d/sms-relayed enable
      /etc/init.d/sms-relayed start
    elif have systemctl; then
      systemctl enable --now sms-relayed
    fi
  fi
}

main "$@"
```

- [ ] **Step 2: Make POSIX syntax pass**

Run:

```bash
sh -n install.sh
```

Expected: no output, exit 0.

- [ ] **Step 3: Smoke architecture mapping**

Run the config-only path:

```bash
SMS_RELAYED_START=0 SMS_RELAYED_CONFIG_ONLY=1 sh install.sh
```

Expected on the local machine: either prints config-only flow or fails before writing root paths with a clear permissions message. It must not produce a shell syntax error.

- [ ] **Step 4: Commit**

```bash
git add install.sh
git commit -m "feat: add openwrt-first installer"
```

---

### Task 9: README and Final Verification

**Files:**
- Modify: `README.md`

**Interfaces:**
- Consumes: final CLI, config, and installer behavior
- Produces: user-facing docs for P1

- [ ] **Step 1: Update README quick start**

Replace old quick-start commands with:

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

- [ ] **Step 2: Document TOML config**

Add a config section matching the P1 config model:

````markdown
Default config path:

```text
/etc/sms-relayed/config.toml
```

Profiles are enabled by `forward.enabled` entries like `bark.personal`.
````

- [ ] **Step 3: Document services**

Add OpenWrt commands:

```sh
/etc/init.d/sms-relayed enable
/etc/init.d/sms-relayed start
/etc/init.d/sms-relayed status
```

Add systemd commands:

```sh
systemctl enable --now sms-relayed
systemctl status sms-relayed
```

- [ ] **Step 4: Run full verification**

Run:

```bash
cargo fmt -- --check
cargo test
cargo clippy -- -D warnings
cargo build --release
sh -n install.sh
git status --short
```

Expected: formatting, tests, clippy, build, and shell syntax checks pass. `git status --short` shows only intended README changes before commit.

- [ ] **Step 5: Commit**

```bash
git add README.md
git commit -m "docs: update p1 cli installer usage"
```

---

## Self-Review Checklist

- Spec coverage:
  - CLI subcommands: Task 1, Task 3, Task 4, Task 6.
  - `inquire` wizard: Task 4.
  - TOML config: Task 2.
  - Multi-profile forwarding: Task 2 and Task 5.
  - Configurable modem path: Task 2 and Task 6.
  - OpenWrt-first installer: Task 8.
  - systemd support: Task 8 and Task 9.
  - P2 exclusion: Task 7 and README updates.
- Placeholder scan: no open requirement slots are intentionally left in this plan.
- Type consistency:
  - `AppConfig`, `ProfileRef`, `ChannelProfile`, and channel config names are introduced in Task 2 before use.
  - `runtime` signatures are introduced before `main.rs` uses them.
  - `dbus::SendTarget` is introduced before `web.rs` and runtime use it.
