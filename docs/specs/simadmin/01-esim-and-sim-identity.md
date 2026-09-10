# 功能 1：eSIM/eUICC 管理、SIM 身份缓存与 Profile 切换短信重同步

状态：设计 spec（不包含实现）
范围：SmsRelayed 单 ModemManager 调制解调器实例
参考基线：`/tmp/SimAdmin` clean `main@eb6f497ad332e59f1f3899acc3b2b9427661fe5c`

### 文档判读规则

- **已确认**：描述上述固定 commit 中可以由源码直接验证的行为。来源使用相对 `/tmp/SimAdmin` 的路径，并在第 17 节列出符号和行号。
- **目标契约**：SmsRelayed 重建时必须实现的行为。第 5 节及之后若无“需决策”标记，均属于目标契约，而不是声称 SimAdmin 已经如此实现。
- **重建建议**：给出适合 SmsRelayed 的默认值或内部结构；只要 API、状态机、不变量和验收结果等价，实现名称可以不同。
- **需决策**：参考实现没有答案、且会影响兼容性或安全边界的产品选择。实现前必须关闭第 16.1 节中的决策项，不能把建议值误记成参考事实。

## 1. 目标与非目标

### 1.1 目标

在 SmsRelayed 现有 Rust/Tokio、ModemManager D-Bus、`mmcli`、SQLite、受保护 HTTP API 和 React 前端架构上增加：

1. 可靠识别当前 eUICC/SIM、当前 Profile、ICCID、IMSI、本机号码和 SMSC，并按 SIM/Profile 缓存。
2. 在已安装 lpac 且硬件支持的情况下，读取 eUICC 信息、列出/下载/重命名/删除 Profile，并启用指定 Profile。
3. 把 Profile enable、基带恢复、ModemManager 重新枚举、Modem 路径重绑定和 SMS 重扫视为一个有界、可观察、可恢复的操作。
4. 在 Profile 切换后，以新的稳定 Modem 身份和新的 SMS 对象路径重建短信监听，补扫 ModemManager 中已经存在但尚未落库的收到短信。
5. 使所有可重试请求幂等，避免重复 enable、重复入库、重复转发和把旧 Modem 路径误当成新路径。
6. 将 EID、ICCID、IMSI、本机号码、SMSC、SM-DP+ 和 Matching ID 的存储、API、日志和前端展示限制在管理员范围内。

### 1.2 非目标

- 不实现物理 eSIM/eUICC 芯片、ModemManager、QMI/MBIM 驱动或 lpac 本身。
- 不把“eSIM 模式”误当作硬件切换；是否有 eUICC 由设备和 Modem 能力决定。
- 不在本功能中实现运营商开户、资费管理、自动续费、远程 SIM provisioning 平台或 QR 码服务。
- 不自动切换数据连接策略、NetworkManager profile 或 APN；恢复阶段只负责短信所需的 Modem 可用性，数据重连作为后续独立能力。
- 不在后台持续轮询 lpac；普通状态读取优先使用缓存，显式 live 请求和 Profile 操作才调用 lpac。
- 不为恢复失败自动启用旧 Profile。切换已经成功后，回滚 Profile 可能再次触发基带断连，必须由管理员明确操作。
- 本次只写 spec，不修改 Rust、前端、数据库或配置文件。

## 2. 术语和不变量

| 术语 | 定义 | 关键不变量 |
| --- | --- | --- |
| eUICC | 支持远程 Profile 的物理芯片 | EID 标识芯片，不等于当前 Profile |
| Profile | eUICC 上的运营商订阅 | ICCID 是 enable/delete 等操作的目标键 |
| SIM identity | 当前 SIM/Profile 的观察结果 | 至少以 `modem_fingerprint + ICCID` 隔离不同卡/Profile |
| Modem fingerprint | 由 SmsRelayed 现有 `ModemService` 从设备/设备标识计算的稳定指纹 | 运行时 D-Bus path 变化不能改变它 |
| runtime path | 当前可读 SMS 的 ModemManager D-Bus path | 可随重新枚举变化，不能作为持久身份 |
| action path | 已验证、允许执行 ModemManager 控制操作的 path | 必须和目标 fingerprint 匹配 |
| SMS resync | 在新 SIM identity/path 上重新扫描已有 SMS 对象 | 依赖现有消息持久化去重，不以对象 path 去重 |
| 切换代次 | 一次成功确认的新 identity 观察周期 | 一个代次最多执行一次同条件重扫 |

以下不变量必须始终成立：

- 没有通过稳定设备身份核验时，不得对重新枚举后的 Modem 执行控制操作。
- lpac enable 成功但基带恢复失败时，状态必须明确为“Profile 已可能切换、恢复未完成”，不能报告为完全成功。
- 当前 Profile 的身份缓存不能覆盖另一个 ICCID 的缓存；空的本机号码/SMSC 结果也可以是合法的负缓存，但必须有时间戳和来源。
- SMS 重扫必须复用现有 `Messaging::receive`/`Store` 的持久化和去重边界，不直接绕过转发策略写数据库。

## 3. SimAdmin 当前行为及来源文件

以下是从固定参考基线读取到的**已确认**现状，作为行为参考，不作为对 SmsRelayed 的现成依赖。

| 主题 | 当前行为 | 来源 |
| --- | --- | --- |
| 功能开关 | `work_mode = esim` 只开启 eSIM UI/API/lpac 调用；不会改变板卡硬件。普通 SIM 模式隐藏 eSIM 页并对 `/api/esim/*` 返回 403。 | `README.md`；`backend/src/config.rs`；`backend/src/esim.rs` |
| eSIM 进程边界 | `EsimSupervisor` 以一个 `lpac_lock` 串行化 lpac status、chip info、profile list、enable、nickname、delete、download 和 repair。没有后台 lpac worker。 | `backend/src/esim.rs` |
| lpac 状态 | 检查架构/glibc、私有 lpac 路径和 `driver list`；QMI APDU 与 curl HTTP driver 都可用才算 usable。 | `backend/src/esim.rs` |
| eUICC | `chip info` 有 20 秒超时；EID、厂商、内存、状态规范化后缓存。可用配置补充自定义总内存。 | `backend/src/esim.rs`；`backend/src/handlers.rs`；`backend/src/db.rs` |
| Profile 列表 | `profile list` 有 20 秒超时；规范化 ICCID、名称、运营商、state、class、IMSI、MSISDN、SMSC、SM-DP+、MCC/MNC、权限；Matching ID 从缓存补齐。 | `backend/src/esim.rs`；`backend/src/handlers.rs` |
| Profile enable | 先做 5 秒 preflight；目标已 active 时跳过；调用 `profile enable <iccid> <refresh_flag>`，默认 refresh=1，遇到宽泛的可重试字符串后等待 800ms、刷新列表，必要时以 refresh=0 重试。操作在后台任务中运行。 | `backend/src/handlers.rs`；`backend/src/esim.rs` |
| enable 后恢复 | enable 成功后停止 ModemManager，寻找 QMI 设备，执行 SIM power off/on，重新启动 ModemManager，轮询重新枚举、Modem state 和注册；注册超时只记 warning。 | `backend/src/modem_manager.rs`；`backend/src/handlers.rs` |
| 路径变化 | 恢复前后重新发现 Modem path；旧 path 不被假设仍有效。`find_modem_path` 会选择排序后的首个 path，并在空列表时触发 `mmcli --scan-modems`；它不核验稳定设备身份。 | `backend/src/modem_manager.rs` |
| SIM identity | 从 ModemManager SIM/3GPP 属性读取 ICCID、IMSI、运营商标识；本机号码和 SMSC 通过 ModemManager、QMI/MBIM、AT、EF_SMSP 等多路径探测。 | `backend/src/modem_manager.rs` |
| identity 缓存 | `smsc_cache` 优先以 ICCID、缺失时以 IMSI/operator key 缓存；`own_number_cache` 只在 ICCID 存在时写入。Profile/eUICC 另有 SQLite cache，profile 详情缺 MSISDN/SMSC 时从当前 identity cache 补齐。 | `backend/src/modem_manager.rs`；`backend/src/handlers.rs`；`backend/src/db.rs` |
| SMS 重同步 | `SmsResyncHandle::request_scan(reason)` 通过无界 channel 通知单一 SMS listener；listener 还每 15 秒扫描一次。切换成功和恢复失败都会请求扫描。参考实现没有 run id、计数、deadline 或合并键。 | `backend/src/state.rs`；`backend/src/sms_listener.rs`；`backend/src/handlers.rs` |
| 下载 | `profile download` 最长 120 秒；下载后读取规范化 Profile，失败时等待并多次 list，通过新 ICCID 差集补缓存；SM-DP+/Matching ID 会写入 profile cache。 | `backend/src/esim.rs`；`backend/src/handlers.rs`；`backend/src/db.rs` |
| lpac 供应链 | 安装器按架构/glibc 选择私有 lpac，检查 qmi/curl 和动态库；可探测 `/dev/cdc-wdm*`、`/dev/wwan*qmi*`，支持 `LPAC_APDU_QMI_DEVICE`；OTA 包不必包含或升级 lpac。 | `docs/environment.md`；`docs/install.md`；`backend/src/esim.rs` |
| 前端 | `EsimManager.tsx` 先读缓存再读 live；Profile enable 显示确认和基带恢复进度，每秒轮询 restart status；Profile 详情显示 ICCID、MSISDN、SMSC、IMSI、MCC/MNC、SM-DP+、Matching ID、ISDP-AID 和权限。 | `frontend/src/pages/EsimManager.tsx`；`frontend/src/api/current.ts` |
| 硬件边界 | “eSIM mode”只是功能/UI gate；物理 eUICC、QMI APDU、UIM slot、ModemManager 和运营商网络必须真实支持。 | `README.md`；`docs/environment.md` |

