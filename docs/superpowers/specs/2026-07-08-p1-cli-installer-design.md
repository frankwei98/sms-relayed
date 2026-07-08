# P1 CLI and Installer Redesign

Date: 2026-07-08

## Context

`sms-relayed` is a Rust port of an older SMS relay project. The current crate works, but its CLI, first-run setup, flat `config.txt`, hard-coded modem path, and Web API are still shaped like the original project. The next major direction has two phases:

- P1: Replace the CLI and first-run setup experience, add a one-line installer, prioritize OpenWrt, and support multiple native push channel profiles.
- P2: Build a new authenticated API and frontend for SMS history review and message replies.

This design covers P1 only. P2 requirements are acknowledged so P1 does not block them, but P1 will not implement the API, frontend, SMS history database, or web password flow.

## Goals

- Make first use pleasant on OpenWrt devices through an arrow-key and enter driven setup wizard.
- Use a new TOML config at `/etc/sms-relayed/config.toml`.
- Support multiple named push channel profiles, including multiple profiles of the same channel type.
- Provide a POSIX `sh` installer suitable for OpenWrt BusyBox environments.
- Install from GitHub latest release assets, with `aarch64` as the primary target.
- Generate and manage an OpenWrt procd service by default, with systemd support for common Linux hosts.
- Keep service startup fully non-interactive.
- Preserve the existing six push channels at the feature level: Bark, Telegram, PushPlus, WeCom, DingTalk, and Shell.

## Non-Goals

- No compatibility with the old `config.txt` format.
- No SMS history database.
- No API or frontend implementation.
- No password-protected web UI.
- No guaranteed `armv7l` release support in P1.
- No attempt to run the service as a dedicated non-root user in P1.
- No full-screen terminal UI.

## Recommended Approach

Use an OpenWrt-first, systemd-compatible redesign.

The CLI will use `clap` for commands and `inquire` for the setup wizard. `ratatui` is not used in P1 because this is a step-by-step configuration wizard, not a full-screen terminal application. `inquire` gives the needed prompt primitives: single select, multi-select, confirmation, password prompts, text input, defaults, validation, and help text.

The installer will be POSIX `sh`, not Bash, because OpenWrt commonly ships BusyBox `ash` without Bash. The official install command will be:

```sh
curl -fsSL https://raw.githubusercontent.com/frankwei98/sms-relayed/main/install.sh | sh
```

## CLI Design

Supported command shape:

```text
sms-relayed
sms-relayed setup
sms-relayed run
sms-relayed send
sms-relayed config check
sms-relayed config show
```

Behavior:

- `sms-relayed`: if stdin/stdout are a TTY, enter the setup wizard. If not a TTY, fail with a clear message instructing the user to run `sms-relayed run` or create a config first.
- `sms-relayed setup`: always enter the setup wizard. It can create, edit, or replace `/etc/sms-relayed/config.toml`.
- `sms-relayed run`: start the SMS forwarding service from config. This command is non-interactive and is the only command used by init systems.
- `sms-relayed send`: interactively prompt for recipient number and message body, then ask for confirmation before sending.
- `sms-relayed config check`: validate config syntax, profile references, required fields, modem path format, and service-relevant values.
- `sms-relayed config show`: print a redacted config summary. Secret values must not be printed in full.

The legacy short flags such as `-fB`, `-fP`, `-sS`, and `--sendsmsapi=enable` do not define the P1 user experience. They can be removed or treated as deprecated compatibility aliases only if that does not complicate the implementation. The default target is the new subcommand and TOML model.

## Setup Wizard Flow

The setup wizard uses `inquire` prompts:

1. Choose a runtime target. The default is SMS forwarding service. API/frontend choices are shown as P2 unavailable if mentioned at all.
2. Configure device name. Default is dynamic hostname; the user can enter a fixed display name.
3. Configure modem path. Default is `/org/freedesktop/ModemManager1/Modem/0`; the user can override it without editing source code.
4. Choose push channel types with multi-select:
   - Bark
   - Telegram
   - PushPlus
   - WeCom
   - DingTalk
   - Shell
5. For each chosen channel type, create one or more named profiles.
6. Prompt for required fields per profile.
7. Prompt for SMS filtering settings. Default behavior continues to ignore `sm` storage.
8. Write `/etc/sms-relayed/config.toml` with `0600` permissions.
9. Print the config path, service command, and log inspection hints.

When config already exists, the wizard asks whether to keep it, edit it, or replace it. Editing can be implemented as a guided rewrite in P1; opening an external editor is optional and not required.

## TOML Config Model

Default config path:

```text
/etc/sms-relayed/config.toml
```

Suggested shape:

```toml
[app]
device_name = "router-sim"
modem_path = "/org/freedesktop/ModemManager1/Modem/0"

[sms]
ignore_storage = ["sm"]
code_keywords = ["验证码", "verification", "code", "인증", "代码", "随机码"]

[forward]
enabled = [
  "bark.personal",
  "telegram.main",
]

[channels.bark.personal]
server_url = "https://api.day.app"
key = "..."

[channels.telegram.main]
bot_token = "..."
chat_id = "..."
api_base = "https://api.telegram.org"

[channels.pushplus.default]
token = "..."

[channels.wecom.default]
corp_id = "..."
agent_id = "..."
secret = "..."
to_user = "@all"

[channels.dingtalk.default]
access_token = "..."
secret = "..."

[channels.shell.default]
path = "/etc/sms-relayed/forward.sh"
```

Rules:

