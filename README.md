# sms-relayed

sms-relayed 是一个面向 Linux 随身 WiFi、4G/5G 模组设备和 OpenWrt/Debian 网关的短信转发与短信发送工具。它通过 ModemManager 暴露的系统 D-Bus 接口实时监听新短信，并把短信内容转发到 企业微信、PlusPlus、Telegram、钉钉、Bark 或自定义 Shell 脚本；同时也可以通过命令行调用 ModemManager 发送短信。

当前实现是仓库根目录下的 Rust crate，使用 Tokio、zbus 和 reqwest 编写。

## 5W1H

| 问题  | 答案                                                                                                                                                                                           |
| ----- | ---------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| Who   | 适合把 SIM 卡插在随身 WiFi、软路由、OpenWrt/Debian 设备或 USB 蜂窝网卡里的用户。                                                                                                               |
| What  | 自动读取设备收到的短信，转发到多个通知渠道；也支持通过命令行发送短信。                                                                                                              |
| When  | 设备收到运营商短信、验证码、告警短信时；或需要远程让设备上的 SIM 卡发短信时。                                                                                                                  |
| Where | 运行在有 ModemManager 和系统 D-Bus 的 Linux 环境中，默认监听 `/org/freedesktop/ModemManager1/Modem/0`。                                                                                        |
| Why   | SIM 卡不在手机里时，短信容易被遗漏；把短信转成即时通知或 API 能降低维护成本。                                                                                                                  |
| How   | 监听 `org.freedesktop.ModemManager1.Modem.Messaging` 的 `Added` 信号，读取 `org.freedesktop.ModemManager1.Sms` 属性，再按配置调用各转发渠道；发送短信时调用 `Messaging.Create` 和 `Sms.Send`。 |

## 功能

- 实时监听 ModemManager 收到的新短信。
- 支持 PushPlus、企业微信自建应用、Telegram Bot、钉钉机器人、Bark、自定义 Shell 脚本。
- 支持一次转发到多个渠道，同一渠道可配置多组独立凭证。
- 支持命令行交互发送短信。
- 支持从验证码短信中提取验证码，并把验证码放进通知标题；Bark 渠道会在识别到验证码时开启自动复制。
- 支持按 ModemManager 的短信存储类型过滤不需要转发的短信。
- 支持 TOML 配置文件和多 profile 转发。
- 支持 OpenWrt procd 和 systemd 服务管理。

- 支持 SQLite 短信历史存储、Web API 管理后台、密码保护（P2 功能）。
- 默认 Web 管理地址：`http://<router-ip>:8080/`。

## Web API 和前端管理后台 (P2)

从 v1.1 开始，sms-relayed 包含一个可选的 Web API 和 React 前端管理控制台，用于短信历史管理、发送、搜索、导出和配置编辑。

### 启用

通过 `setup` 向导启用 Web API，或在配置文件中设置：

