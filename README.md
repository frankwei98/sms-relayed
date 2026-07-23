# sms-relayed

通过 ModemManager 收发、保存并转发短信的 Linux 服务。<br>
A Linux service for receiving, storing, sending, and forwarding SMS through ModemManager.

[中文](#中文) · [English](#english)

---

<a id="中文"></a>

## 中文

### 项目简介

sms-relayed 适用于插有 SIM 卡的 OpenWrt 路由器、Debian 网关、随身 Wi-Fi、USB 蜂窝网卡和其他 Linux 设备。它通过系统 D-Bus 连接 ModemManager，监听新短信并转发到多个通知渠道，同时提供短信历史、Web 管理控制台、命令行发送和设备状态管理。

项目由 Rust 后端和嵌入二进制的 React 前端组成，不需要单独部署 Web 服务器。

### 功能

- 实时接收 ModemManager 的新短信，持久化到 SQLite，并避免重复入库。
- 将短信转发到 Bark、Telegram、PushPlus、企业微信、钉钉或自定义 Shell 脚本。
- 支持多个命名 profile，同一条短信可投递到多个渠道，并记录投递结果、重试和延迟。
- 从常见验证码短信中提取 4–7 位字母数字验证码；Bark 通知可自动复制验证码。
- 通过 CLI 或密码保护的 Web 控制台发送短信。
- 在 Web 控制台中搜索、筛选、标记已读、删除和导出 CSV/JSON。
- 查看 Modem 状态，执行启用、禁用和确认后的重置操作。
- 可选的历史保留策略，分批清理超过指定天数且没有待处理投递的消息。
- 支持 OpenWrt procd 与 systemd 服务。
- 支持带 SHA-256 校验、原子替换和服务重启的自更新。

### 运行要求

- Linux 与可用的系统 D-Bus。
- ModemManager，以及一个支持短信功能的 Modem。
- 服务用户需要访问 ModemManager 系统 D-Bus；路由器上通常以 `root` 运行。
- `mmcli` 是 Web Modem 状态、健康诊断和设备控制的运行时依赖。没有 `mmcli` 时，短信收发仍通过 D-Bus 工作，但相关状态会显示为 `unknown`。
- 官方 Release 当前提供静态 Linux x86_64、aarch64 和 armv7 二进制。

### 快速开始

在 OpenWrt 的 root shell 中，或在有写入 `/usr/bin` 和 `/etc` 权限的 Linux 环境中运行：

```sh
curl -fsSL https://raw.githubusercontent.com/frankwei98/sms-relayed/main/install.sh | sh
```

安装脚本会：

1. 检测系统和架构并下载最新 GitHub Release。
2. 安装二进制到 `/usr/bin/sms-relayed`。
3. 创建 `/etc/sms-relayed`。
4. 注册 OpenWrt procd 或 systemd 服务。
5. 在交互式终端中启动配置向导；配置存在后启用并启动服务。

如果当前账号需要提权，可以先下载脚本、检查内容，再用 `sudo sh install.sh` 执行。

常用安装变量：

| 变量 | 默认值 | 说明 |
| --- | --- | --- |
| `SMS_RELAYED_VERSION` | `latest` | 安装指定 Release tag。 |
| `SMS_RELAYED_BIN_DIR` | `/usr/bin` | 二进制安装目录。 |
| `SMS_RELAYED_CONFIG_DIR` | `/etc/sms-relayed` | 配置和默认数据库目录。 |
| `SMS_RELAYED_START` | `1` | 设为 `0` 时不自动启动服务。 |
| `SMS_RELAYED_CONFIG_ONLY` | `0` | 设为 `1` 时不安装二进制，并使用已有二进制运行配置流程。 |
| `SMS_RELAYED_UPDATE` | 自动/交互确认 | 已安装二进制存在时，设为 `1` 强制更新，设为 `0` 跳过。 |
| `SMS_RELAYED_ROOT` | 空 | 将文件写入指定根目录，用于镜像或离线文件系统准备；不会启动服务。 |

### 配置

默认配置路径是 `/etc/sms-relayed/config.toml`。推荐运行交互式向导：

```sh
sudo sms-relayed setup
sudo sms-relayed config check
sudo sms-relayed config show
```

也可以使用全局 `--config` 参数指定其他路径：

```sh
sms-relayed --config /path/to/config.toml config check
```

完整示例：

```toml
[app]
device_name = "router-sim"
modem_path = "/org/freedesktop/ModemManager1/Modem/0"

[sms]
ignore_storage = ["sm"]
code_keywords = ["验证码", "verification", "code", "인증", "代码", "随机码"]

[forward]
enabled = ["bark.personal", "telegram.main"]

[delivery]
concurrency = 2

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

[api]
enabled = true
bind = "0.0.0.0"
port = 8080
enable_ipv6 = false
password = "change-this-password"
database_path = "/etc/sms-relayed/sms-relayed.sqlite"

[http]
connect_timeout_secs = 10
request_timeout_secs = 30
shell_timeout_secs = 30

[retention]
enabled = false
max_age_days = 90
batch_size = 500
```

配置说明：

- `forward.enabled` 使用 `渠道类型.profile名称`，例如 `telegram.main`。同一渠道可定义多个 profile。
- 支持的渠道类型为 `bark`、`telegram`、`pushplus`、`wecom`、`dingtalk` 和 `shell`。
- Bark 使用 [API v2](https://github.com/Finb/bark-server/blob/master/docs/API_V2.md)：将 `server_url` 设为服务根地址，程序会向 `<server_url>/push` 发送 JSON 请求。
- `delivery.concurrency` 控制同时执行的转发任务数，默认值为 `2`，有效范围为 `1` 到 `16`。
- 新 delivery 在数据库事务提交后会立即唤醒 worker；worker 启动时扫描一次，并保留固定 30 秒安全扫描。重试按最近的 `next_attempt_at` 精确唤醒。
- `api.enabled = true` 时必须设置非空 `api.password`。
- 配置文件包含凭据；程序写入配置时会将权限设为 `0600`。
- 修改配置后需要重启服务才能生效。

### Web 管理控制台

启用 `[api]` 后，访问：

```text
http://<设备 IP>:8080/
```

控制台包含：

- SMS 收件箱/发件箱、号码会话、搜索、筛选和未读状态。
- SMS 发送、单条/批量删除、CSV/JSON 流式导出。
- Modem 状态与启用、禁用、重置操作。
- 各转发 profile 最近五次已完成投递的结果与延迟；`Dispatch` 是任务到期后的调度等待，`Request` 是渠道请求耗时，迁移前的历史调度延迟显示为 `—`。
- 配置编辑、验证、保存和服务重启。
- 通过 SSE 推送新消息和状态变化。

主要 API：

| 路径 | 用途 |
| --- | --- |
| `/api/auth/login`, `/api/auth/logout`, `/api/auth/me` | 会话认证。 |
| `/api/messages`, `/api/conversations` | 消息与会话查询。 |
| `/api/messages/send` | 发送短信。 |
| `/api/messages/export` | 导出 CSV 或 JSON。 |
| `/api/events` | SSE 事件流。 |
| `/api/config`, `/api/config/check` | 配置读取、保存和验证。 |
| `/api/status`, `/api/service/restart` | 服务状态与重启。 |
| `/api/modem/*` | Modem 状态和控制。 |
| `/api/forwarding/attempts` | 转发投递状态。 |
| `/api/health` | 无需登录的服务与 Modem 健康检查。 |

`/api/health` 会隐藏对象路径、运营商、信号、SIM 标识、号码、短信正文和原始命令输出。Web 服务当前直接提供 HTTP；不要将它裸露到公网，请放在可信网络、VPN 或带 TLS 和访问控制的反向代理后面。

### CLI

```text
sms-relayed [--config <path>]
sms-relayed [--config <path>] setup
sms-relayed [--config <path>] run
sms-relayed [--config <path>] send
sms-relayed update
sms-relayed [--config <path>] config check
sms-relayed [--config <path>] config show
```

- 无子命令：在交互式终端中打开配置向导；非交互环境会返回错误。
- `setup`：打开配置向导。
- `run`：启动短信监听、持久化、投递 worker、保留策略 worker，以及可选 Web API。
- `send`：交互式输入号码和正文，确认后发送并保存记录。
- `update`：查询最新 Release，校验配套 SHA-256，验证下载文件的版本/commit，原子替换二进制并重启已检测到的服务。
- `config check`：加载并验证配置。
- `config show`：输出脱敏后的配置摘要。

自更新当前仅支持官方发布的 Linux x86_64、aarch64 与 armv7 资产。已是相同 commit 时不会替换或重启。更新系统目录通常需要以有权限的账号运行，例如 `sudo sms-relayed update`。

### 服务管理

OpenWrt：

```sh
/etc/init.d/sms-relayed enable
/etc/init.d/sms-relayed start
/etc/init.d/sms-relayed restart
/etc/init.d/sms-relayed status
logread | grep sms-relayed
```

systemd：

```sh
sudo systemctl enable --now sms-relayed
sudo systemctl restart sms-relayed
systemctl status sms-relayed
journalctl -u sms-relayed
```

### Shell 转发参数

Shell profile 会直接执行配置的可执行脚本，并依次传入：

```text
1. 发信号码
2. 短信时间
3. 短信正文
4. 识别出的验证码；没有则为空
5. 验证码来源；没有则为空
6. 设备名称
```

脚本必须安全处理参数和凭据，并在 `http.shell_timeout_secs` 内结束。每个值均作为独立的命令行参数传入，不会由 shell 再次解析。

### 构建与开发

需要稳定版 Rust、Node.js 和 pnpm。发布构建应先生成前端资产，再编译 Rust：

```sh
cd frontend
pnpm install --frozen-lockfile
pnpm build
cd ..
cargo build --release --locked
```

常用检查：

```sh
cargo fmt --check
cargo test
cargo build

cd frontend
pnpm check
pnpm test
pnpm build
```

如果 `frontend/dist` 不存在，`build.rs` 会生成一个提示页面，便于 Rust 开发构建；正式打包前必须运行前端构建。

### 错误监测与隐私

正式后端服务和生产前端默认启用独立的 Sentry 错误项目。上报会移除请求、用户、breadcrumb、主机名、上下文变量和异常正文，不发送短信正文、电话号码或配置凭据；重复运行时错误会限流。

- 后端：设置 `SMS_RELAYED_SENTRY_DSN` 可覆盖 DSN，设为空字符串可关闭。
- 前端：构建时设置 `VITE_SENTRY_DSN` 可覆盖 DSN，设置 `VITE_SENTRY_ENABLED=false` 可关闭。
- 前端开发模式不发送 Sentry 事件。

### 许可证与商标

本项目采用 [GNU GPL v3 only](LICENSE)（`GPL-3.0-only`）。项目名称、Logo 和官方 Release 名称另受 [Trademark Policy](TRADEMARKS.md) 约束。允许将本软件集成进硬件产品并如实说明，但修改版或第三方构建不得冒充官方 Release。

参考资料：

- [原 C++ 上游项目](https://github.com/lkiuyu/DbusSmsForwardCPlus)
- [ModemManager API](https://www.freedesktop.org/software/ModemManager/api/latest/)
- [zbus](https://docs.rs/zbus/latest/zbus/)

---

<a id="english"></a>

## English

### Overview

sms-relayed is designed for OpenWrt routers, Debian gateways, portable hotspots, USB cellular modems, and other Linux devices with a SIM card. It connects to ModemManager over the system D-Bus, receives and forwards SMS messages, and provides message history, a web dashboard, interactive sending, and modem status controls.

The project consists of a Rust backend and a React frontend embedded in the binary. No separate web server is required.

### Features

- Receive new messages from ModemManager, persist them in SQLite, and suppress duplicate inserts.
- Forward messages to Bark, Telegram, PushPlus, WeCom, DingTalk, or a custom shell script.
- Configure multiple named profiles, deliver one message to multiple channels, and record outcomes, retries, and latency.
- Extract 4–7 character alphanumeric codes from common verification messages; Bark can copy detected codes automatically.
- Send SMS from the CLI or the password-protected web dashboard.
- Search, filter, mark, delete, and export message history as CSV or JSON.
- Inspect modem health and enable, disable, or explicitly reset the modem.
- Optionally delete old terminal messages in batches while retaining messages with pending deliveries.
- Run under OpenWrt procd or systemd.
- Self-update with SHA-256 verification, atomic replacement, and service restart.

### Requirements

- Linux with a working system D-Bus.
- ModemManager and an SMS-capable modem.
- Permission to access ModemManager on the system bus; router installations commonly run as `root`.
- `mmcli` is required for dashboard modem diagnostics, health details, and modem controls. SMS receive/send still uses D-Bus without it, but those status features report `unknown`.
- Official releases currently provide static Linux binaries for x86_64, aarch64, and armv7.

### Quick start

Run this from an OpenWrt root shell or another Linux account that can write to `/usr/bin` and `/etc`:

```sh
curl -fsSL https://raw.githubusercontent.com/frankwei98/sms-relayed/main/install.sh | sh
```

The installer detects the platform, downloads the latest release, installs `/usr/bin/sms-relayed`, creates `/etc/sms-relayed`, registers a procd or systemd service, and opens the setup wizard when a TTY is available. It starts the service once a configuration exists.

If elevation is required, download and inspect the script first, then run it with `sudo sh install.sh`.

Installer variables:

| Variable | Default | Purpose |
| --- | --- | --- |
| `SMS_RELAYED_VERSION` | `latest` | Install a specific release tag. |
| `SMS_RELAYED_BIN_DIR` | `/usr/bin` | Binary installation directory. |
| `SMS_RELAYED_CONFIG_DIR` | `/etc/sms-relayed` | Configuration and default database directory. |
| `SMS_RELAYED_START` | `1` | Set to `0` to avoid starting the service. |
| `SMS_RELAYED_CONFIG_ONLY` | `0` | Set to `1` to skip binary installation and configure with an existing binary. |
| `SMS_RELAYED_UPDATE` | automatic/prompt | Set to `1` to replace an existing binary or `0` to skip it. |
| `SMS_RELAYED_ROOT` | empty | Write into an alternate root for image/offline filesystem preparation; services are not started. |

### Configuration

The default configuration path is `/etc/sms-relayed/config.toml`. The setup wizard is the recommended starting point:

```sh
sudo sms-relayed setup
sudo sms-relayed config check
sudo sms-relayed config show
```

Use the global `--config` option for another path:

```sh
sms-relayed --config /path/to/config.toml config check
```

Complete example:

```toml
[app]
device_name = "router-sim"
modem_path = "/org/freedesktop/ModemManager1/Modem/0"

[sms]
ignore_storage = ["sm"]
code_keywords = ["验证码", "verification", "code", "인증", "代码", "随机码"]

[forward]
enabled = ["bark.personal", "telegram.main"]

[delivery]
concurrency = 2

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

[api]
enabled = true
bind = "0.0.0.0"
port = 8080
enable_ipv6 = false
password = "change-this-password"
database_path = "/etc/sms-relayed/sms-relayed.sqlite"

[http]
connect_timeout_secs = 10
request_timeout_secs = 30
shell_timeout_secs = 30

[retention]
enabled = false
max_age_days = 90
batch_size = 500
```

Important rules:

- `forward.enabled` contains `channel.profile` references such as `telegram.main`; multiple profiles of the same channel are supported.
- Channel types are `bark`, `telegram`, `pushplus`, `wecom`, `dingtalk`, and `shell`.
- Bark uses [API v2](https://github.com/Finb/bark-server/blob/master/docs/API_V2.md): set `server_url` to the server root and sms-relayed sends JSON to `<server_url>/push`.
- `delivery.concurrency` controls concurrent forwarding jobs. It defaults to `2` and accepts values from `1` through `16`.
- A committed delivery wakes the worker immediately. The worker also scans on startup, keeps a fixed 30-second safety scan, and wakes precisely for the earliest `next_attempt_at` retry deadline.
- A non-empty `api.password` is required when `api.enabled = true`.
- The configuration contains credentials. Files written by sms-relayed are restricted to mode `0600`.
- Restart the service after changing the configuration.

### Web dashboard

With `[api]` enabled, open:

```text
http://<device-ip>:8080/
```

The dashboard provides inbox/outbox conversations, search and filters, read state, sending, deletion, streamed CSV/JSON exports, modem diagnostics and controls, recent forwarding results, configuration editing, validation, service restart, and live SSE updates. Forwarding timing separates queue `Dispatch` delay from channel `Request` latency; pre-migration dispatch history is shown as `—`.

Main API groups:

| Path | Purpose |
| --- | --- |
| `/api/auth/login`, `/api/auth/logout`, `/api/auth/me` | Session authentication. |
| `/api/messages`, `/api/conversations` | Message and conversation queries. |
| `/api/messages/send` | Send an SMS. |
| `/api/messages/export` | Export CSV or JSON. |
| `/api/events` | SSE event stream. |
| `/api/config`, `/api/config/check` | Read, save, and validate configuration. |
| `/api/status`, `/api/service/restart` | Service status and restart. |
| `/api/modem/*` | Modem status and controls. |
| `/api/forwarding/attempts` | Forwarding delivery status. |
| `/api/health` | Public service and modem health check. |

`/api/health` omits object paths, operator details, signal values, SIM identifiers, phone numbers, message bodies, and raw command output. The web service currently speaks plain HTTP. Do not expose it directly to the public Internet; use a trusted network, VPN, or a reverse proxy with TLS and access control.

### CLI

```text
sms-relayed [--config <path>]
sms-relayed [--config <path>] setup
sms-relayed [--config <path>] run
sms-relayed [--config <path>] send
sms-relayed update
sms-relayed [--config <path>] config check
sms-relayed [--config <path>] config show
```

- No subcommand opens the setup wizard on an interactive terminal and fails in non-interactive mode.
- `setup` opens the configuration wizard.
- `run` starts SMS monitoring, persistence, the delivery and retention workers, and the optional web API.
- `send` prompts for a recipient and message, asks for confirmation, sends it, and stores the result.
- `update` queries the latest release, verifies its matching SHA-256 file and embedded version/commit, atomically replaces the binary, and restarts a detected service.
- `config check` loads and validates the configuration.
- `config show` prints a redacted configuration summary.

Self-update supports the official Linux x86_64, aarch64, and armv7 assets. A build at the same commit is left untouched and the service is not restarted. Updating a system directory usually requires sufficient permissions, for example `sudo sms-relayed update`.

### Service management

OpenWrt:

```sh
/etc/init.d/sms-relayed enable
/etc/init.d/sms-relayed start
/etc/init.d/sms-relayed restart
/etc/init.d/sms-relayed status
logread | grep sms-relayed
```

systemd:

```sh
sudo systemctl enable --now sms-relayed
sudo systemctl restart sms-relayed
systemctl status sms-relayed
journalctl -u sms-relayed
```

### Shell forwarding arguments

A shell profile directly executes the configured executable script with these positional arguments:

```text
1. Sender phone number
2. Message timestamp
3. Message body
4. Detected verification code, or empty
5. Detected code source, or empty
6. Device name
```

The script is responsible for safely handling arguments and credentials, and must finish within `http.shell_timeout_secs`. Each value is passed as a separate command-line argument and is never reparsed by a shell.

### Build and development

Stable Rust, Node.js, and pnpm are required. Build frontend assets before the Rust release binary:

```sh
cd frontend
pnpm install --frozen-lockfile
pnpm build
cd ..
cargo build --release --locked
```

Useful checks:

```sh
cargo fmt --check
cargo test
cargo build

cd frontend
pnpm check
pnpm test
pnpm build
```

If `frontend/dist` is missing, `build.rs` creates a fallback page for Rust development builds. Always build the frontend before packaging a release.

### Error monitoring and privacy

The production backend and frontend use separate Sentry projects by default. Reports strip requests, users, breadcrumbs, host names, contextual variables, and exception text. SMS bodies, phone numbers, and configuration credentials are not sent, and repeated operational errors are rate-limited.

- Backend: override with `SMS_RELAYED_SENTRY_DSN`, or set it to an empty string to disable reporting.
- Frontend: override at build time with `VITE_SENTRY_DSN`, or set `VITE_SENTRY_ENABLED=false` to disable reporting.
- Frontend development mode does not send Sentry events.

### License and trademark

sms-relayed is licensed under [GNU GPL v3 only](LICENSE) (`GPL-3.0-only`). The project name, logos, and official release names are separately covered by the [Trademark Policy](TRADEMARKS.md). You may integrate the software into hardware and describe that inclusion truthfully, but modified or third-party builds must not be presented as official releases.

References:

- [Original C++ upstream project](https://github.com/lkiuyu/DbusSmsForwardCPlus)
- [ModemManager API](https://www.freedesktop.org/software/ModemManager/api/latest/)
- [zbus](https://docs.rs/zbus/latest/zbus/)