SimAdmin 的几个实现教训也应保留为约束：lpac 输出可能包含敏感 raw JSON，不能直接复制到 SmsRelayed；Profile enable、基带恢复和 SMS scan 不能只靠彼此独立的锁；重新枚举后必须重新验证稳定设备身份；空结果需要区分“已探测为空”和“尚未探测”。

### 3.1 已确认的 SimAdmin HTTP 兼容事实

SimAdmin 所有响应使用 `{ "status": "ok|error", "message": string, "data"?: T }` envelope。以下是参考实现的真实路由，而不是第 8 节的目标 API：

| 方法与路径 | 请求 | 成功响应/异步语义 | 已确认错误语义 |
| --- | --- | --- | --- |
| `GET /api/esim/lpac/status` | 无 | `200`，`data={installed,usable,path,arch,glibc_version,asset_name,message,source?}` | 非 eSIM mode 为 `403`；binary 不可用为 `503`；lpac command 类错误可能仍是 `200 + status=error` |
| `GET /api/esim/euicc?live=1` | `live` 缺省 false；无 live 时优先返回最新 cache | `200`，`data=EsimEuiccInfo`，其中包含 `raw` | 与上相同 |
| `GET /api/esim/profiles?cached=1` | `cached` 缺省 false；不是 `live` 参数 | `200`，`data={profiles:[EsimProfile]}` | cache 读取失败仍为 `200 + status=error` |
| `POST /api/esim/profiles/{iccid}/enable` | 空 JSON 可接受；ICCID 位于 URL | handler 立即返回 `200` 和“task started”，随后 `tokio::spawn`；没有 operation id、Idempotency-Key 或 per-request result | 后台 enable/recovery 失败不改变已返回的 HTTP 响应；结果只能从全局 baseband progress/通知推断 |
| `GET /api/baseband/restart/status` | 无 | `200`，`data={steps,running,current_registration?}`；这是进程内单例进度，不是持久化 operation | 查询本身不表达 enable 的最终业务状态 |
| `POST /api/esim/profiles/{iccid}/rename` | `{ "name": string }` | 同步 `200` + `EsimCommandResponse` | 空 name 为 `400`；lpac command failure 仍可能是 `200` envelope |
| `DELETE /api/esim/profiles/{iccid}` | 无 confirm body | 同步 `200`；命令成功后删除 cache | lpac command failure 仍可能是 `200` envelope |
| `POST /api/esim/profiles` | `{smdp,matching_id,confirmation_code?,imei?}` | 最长等待 lpac download；`200` + command response | provisioning 值参与缓存回填；业务失败多在 envelope/command code 表达 |
| `POST /api/esim/lpac/repair` | `{proxy_prefix?,asset_url?}` | 最长 120 秒，成功 `200` | 不可用/下载/安装错误按 `EsimApiError` 分流；自定义 asset URL 被接受 |

为兼容既有客户端可以保留一层 legacy adapter，但目标实现的唯一权威契约是第 8 节：敏感 ICCID 放 body、异步动作返回 `202 + operation_id`，业务错误使用真实 4xx/5xx。legacy adapter 也必须返回相同 operation 的引用，不能另起第二次 enable。

### 3.2 已确认的持久化与并发局限

- `esim_profile_cache` 以 ICCID 全局主键保存 IMSI、MSISDN、SMSC、SM-DP+、Matching ID 和 raw-derived 字段；没有 modem fingerprint/EID namespace、TTL 或加密。`esim_euicc_cache` 还保存 `raw` JSON。
- SMSC/本机号码 cache 接受空字符串/空列表作为已探测结果，但没有 `expires_at`；读取无法区分 fresh 与 stale。Profile cache 的 `COALESCE` upsert 会保留旧的非空敏感字段。
- `lpac_lock` 只串行 lpac 进程；基带恢复依赖另一把全局 serial lock，SMS resync 又是 channel。enable handler 没有 compare-and-set admission，多个请求可能先后排队 lpac、并共享同一个全局 progress buffer。
- `find_modem_path` 选排序后的第一个 Modem；QMI fallback 也选预设/排序后的第一个设备节点。参考实现没有 fingerprint 绑定，不能把这个选择算法当成目标安全行为。
- `run_lpac_command` 会把无法解析的完整 stdout 拼入错误，恢复命令还可能把 stdout/stderr 放入 progress/detail；这些都是需要修正的隐私边界，而不是要复刻的输出契约。

## 4. SmsRelayed 当前能力和缺口

### 4.1 已有能力

- `src/dbus.rs`、`src/dbus/inbound.rs` 和 `src/dbus/outbound.rs` 已提供系统 D-Bus SMS 接收/发送抽象、对象管理器快照、信号订阅和超时边界。
- `src/inbound.rs` 的 `InboundWorker` 会解析配置的 Modem path，订阅初始 SMS 和后续信号；监听断开后按退避重连，并通过 `ModemService` 的 runtime/action targets 区分可读 path 与可控 path。
- `src/modem.rs` 的 `ModemService` 已有 mmcli 能力探测、健康缓存、action lock、path drift candidate、设备身份提取、fingerprint 和启用/禁用/重置动作。
- `src/persistence` 的 `Store`/SQLite migrations 已保存消息、入站去重、转发状态和 Modem identity/fingerprint 元数据。
- `src/messaging.rs`、`src/inbound.rs` 已有统一的入库、转发、重试和 profile routing 边界；新增 reconcile 不应另写一套持久化路径。
- `src/api/mod.rs` 已有认证 API、消息/配置/服务/Modem 路由；`src/api/modem.rs` 已规定 action 进行中返回 409、需要已验证 action path，并有 reset 限流。
- `src/events.rs` 已有广播总线，但事件类型目前主要是消息、配置和服务事件；没有 eSIM、SIM identity、基带恢复或重扫事件。
- `src/config.rs` 已有 `app.modem_path`、SMS、delivery、forwarding、HTTP 等 TOML 配置，配置由 `Arc<AppConfig>` 提供给 API/worker；现有结构没有 eSIM/identity/cache section。
- `src/modem.rs` 的 `ModemDetails` 目前只有有限的 Modem 状态和一个 `own_number`，没有 EID、ICCID、IMSI、MSISDN、SMSC、Profile 列表。

