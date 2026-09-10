# SimAdmin 参考实现证据索引

本文是 `01`—`06` 六份重建规格的跨文档证据入口。它记录的是固定版本 SimAdmin 的**可观察实现事实**，不把参考实现中的缺陷自动升级为 SmsRelayed 的目标设计。除特别说明外，下文形如 `backend/src/main.rs:511-1015` 的引用均相对于 `/tmp/SimAdmin`，且行号只对本文固定的 commit 有效。

## 1. 证据边界、版本与许可证

### 1.1 固定参考版本

| 项目 | 固定值 | 证据 |
|---|---|---|
| Git commit | `eb6f497ad332e59f1f3899acc3b2b9427661fe5c` | `git -C /tmp/SimAdmin rev-parse HEAD` |
| 分支 | `main`，跟踪 `origin/main`，ahead/behind 均为 `0` | `git -C /tmp/SimAdmin status --short --branch`；`git -C /tmp/SimAdmin rev-list --left-right --count HEAD...@{upstream}` |
| 工作区 | clean | `git -C /tmp/SimAdmin status --porcelain` 无输出 |
| 后端包 | `simadmin 1.1.10` | `backend/Cargo.toml:1-4` |
| Cargo workspace | Rust 2021；后端和 4 个本地 crate | `Cargo.toml:1-20` |

任何重新考证都应先验证 commit；若 `/tmp/SimAdmin` 已移动，不能继续沿用本文行号。

### 1.2 如何使用这些证据

本文采用三个标签：

- **兼容事实**：若目标是替换现有 SimAdmin 客户端，默认需要保留的 HTTP 方法、路径、字段或异步语义。
- **实现观察**：参考代码如何完成工作，可用于理解设备约束，但不代表推荐架构。
- **重建决策**：参考实现没有可靠定义、存在明显风险，或 `01`—`06` 已明确要求改进的地方；实现前必须显式定案。

证据优先级为：路由注册与后端实际代码 > 后端模型/配置/数据库定义 > 前端调用与类型 > README、环境和开发文档。后端模型位于 `backend/src/models.rs`，前端类型和客户端分别位于 `frontend/src/api/contracts.ts`、`frontend/src/api/current.ts`；第一方开发文档也明确了这三层契约来源（`docs/developer.md:109-137`）。

### 1.3 许可证事实与重建边界

参考仓库根许可证文本是 GNU GPL Version 3，Cargo 元数据声明 `GPL-3.0-or-later`；README 的“开源协议声明”还明确写出了衍生和分发时的源码、许可证等义务（`LICENSE:1-5`，`Cargo.toml:11-20`，`README.md:203-219`）。本文只记录这一事实，不给法律结论；重建者应自行确认目标产品、分发方式和源码复用方式的合规要求。

据此应区分：

- 从运行行为提炼 HTTP 契约、状态机和测试用例，是**行为兼容规格**；
- 复制或改编 Rust/TypeScript/脚本源码，是另一项需要单独评估的源码复用行为；
- 本索引只摘录符号、路径和少量必要语义，不复制大段源码。

## 2. 运行拓扑与模块映射

### 2.1 进程与依赖拓扑

SimAdmin 是一个以 root 运行的 Axum 后端，托管同目录 `www/` SPA，并通过 system D-Bus、ModemManager 和 NetworkManager 管理设备；默认二进制、SPA、数据库分别位于 `/opt/simadmin/simadmin`、`/opt/simadmin/www/`、`/opt/simadmin/data.db`（`README.md:39-46`，`docs/environment.md:1-39`，`scripts/simadmin.service:6-16`）。

后端启动时会连接 system D-Bus、打开 SQLite、加载 JSON 配置、确保 NetworkManager modem profile，然后启动通知、系统事件、设备状态、DDNS、OTA、SMS、数据连接、watchdog、统计和自动化等任务（`backend/src/main.rs:314-508`）。共享状态集中在 `AppState`，其中 cell lock 和若干用户意图只在内存中保存（`backend/src/state.rs:30-90`）。

### 2.2 功能到源码映射

