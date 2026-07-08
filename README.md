# sms-relayed

sms-relayed 是一个面向 Linux 随身 WiFi、4G/5G 模组设备和 OpenWrt/Debian 网关的短信转发与短信发送工具。它通过 ModemManager 暴露的系统 D-Bus 接口实时监听新短信，并把短信内容转发到 企业微信、PlusPlus、Telegram、钉钉、Bark 或自定义 Shell 脚本；同时也可以通过命令行或一个轻量 Web API 调用 ModemManager 发送短信。

当前实现是仓库根目录下的 Rust crate，使用 Tokio、zbus、reqwest 和 axum 编写。

## 5W1H

| 问题  | 答案                                                                                                                                                                                           |
| ----- | ---------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| Who   | 适合把 SIM 卡插在随身 WiFi、软路由、OpenWrt/Debian 设备或 USB 蜂窝网卡里的用户。                                                                                                               |
| What  | 自动读取设备收到的短信，转发到多个通知渠道；也支持通过命令行或 Web API 发送短信。                                                                                                              |
| When  | 设备收到运营商短信、验证码、告警短信时；或需要远程让设备上的 SIM 卡发短信时。                                                                                                                  |
| Where | 运行在有 ModemManager 和系统 D-Bus 的 Linux 环境中，默认监听 `/org/freedesktop/ModemManager1/Modem/0`。                                                                                        |
| Why   | SIM 卡不在手机里时，短信容易被遗漏；把短信转成即时通知或 API 能降低维护成本。                                                                                                                  |
| How   | 监听 `org.freedesktop.ModemManager1.Modem.Messaging` 的 `Added` 信号，读取 `org.freedesktop.ModemManager1.Sms` 属性，再按配置调用各转发渠道；发送短信时调用 `Messaging.Create` 和 `Sms.Send`。 |

## 功能

- 实时监听 ModemManager 收到的新短信。
- 支持 PushPlus、企业微信自建应用、Telegram Bot、钉钉机器人、Bark、自定义 Shell 脚本。
- 支持一次转发到多个渠道。
- 支持命令行交互发送短信。
- 支持 Web 页面和 `GET /api` 发送短信。
- 支持从验证码短信中提取验证码，并把验证码放进通知标题；Bark 渠道会在识别到验证码时开启自动复制。
- 支持按 ModemManager 的短信存储类型过滤不需要转发的短信。

## 技术原理

### 收短信转发链路

1. 程序连接系统 D-Bus：`zbus::Connection::system()`。
2. 对固定 Modem 路径 `/org/freedesktop/ModemManager1/Modem/0` 添加 signal match rule。
3. 监听接口 `org.freedesktop.ModemManager1.Modem.Messaging` 上的 `Added` 信号。
4. `Added` 信号会带出短信对象路径和 `is_received` 标记；程序只处理 `is_received = true` 的短信。
5. 程序对短信对象调用 `org.freedesktop.DBus.Properties.GetAll("org.freedesktop.ModemManager1.Sms")`，读取：
   - `Number`：发信号码
   - `Text`：短信正文
   - `Timestamp`：短信时间
   - `Storage`：短信存储类型
6. 如果短信正文暂时为空，程序会每 100ms 重试，最多约 60 秒，避免 ModemManager 刚发出信号但短信内容尚未填充。
7. 程序根据 `forwardIgnoreStorageType` 过滤存储类型，再把短信交给配置的转发器。

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
- Telegram 默认使用 `https://api.telegram.org`，也可以配置自定义 Bot API 地址。
- 钉钉机器人使用 access token 和加签 secret。
- Bark 使用 `{BarkUrl}/{BrakKey}/{content}`，识别到验证码时附带 `autoCopy=1` 和 `copy=验证码`。
- Shell 使用 `sh -c` 调用用户指定脚本，并传入 6 个参数。

### 发短信链路

命令行发送和 Web API 发送都走同一套 ModemManager 调用：

1. 调用 `org.freedesktop.ModemManager1.Modem.Messaging.Create` 创建短信草稿。
2. 命令行模式会询问是否确认发送；Web API 模式会直接发送。
3. 调用短信对象上的 `org.freedesktop.ModemManager1.Sms.Send`。

Web API 默认监听 `0.0.0.0:{apiPort}`，没有内置鉴权。不要直接暴露到公网；如需远程访问，建议放在内网、VPN、Cloudflare Access、反向代理鉴权或防火墙之后。

### 验证码识别

程序用 `smsCodeKey` 判断一条短信是否像验证码短信。默认关键字为：