```toml
[api]
enabled = true
bind = "0.0.0.0"
port = 8080
enable_ipv6 = false
password = "your-password"
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

`api.enabled = true` 且 `api.password` 为空时，服务将拒绝启动。
`http` 和 `retention` 都可以省略并使用以上默认值。历史保留清理默认关闭；启用后只分批删除超过保留期且没有待投递任务的消息。

### 功能

- 密码登录，7 天会话 cookie（HttpOnly、SameSite=Lax）。
- SMS 收件箱/发件箱列表，按号码分组会话，未读计数。
- 搜索：手机号、正文全文、方向（接收/发送）、状态、未读、时间范围。
- 标记已读/未读，单条和批量删除。
- 以流式响应导出 CSV 或 JSON，避免将全部历史一次载入内存。
- 发送短信（通过 ModemManager）。
- 配置全量编辑：`app`、`sms`、`forward`、`channels`、`api` 各节。
- 配置验证（JSON 和 TOML 均支持）。
- 配置保存后需要重启服务。
- SSE 实时事件推送（新短信、状态更新、配置变更）。
- 前端资产嵌入在二进制文件中，无需独立 Web 服务器。

### Modem 健康与控制的运行时依赖

Web Modem 页面和 `/api/health` 的 modem 子状态使用 `mmcli`（ModemManager 命令行工具）。
SMS 收发仍然使用 ModemManager 系统 D-Bus，不受此依赖影响。

如果 `mmcli` 缺失，SmsRelayed 仍可正常启动，但 Web modem 状态/控制和健康检查会报告 `unknown`。
在目标设备上安装 `mmcli` 即可启用 Web modem 诊断和设备控制功能。

### 公开健康检查的安全说明

`GET /api/health` 公开返回 `service` 和 `modem` 子状态。
如果服务可在设备网络外访问，请仅在受信网络或置于访问控制之后暴露此接口。
公开响应已隐去 modem 对象路径、运营商名称、信号值、SIM 标识符、手机号、短信正文和原始命令输出。

### 路由

| 路径 | 方法 | 说明 |
|------|------|------|
| `/api/auth/login` | POST | 登录 |
| `/api/auth/logout` | POST | 登出 |
| `/api/auth/me` | GET | 当前认证状态 |
| `/api/messages` | GET | 消息列表（分页、筛选） |
| `/api/messages/send` | POST | 发送短信 |
| `/api/messages/:id/read` | POST | 标记已读 |
| `/api/messages/:id/unread` | POST | 标记未读 |
| `/api/conversations` | GET | 会话列表 |
| `/api/conversations/:phone_number/read` | POST | 标记号码下所有未读为已读 |
| `/api/messages/:id` | DELETE | 删除单条 |
| `/api/messages/delete` | POST | 批量删除 |
| `/api/messages/export` | GET | 导出（CSV/JSON，忽略分页 limit） |
| `/api/events` | GET | SSE 事件流 |
| `/api/config` | GET/PUT | 获取/保存配置 |
| `/api/config/check` | POST | 校验配置 |
| `/api/status` | GET | 运行时状态 |
| `/api/service/restart` | POST | 重启服务 |
| `/login` | - | 前端登录页 |
| `/config` | - | 前端配置编辑页 |
| `/modem` | - | 前端调制解调器状态和控制页 |
| `/api/health` | GET | 公开健康检查（service + modem 子状态） |
| `/api/modem/status` | GET | 调制解调器详细状态（需登录） |
| `/api/modem/enable` | POST | 启用调制解调器（需登录） |
| `/api/modem/disable` | POST | 禁用调制解调器（需登录） |
| `/api/modem/reset` | POST | 重置调制解调器（需登录，需 `{ "confirm": true }`） |

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

## 编译

```bash
cargo build --release
```

编译产物位于：

```text
target/release/sms-relayed
```

开发期也可以直接：

```bash
cargo run
```

## 命令行

```text
sms-relayed
sms-relayed setup
sms-relayed run
sms-relayed send
sms-relayed update
sms-relayed config check
sms-relayed config show
```

行为：
- `sms-relayed`：在交互式终端中进入设置向导。
- `sms-relayed setup`：始终进入设置向导。支持 Keep（保留现有配置）、Edit guided（引导编辑）、Replace from scratch（重建）、Cancel（取消）。
- `sms-relayed run`：启动短信转发服务。此命令非交互式，用于 init 系统启动。
- `sms-relayed send`：交互式发送短信。
- `sms-relayed update`：下载最新 GitHub Release，校验发布的 SHA-256 后原子替换已安装的二进制，并重启 OpenWrt 或 systemd 服务。目标路径依次取服务注册路径、`PATH` 中的 `sms-relayed`、当前可执行文件；符号链接会解析到真实文件。已是最新版时不会覆盖或重启。目前仅支持 Linux x86_64 和 aarch64。更新系统目录通常需要使用 `sudo`；未发现服务时只更新二进制并打印警告。
- `sms-relayed config check`：验证配置文件语法、profile 引用、必填字段、modem 路径格式。
- `sms-relayed config show`：打印脱敏后的配置摘要。

## 配置文件

默认配置文件路径：

```text
/etc/sms-relayed/config.toml
```

通过 `--config` 参数指定自定义路径。

配置格式：

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

### 配置规则

- `forward.enabled` 包含 `类型.名称` 格式的 profile 引用。
- 同类型渠道可以配置多个命名 profile。
- `modem_path` 支持自定义 ModemManager modem 路径。
- `ignore_storage` 支持数组，允许多个过滤值。
- 配置文件权限会自动限制为 600。

## Sentry 远程错误监测

发布版默认启用独立的 Rust 后端和 Web 前端 Sentry 项目。上报会移除请求、用户、breadcrumb、额外字段、主机名和异常消息正文，不发送短信正文、电话号码或配置凭据。

- 后端运行时可设置 `SMS_RELAYED_SENTRY_DSN` 覆盖默认 DSN；设为空字符串可关闭上报。
- 前端构建时可设置 `VITE_SENTRY_DSN` 覆盖默认 DSN；设置 `VITE_SENTRY_ENABLED=false` 可关闭生产构建中的上报。
- 前端开发模式不会发送 Sentry 事件。
- Sentry 仅用于错误与崩溃诊断，设备在线状态仍以 `/api/health` 和外部健康检查为准。

## 服务管理

### OpenWrt

```sh
/etc/init.d/sms-relayed enable
/etc/init.d/sms-relayed start
/etc/init.d/sms-relayed status
```

### systemd

```sh
systemctl enable --now sms-relayed
systemctl status sms-relayed
```

## 技术原理

### 收短信转发链路

1. 程序连接系统 D-Bus：`zbus::Connection::system()`。
2. 对 Modem 路径添加 signal match rule。
3. 监听接口 `org.freedesktop.ModemManager1.Modem.Messaging` 上的 `Added` 信号。
4. `Added` 信号会带出短信对象路径和 `is_received` 标记；程序只处理 `is_received = true` 的短信。
5. 程序对短信对象调用 `org.freedesktop.DBus.Properties.GetAll("org.freedesktop.ModemManager1.Sms")`，读取：
   - `Number`：发信号码
   - `Text`：短信正文
   - `Timestamp`：短信时间
   - `Storage`：短信存储类型
6. 如果短信正文暂时为空，程序会每 100ms 重试，最多约 60 秒，避免 ModemManager 刚发出信号但短信内容尚未填充。
7. 程序根据 ignore_storage 过滤存储类型，再把短信交给启用的 profile。

### 转发器链路

每条短信会被整理成统一字段：

- 发信电话
- 收信时间
- 转发设备名称
- 短信正文
- 验证码和验证码来源，如果能识别

不同渠道的实现不同：

- 企业微信先获取 access token，再调用应用消息发送接口，默认发给 `@all`。
- PushPlus 使用 `https://www.pushplus.plus/send`。
- Telegram 默认使用 `https://api.telegram.org`，也可配置自定义 Bot API 地址。
- 钉钉机器人使用 access token 和加签 secret。
- Bark 使用 `{server_url}/{key}/{content}`，识别到验证码时附带 `autoCopy=1` 和 `copy=验证码`。
- Shell 使用 `sh -c` 调用用户指定脚本，并传入 6 个参数。