| 规格 | 后端主入口 | 持久化/配置 | 前端与辅助文件 |
|---|---|---|---|
| `01` eSIM、SIM identity、切换后 SMS resync | `EsimSupervisor`、`run_lpac_command`（`backend/src/esim.rs:60-220,915-1007`）；eSIM/SIM handlers（`backend/src/handlers.rs:580-1357`）；identity、SMSC、本机号码、基带恢复（`backend/src/modem_manager.rs:723-832,1167-1890,5812-6244`）；SMS listener（`backend/src/sms_listener.rs:18-535`） | SIM/eSIM cache schema 与方法（`backend/src/db.rs:539-631,1684-2042`）；`WorkMode`/eSIM config（`backend/src/models.rs:39-45`，`backend/src/config.rs:1869-1915`） | `frontend/src/pages/SimCard.tsx`、`frontend/src/pages/EsimManager.tsx`；客户端调用（`frontend/src/api/current.ts:430-538`） |
| `02` 系统事件通知 | 事件目录、信封和 emitter（`backend/src/system_event.rs:13-106,461-662`）；资源采样器（`backend/src/system_event_monitor.rs:11-101`）；匹配、发送、队列 worker（`backend/src/notification.rs:69-240,700-785,974-1207`）；队列 API（`backend/src/notification_queue.rs:23-118`） | 通知日志/队列（`backend/src/db.rs:452-511`）；channel/rule/quiet hours/rate limit/retention（`backend/src/config.rs:301-629`） | `frontend/src/pages/NotificationCenter.tsx` 与 `frontend/src/pages/notifications/*`；客户端调用（`frontend/src/api/current.ts:924-990`） |
| `03` modem watchdog 与恢复 | 启动和 15 秒 watchdog（`backend/src/main.rs:446-466`）；阈值、发现、恢复、基带重启（`backend/src/modem_manager.rs:56-94,1081-1157,5812-6686`） | 恢复计数、cooldown、重启进度主要为进程内状态（`backend/src/modem_manager.rs:87-94,6256-6271`） | `scripts/simadmin-modem-recovery.service:1-11`、`scripts/simadmin-modem-recovery.sh:1-95`；基带状态客户端（`frontend/src/api/current.ts:574-582`） |
| `04` 自动化调度 | scheduler 与执行器（`backend/src/automation/scheduler.rs:20-245`）；handler registry（`backend/src/automation/tasks/mod.rs:10-35`）；4 个 task handlers（`backend/src/automation/tasks/*.rs`） | task 配置（`backend/src/config.rs:1446-1503`）；仅结果日志表（`backend/src/db.rs:656-673`） | `frontend/src/pages/AutomationCenter.tsx`、`frontend/src/pages/automation/*`；客户端调用（`frontend/src/api/current.ts:1145-1180`） |
| `05` 备份恢复 | 归档、校验、应用、local file handlers（`backend/src/backup.rs:31-770,898-1502,2059-2258`） | component 定义和 manifest（`backend/src/backup.rs:31-183,232-349`）；backup config（`backend/src/config.rs:1505-1647`） | `frontend/src/pages/BackupRestore.tsx`、`frontend/src/components/backup/BackupStorageSelector.tsx`；客户端调用（`frontend/src/api/current.ts:1054-1143`） |
| `06` 蜂窝网络 | 状态、cell、radio、band、data、roaming、airplane、operator、APN adapters（`backend/src/modem_manager.rs:233-4886,5574-5760`）；HTTP handlers（`backend/src/handlers.rs:1431-1946`） | APN/roaming/data intent 在 JSON 配置（`backend/src/config.rs:1843-1915`）；cell lock 仅内存（`backend/src/cell_lock_store.rs:1-68`） | `frontend/src/pages/Network.tsx`；客户端调用（`frontend/src/api/current.ts:540-705`） |

共享基础设施还包括统一 `ApiResponse<T>`（`backend/src/models.rs:8-37`）、全局设备串行锁 `with_serial`（`backend/src/serial.rs:1-28`）、SQLite wrapper（`backend/src/db.rs:226-229,381-690`）和所有路由的实际注册点（`backend/src/main.rs:511-1015`）。

## 3. HTTP 路由与契约来源

### 3.1 全局契约

- **兼容事实**：普通 JSON envelope 为 `{status,message,data?}`，业务成功使用 `status: "ok"`，业务失败使用 `status: "error"`（`backend/src/models.rs:8-37`）。
- **兼容事实**：除 health、hub provision 和 auth status/setup/login/logout 外，下面列出的设备 API 都在 `auth_middleware` 之后；CORS 当前允许任意 origin、method 和 header（`backend/src/main.rs:978-1018`）。
- **兼容事实**：不少业务错误仍返回 HTTP `200`，调用方必须同时检查 envelope；例如 eSIM command error、通知队列错误、cell-lock 校验错误和备份恢复错误（`backend/src/handlers.rs:78-84,1857-1875`，`backend/src/notification_queue.rs:23-118`，`backend/src/backup.rs:642-667`）。eSIM disabled/unavailable 是明确的 `403`/`503` 例外（`backend/src/handlers.rs:78-84`）。
- **重建决策**：若提供兼容层，可保留旧 envelope/HTTP 行为；新 API 应另行固化语义状态码、稳定 error code、operation ID 和幂等键，不能在同一端点里悄然改变错误判定规则。

### 3.2 新实现的跨文档统一契约

以下约束优先于 `01`—`06` 中未注明理由的局部写法；具体文档若确需不同契约，必须在该文档写出“覆盖项、理由、兼容影响和迁移方式”。它们是 SmsRelayed 的目标约束，不是 SimAdmin 已有行为：

1. **事件 registry**：统一使用 `GET /api/system-events/registry`。`/api/notifications/registry` 不是统一路径；`02` 或前端段若仍出现旧路径，应更正或显式标成 versioned compatibility alias。参考实现事实上没有任一路由（完整注册表：`backend/src/main.rs:511-1015`）。
2. **机器可判定字符串**：所有新增的 `status`、`state`、`error_code` 等单段 token 使用 ASCII `lower_snake_case`；`event_code`、`operation_type` 和 resource key 可以使用点号建立命名空间，但每一段仍必须是 `lower_snake_case`，例如 `delivery.profile_unhealthy`、`registration.manual`、`modem.control`。展示文本放在独立 `message`/`label` 字段。兼容 adapter 可继续解析参考实现的 `ok|error` envelope，但不能把自由文本当错误码（参考 envelope：`backend/src/models.rs:8-37`）。
3. **长操作**：eSIM profile switch、baseband recovery、operator scan/register、backup export/apply、危险 automation action 等不能用同步成功暗示完成。创建请求统一返回 HTTP `202 Accepted` 和可持久化的 operation resource，至少含 `operation_id`、`operation_type`、`status`、`created_at`、`updated_at`、可选 `progress`/`result`/`error_code`；REST 可重新读取最终状态。领域 `run`（例如 automation run）可以充当 operation resource，但不得再创建一份会漂移的平行状态。旧 eSIM enable 立即 HTTP 200 只作为兼容事实（`backend/src/handlers.rs:839-999`）。
4. **事实源**：SQLite 中的 durable event/operation/run 记录及其 REST 查询是事实源；SSE 只加速 UI 更新，断线、丢帧或服务重启后必须通过 cursor/REST 补读，不能让浏览器内存或 SSE 到达顺序决定最终状态。参考实现没有这层 durable operation/event source（`backend/src/system_event.rs:461-568`，`backend/src/db.rs:452-511,656-673`）。
5. **跨功能资源锁顺序**：需要多资源时，统一按 `system.lifecycle > modem.recovery > modem.control > modem.sms > storage.maintenance` 获取，释放顺序相反；禁止在持有较右侧资源时再申请较左侧资源。单一功能文档若要覆盖该顺序，必须给出避免死锁的证明和跨 worker 测试。参考实现只有单进程 `with_serial` 与独立 `lpac_lock`，没有这套 durable/typed lock hierarchy（`backend/src/serial.rs:1-28`，`backend/src/esim.rs:60-70,202-220`）。