- `forward.enabled` contains profile references in `type.name` form.
- A profile reference must point to an existing table under `[channels.<type>.<name>]`.
- The same channel type can have multiple named profiles.
- Each sender receives a typed profile config instead of reading global string keys from a flat map.
- `modem_path` must be configurable for both listening and sending.
- `ignore_storage` is an array to allow multiple ignored storage types.
- Secret values are stored locally, so the installer and config writer must restrict file permissions.
- P1 may include `[api] enabled = false` as a reserved section, but P1 must not start an API server.

## Installer Design

Installer file:

```text
install.sh
```

Shell requirements:

- POSIX `sh` compatible.
- No Bash-only syntax.
- Works under OpenWrt BusyBox `ash`.

Release source:

- Default version: GitHub latest release.
- Asset pattern follows existing release assets:
  - `sms-relayed-<shortsha>-linux-musl-aarch64`
  - `sms-relayed-<shortsha>-linux-musl-x64`
- Because current asset names include the short commit SHA, the installer must resolve the real asset URL instead of assuming a stable filename.
- For `SMS_RELAYED_VERSION=latest`, query the GitHub latest release metadata and select the asset whose name ends with the mapped platform suffix.
- For `SMS_RELAYED_VERSION=<tag>`, query that release tag metadata and select the asset whose name ends with the mapped platform suffix.
- The selected URL should be the release asset `browser_download_url`.

Architecture mapping:

- `aarch64` or `arm64`: `linux-musl-aarch64`
- `x86_64`: `linux-musl-x64`
- `armv7l`: try `linux-musl-armv7l`, but fail clearly if unavailable

Supported environment variables:

```text
SMS_RELAYED_VERSION=latest|<tag>
SMS_RELAYED_START=0
SMS_RELAYED_CONFIG_ONLY=1
SMS_RELAYED_BIN_DIR=/usr/bin
SMS_RELAYED_CONFIG_DIR=/etc/sms-relayed
```

Installer checks:

- CPU architecture.
- Ability to write binary directory and config directory.
- Presence of `curl` or `wget`.
- Ability to resolve a matching GitHub release asset for the detected architecture.
- OpenWrt procd support via `/etc/init.d` and `procd` conventions.
- systemd support via `systemctl`.
- Presence of `mmcli` if available.

Only binary download, file write, and service file generation failures are hard failures. Missing `mmcli` or no detected modem is a warning, because users may configure before attaching or enabling the modem.

## Service Design

OpenWrt is the priority service target.

OpenWrt init script:

```text
/etc/init.d/sms-relayed
```

Runtime command:

```sh
/usr/bin/sms-relayed run --config /etc/sms-relayed/config.toml
```

Installer actions:

```sh
/etc/init.d/sms-relayed enable
/etc/init.d/sms-relayed start
```

Systemd service:

```ini
[Unit]
Description=sms-relayed
After=network-online.target ModemManager.service
Wants=network-online.target

[Service]
Type=simple
ExecStart=/usr/bin/sms-relayed run --config /etc/sms-relayed/config.toml
Restart=always
RestartSec=5

[Install]
WantedBy=multi-user.target
```

P1 runs the service as root by default. This is intentional for OpenWrt and ModemManager/system D-Bus access. A dedicated user and D-Bus policy can be revisited later.

## Code Boundaries

The refactor should create or preserve these responsibilities:

- `cli`: command definitions and argument parsing only.
- `wizard`: `inquire` prompt flow that returns typed config.
- `config`: TOML structs, defaults, load/save, validation, and redacted summary.
- `runtime`: converts config into active forwarding profiles and starts the selected runtime.
- `dbus` or `modem`: ModemManager D-Bus integration, parameterized by `modem_path`.
- `forward`: sender implementations for channel profiles.
- `install.sh`: standalone installer script.

`main.rs` should become orchestration glue instead of holding setup, runtime branching, and prompt logic directly.

## Error Handling

- Missing config in `run`: fail with a clear message that points to `sms-relayed setup`.
- Non-TTY `sms-relayed` with no subcommand: fail instead of attempting prompts.
- Config syntax errors: include path and TOML parse error.
- Missing profile field: include profile reference and field name.
- Unknown profile reference in `forward.enabled`: fail before starting.
- Single push channel failure: log the error and continue remaining channels.
- All push channels fail for one SMS: log the aggregate failure, keep listening.
- D-Bus connection failure: fail service startup with a clear error.
- Installer download failure: print URL, architecture, requested version, and exit non-zero.

## Testing Strategy

Rust tests:

- TOML default config serialization and deserialization.
- Profile reference parsing.
- Multiple profiles for the same channel type.
- Required field validation per channel.
- Secret redaction.
- `modem_path` is passed through runtime configuration.
- `ignore_storage` handles multiple storage values.
- Non-TTY no-subcommand behavior returns an error path.

CLI integration tests:

- `--help` output succeeds.
- `config check` succeeds for a valid config.
- `config check` fails for missing fields.
- `run` fails clearly when config is missing.

Installer smoke tests:

- Syntax check with `sh -n`.
- ShellCheck if available.
- Temporary install root via environment overrides.
- Architecture-to-asset mapping.
- OpenWrt service file generation.
- systemd service file generation.

Manual validation:

- OpenWrt on the Qualcomm 410 board:
  - install through the one-liner
  - run setup
  - start and stop `/etc/init.d/sms-relayed`
  - inspect service logs
- Linux/systemd host:
  - install with override paths where practical
  - verify service file
  - run `cargo test`
  - run `cargo clippy`
  - run `cargo build --release`

## P2 Boundary

P2 will introduce the new API and frontend:

- Password-protected frontend.
- SMS history storage.
- Received SMS review.
- Reply/send message support from the web UI.
- API endpoints for receiving history and sending replies.

P1 should not implement those features. It should only leave config and module boundaries that make P2 straightforward: typed config, parameterized modem access, non-interactive runtime startup, and a clean place for future API settings.