### 4.2 缺口

1. 没有 lpac runner、eUICC/Profile domain model、QMI APDU 配置或 LPAC 供应链检查。
2. 没有 SIM identity、MSISDN、本机号码和 SMSC 的按 Profile 缓存；当前消息库也没有这些字段。
3. 没有 Profile enable 的全局操作协调器。现有 `ModemService.action_lock` 只覆盖已有 mmcli action，不覆盖 lpac、QMI power-cycle、入站 resync 或 eSIM download/repair。
4. 没有“停止 ModemManager—SIM power-cycle—重新枚举—恢复监听”的基带恢复能力。
5. `InboundWorker` 没有外部 resync channel；初始扫描/信号订阅在 path 变化后可重建，但没有由 Profile 切换明确触发的 reconcile generation。
6. 当前去重命名空间和运行时 fingerprint 能处理 Modem identity 变化，但新的 SMS object path 不能成为唯一去重依据；需为重扫定义跨 path 的稳定 key。
7. 没有 eSIM/SIM API、前端页面、进度事件和操作持久化。
8. 当前 health API 设计会主动隐藏 SIM ID/电话号码/SMS body；新增字段不能进入 public health、普通 metrics、URL access log 或未授权 SSE。

## 5. 推荐领域模型

建议新增一个以服务为边界的 `SimAdminService`（名称可按仓库惯例调整），内部由 `LpacClient`、`SimIdentityReader`、`BasebandRecovery`、`SimSwitchCoordinator` 和 `SmsResyncCoordinator` 组成。它们共享 `ModemService` 的稳定身份和 action target，不复制一套 Modem 选择逻辑。

### 5.1 `ModemBinding`

```text
ModemBinding {
    modem_fingerprint: String,     // 现有 Store/ModemService 的稳定指纹
    configured_path: String,        // AppConfig.app.modem_path
    runtime_path: String,           // 当前 D-Bus path，可变
    action_path: Option<String>,    // 已验证后才能控制
    qmi_device: Option<String>,     // 与 D-Bus path 分开管理
    observed_at: DateTime<Utc>,
}
```

`runtime_path`、`action_path` 和 QMI character device path 都是运行时定位信息，不作为 SIM identity 的主键。重新枚举后只替换 runtime/action binding，不修改用户的 configured path。

### 5.2 `SimIdentity`

```text
SimIdentity {
    modem_fingerprint: String,
    eid: Option<String>,
    iccid: Option<String>,
    imsi: Option<String>,
    msisdn: Vec<String>,
    smsc: Option<String>,
    operator_id: Option<String>,
    source: IdentitySource,
    observed_at: DateTime<Utc>,
    expires_at: DateTime<Utc>,
}
```

- ICCID 只保留数字并统一规范化；电话号码/SMSC 使用 E.164 或已有 ModemManager 可接受格式规范化。
- `identity_key` 使用带 domain separation 的稳定摘要，建议由 `modem_fingerprint + eid + iccid` 计算；缺失字段不能用另一个 Profile 的值填补。
- 同一个 eUICC 的多个 Profile 允许各有独立 identity cache；当前活跃 Profile 由当前 ICCID 和 ModemManager 状态确认，而不是由列表顺序推断。
- `source` 至少区分 `modemmanager`、`qmi`、`mbim`、`at`、`ef_sms`、`manual`、`cache`；API 返回来源和 age，便于诊断冲突。

### 5.3 `EsimProfile` 和 `Euicc`

Profile 字段分为：

- 操作必需：ICCID、normalized state、profile class、enable/disable/delete capability。
- 展示/身份：name、provider、IMSI、MSISDN、SMSC、MCC、MNC、ISDP-AID。
- provisioning：SM-DP+ 和 Matching ID；默认不持久化明文，见第 8 节。

`Euicc` 包含 EID、manufacturer、state、memory total/available、`updated_at` 和 capability flags。lpac 未提供的字段为 `unknown`，不得从旧 Profile 猜测。

Profile state 建议统一为 `active`、`inactive`、`disabled`、`error`、`unknown`；保留 lpac 原始数值/字符串作为受限诊断字段，但不把 raw JSON 原样返回。

### 5.4 `EsimSwitchOperation`

```text
EsimSwitchOperation {
    operation_id: UUID,
    idempotency_key_hash: String,
    modem_fingerprint: String,
    target_iccid: String,
    previous_iccid: Option<String>,
    phase: Preflight | LpacEnable | ModemRecovery | Rebind | SmsResync | Succeeded | Failed | Degraded,
    status: Queued | Running | Succeeded | Failed | Degraded,
    previous_path: Option<String>,
    current_path: Option<String>,
    resync_run_id: Option<UUID>,
    started_at: DateTime<Utc>,
    finished_at: Option<DateTime<Utc>>,
    error_code: Option<String>,
}
```

`target_iccid` 只在受保护存储中保存；日志和事件使用 operation ID 与摘要。`Degraded` 表示 lpac enable 已返回成功或新 Profile 已确认 active，但 Modem 恢复/监听重绑定未完成。

## 6. 配置建议

在 `src/config.rs` 的现有 TOML serde 结构中添加带默认值的 section，不引入第二套 JSON 配置，也不要求修改现有 `app.modem_path` 语义。建议配置如下，数值是初始默认值：

```toml
[esim]
enabled = false
lpac_path = "/opt/sms-relayed/lpac/lpac"
qmi_device = ""                 # 空值表示按受限探测规则寻找
qmi_uim_slot = 1
lpac_http_driver = "curl"
lpac_apdu_driver = "qmi"
allow_lpac_repair = false        # 安装/升级仍需显式管理员动作
profile_list_timeout_secs = 20
profile_enable_timeout_secs = 60
profile_download_timeout_secs = 120
switch_overall_timeout_secs = 240
recovery_timeout_secs = 120

[sim_identity]
live_timeout_secs = 3
cache_ttl_secs = 900
negative_cache_ttl_secs = 300
manual_cache_enabled = false

[sms_resync]
enabled = true
forward_reconciled = false
reconcile_window_secs = 86400
resync_timeout_secs = 60
```

实现时应：

- 对已有配置提供 serde default，旧配置无需手工改写；缺失 `esim.enabled` 时保持关闭。
- `lpac_path`、QMI device、UIM slot 和 timeout 在启动时做语法校验；路径必须是绝对路径、不可包含 shell 片段。
- 不把 Matching ID、confirmation code、API token 放进长期配置；下载请求只接收 HTTPS API body 中的短生命周期值。
- `esim.enabled` 是功能 gate，不代表设备有 eUICC；硬件不满足时 API 返回 `esim_unavailable`。
- 配置保存沿用现有服务重启/配置事件语义；正在运行的切换操作不因普通配置 reload 改变 timeout 或设备目标。

## 7. SQLite 数据模型

SmsRelayed 当前由 `src/persistence`/SQLite migrations 管理消息和 delivery 状态；本功能应新增 migration，不在启动代码中散落 `CREATE TABLE IF NOT EXISTS`，也不改写既有消息表的历史数据。

### 7.1 `sim_identity_cache`

```sql
CREATE TABLE sim_identity_cache (
    identity_key TEXT PRIMARY KEY,
    modem_fingerprint TEXT NOT NULL,
    eid TEXT,
    iccid TEXT,
    imsi TEXT,
    msisdn_json TEXT NOT NULL DEFAULT '[]',
    smsc TEXT,
    operator_id TEXT,
    source TEXT NOT NULL,
    observed_at TEXT NOT NULL,
    expires_at TEXT NOT NULL,
    updated_at TEXT NOT NULL,
    UNIQUE (modem_fingerprint, eid, iccid)
);
CREATE INDEX sim_identity_cache_current_idx
    ON sim_identity_cache(modem_fingerprint, iccid, updated_at DESC);
```

SQLite 的 `NULL` unique 语义需要 migration/写入层明确处理缺失 EID/ICCID，不能依赖这条 unique 约束完成去重。空号码/SMSC 必须以有效的 cache row 表示；读取时按 `expires_at` 区分 fresh、stale、negative 和 missing。