### 3.3 `01` eSIM 与 SIM identity

| 方法与路径 | handler / 语义 | 契约证据 |
|---|---|---|
| `GET /api/sim` | 当前 SIM 概览 | 注册：`backend/src/main.rs:523-525`；后端模型：`backend/src/models.rs:292-317`；前端：`frontend/src/api/current.ts:519-523` |
| `POST /api/sim/details/refresh` | 触发 identity 详情刷新 | 注册：`backend/src/main.rs:527-529`；前端 timeout 2.5 秒：`frontend/src/api/current.ts:525-530` |
| `POST /api/sim/cache` | 写 SIM cache | 注册：`backend/src/main.rs:531-533`；请求：`frontend/src/api/current.ts:533-537` |
| `GET/POST /api/work-mode` | 查询/切换 `sim|esim` feature gate | 注册：`backend/src/main.rs:691-695`；`WorkMode`：`backend/src/models.rs:39-45` |
| `GET/POST /api/esim/config` | lpac 路径与内存覆盖配置 | 注册：`backend/src/main.rs:697-701`；配置：`backend/src/config.rs:1869-1884` |
| `GET /api/esim/lpac/status`、`POST /api/esim/lpac/repair` | 探测/安装 lpac | 注册：`backend/src/main.rs:703-709`；超时：`frontend/src/api/current.ts:470-481` |
| `GET /api/esim/euicc` | cached-first；`?live=1` 强制 live | 注册：`backend/src/main.rs:711-713`；前端 query：`frontend/src/api/current.ts:452-455`；handler：`backend/src/handlers.rs:689-750` |
| `GET/POST /api/esim/profiles` | 列表；download | 注册：`backend/src/main.rs:715-719`；前端 cached/live/download：`frontend/src/api/current.ts:458-468,507-512` |
| `POST /api/esim/profiles/{iccid}/enable` | path ICCID；立即返回“后台任务已启动” | 注册：`backend/src/main.rs:721-723`；后台 spawn 与 HTTP 200：`backend/src/handlers.rs:839-999` |
| `POST /api/esim/profiles/{iccid}/rename` | path ICCID + `{name}` | 注册：`backend/src/main.rs:725-727`；handler：`backend/src/handlers.rs:1001-1022`；前端：`frontend/src/api/current.ts:492-497` |
| `DELETE /api/esim/profiles/{iccid}` | path ICCID | 注册：`backend/src/main.rs:729-730`；handler：`backend/src/handlers.rs:1025-1052`；前端：`frontend/src/api/current.ts:500-504` |

### 3.4 `02` 系统事件通知

| 方法与路径 | 语义 | 契约证据 |
|---|---|---|
| `GET/POST /api/notifications/config` | channels、rules、templates、quiet hours、retention | `backend/src/main.rs:839-845`；`backend/src/config.rs:301-629`；`frontend/src/api/current.ts:924-933` |
| `POST /api/notifications/test/{channel}` | 测试单一 channel | `backend/src/main.rs:846-849`；`frontend/src/api/current.ts:935-939` |
| `GET /api/notifications/logs`、`POST .../logs/clear` | 查询/清理投递日志 | `backend/src/main.rs:851-858`；query 字段：`frontend/src/api/current.ts:941-958` |
| `GET /api/notifications/queue` | 查看重试队列 | `backend/src/main.rs:859-862`；`frontend/src/api/current.ts:961-966` |
| `POST .../queue/retry-all`、`POST .../queue/clear` | 批量操作 | `backend/src/main.rs:863-870`；`frontend/src/api/current.ts:980-989` |
| `POST .../queue/{id}/retry`、`DELETE .../queue/{id}` | 单项重试/删除 | `backend/src/main.rs:871-878`；`frontend/src/api/current.ts:968-977` |

参考实现没有 canonical event 查询 API，也没有 SSE 路由；`SystemEventEmitter` 直接把启用的事件送入 notification path（`backend/src/system_event.rs:532-568`，完整路由表 `backend/src/main.rs:511-1015`）。因此 `02` 中的事件历史、SSE replay、事件 ID 和恢复关联均是目标设计，不是兼容事实。

### 3.5 `03` watchdog 与恢复