### 发短信链路

1. 调用 `org.freedesktop.ModemManager1.Modem.Messaging.Create` 创建短信草稿。
2. 交互式发送会询问是否确认发送。
3. 调用短信对象上的 `org.freedesktop.ModemManager1.Sms.Send`。

### 验证码识别

程序用 `sms.code_keywords` 判断一条短信是否像验证码短信。默认关键字为：

```text
验证码, verification, code, 인증, 代码, 随机码
```

命中关键字后，会从正文中提取 4 到 7 位字母数字组合；如果存在多个候选值，会优先选择数字更多的候选值。短信来源会尝试从中文方括号中提取，例如 `【某服务】`。

## Shell 转发

Shell 模式会调用配置的脚本路径，并传入 6 个参数：

```text
1. telnum       发信电话号码
2. smsdate      短信时间
3. smscontent   短信内容
4. smscode      验证码，如果识别不到则为空
5. smscodefrom  验证码来源，例如【某服务】，如果识别不到则为空
6. devicename   转发设备名称
```

示例脚本见：

```text
ShellExample/sendbypushplus.sh
```

## 注意事项

- 默认 modem 路径为 `/org/freedesktop/ModemManager1/Modem/0`，可在配置中修改。
- 配置文件中保存了通知渠道 token 和机器人 secret，程序会自动限制文件权限为 600。
- 服务运行用户需要有系统 D-Bus 上的 ModemManager 访问权限；最简单的方式是用 `sudo` 或以 root 运行。
- P2 Web API 和前端管理后台已实现（v1.1+）：SMS 历史存储、搜索、导出、密码保护、配置编辑。

## License

sms-relayed is licensed under the GNU General Public License version 3 only
(`GPL-3.0-only`). See [LICENSE](LICENSE).

The "sms-relayed" name, project logos, and official release names are covered by
the separate [Trademark Policy](TRADEMARKS.md). You may include this software in
hardware products and may truthfully describe that inclusion, but modified or
third-party builds must not be presented as official sms-relayed releases.

## 参考

- [原 C++ 上游项目](https://github.com/lkiuyu/DbusSmsForwardCPlus)
- [ModemManager API 文档](https://www.freedesktop.org/software/ModemManager/api/latest/)
- [zbus](https://docs.rs/zbus/latest/zbus/)