### 7.2 `esim_euicc_cache`

```sql
CREATE TABLE esim_euicc_cache (
    cache_key TEXT PRIMARY KEY,
    modem_fingerprint TEXT NOT NULL,
    eid TEXT NOT NULL,
    manufacturer TEXT,
    status TEXT,
    memory_total_kb INTEGER,
    memory_available_kb INTEGER,
    memory_total_customizable INTEGER NOT NULL DEFAULT 0,
    capabilities_json TEXT NOT NULL DEFAULT '{}',
    source TEXT NOT NULL,
    updated_at TEXT NOT NULL
);
```

不要保存 lpac 原始 stdout/stderr；若确实需要诊断，保存经过 allowlist 和脱敏的 JSON，并设置短保留期。

### 7.3 `esim_profile_cache`

```sql
CREATE TABLE esim_profile_cache (
    iccid TEXT PRIMARY KEY,
    modem_fingerprint TEXT NOT NULL,
    eid TEXT,
    name TEXT,
    provider TEXT,
    profile_class TEXT,
    state TEXT NOT NULL,
    imsi TEXT,
    msisdn TEXT,
    smsc TEXT,
    mcc TEXT,
    mnc TEXT,
    isdp_aid TEXT,
    disable_allowed INTEGER NOT NULL DEFAULT 0,
    delete_allowed INTEGER NOT NULL DEFAULT 0,
    source TEXT NOT NULL,
    updated_at TEXT NOT NULL
);
CREATE INDEX esim_profile_cache_modem_idx
    ON esim_profile_cache(modem_fingerprint, state, updated_at DESC);
```

SM-DP+/Matching ID 不放在默认 profile cache；如产品确实要求离线显示，必须使用独立加密 secret store/密钥，并在备份、导出和 API 中明确排除。

### 7.4 `esim_switch_operations` 和 `sms_resync_runs`

```sql
CREATE TABLE esim_switch_operations (
    operation_id TEXT PRIMARY KEY,
    idempotency_key_hash TEXT NOT NULL,
    modem_fingerprint TEXT NOT NULL,
    target_iccid TEXT NOT NULL,
    previous_iccid TEXT,
    phase TEXT NOT NULL,
    status TEXT NOT NULL,
    previous_path TEXT,
    current_path TEXT,
    resync_run_id TEXT,
    error_code TEXT,
    error_detail_redacted TEXT,
    started_at TEXT NOT NULL,
    finished_at TEXT
);
CREATE UNIQUE INDEX esim_switch_idempotency_idx
    ON esim_switch_operations(modem_fingerprint, idempotency_key_hash);

CREATE TABLE sms_resync_runs (
    run_id TEXT PRIMARY KEY,
    operation_id TEXT,
    reason TEXT NOT NULL,
    identity_key TEXT,
    modem_fingerprint TEXT NOT NULL,
    path_before TEXT,
    path_after TEXT,
    scanned_count INTEGER NOT NULL DEFAULT 0,
    persisted_count INTEGER NOT NULL DEFAULT 0,
    forwarded_count INTEGER NOT NULL DEFAULT 0,
    status TEXT NOT NULL,
    error_code TEXT,
    started_at TEXT NOT NULL,
    finished_at TEXT
);
```

操作表用于重启/前端刷新后显示最后状态，不用于恢复一个不确定的外部 lpac 操作。启动时只能把 `Queued/Running` 标为 `unknown`/`degraded`，再做安全的只读 profile/identity probe；不得自动重复 enable。

### 7.5 与现有消息去重的关系

- 重扫必须调用现有 `Messaging::receive`，让已有消息表、delivery 和 forwarding profile 逻辑继续生效。
- 对 reconcile 生成一个不依赖 D-Bus SMS object path 的 `reconcile_dedupe_key`，至少基于 `modem_fingerprint + received timestamp + normalized sender + normalized body + storage` 的 domain-separated hash；若 Modem 提供稳定 SMS ID，优先加入稳定 ID。
- 旧消息已存在时只增加 `last_seen`/resync 统计，不重新创建消息和 delivery。
- 默认 `forward_reconciled = false`：重扫的未知短信落库但不把历史 SMS 批量转发。若开启，只有本次新落库且通过现有 forwarding policy 的记录才允许进入 delivery queue。
- 日志只输出计数、operation ID、hash 前缀和错误 code，不输出 body、完整号码或对象 path。

## 8. API 与前端交互

### 8.1 API

沿用 `src/api/mod.rs` 的 protected/authenticated router、`ApiError` 和 HTTP 状态码风格；所有新 API 均禁止挂到 public health。

目标 API 不沿用 SimAdmin 的 `status/message/data` envelope。成功直接返回资源 JSON；错误严格沿用 SmsRelayed 当前结构：

```json
{
  "error": {
    "code": "profile_not_found",
    "message": "target profile was not found"
  }
}
```

`message` 必须是可展示、已脱敏的稳定文案；客户端只按 `code` 和 HTTP status 分支。异步写操作响应统一包含 `operation_id`、`status`、`phase` 和相对 `poll` URL；查询 operation 不需要客户端保留敏感目标值。

| 方法 | 路径 | 成功契约 | 主要错误 |
| --- | --- | --- | --- |
| GET | `/api/sim/identity?live=false&reveal=false` | `200 SimIdentityView`，含 `cache_state=fresh|stale|negative|missing`、`age_seconds`、逐字段 source/observed_at；`reveal=false` mask 敏感值 | `400` 参数；`503 modem_unavailable`；live deadline 为 `504 identity_probe_timeout` |
| POST | `/api/sim/identity/refresh` | body `{}`；同步能在 live deadline 内完成时 `200 SimIdentityView`，否则如实现为后台 operation 则 `202` | 已有刷新可返回相同 operation；请求不等价时 `409 operation_in_progress` |
| GET | `/api/esim/lpac/status` | `200 LpacStatusView`，返回安装、架构、driver、可用性和 safe reason code | gate 关闭 `403 esim_disabled`；不可用本身仍可是 `200 usable=false`，probe 执行失败为 `503` |
| GET | `/api/esim/euicc?live=false&reveal=false` | `200 EuiccView`；cache miss 且 `live=false` 返回 `200` + `cache_state=missing`，不隐式调用 lpac | live deadline `504 lpac_timeout`；硬件不支持 `503 esim_unavailable` |
| GET | `/api/esim/profiles?live=false&reveal=false` | `200 {items:[EsimProfileView],cache_state,updated_at}`；`live=true` 才调用 lpac，并由后端标记 active | 同上；解析失败 `502 lpac_invalid_response` |
| POST | `/api/esim/profile-actions/enable` | body `{ "iccid": "...", "confirm": true }`；要求 `Idempotency-Key`，返回 `202 OperationAccepted`；已 active 返回 `200` 的 terminal operation view | `400 invalid_sensitive_input`；`404 profile_not_found`；`409` action/idempotency conflict；`503/504` preflight |
| POST | `/api/esim/profile-actions/download` | body `{smdp,matching_id,confirmation_code?,imei?,confirm:true}`；`202 OperationAccepted`，响应永不回显凭据 | `400` validation；`409` conflict；网络/lpac 故障作为 operation final error；只有 admission 失败直接 4xx/5xx |
| POST | `/api/esim/profile-actions/nickname` | body `{iccid,name}`；可同步 `200 EsimProfileView` 或统一 `202`，但选定后须固定；与 enable 共享 gate | `400/404/409/503` |
| POST | `/api/esim/profile-actions/delete` | body `{iccid,confirm:true}`；`202 OperationAccepted`；active profile 拒绝 | `400` confirm/validation；`404`；`409 active_profile|operation_in_progress` |
| GET | `/api/esim/operations/{operation_id}` | `200 EsimSwitchOperationView`，返回 phase/status/step timestamps、path 是否 verified、resync 统计和 safe next action | `404 operation_not_found`；无权访问用既有认证错误 |
| GET | `/api/sms/resync/runs/{run_id}` | `200 SmsResyncRunView`，返回计数、status、identity generation digest 和 error code | `404` |
| POST | `/api/sms/resync` | body `{reason:"manual"}` + `Idempotency-Key`；`202 {run_id,status}`；同一 fingerprint + generation 的相同请求返回同一 run | `409` 不等价冲突；`503` listener unavailable |
| GET | `/api/events` | 增加 `esim.operation.updated`、`sim.identity.updated`、`sms.resync.updated`，事件只携带 ID、阶段、状态、计数和脱敏摘要 | 继续使用现有 session/SSE 错误语义 |