| 方法与路径 | 语义 | 契约证据 |
|---|---|---|
| `POST /api/baseband/restart` | 人工启动基带恢复 | `backend/src/main.rs:682-684`；`frontend/src/api/current.ts:574-579` |
| `GET /api/baseband/restart/status` | 读取进程内全局步骤/状态 | `backend/src/main.rs:685-688`；`backend/src/modem_manager.rs:5812-5876` |

watchdog 本身没有 enable/config/status/history/cancel API；它在启动时固定以 15 秒间隔运行（`backend/src/main.rs:446-466`，路由表 `backend/src/main.rs:511-1015`）。`03` 中的 recovery operation、lease、审计历史和人工取消因此都是重建新增契约。

### 3.6 `04` 自动化

| 方法与路径 | 语义 | 契约证据 |
|---|---|---|
| `GET/POST /api/automation/config` | 整体读写 task 数组 | `backend/src/main.rs:879-885`；配置 union：`backend/src/config.rs:1446-1503`；前端：`frontend/src/api/current.ts:1145-1153` |
| `GET /api/automation/logs`、`POST .../logs/clear` | 查询/清理结果日志 | `backend/src/main.rs:886-893`；query：`frontend/src/api/current.ts:1162-1179` |
| `POST /api/automation/test/{task_id}` | 手工测试已配置 task | `backend/src/main.rs:894-897`；`frontend/src/api/current.ts:1156-1159` |

参考实现没有 definition CRUD、run registry、run detail、cancel/retry API；其持久化事实只有整体 JSON task 配置和完成后的日志行（`backend/src/config.rs:1446-1503`，`backend/src/db.rs:656-673`）。

### 3.7 `05` 备份恢复

| 方法与路径 | 语义 | 契约证据 |
|---|---|---|
| `GET /api/backup/options` | component 元数据 | `backend/src/main.rs:899-902`；`backend/src/backup.rs:46-190` |
| `GET/POST /api/backup/config` | schedule/cleanup/local storage 配置 | `backend/src/main.rs:903-908`；`backend/src/config.rs:1547-1647` |
| `POST /api/backup/export` | 返回 ZIP 二进制 | `backend/src/main.rs:909-912`；`frontend/src/api/current.ts:1069-1074` |
| `POST /api/backup/export-local` | 保存本地 ZIP | `backend/src/main.rs:913-916`；`frontend/src/api/current.ts:1076-1081` |
| `POST /api/backup/data/clear` | 清理选定组件 | `backend/src/main.rs:917-920`；`frontend/src/api/current.ts:1084-1089` |
| `POST /api/backup/import/preview`、`POST .../apply` | 上传预览/恢复；body 上限 50 MiB | `backend/src/main.rs:921-932`；query 与二进制调用：`frontend/src/api/current.ts:1092-1104` |
| `GET /api/backup/files` | 列本地备份与 pre-restore | `backend/src/main.rs:933-936`；`frontend/src/api/current.ts:1126-1128` |
| `GET .../files/{filename}/preview`、`POST .../apply` | 本地文件预览/恢复 | `backend/src/main.rs:937-944`；`frontend/src/api/current.ts:1107-1123` |
| `GET/DELETE /api/backup/files/{filename}` | 下载/删除 | `backend/src/main.rs:945-950`；`frontend/src/api/current.ts:1130-1142` |

### 3.8 `06` 蜂窝网络

| 能力 | 实际方法与路径 | 契约证据 |
|---|---|---|
| 只读状态 | `GET /api/network`、`/api/cells`、`/api/network/signal-strength`、`/api/location/cell-info` | `backend/src/main.rs:535-539,623-629`；响应模型：`backend/src/models.rs:182-220,320-329,719-750` |
| cell monitor | `POST /api/cell-monitor/start|stop` | `backend/src/main.rs:541-547` |
| radio/band | `GET/POST /api/radio-mode`、`GET/POST /api/band-lock` | `backend/src/main.rs:549-559`；模型：`backend/src/models.rs:331-384` |
| operator | `GET /api/network/operators`、**`GET /api/network/operators/scan`** | `backend/src/main.rs:631-637`；前端同样用 GET：`frontend/src/api/current.ts:645-651` |
| register | `POST /api/network/register-manual|register-auto` | `backend/src/main.rs:639-645` |
| APN | `GET/POST /api/apn` | `backend/src/main.rs:647-651`；模型：`backend/src/models.rs:773-800` |
| cell lock | `GET/POST /api/cell-lock`、`POST /api/cell-lock/unlock-all` | `backend/src/main.rs:653-661`；模型：`backend/src/models.rs:386-417` |
| user policy | `GET/POST /api/data`、`/api/roaming`、`/api/airplane-mode` | `backend/src/main.rs:663-680`；模型：`backend/src/models.rs:233-280` |

**兼容注意**：operator scan 是 GET，不是 POST；该 GET 可等待 D-Bus Scan 最多 45 秒并继续轮询 cache 20 秒，即使 Scan 失败/超时也可能返回旧 operators（`backend/src/modem_manager.rs:61-63,4324-4358`）。若新设计改为异步 operation，旧 GET 需保留为兼容 adapter 或明确版本化。

## 4. 持久化、表与配置来源

### 4.1 SQLite

数据库路径是可执行文件同目录的 `data.db`；连接被包在 `Arc<std::sync::Mutex<rusqlite::Connection>>` 中（`backend/src/main.rs:80-86,320-323`，`backend/src/db.rs:226-229`）。schema 不是编号 migration，而是在 `Database::new` 中执行 `CREATE TABLE IF NOT EXISTS` 和少量按列 `ALTER TABLE`（`backend/src/db.rs:381-678`）。