```text
验证码±verification±code±인증±代码±随机码
```

命中关键字后，会从正文中提取 4 到 7 位字母数字组合；如果存在多个候选值，会优先选择数字更多的候选值。短信来源会尝试从中文方括号中提取，例如 `【某服务】`。

## 典型用例

### 1. 随身 WiFi / 软路由短信通知

SIM 卡插在随身 WiFi 或软路由里时，运营商余额、流量、停机、验证码短信不会出现在手机通知里。运行本程序后，可以把短信实时转发到 Bark、Telegram、企业微信等常用通知渠道。

### 2. 验证码集中接收

把低频登录、设备管理、运营商业务相关的验证码短信转发到个人通知渠道。识别到验证码后，通知标题会带上验证码；Bark 渠道还会尽量把验证码放进剪贴板自动复制字段。

### 3. 多渠道冗余转发

可以同时启用多个 `-f*` 参数，例如同时转发到 Bark、PushPlus 和钉钉。某个渠道失败时，其他渠道仍会继续尝试。

### 4. 远程发送设备 SIM 卡短信

开启 Web API 后，可以通过浏览器页面或 HTTP GET 让设备上的 ModemManager 发送短信。适合临时查询运营商业务、发送特定控制短信，或把短信发送能力接入自己的内网工具。

### 5. 自定义转发逻辑

如果内置渠道不够用，可以使用 Shell 模式。程序把短信内容和验证码信息交给脚本，由脚本自行调用任意 API，例如自建通知服务、飞书、Server 酱、MQTT、日志系统等。

## 目录结构

```text
.
├── Cargo.toml
├── Cargo.lock
├── src/
│   ├── main.rs              # CLI 入口和运行模式编排
│   ├── dbus.rs              # ModemManager D-Bus 监听与短信发送
│   ├── web.rs               # Web 页面和发送短信 API
│   ├── config.rs            # config.txt 读写
│   ├── smscode.rs           # 验证码识别
│   └── forward/             # 各转发渠道
├── ShellExample/            # Shell 转发脚本示例
└── README.md
```

## 运行要求

- Linux 系统。
- ModemManager 已安装并能识别蜂窝模组。
- 系统 D-Bus 可用。
- 程序运行用户有权限访问系统 D-Bus 上的 ModemManager；最简单的方式是用 `sudo` 运行。
- 网络能访问你启用的通知服务。
- 如需自行编译，需要 Rust toolchain。

可以先确认 ModemManager 是否能看到短信能力：

```bash
mmcli -L
mmcli -m 0
```

当前代码默认使用 `/org/freedesktop/ModemManager1/Modem/0`。如果你的 modem 不是 `0`，需要先调整源码里的 `MODEM_PATH`。

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
cargo run --release
```

## 快速开始

首次运行会在当前目录创建 `config.txt`，并按你选择的模式交互询问所需配置。

```bash
sudo ./target/release/sms-relayed
```

交互模式会让你选择：

1. 短信转发模式
2. 发短信模式
3. 短信转发模式并开启发短信 Web API
4. 只运行发短信 Web API

短信转发模式下再选择渠道：

1. PushPlus
2. 企业微信
3. Telegram Bot
4. 钉钉机器人
5. Bark
6. Shell 脚本
7. 自选多渠道

## 命令行参数

为了兼容原项目习惯，程序支持旧式短参数写法，例如 `-fB`；内部会转换成 clap 可识别的 `--fB`。

| 参数                            | 作用                   |
| ------------------------------- | ---------------------- |
| `-fP` / `--fP`                  | 启用 PushPlus 转发     |
| `-fW` / `--fW`                  | 启用企业微信转发       |
| `-fT` / `--fT`                  | 启用 Telegram Bot 转发 |
| `-fD` / `--fD`                  | 启用钉钉机器人转发     |
| `-fB` / `--fB`                  | 启用 Bark 转发         |
| `-fS` / `--fS`                  | 启用 Shell 脚本转发    |
| `-sS` / `--sS`                  | 进入命令行发短信模式   |
| `--configfile=/path/config.txt` | 使用自定义配置文件     |
| `--sendsmsapi=enable`           | 开启发短信 Web API     |

示例：

```bash
# Bark 转发，使用默认 config.txt
sudo ./target/release/sms-relayed -fB

# Bark + PushPlus + 钉钉多渠道转发
sudo ./target/release/sms-relayed -fB -fP -fD

# 使用指定配置文件
sudo ./target/release/sms-relayed -fB --configfile=/root/config.txt

# 转发短信，同时开启发短信 Web API
sudo ./target/release/sms-relayed -fB -fP --configfile=/root/config.txt --sendsmsapi=enable