建议错误码：`esim_disabled`（403）、`esim_unavailable`、`lpac_unusable`、`lpac_timeout`、`profile_not_found`、`profile_already_active`、`modem_action_in_progress`、`modem_identity_unverified`、`baseband_recovery_failed`、`sms_resync_failed`、`operation_in_progress`（409）、`idempotency_conflict`（409）、`invalid_sensitive_input`（400）。lpac stderr 只进入内部 redacted detail，不能成为 API message 原文。

`profile_already_active` 不作为错误返回；它只能作为 terminal operation 的 `result_code`，HTTP 为 200。`baseband_recovery_failed` 和 `sms_resync_failed` 通常是已接受 operation 的最终状态，poll 返回仍为 200 且 `status=degraded|failed`；它们只在同步 recovery-only 请求无法 admission 时映射 503。这样客户端不会把“查询成功”与“操作成功”混为一谈。

启用请求返回示例：

```json
{
  "operation_id": "uuid",
  "status": "queued",
  "phase": "preflight",
  "poll": "/api/esim/operations/uuid"
}
```

### 8.2 前端

在当前 `frontend/` 的 React + TanStack Router + Tailwind/shadcn 结构中新增 eSIM/SIM 管理页；不要复制 SimAdmin 的 MUI 页面，也不要手写 `routeTree.gen.ts`。新增路由后使用现有 route generator 生成。

页面交互：

1. SIM identity 卡片显示当前/缓存状态、更新时间、来源和脱敏 EID/ICCID/IMSI/MSISDN/SMSC；“刷新”是显式 live 请求，并显示单独 timeout/不可用原因。
2. Profile 列表先加载缓存，再按用户点击或过期状态加载 live；active profile 由后端标记，前端不自行根据列表顺序决定 active。
3. enable 按钮必须先显示“会造成 ModemManager 短暂不可用并触发短信重扫”的确认；active profile 的重复点击显示幂等成功，不启动基带恢复。
4. enable 后切换到 operation progress：preflight、lpac enable、Modem recovery、重新绑定、SMS resync。前端可用 operation polling，SSE 只作加速通知；刷新页面后根据 operation ID 恢复显示。
5. `Succeeded`、`Failed`、`Degraded` 分开呈现。`Degraded` 必须提示“Profile 可能已切换，请检查 Modem/短信监听”，提供只读 refresh/recovery retry，不提供自动旧 Profile 回滚。
6. Matching ID、confirmation code 等 provisioning 字段默认不展示；复制按钮必须是显式管理员动作且不写剪贴板日志。全量号码只在受保护详情中显示，列表始终 mask。
7. lpac 不可用时显示硬件/依赖诊断，不显示可执行命令的原始 stderr；repair/install 若纳入后续阶段，必须单独确认和权限。

## 9. Profile enable 工作流

### 9.1 并发控制和锁顺序

新增一个按 `modem_fingerprint` 的 `SimSwitchCoordinator`，同时保护以下完整区段：

```text
preflight -> lpac enable -> ModemManager/QMI recovery -> identity rebind -> SMS resync
```

要求：

- 同一 fingerprint 同时最多一个 Profile enable、download、delete、rename、lpac repair 和 baseband recovery；读 cache 可并发，live profile list 只能与写操作按 lpac gate 排队。
- `/api/modem/*` 的 reset/enable/disable 和本工作流共享一个 Modem action gate；冲突立即 409，不排队执行。
- 锁顺序固定为 `SimSwitchCoordinator -> lpac gate -> Modem action/recovery gate -> SMS resync gate`；任何回调不得反向取得外层锁，避免 D-Bus/SMS listener 死锁。
- `InboundWorker` 的正常信号处理不应被全局暂停；在 recovery 期间暂时丢失的信号由后续 reconcile 补齐。重扫请求可合并但不能丢失。

### 9.2 正常流程

1. **Admission**：校验管理员、`Idempotency-Key`、ICCID 为纯数字且长度合理、confirm=true、eSIM enabled、设备 fingerprint 已验证；记录 operation row。
2. **Preflight**：在 5 秒内读取 profile list 和当前 identity。目标不存在则结束失败；目标已经 active 且 ICCID 与当前 identity 一致则直接成功，`recovery_skipped=true`。
3. **依赖预检**：检查 lpac binary 可执行、架构/glibc、QMI APDU/curl driver、QMI device/UIM slot 和 ModemManager 状态；未通过时不要调用 enable。
4. **lpac enable**：调用无 shell 的进程 runner，参数为 `profile enable <normalized_iccid> <refresh_flag>`；单次硬超时 60 秒，进程及子进程必须可终止，stderr 截断并脱敏。
5. **安全重试**：仅对明确的 transient/refresh-required marker 重试。等待约 800ms 后刷新 profile list；仍未 active 才以 refresh=0 再调用一次。非幂等/认证/目标不存在/硬件错误不重试。
6. **确认 Profile**：lpac 返回成功后再 list，确认目标 active 或至少确认当前 identity ICCID 已变为目标。无法确认时进入 `Failed`/`Degraded`，不得直接假定成功。
7. **基带恢复**：进入第 9.4 节的 bounded recovery；enable 之后不再执行旧 Profile 回滚。
8. **重新绑定**：按 fingerprint 重新发现 ModemManager path 和 QMI device，更新 runtime/action targets；只有 identity 与切换前记录匹配或明确表现为目标 ICCID 时才信任新 path。
9. **身份刷新**：读取目标 ICCID 的 IMSI、MSISDN、SMSC，写入 identity/profile cache；探测为空的结果也按 negative cache 保存。
10. **短信重扫**：向 `SmsResyncCoordinator` 提交 `profile-switch`，执行第 9.5 节流程；重扫失败时 operation 为 `Degraded` 而不是假成功。
11. **完成**：写入 step counts、new path 是否 verified、run ID 和结束状态，广播脱敏事件。

### 9.3 超时

| 阶段 | 默认超时 | 超时结果 |
| --- | ---: | --- |
| lpac driver probe | 3 秒 | `lpac_unusable` |
| profile preflight/list | 5 秒（普通 list 20 秒） | 失败，不进入 power-cycle |
| lpac enable 单次 | 60 秒 | 只按安全规则重试一次，否则失败 |
| 停止 ModemManager | 10 秒 | 进入 degraded，记录恢复失败 |
| QMI device 重新发现 | 15 秒 | 不使用旧 device path，恢复失败 |
| SIM power off/on | 每步 5 秒，必要 polling | 恢复失败 |
| 启动 ModemManager | 20 秒 | 恢复失败 |
| 新 Modem path/identity | 30 秒 | 恢复失败 |
| Modem ready/短信接口 | 30 秒 | 可标记 degraded，继续有限 resync 尝试 |
| SMS resync | 60 秒 | operation degraded，后台 listener 仍可继续重连 |
| 整体切换 | 240 秒硬 deadline | 取消剩余步骤并保存最终 phase；不得无限等待 |

所有等待用 deadline + bounded polling（例如 250ms–1s 间隔）；不使用无法取消的固定长 sleep。外部命令必须设置 stdout/stderr 上限、环境白名单和 kill-on-drop。

### 9.4 Modem 路径变化和基带恢复

SmsRelayed 现有 `ModemService` 已区分 configured/runtime/action target 和 fingerprint；本流程必须复用该边界：