| 表 | 主要用途/字段 | 对应规格与证据 |
|---|---|---|
| `sms_messages` | 收发方向、号码、正文、时间、状态、notification/hub 状态、PDU marker | `01`,`02`,`04`,`05`；`backend/src/db.rs:386-434` |
| `hub_notification_events` | Hub 同步 outbox 风格事件；不是本地 canonical system event history | `02`；`backend/src/db.rs:435-449` |
| `notification_logs` | event/rule/channel/message/result | `02`,`05`；`backend/src/db.rs:452-474` |
| `notification_queue` | 已渲染 title/body、next attempt、attempt/max=5、error、状态 | `02`,`05`；`backend/src/db.rs:478-511` |
| `smsc_cache`、`own_number_cache`、`sms_storage_cache` | `identity_key` 主键下的 SIM 派生缓存 | `01`,`05`；`backend/src/db.rs:538-576` |
| `esim_profile_cache` | ICCID 主键以及 profile、matching ID、SM-DP、状态/权限字段 | `01`,`05`；`backend/src/db.rs:578-616` |
| `esim_euicc_cache` | EID、容量、manufacturer、`raw` | `01`,`05`；`backend/src/db.rs:618-631` |
| `auth_config`、`auth_sessions` | 密码散列/认证配置与 session hash/过期时间 | `05`；`backend/src/db.rs:633-654` |
| `automation_logs` | task ID/name/type、最终 status/detail/time；无 run ID/lease/attempt | `04`,`05`；`backend/src/db.rs:656-673` |

`call_history` 也在同库中，但不属于 `01`—`06` 本轮重建范围（`backend/src/db.rs:513-536`）。

### 4.2 JSON 配置

SimAdmin 配置不是 TOML。若 `/data` 存在，路径为 `/data/config.json`，否则回退到可执行文件同目录 `config.json`（`backend/src/config.rs:2317-2330`）。`AppConfig` 包含 notification、device network、security、roaming/data/APN、work mode/eSIM、automation、backup 等（`backend/src/config.rs:1886-1916`）。

重要配置来源：

- 通知渠道、规则、quiet hours、rate limit 和日志清理：`backend/src/config.rs:301-629`；
- automation triggers/actions：`backend/src/config.rs:1446-1503`；
- backup components/schedule/cleanup/local dir：`backend/src/config.rs:1505-1647`；
- APN username/password、lpac path：`backend/src/config.rs:1843-1884`。

配置读取/解析失败时进程记录 warning 后使用默认值；保存使用直接 `fs::write`，没有临时文件、rename、fsync 或显式权限设置（`backend/src/config.rs:2077-2112,2298-2313`）。重建不能把这一写入方式当作可靠性目标。

### 4.3 备份归档是另一层逻辑 schema

备份格式为 ZIP，`manifest.json` 记录 format version、app、component、record count、per-file SHA-256、创建时间、SimAdmin version 和敏感标志（`backend/src/backup.rs:31-44,232-249,898-969`）。component 是 `config`、`sms`、notification config/log/queue、automation config/log、SIM/eSIM cache、auth（`backend/src/backup.rs:46-152`）；默认不含 notification logs/queue、automation logs、auth（`backend/src/backup.rs:31-44,135-152`）。

## 5. 后台任务与并发模型

| 任务/资源 | 参考实现并发语义 | 证据与重建含义 |
|---|---|---|
| 全局设备写操作 | `with_serial` 是单进程全局 async mutex；许多 D-Bus/AT/NetworkManager mutation 在持锁期间 await | `backend/src/serial.rs:1-28`；这是 adapter 互斥，不是 durable lease，也没有 operation owner/cancel |
| lpac | `EsimSupervisor` 有独立 `lpac_lock`；与 `with_serial` 不是同一把锁 | `backend/src/esim.rs:60-70,202-220`；重建必须定义跨 eSIM、基带和蜂窝写操作的统一锁序 |
| eSIM enable | handler 重置一个全局 restart progress，spawn 后立即 HTTP 200；没有 operation ID 或原子“已在运行”拒绝 | `backend/src/handlers.rs:839-999`；重复请求可能竞争同一进度面板 |
| SMS listener/resync | 一个 unbounded mpsc handle 驱动单 listener；同时接收 D-Bus Added、15 秒 poll、显式 resync；modem path 变化后重绑 | `backend/src/sms_listener.rs:18-58,400-535` |
| modem watchdog | 每 15 秒循环；只在全局 baseband restart flag 为 true 时暂停；阈值/cooldown 都在内存 | `backend/src/main.rs:446-466`，`backend/src/modem_manager.rs:6256-6283` |
| system resource monitor | 每 60 秒采样；alarm counter/hysteresis 在内存，重启即丢失 | `backend/src/system_event_monitor.rs:11-101` |
| notification queue | 5 秒 tick，每批最多 20 项，每项 spawn；DB 条件更新抢占 `sending`，指数退避，最多 5 attempts | `backend/src/notification.rs:1059-1207`；未见启动时回收遗留 `sending` 的流程 |
| automation | 30 秒 tick；fixed minute dedupe 仅 `HashMap`；interval 查最后日志；每个到期任务独立 spawn | `backend/src/automation/scheduler.rs:20-125`；没有 durable due slot、lease、max concurrency 或资源锁 |
| backup/restore | handler 内同步构建/读写 ZIP 和 DB transaction；没有全局 maintenance barrier 暂停 SMS、通知、scheduler、watchdog | `backend/src/backup.rs:360-770,1336-1502`；`AppState` 没有 restore coordinator（`backend/src/state.rs:33-55`） |