# 只开启发短信 Web API
sudo ./target/release/sms-relayed --configfile=/root/config.txt --sendsmsapi=enable

# 命令行发送短信
sudo ./target/release/sms-relayed -sS
```

## 配置文件

默认配置文件是当前工作目录下的 `config.txt`。也可以用 `--configfile` 指定路径。

首次创建的配置项如下：

| 配置项                      | 说明                                                         |
| --------------------------- | ------------------------------------------------------------ |
| `pushPlusToken`             | PushPlus token                                               |
| `WeChatQYID`                | 企业微信企业 ID                                              |
| `WeChatQYApplicationSecret` | 企业微信自建应用 secret                                      |
| `WeChatQYApplicationID`     | 企业微信自建应用 agent id                                    |
| `TGBotToken`                | Telegram Bot token                                           |
| `TGBotChatID`               | Telegram chat id                                             |
| `IsEnableCustomTGBotApi`    | 是否启用自定义 Telegram Bot API，值为 `true` 或 `false`      |
| `CustomTGBotApi`            | 自定义 Telegram Bot API base URL，例如 `https://example.com` |
| `DingTalkAccessToken`       | 钉钉机器人 access token                                      |
| `DingTalkSecret`            | 钉钉机器人加签 secret                                        |
| `BarkUrl`                   | Bark 服务地址，例如 `https://api.day.app`                    |
| `BrakKey`                   | Bark key，沿用原项目字段拼写                                 |
| `ShellPath`                 | Shell 转发脚本路径                                           |
| `apiPort`                   | Web API 监听端口                                             |
| `ForwardDeviceName`         | 通知里的设备名；为空或 `*Host*Name*` 时使用系统 hostname     |
| `smsCodeKey`                | 验证码识别关键字，使用 `±` 分隔                              |
| `forwardIgnoreStorageType`  | 忽略转发的短信存储类型，默认 `sm`                            |

`forwardIgnoreStorageType` 支持的值包括 `unknown`、`sm`、`me`、`mt`、`sr`、`bm`、`ta`。如果设置为其他值，例如 `all`，当前实现会视为不过滤任何存储类型。

## Web API

开启方式：

```bash
sudo ./target/release/sms-relayed --configfile=/root/config.txt --sendsmsapi=enable
```

浏览器页面：

```text
http://设备IP:端口/
```

HTTP API：

```text
GET http://设备IP:端口/api?telnum=10010&smstext=1071
```

参数：

- `telnum`：收信号码
- `smstext`：短信内容

接口返回固定文本 `ok`。如果底层 D-Bus 发送失败，错误会写入日志，但当前 API 响应仍然是 `ok`。

## Shell 转发

Shell 模式会调用 `ShellPath` 指定的脚本，并传入 6 个参数：

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

使用前需要让脚本可执行，并在配置里填入脚本路径：

```bash
chmod +x /path/to/your-script.sh
sudo ./target/release/sms-relayed -fS --configfile=/root/config.txt
```

## 自启动示例

systemd 示例：

```ini
[Unit]
Description=sms-relayed
After=network-online.target ModemManager.service
Wants=network-online.target

[Service]
Type=simple
WorkingDirectory=/opt/sms-relayed
ExecStart=/opt/sms-relayed/sms-relayed -fB -fP --configfile=/etc/sms-relayed/config.txt --sendsmsapi=enable
Restart=always
RestartSec=5

[Install]
WantedBy=multi-user.target
```

OpenWrt 上也可以在启动脚本中后台运行，注意使用绝对路径和固定配置文件路径：

```sh
(/usr/bin/sms-relayed -fB --configfile=/root/config.txt > /tmp/sms-relayed.log 2>&1) &
```

## 注意事项

- 当前 Rust 代码默认 modem 路径固定为 `/org/freedesktop/ModemManager1/Modem/0`。
- Web API 没有鉴权，且监听 `0.0.0.0`，不要直接暴露公网。
- `config.txt` 会保存通知渠道 token 和机器人 secret，请限制文件权限。
- Bark key 字段沿用原项目拼写：`BrakKey`。
- 当前构建入口是仓库根目录的 `Cargo.toml`。

## 参考

- [原 C++ 上游项目](https://github.com/lkiuyu/DbusSmsForwardCPlus)
- [ModemManager API 文档](https://www.freedesktop.org/software/ModemManager/api/latest/)
- [zbus](https://docs.rs/zbus/latest/zbus/)
- [axum](https://docs.rs/axum/latest/axum/)