- 操作开始保存 `old_fingerprint`、`old_runtime_path`、`old_action_path`、旧 ICCID 和旧 QMI device。
- 停止 ModemManager 后旧 D-Bus path 立即视为失效；不能把旧 path 传给后续 `mmcli`/D-Bus 调用。
- QMI device path 与 D-Bus path 独立变化；优先使用显式 `esim.qmi_device` 或稳定 udev 名称，其次只在唯一设备且能与 fingerprint 关联时探测 `/dev/cdc-wdm*`、`/dev/wwan*qmi*`。不能因为“第一个设备”就执行 power 操作。
- 启动 ModemManager 后重新枚举所有候选，读取设备标识并与 `old_fingerprint` 比对；只有唯一匹配时设置 runtime/action targets。若旧设备 fingerprint 变化，保留 runtime-only，禁止控制动作并返回 `modem_identity_unverified`。
- 新 path 不写回 `app.modem_path`；它只是本次 runtime binding。持久配置仍是用户选择的目标，需管理员显式修正。
- 注册网络不是短信重扫的必要条件；只需 Modem 可用、SIM ready、messaging interface 可读。若现有设备实现要求注册才可读，最多等待配置的 deadline，不因网络注册警告无限重试。
- 恢复失败后的安全动作是尝试一次有界的 ModemManager restart/重新发现，保存 `Degraded`，提供手工 retry recovery。绝不自动切回旧 Profile。

### 9.5 SMS 重扫和监听重绑定

新增 `SmsResyncHandle`/channel，接入 `InboundWorker`，而不是从 API 直接创建第二个 D-Bus listener：

1. 请求携带 `reason=profile-switch`、operation ID、目标 identity key 和目标 fingerprint。
2. coordinator 让 listener 重新调用现有 `resolve_monitor_path`，取得新的 runtime path；若 path 尚未可读则按 5 秒起始、60 秒上限退避重试。
3. 当 path 变化时，关闭旧订阅、订阅新 path 的 Added/PropertiesChanged，并先获取新 path 的 received SMS 快照；路径未变化也必须执行一次显式快照扫描。
4. 对快照中的 received SMS 读取 properties/body，过滤现有 `ignore_storage`，通过统一 `Messaging::receive` 入库。D-Bus object path 只用于本次读取，不用于最终去重。
5. 以新 identity generation 写入每条新增记录的 reconcile key；已有记录不重新触发 delivery。默认不转发历史 reconcile 记录，除非 `forward_reconciled=true`。
6. 重扫同时刷新 SIM identity cache；从 SMS 对象取得的 SMSC 只能作为一个来源，不能无条件覆盖更新更近或更可信的 EF_SMSP/ModemManager 值。
7. 完成后把新 path、identity key、scanned/persisted/forwarded counts 写入 `sms_resync_runs`，广播 `sms.resync.updated`。
8. 若 listener 在恢复期间已经自行重新订阅，收到的 resync 请求仍执行一次快照；同一个 operation + generation 的重复请求合并为一个 run。

## 10. 幂等、失败恢复和状态矩阵

### 10.1 HTTP 幂等

- enable/download/delete 等写操作要求 `Idempotency-Key`，沿用 SmsRelayed 发送 API 的长度和冲突语义：同 key + 同规范化请求返回同一 operation/result；同 key + 不同 target/参数返回 409。
- 相同 target 已 active 时返回可重复的成功结果，不创建新的基带恢复。
- 服务重启后，`Queued/Running` 不自动重放；只读 probe 后标记 `unknown` 或 `Degraded`，用户重新发起新 key。
- lpac enable 在外部看来不是可任意重放的操作；只有文档化的 transient marker 才允许内部一次 retry。

### 10.2 失败矩阵

| 失败点 | 状态 | 后续动作 |
| --- | --- | --- |
| eSIM gate/lpac/driver preflight | Failed | 不碰 Modem；修复依赖后重新请求 |
| profile 不存在/参数不合法 | Failed | 不碰 Modem；不暴露原始 lpac 输出 |
| lpac enable 非 transient 失败 | Failed | 保留当前 identity，允许新的明确操作 |
| lpac timeout、状态无法确认 | Failed 或 Degraded（取决于 probe） | 先只读 list/identity；不要盲目重复 enable |
| enable 明确成功，power-cycle 失败 | Degraded | 记录“目标可能已 active”，手动 recovery；不自动回滚 |
| 新 path 多个候选/身份不匹配 | Degraded | runtime-only，禁止控制；提示检查设备和配置 |
| identity 读取为空 | Degraded | 保留旧 cache 但标记 stale；不把旧值冒充当前值 |
| SMS resync 部分失败 | Degraded | 已入库的保持；按 run 重试剩余扫描，去重保证安全 |
| API/进程重启 | Unknown/Degraded | 恢复 listener；只读探测，不重放外部 enable |

## 11. 敏感字段处理

这些字段均按个人/网络身份信息处理；Matching ID、confirmation code 和 SM-DP+/Matching 组合还应按 provisioning credential 处理。

| 字段 | 用途 | 存储 | API/前端 | 日志/事件 |
| --- | --- | --- | --- | --- |
| EID | 标识 eUICC、绑定 cache | 0600 SQLite 中按需保存完整值；备份默认脱敏 | 管理员详情可显示，列表 mask | 只用不可逆短摘要 |
| ICCID | Profile 操作和 identity key | 必须保存完整值才能 enable；不放 URL | 操作 body 传入；显示 mask/显式 reveal | 不记录完整值 |
| IMSI | 识别当前订阅、诊断 | identity cache 完整值，按 fingerprint+ICCID 隔离 | admin-only，默认 mask | 不记录 |
| MSISDN/本机号码 | 呼叫/SMS identity | `msisdn_json`，规范化并允许负缓存 | admin-only，默认 mask | 不记录 |
| SMSC | 短信提交/诊断 | identity cache 完整值；来源和时间戳一起存 | admin-only，默认 mask | 不记录 |
| SM-DP+ | 下载服务器 | 可作为非秘密 endpoint，但和 Matching ID 绑定时视为敏感；默认仅短期内存 | 不在 profile list 回显，需显式 reveal | host 可按策略脱敏，凭据不记录 |
| Matching ID/activation code | 下载凭据 | 默认不持久化、不进 cache、不进 operation row；需要重试则由加密 secret store 保存短 TTL | 只在 HTTPS body 中提交，响应不回显 | 永不记录 |

通用要求：

- SQLite 文件、导出、备份和诊断 bundle 的权限沿用并加强现有配置文件 0600 约束；默认不把这些字段送入 Sentry、metrics、public health、webhook 或 SSE payload。
- raw lpac JSON、完整 stdout/stderr、AT 命令及其响应不得保存或日志化；只保留 allowlist 字段和 redacted error code。调试开关也必须先脱敏。
- API access log、反向代理日志和前端 analytics 不应包含 ICCID/手机号 URL；因此 enable/download 推荐 body 参数而非敏感 path segment。
- 输入严格做长度、字符集、数字/URL 校验；SMSC/号码不得接受控制字符。对日志需要关联时使用带 domain 的 HMAC/摘要，而不是前缀截断。
- 前端使用 `data-sensitive`/隐私组件和显式复制动作；页面 state 卸载、登出或超时后清理 full reveal 值。
- 不以加密“替代”访问控制；如果 SQLite 仍为明文，必须明确文件权限、主机 root 风险、备份保留和恢复文档。

## 12. lpac 供应链和硬件依赖

### 12.1 软件供应链

参考 SimAdmin 的私有 lpac 目录和架构选择，但 SmsRelayed 不应在每次请求时下载二进制：

- 发布物提供经过固定版本、SHA-256/签名验证、架构/glibc 兼容性检查的 lpac asset、许可证和 SBOM；运行时只执行配置的绝对路径。
- lpac repair/install 若实现，必须是独立管理员操作：下载超时和大小上限、临时目录校验、原子替换、旧版本保留、probe 失败自动回滚；禁止 shell 拼接。
- lpac 需要 QMI APDU driver 和 curl HTTP driver；检查动态库、执行权限、`qmi-proxy` 冲突、QMI device 权限和 UIM slot。
- 生产镜像不把未经审核的用户自定义 URL 当作可信升级来源；custom asset 仅诊断/开发配置，并在 API 返回警告。
- CI 记录支持的 `aarch64/x86_64`、glibc 最低版本和 lpac 版本；没有对应 asset 时明确 `lpac_unusable`，不降级执行未知本地二进制。