## 6. 外部命令、设备和权限依赖

第一方环境文档要求 Debian/Linux、ARM64 或 x86_64、systemd、root、system D-Bus，并列出 ModemManager/mmcli、NetworkManager/nmcli、qmicli、libqmi/libmbim/libpcsclite、网络诊断和解压工具（`docs/environment.md:1-19`）。实际实现还应按下表核对：

| 依赖 | 使用位置 | 风险/前置条件 |
|---|---|---|
| ModemManager system D-Bus | modem/SIM/SMS/radio/band/operator/bearer 主接口 | 接口/属性随固件和 ModemManager 版本变化；第一方文档列出使用的 D-Bus interfaces（`docs/developer.md:162-207`） |
| `mmcli` | modem scan、信息/SMSC fallback、boot recovery | discovery 直接取排序后的第一个 modem（`backend/src/modem_manager.rs:1081-1157`）；多 modem 不具稳定绑定 |
| `qmicli`、`mbimcli`、AT 端口 | cell/SMSC/SIM fallback、profile power-cycle | helper timeout 和端口类型在 `backend/src/modem_manager.rs:56-78`；需 QMI/MBIM/AT 权限和设备节点 |
| `lpac` | eUICC/profile info/list/download/enable/rename/delete | 默认 `/opt/simadmin/lpac/lpac`；QMI APDU + curl HTTP；自动发现 `/dev/cdc-wdm*`/`/dev/wwan*qmi*`，默认 slot 1 和 `/dev/wwan0at0`（`backend/src/esim.rs:24-36,915-1080`） |
| `nmcli` / NetworkManager | GSM profile 创建、APN、连接/断开/autoconnect | 启动时无条件调用 profile ensure，可能删除旧 unmanaged 文件并重启 NetworkManager（`backend/src/main.rs:333-334`，`backend/src/modem_manager.rs:5574-5688`） |
| `systemctl` / systemd | MM/NM/service/OS 恢复 | 主服务为 root 且 `Restart=always`（`scripts/simadmin.service:6-16`）；启动还会写 MM debug override 并重启 ModemManager（`backend/src/main.rs:148-186,314-315`） |
| `ping`/`ping6`、`/sys/class/thermal` | 连通性和温度事件 | 目标硬编码为 `223.5.5.5`、`2400:3200::1`；温度依赖 Linux thermal sysfs（`backend/src/system_event_monitor.rs:447-470`） |
| boot recovery script | `journalctl`、`killall qmi-proxy`、`udevadm`、`logger`、remoteproc sysfs | 会清空 `/var/lib/ModemManager` 子项并 stop/start modem remoteproc；这是高权限、设备专用恢复动作（`scripts/simadmin-modem-recovery.sh:1-95`） |

## 7. 关键不变量、已知不一致与重建决策

### 7.1 应显式保留或提供兼容层的行为

1. eSIM 是插在物理卡槽中的 eUICC profile 管理开关，不是板级 SIM 通路切换，也没有常驻 eSIM worker（`backend/src/esim.rs:1-5,73-85`）。
2. eSIM enable/rename/delete 的 ICCID 在 path 中；enable 是 fire-and-observe，通过全局 baseband progress/status 观察，而不是同步完成（`backend/src/main.rs:721-730`，`backend/src/handlers.rs:839-999`）。
3. `/api/network/operators/scan` 是可能长时间等待的 GET；多数 API 的 HTTP 200 不代表业务成功（`backend/src/main.rs:631-637`，`backend/src/models.rs:8-37`，`backend/src/handlers.rs:78-84`）。
4. profile 真正切换成功后执行基带/SIM power-cycle，再请求 SMS resync；即使 recovery 失败也会请求一次 resync（`backend/src/handlers.rs:856-909`）。
5. radio mode/band/operator registration/APN/airplane/data 等设备 mutation 依赖序列化，空 band list 表示恢复全部支持频段（`backend/src/modem_manager.rs:3269-3290,3436-3467,4385-4417,4833-4886`）。

### 7.2 不能照搬为目标不变量的参考行为

1. **身份绑定**：`find_modem_path` 取排序后的第一个 modem；没有 USB path、设备序列、IMEI 等稳定 fingerprint 验证（`backend/src/modem_manager.rs:1081-1157`）。
2. **SMS 去重/转发差异**：无时间戳 marker 含 D-Bus object path；reconcile 还按号码+正文做 legacy 全局去重，可能吞掉合法重复短信。周期 poll 会转发新发现短信，显式 profile-switch resync 不转发（`backend/src/sms_listener.rs:78-103,192-288,497-521`）。
3. **eSIM operation 竞态与敏感输出**：enable 没有 durable operation/CAS；lpac JSON 解析错误会把完整 stdout 放进 error，cache 还保存 eUICC `raw`、matching ID 和 SM-DP（`backend/src/handlers.rs:839-999`，`backend/src/esim.rs:992-1007`，`backend/src/db.rs:578-631`）。
4. **事件不是事实流**：SystemEvent 没有 event ID、dedupe/recovery correlation 或本地事件表；未启用通知规则时 emitter 直接跳过事件（`backend/src/system_event.rs:461-568`）。
5. **通知可靠性**：队列存已渲染 title/body，没有 dedupe key；5 次尝试后失败，且未见进程重启回收 `sending` 的 lease（`backend/src/db.rs:478-511`，`backend/src/notification.rs:1059-1207`）。
6. **watchdog 用户意图冲突**：`data_user_disabled` 分支会清空 searching/radio recovery 计数并提前返回，导致“只关闭数据”同时停止注册恢复；这与 `03`/`06` 所需的控制面/数据面意图分离不一致（`backend/src/modem_manager.rs:6330-6348`）。
7. **硬恢复越权面大**：boot helper 固定等 60 秒，然后可能删除全部 ModemManager cache、kill qmi-proxy 和写 remoteproc sysfs；它不与主进程 lease、SMS resync、用户意图或稳定 modem identity 协调（`scripts/simadmin-modem-recovery.sh:12-95`）。
8. **scheduler 不耐重启**：fixed dedupe 在内存；interval 只看最终日志；并发 task 没有 lease/资源锁；`reboot_device` spawn 后立即返回成功，日志不能证明 reboot 完成（`backend/src/automation/scheduler.rs:20-125,208-245`，`backend/src/automation/tasks/device_reboot.rs:24-33`）。
9. **短信发送不具 exactly-once 证据**：automation SMS 在 modem send 返回后才写 DB；超时/崩溃落在“可能已发但未记账”的不确定窗口，重试可能重复（`backend/src/automation/tasks/send_sms.rs:94-145`）。
10. **备份不等于安全快照**：ZIP 只有 SHA-256 完整性，无加密/签名；可包含短信、APN、SIM/eSIM raw/identity 等敏感数据（`backend/src/backup.rs:124-133,232-249,898-1127`）。
11. **恢复非跨介质原子**：DB transaction 先提交，随后才写配置；配置写失败不会回滚 DB。pre-restore 固定为默认组件（可附 auth），可能没有覆盖本次所选 logs/queue；workers 没暂停（`backend/src/backup.rs:670-714,1336-1502`）。
12. **归档防御不足**：上传只限制压缩 body 50 MiB；解压把所有 entry 读入内存，没有累计展开大小/entry 数/压缩比限制；同名 entry 由 map 后写覆盖（`backend/src/main.rs:921-932`，`backend/src/backup.rs:1202-1275`）。
13. **cell lock 是 UI 假状态**：GET/POST 仅读写内存，不下发硬件，重启即丢失（`backend/src/cell_lock_store.rs:1-68`，`backend/src/handlers.rs:1846-1888`）。
14. **APN secret 暴露**：APN GET 返回 bearer username/password；POST 信任客户端 `context_path` 创建 D-Bus proxy；配置明文保存，NM 修改把凭据放入 `nmcli` argv（`backend/src/modem_manager.rs:4463-4568,4833-4886,5691-5718`，`backend/src/config.rs:1843-1867`）。
15. **启动默认接管设备**：进程启动会写 ModemManager override、可能重启 MM/NM、确保 GSM profile 并自动初始化数据连接；不能作为 SmsRelayed “安装后只观测、不接管”的默认行为（`backend/src/main.rs:148-186,314-334,425-444`，`backend/src/modem_manager.rs:5574-5688`）。
16. **配置和本地备份写入非原子**：两者都直接 `fs::write`；掉电可能留下部分文件（`backend/src/config.rs:2298-2313`，`backend/src/backup.rs:2059-2077`）。
17. **前端真相漂移**：Network 页面把 band 配置另存 localStorage，而真实 band 状态来自 modem；Backup 页面把实际 ZIP pre-restore 名称模拟成 `.db`（`frontend/src/pages/Network.tsx:91,135-148,274-299`，`frontend/src/pages/BackupRestore.tsx:877`）。

### 7.3 实现前必须固化的跨文档决策

- modem/SIM/eUICC 的稳定 fingerprint、换卡/换 profile 时 cache ownership 和多 modem 选择策略；
- 一把 durable resource lock 是否覆盖 eSIM、基带恢复、watchdog、automation、backup barrier 和人工蜂窝 mutation，以及锁序、租约、超时和取消；
- 旧 HTTP 兼容层与新 versioned API 的边界，特别是 HTTP 200 error envelope、scan GET、eSIM background enable；
- canonical event/run/recovery operation 的 ID、状态、重启恢复、SSE replay、retention 和敏感字段策略；
- SMS reconcile 是否通知、去重键、unknown-send-result 的处理和幂等边界；
- backup 加密/密钥托管、签名/真实性、展开限制、跨 DB+配置原子恢复、worker quiesce 和 rollback；
- NetworkManager 接管是否 opt-in，APN secret 是否 write-only，真实 cell-lock capability 的探测与“不支持”语义；
- root/systemd helper 的最小权限、设备 allowlist、remoteproc/cache 清理开关和审计；
- 引入或改编 GPLv3 参考源码前的合规确认。以上决策分别由 `01`—`06` 展开，本文只维持交叉约束。

## 8. `01`—`06` 覆盖与缺口清单

这里的“缺口”不是要求模仿 SimAdmin，而是 AI 重建前仍需由规格或 ADR 固化的选择。

### `01-esim-and-sim-identity.md`

- [x] 覆盖 eSIM feature gate、lpac 供应、Euicc/Profile 模型、SIM identity/cache、profile enable、基带恢复、SMS resync、敏感字段和分阶段验收。
- [x] 已在本文补齐实际 path ICCID、background enable、lpac 独立锁、cache table、周期 poll 与显式 resync 的转发差异证据。
- [ ] 仍需最终定案：stable modem/SIM/eUICC fingerprint；重复 enable 的 operation 幂等键；把文档内 `SimSwitchCoordinator -> lpac -> Modem -> SMS` 局部 gate 明确嵌入第 3.2 节全局资源顺序；所有 wire status/error code 的 `lower_snake_case` 映射；`raw`、matching ID、SM-DP 和 ICCID 的落盘/返回/日志策略；合法重复 SMS 的去重窗口。