### 12.2 硬件/系统依赖

- 必须是实际带 eUICC 的 modem/板卡，并支持 ES10/APDU profile management；普通可插拔 SIM 或仅有 UI eSIM 选项不满足。
- Linux 需要 ModemManager/system D-Bus、libqmi/QMI character device（如 `/dev/cdc-wdm*` 或 `/dev/wwan*qmi*`）、必要的 QMI proxy/权限，以及能访问 SM-DP+ 的 HTTPS 网络。
- ModemManager 可能独占 QMI device；power-cycle 前必须有明确的 stop/ownership 约定，恢复后再启动并重新枚举，不能与正常 SMS D-Bus 调用并发抢设备。
- UIM slot、固件、运营商锁、profile policy、SM-DP+ activation code 和网络 TLS 都可能使“硬件可见但 lpac 不可用”；这些作为可诊断原因而非通用重试。
- 预检必须识别实际设备路径和稳定 Modem fingerprint；不能只检查 `lpac --version` 就允许 enable。

## 13. 分阶段交付

### Phase 0：契约和安全基础

- 完成 domain types、配置 defaults、SQLite migration、redaction policy、错误码和 operation 状态机。
- 增加 mockable command runner、identity reader 和 clock；不需要真实 eUICC。
- 加入 protected API 骨架和 frontend loading/error/敏感字段组件，但默认 `esim.enabled=false`。

### Phase 1：SIM identity 和缓存

- 复用 `ModemService` 的 verified target/fingerprint 读取 ICCID/IMSI；实现 MSISDN/SMSC 多来源读取和 negative cache。
- 提供 `/api/sim/identity`、refresh、admin UI；只读验证 path drift、缓存隔离和过期策略。
- 为 SMS 入库附加 identity generation/稳定 reconcile key 所需 metadata，不改变既有正常 live forwarding 行为。

### Phase 2：lpac 只读管理

- 实现供应链检查、lpac status、chip info、profile list、cache hydration；不做 enable。
- 完成 eUICC/Profile 页面和 masked detail；测试真实/模拟 QMI driver unavailable、无 eUICC、旧架构。

### Phase 3：Profile 非破坏性写操作

- 在 coordinator 下实现 nickname/delete/download；所有 provisioning secret ephemeral，写操作幂等。
- 增加 operation persistence、SSE/polling 和前端恢复显示。

### Phase 4：Profile enable、基带恢复和 SMS resync

- 先在 mock runner 上验证 preflight、lpac retry、path drift、recovery state machine；再接入真实 QMI power-cycle。
- 接入 `InboundWorker` resync channel、跨 path 去重、identity refresh 和 reconciliation policy。
- 默认 `forward_reconciled=false`，经真实硬件验收后再开放配置。

### Phase 5：生产化

- 完成 lpac 签名/哈希发布流程、最小权限/udev/systemd 文档、故障诊断和备份脱敏。
- 进行长时间重枚举、ModemManager crash、服务重启、网络不可用和多次切换 soak test。

## 14. 验收标准

### 功能

- 无 eSIM/无 lpac/driver 不可用时，系统保持现有 SMS 功能，eSIM API 只返回明确错误，不阻塞主 SMS worker。
- 能读取并缓存当前 EID/ICCID/IMSI、本机号码和 SMSC；不同 ICCID 的值不会串写，空结果和未读取可区分。
- 能列出 Profile 并正确标记 active；缓存优先和显式 live 行为可观察。
- 对已 active Profile 重复 enable 是无副作用幂等成功，不触发 power-cycle 或 resync。
- 对 inactive Profile enable 至少能完成：lpac enable、基带恢复、重新发现新 path、验证 fingerprint/identity、SMS resync；每一步都有状态。
- Profile 切换后，新 path 上已有 SMS 能补扫入库；同一 SMS 不重复创建、不重复 delivery；默认不批量转发旧短信。
- Modem path 变化、D-Bus 断开、ModemManager 重启后，listener 能重新绑定；无法确认设备身份时不执行控制动作。
- lpac timeout、恢复失败、重扫部分失败均留下可查询 operation/run 状态，服务重启后不盲目重放 enable。

### 安全

- public health、未授权 API、普通事件、metrics、日志、诊断 bundle 不包含完整 EID/ICCID/IMSI/号码/SMSC/Matching ID 或 SMS body。
- Matching ID/confirmation code 默认不会出现在 SQLite、operation row、前端缓存和日志。
- 所有进程参数、外部命令和 timeout 可测试；不存在 shell injection、无限等待和未限制 stdout/stderr。
- eSIM 写操作与现有 Modem action 互斥，重复/冲突请求返回 409 或已有 operation。

### 兼容性

- 既有配置无需修改即可启动；`esim.enabled=false` 时行为与当前版本一致。
- 既有 live SMS 接收、发送、delivery retry、公开 health 和 Modem API 测试不回归。
- `cargo fmt --check`、`cargo test`、frontend 的 `pnpm check` 和相关构建在无硬件环境下通过；硬件依赖测试以 mock/标记测试隔离。

## 15. 测试计划

### 15.1 Rust 单元测试

- ICCID、EID、IMSI、MSISDN、SMSC、SM-DP+、Matching ID 的规范化、长度/字符集验证和 redaction。
- `identity_key`/reconcile key 的 domain separation、不同 fingerprint/ICCID 不碰撞、D-Bus path 变化不改变 key。
- lpac JSON/stdout parser：正常 JSON、progress lines、字段缺失、未知 state、超长/恶意 raw、stderr 不泄漏。
- `LpacClient` timeout、子进程终止、参数 argv、driver probe 和 safe retry marker；确认 non-transient error 不重试。
- operation 状态机和 deadline：每个 phase 只允许合法迁移；服务重启后 Running 不自动 replay。
- idempotency：相同 key 相同请求返回同 operation；不同请求 409；active target 不触发 recovery。
- path rebinding：旧 path 失效、唯一 fingerprint 候选成功、多候选/身份变化拒绝 action、QMI path 与 D-Bus path 独立变化。
- cache freshness：fresh/stale/negative/missing、按 Profile 隔离、可信来源优先级和并发 upsert。

### 15.2 SQLite/持久化测试

- migration 从空库和当前生产 schema 升级成功；索引/unique 语义覆盖缺失字段。
- 事务中 operation phase、resync counts 和最终状态一致；进程中断后不产生可执行的半截 enable replay。
- identity/profile cache 不覆盖其他 Modem fingerprint 或 ICCID；备份/导出默认脱敏。
- reconcile key 在 object path 改变、重复快照、signal+snapshot 交错时保持一次入库。

### 15.3 入站和重扫集成测试

- fake D-Bus source 返回 initial snapshot、Added signal、path change、PropertiesChanged 和断线；确认 listener 通过现有 `InboundWorker` 重订阅。
- Profile switch 期间到达的短信在旧 path、新 path 和 resync snapshot 三种来源下只入库一次。
- `forward_reconciled=false` 不触发 delivery；开启后只对新入库且满足现有 profile policy 的消息投递。
- SMS body 延迟/空 body、storage ignore、重复时间戳、不同 sender/body 和缓存 SMSC 来源均有测试。
- resync request 合并、重试退避、超时和部分成功统计可查询。

### 15.4 API/前端测试

- 未授权和 public health 不可访问敏感 API；错误响应不含 lpac 原文。
- API 202/409/400/503/504 contract、Idempotency-Key、刷新后 operation polling 和 SSE event schema。
- 前端缓存优先、live refresh、active 标记、enable confirmation、进度/Degraded/失败状态、刷新页面恢复 operation。
- 任何列表、toast、analytics 和 error boundary 都不输出完整敏感字段。

### 15.5 硬件验收和故障注入

至少在一台真实支持 QMI APDU 的 aarch64 和一台 x86_64 设备上验证：