### `02-system-event-notifications.md`

- [x] 覆盖事件信封、category/severity/status、阈值/hysteresis、规则/channel/quiet hours、队列重试、日志、SSE、隐私和测试。
- [x] 已在本文补齐“SimAdmin emitter 是通知桥而非事实流”、实际路由、queue schema/worker、rate limit 与无事件历史 API 的证据。
- [x] registry 路径已统一为 `GET /api/system-events/registry`；SQLite/REST 为事实源，SSE 只作加速。
- [ ] 仍需最终定案：event ID/dedupe key/recovery correlation；`sending` lease 回收；SSE cursor/replay/retention；已渲染正文与原始事件字段的敏感数据边界；批量 retry/clear 的 `202 operation` 与通用 operation schema 对齐。

### `03-modem-watchdog-and-recovery.md`

- [x] 覆盖探测、阈值、状态机、恢复阶梯、cooldown、路径漂移、SMS resync、用户意图、systemd helper 边界、事件和故障注入。
- [x] 已在本文补齐精确阈值来源、仅在 baseband flag 时暂停、data-off 提前绕过注册恢复、boot helper 清 cache/remoteproc 的证据。
- [ ] 仍需最终定案：主进程与 helper 的唯一 recovery lease；人工/自动恢复 admission 统一返回 `202 operation resource`（不是只返回裸 epoch）；`modem.recovery`/`modem.control`/`modem.sms` 与 `system.lifecycle` 的全局锁顺序；hard recovery capability/allowlist；失败后 SMS reconcile 的完成条件；人工取消；`data disabled`、`airplane`、`maintenance` 三种意图的正交状态；重启后的 cooldown/run 恢复。

### `04-automation-scheduler.md`

- [x] 覆盖 task/trigger/run 模型、时区、durable due slot/lease、资源锁、timeout/cancel/retry/idempotency、首批任务、API/UI、审计和测试。
- [x] 已在本文补齐 reference 的 30 秒 loop、北京固定时区、fixed 内存 dedupe、interval 最后日志、无并发上限、reboot 过早成功和 SMS unknown-result 证据。
- [x] `04` 已给出第 3.2 节同一套五级资源锁顺序；automation run 可作为长 action 的唯一 operation resource。
- [ ] 仍需最终定案：DST/catch-up/misfire policy；run uniqueness key；lease 续租/回收；run wire state/error code 的 `lower_snake_case` 约束；危险 action 的批准与 maintenance window；outbound SMS provider correlation；definition 版本化和删除后的历史保留。

### `05-backup-and-restore.md`

- [x] 覆盖 component archive/manifest、敏感数据、checksum、preview/apply、merge/replace、pre-restore、worker coordination、原子写入、CLI/API/UI、回滚和测试。
- [x] 已在本文补齐 actual route、50 MiB body、component/schema、未加密 ZIP、展开限制、DB 先提交再写配置、pre-restore 范围和 local `fs::write` 证据。
- [ ] 仍需最终定案：export/apply 的 domain operation 与通用 operation schema 对齐；quiesce/restart 同时需要 `storage.maintenance` 和 `system.lifecycle` 时遵守全局锁顺序；加密与密钥恢复；archive authenticity/signature；expanded bytes/entry count/ratio limits；一致性 snapshot 和 quiesce barrier；跨 SQLite+TOML/secret store 原子 cutover；自动 rollback 与旧版本 migration matrix。

### `06-cellular-network-management.md`

- [x] 覆盖只读能力到危险 mutation 的分层、ModemManager/NetworkManager/QMI/mmcli/AT adapter、用户意图、并发/rollback、API/config/UI、测试和分阶段交付。
- [x] 已在本文补齐 operator scan 实际为 GET、65 秒级等待/cache fallback、radio/band 的 D-Bus 真相、cell lock 仅内存、APN secret/path 风险、启动接管 NM 的证据。
- [ ] 仍需最终定案：cellular domain operation 与通用 operation schema/`lower_snake_case` 状态对齐；安装后 NetworkManager 接管是否 opt-in；capability discovery 与真实 cell lock vendor adapter；APN password write-only/secret store；客户端 bearer path allowlist；配置先写还是设备先写及补偿；与 watchdog/automation/eSIM 的资源冲突矩阵和全局锁顺序。

## 9. AI 重建时的最小取证顺序

1. 先固定本文第 1 节 commit，并阅读目标功能对应的 `01`—`06`；
2. 从 `backend/src/main.rs:511-1015` 建立旧 API contract tests；
3. 用 `backend/src/models.rs`、相关 handler 和 `backend/src/config.rs`/`db.rs` 核对字段与持久化，不以 TypeScript 类型替代后端事实；
4. 用 `frontend/src/api/current.ts` 核对浏览器真实 method/query/timeout，再用页面代码确认交互期望；
5. 对所有 modem mutation 检查 `backend/src/modem_manager.rs`、`backend/src/serial.rs`、`backend/src/esim.rs` 和 systemd helper，建立 capability/权限/失败矩阵；
6. 将第 7、8 节的未决项转成 ADR 或可执行验收测试后再编码；
7. 行为兼容测试和源码复用合规是两条独立工作流，不应互相替代。