1. 读取 chip/profile/identity；下载一个测试 Profile；enable inactive Profile。
2. 观察 ModemManager 停止、QMI power off/on、重新枚举的 path 变化，并确认新 fingerprint/ICCID。
3. 在切换前、power-cycle 中、恢复后分别注入短信，确认入库/去重/转发策略。
4. 注入 lpac timeout、QMI device 消失、ModemManager 启动失败、多个候选 modem、网络不可用、D-Bus 断线和服务重启。
5. 连续切换至少 10 次并执行 soak test；每次都能查到 operation/resync 结果，不能出现旧号码/SMSC/Profile 泄漏或重复 delivery。

## 16. 与现有源码的落点

这是设计边界，不是本次实现清单：

- `src/config.rs`：增加默认配置和校验，保持现有 TOML/AppConfig 及 `app.modem_path` 语义。
- `src/modem.rs`：复用/扩展 verified runtime/action target、fingerprint 和统一 action gate；不让 eSIM 自己枚举第一个 Modem。
- `src/dbus.rs`、`src/dbus/inbound.rs`：复用 D-Bus timeout、对象快照和订阅适配器。
- `src/inbound.rs`：增加 resync channel/reconcile 入口，保持现有 `Messaging::receive` 与退避行为。
- `src/persistence`：以 migration 增加 cache、operation、resync 表和稳定去重所需 metadata。
- `src/api/mod.rs`：挂载 authenticated SIM/eSIM/operation/resync routes；沿用 `ApiError` 和当前 action 409 语义。
- `src/events.rs`：只增加脱敏的 operation/identity/resync 事件。
- `frontend/src/`：按现有 TanStack Router/React 结构新增页面和生成路由；不直接复制 SimAdmin 的 MUI 实现。

本 spec 的推荐设计以 SimAdmin 的行为为参考，但以 SmsRelayed 当前的 `ModemService`、D-Bus source、`InboundWorker`、`Messaging`、`Store` 和受保护 API 为唯一集成基础。

### 16.1 实现前必须关闭的决策项

| ID | 需决策事项 | 本 spec 默认建议 | 未决时的安全行为 |
| --- | --- | --- | --- |
| E1 | 是否保留 SimAdmin URL 形态的 compatibility adapter | 可保留一个 release；内部必须转发到同一 operation coordinator，并返回 operation id | 只发布第 8 节目标 API，不暴露 ICCID path route |
| E2 | nickname 使用同步 200 还是统一异步 202 | 若 lpac 99% 能在 5 秒内完成可同步，否则统一 operation | 不实现双重语义；选定一种并写 contract test |
| E3 | 是否离线显示 SM-DP+/Matching ID | SM-DP+ 可按 endpoint 单独评估；Matching ID/confirmation code 默认永不持久化 | 不显示、不备份、不重试下载凭据 |
| E4 | `forward_reconciled` 是否可由管理员开启 | 首版固定 false，硬件 soak test 后再开放 | reconcile 只入库、不创建 delivery |
| E5 | fingerprint 无法读取但只有一个 Modem 时是否允许 runtime-only | 允许只读 SMS；禁止任何 enable/reset/power 操作 | `ManualRequired`/`modem_identity_unverified` |
| E6 | 真实支持矩阵及 lpac asset 来源 | 在发布前锁定架构、glibc、lpac 版本、hash/签名和 license | `esim.enabled=false`，不提供 repair/download |

## 17. 参考实现来源索引

行号固定到 `eb6f497ad332e59f1f3899acc3b2b9427661fe5c`；后续参考仓变更时应优先按符号搜索，再更新行号和“已确认”结论。

| 主题 | 相对 `/tmp/SimAdmin` 的来源（符号/行号） | 支撑的事实 |
| --- | --- | --- |
| API envelope / eSIM models | `backend/src/models.rs: ApiResponse` 8–37；`WorkMode` 39–67；`EsimCommandResponse`/`EsimProfile` 69–137；`BasebandRestartResponse` 266–280 | envelope、字段、work mode、progress shape |
| eSIM gate 与 lpac 串行 | `backend/src/esim.rs: EsimSupervisor` 60–334 | work mode gate、单一 `lpac_lock`、20/60/120 秒命令 timeout |
| lpac probe/driver | `backend/src/esim.rs: probe_lpac_binary` 537–607；`lpac_driver_list_has_required_drivers` 609–625 | 3 秒 probe，要求 qmi APDU + curl HTTP |
| lpac argv/env/解析 | `backend/src/esim.rs: run_lpac_command` 915–1007；`configure_lpac_environment` 1022–1044；`discover_lpac_qmi_device` 1046–1080 | 无 shell argv、默认 env、首设备 fallback、raw stdout 风险 |
| eSIM 路由 | `backend/src/main.rs` 690–730 | 真实 method/path |
| eSIM error HTTP 映射 | `backend/src/handlers.rs: esim_error_response` 78–85 | Disabled=403、Unavailable=503、Command=200 |
| enable preflight/retry | `backend/src/handlers.rs: retry_enable_profile_after_refresh` 388–466；`enable_esim_profile_for_switch` 468–531 | 5 秒 list、800ms、refresh=0 重试、already-active skip |
| enable 后台任务 | `backend/src/handlers.rs: enable_esim_profile_handler` 839–999 | 立即 HTTP 200、spawn、恢复、成功/失败 resync 请求、无 operation id |
| cache-first handler | `backend/src/handlers.rs: get_esim_euicc_handler` 714–750；`get_esim_profiles_handler` 753–836 | eUICC 默认 cache；profiles 以 `cached=1` 取 cache；identity enrichment |
| SIM identity/key | `backend/src/modem_manager.rs: SimIdentity` 722–727；`smsc_identity_keys`/`own_number_identity_key` 729–757；`current_sim_identity` 1167–1207 | ICCID/IMSI/operator 字段与 cache key 选择 |
| 号码/SMSC 探测与负结果 | `backend/src/modem_manager.rs: refresh_sim_details_background_inner` 1780–1833 | 多级 fallback、`source=empty` 写 cache |
| Modem path 发现 | `backend/src/modem_manager.rs: find_modem_path` 1122–1157 | 首 path、scan、5 秒发现、30 秒 failure cache，无 fingerprint |
| QMI device/power | `backend/src/modem_manager.rs: find_qmi_device_path` 2218–2239；`qmicli_sim_power` 2262–2275 | 设备节点优先级、UIM slot 1、20 秒 qmicli timeout |
| Profile 切换恢复 | `backend/src/modem_manager.rs: power_cycle_sim_for_profile_switch_inner` 5916–6131 | stop/start MM、QMI off/on、15 秒枚举、state/registration polling |
| progress singleton | `backend/src/modem_manager.rs: reset_baseband_restart_progress` 5829–5875 | 进程内全局 steps/running，无 operation identity |
| SMS resync channel | `backend/src/sms_listener.rs: SmsResyncHandle` 34–59；`start_sms_listener` 400–535 | 无界请求 channel、15 秒 poll、重绑触发、无 run result |
| SMS reconcile/dedupe | `backend/src/sms_listener.rs: sms_marker` 78–103；`process_sms_path` 192–289；`scan_sms_paths` 297–337 | timestamp/body/path fallback key、DB 多级去重、reconcile 默认不转发、处理后删 modem SMS |
| SQLite cache schema | `backend/src/db.rs` 538–631；`upsert_esim_profile_cache` 1820–1892 | 无 TTL/fingerprint，保存 provisioning fields/raw，COALESCE upsert |
| 前端 API compatibility | `frontend/src/api/current.ts` 441–504、574–583 | `cached=1`、ICCID URL、10 秒 enable request、restart status |
| 前端 progress UI | `frontend/src/pages/EsimManager.tsx` 590–595、939–1000、1003–1060 | 每秒 polling、确认、乐观 active、全局 recovery dialog |
| 环境/安装边界 | `docs/environment.md`；`docs/install.md`；`README.md` | lpac、QMI/APDU、glibc/arch 和 work mode 的部署边界 |
