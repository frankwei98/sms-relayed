# 功能 2：系统事件通知中心

状态：Spec（仅设计，不包含实现）
参考基线：`/tmp/SimAdmin` clean `main@eb6f497ad332e59f1f3899acc3b2b9427661fe5c`

### 文档判读规则

- **已确认**：固定参考 commit 的源码事实；第 14 节以相对 `/tmp/SimAdmin` 的路径、符号和行号索引证据。
- **目标契约**：SmsRelayed 重建时必须达到的外部行为。第 4 节及之后若无“需决策”标记，均是目标设计，不代表 SimAdmin 已实现。
- **重建建议**：内部表名、模块名、阈值默认值可以按 SmsRelayed 习惯实现，但不能削弱事务、幂等、脱敏和恢复语义。
- **需决策**：参考实现没有可靠答案的产品选择；实现前关闭第 12.5 节决策表。

## 1. 目标与非目标

### 1.1 目标

为 SmsRelayed 增加一个可持久化、可追踪、可恢复的系统事件通知中心：

- 统一定义系统事件模型、分类、事件 code、严重度和状态语义。
- 将 ModemManager/网络、服务、配置、安全、资源、转发投递和通知队列异常转换为结构化事件。
- 支持按事件 code、分类、严重度、状态、实体和条件匹配规则。
- 支持连续失败阈值、时间窗口、去抖、冷却和恢复通知，避免单次瞬态故障造成通知风暴。
- 复用现有 `channel.profile` 转发 profile 作为通知渠道，不复制或重新存储渠道凭据。
- 提供通用通知队列、租约、重试、幂等、失败日志和清理机制。
- 通过现有 SSE 能力实时更新系统事件、队列和通知日志，并提供受保护的查询 API 和前端 Notification Center。
- 保持现有短信接收、短信投递、配置安全和 API 兼容行为不变。

### 1.2 非目标

- 本功能不替代短信 `deliveries` 的投递状态机，也不把系统通知伪装成一条短信。
- 不新增渠道供应商；Bark、Telegram、企业微信、钉钉、飞书和 Webhook 继续由现有 forwarding profile 提供。
- 不在第一阶段实现通用工作流、告警编排、值班轮换、升级策略或外部告警平台集成。
- 不保存完整短信正文、手机号、token、URL 查询参数或供应商原始响应作为系统事件详情。
- 不为历史数据库凭空补造系统事件；迁移完成后只记录新发生的事件。
- 不在通知规则中暴露或复制现有渠道密钥。

## 2. 参考实现结论：SimAdmin 的行为与局限

### 2.1 SimAdmin 当前保留的设计

SimAdmin 已形成一套可复用的产品交互和基础概念：

- Notification Center 以“日志、规则、渠道”组织配置，并提供队列抽屉、测试渠道、清理日志和手工重试。
- 事件类型覆盖 SMS、DDNS、版本更新、系统事件、设备状态和自动化；系统事件按 `baseband`、`cellular`、`device_network`、`system_service`、`security`、`esim`、`resource` 分类。
- 系统事件使用稳定的事件 code；规则可以精确选择 code，非系统事件还支持 Always、Contains、NotContains、Equals、Regex。
- 渠道实例和规则分离；一个规则可以引用多个渠道，渠道有启用开关、名称和限流配置。
- 通知日志记录规则、渠道、结果和摘要；队列支持 pending、scheduled、retrying、sending、sent、failed、cancelled 等生命周期操作。
- 安静时段、DDNS 失败阈值、标题模板、正文模板和自定义 JSON 是有价值的用户能力，应保留其产品意图。

### 2.2 SimAdmin 行为摘要

- `SystemEvent` 包含 category、event code、label、severity、status、entity、message 和 timestamp。
- 系统事件默认定义在编译期；只有存在启用且包含该 event code 的 system-event 规则时才向通知发送器转发。
- 登录失败使用进程内窗口：5 分钟内达到 5 次触发一次 warning，冷却 5 分钟。
- DDNS 阈值依赖事件携带的 `failure_count`，按失败次数 modulo threshold 发送，而不是维护通用条件状态。
- 队列每 5 秒轮询一次，单项最多 5 次尝试；失败以 60 秒起步、指数退避至 3600 秒；限流或发送失败进入队列。
- 日志写入后立即按保留天数或最大条数清理；系统事件本身没有独立的持久化历史表。
- 前端按系统事件分组展示 code 复选框，日志支持类型、状态、日期和关键字过滤，队列支持逐项及批量重试/取消。

### 2.2.1 已确认的 SimAdmin HTTP 契约

SimAdmin 使用 `{ "status":"ok|error", "message":string, "data"?:T }` envelope。除认证/JSON extractor 等框架级错误外，下列 handler 的数据库、配置和 provider 业务失败通常仍返回 HTTP 200，由 envelope 或 `data.success` 表达失败。这只是 compatibility fact，不是 SmsRelayed 目标错误语义。

| 方法与路径 | 请求 | 响应/行为 |
| --- | --- | --- |
| `GET /api/notifications/config` | 无 | `200`，返回 config v2：`{version:2,channels,rules,log_cleanup}`；渠道 config 中含 provider 配置，因此参考前端能编辑 secret |
| `POST /api/notifications/config` | 完整 config v2 | 成功或保存失败均 `200` envelope；没有 ETag/If-Match，也没有 rule 细粒度 CRUD |
| `POST /api/notifications/test/{channel}` | channel id/type 位于 URL | 始终 `200` success envelope，真正结果在 `data={success,message}` |
| `GET /api/notifications/logs` | `type,status,q,start_date,end_date,limit=50,offset=0` | `200`，`data={logs,total}`；limit 在 DB clamp 到 1..200 |
| `POST /api/notifications/logs/clear` | `{type?,status?,start_date?,end_date?}` | `200`，`data={deleted}`；失败仍是 200 error envelope |
| `GET /api/notifications/queue?limit=100` | limit clamp 到 1..500 | `200`，返回 active/failed items 和 total；item 包含 title/body/last error 派生 reason |
| `POST /api/notifications/queue/{id}/retry` | 无 | `200`，`data={updated}`；会把 attempt_count 清零，甚至允许改写 `sending` |
| `DELETE /api/notifications/queue/{id}` | 无 | `200`，把 active/failed（包括 `sending`）标为 cancelled |
| `POST /api/notifications/queue/retry-all` | 无 | `200`，所有 active/failed（包括 `sending`）改回 pending 并清零 attempt_count |
| `POST /api/notifications/queue/clear` | 无 | `200`，把所有 active/failed（包括 `sending`）标为 cancelled |

参考实现没有 `/api/system-events`、event registry API、事件详情、cursor/replay、notification job idempotency 或独立 attempts API。系统事件仅在匹配到启用 rule 时才进入通知路由，未形成可查询审计资源。

### 2.3 SimAdmin 需要改进的局限

以下问题不能原样带入 SmsRelayed：

1. 编译期事件注册表没有版本化；后端和前端的 system-event code 已出现漂移，例如前端遗漏 profile download 的成功/失败 code。
2. 规则是否启用同时决定系统事件是否被发出，导致关闭所有通知规则后连审计事件也丢失。
3. 事件不落库，没有事件 id、关联 id、指纹、来源或跨重启的条件状态。
4. 登录失败计数只在内存中，重启会清零，也没有真正的恢复事件。
5. DDNS modulo 阈值不能表达连续失败、时间窗口、恢复阈值和滞回。
6. 队列只保存“通知发送项”，不是通用事件队列；没有幂等键，`expires_at` 未实际参与处理，轮询时会无界 spawn 任务，手工重试还会清零历史尝试次数。
7. 日志没有事件/通知尝试的稳定关联，可能把消息正文或敏感摘要写入数据库，清理也只覆盖日志。
8. quiet hours 主要是直接跳过发送；系统事件的 warning/critical 不应无条件丢弃。
9. `triggered`、`recovered`、`succeeded`、`failed`、`changed` 与日志结果状态混用，无法区分“条件状态”和“通知投递状态”。
10. `mask_identifier` 只在少数路径使用；自定义 JSON 和模板变量的敏感信息边界不够明确。
11. queue item 和 notification log 均保存渲染后的正文/message；queue GET 也把 `body` 返回前端。系统事件模板变量还包含本机号码和运营商，不能把这套 schema 原样用于安全事件审计。
12. queue 没有 lease owner/lease deadline。虽然 `mark ... sending` 是条件更新，同一进程内可减少重复 claim，但 worker crash 会留下无法自动回收的 `sending`；取消/手工 retry 又可与正在发送的 task 竞态。
13. rate limit 默认每 channel 60 秒 20 条，仅统计成功日志；quiet hours 命中时直接跳过并写日志，不 defer。日志清理在每次写日志之后同步执行，队列表和事件状态不在清理范围。

### 2.4 保留与改进决策

| SimAdmin 设计 | 决策 | SmsRelayed 方案 |
| --- | --- | --- |
| 日志/规则/渠道三块 UI | 保留 | 增加事件时间线和队列状态，四者互相可跳转 |
| 稳定 event code 和分类 | 保留并改进 | 建立版本化 code registry，后端返回 registry，前端不再维护独立副本 |
| 精确 code 规则 | 保留 | 以 code 为主，补充 category、severity、status、entity 条件 |
| 渠道实例 + 多渠道规则 | 保留 | 规则只引用 `channel.profile`，复用现有凭据和 dispatcher |
| quiet hours | 保留并改进 | 默认 defer；info 可 drop，warning/critical 可 defer 或 bypass |
| DDNS/登录失败阈值 | 保留意图并改进 | 统一为持久化 condition state、窗口、去抖、恢复和滞回 |
| 5 秒轮询队列 | 改进实现 | 复用 `Notify`、精确 next-at 唤醒和安全扫描，不依赖固定轮询完成工作 |
| 通知日志清理 | 保留 | 改为后台批量维护，同时清理已结束队列和过期事件 |
| 自定义 JSON | 延后并收紧 | 第一阶段只允许 schema 校验后的 allowlist 字段，禁止原始敏感字段 |

### 2.5 已确认默认值与状态词汇

| 项目 | SimAdmin 参考默认/状态 | 目标处理 |
| --- | --- | --- |
| config schema | notification version 2；channels/rules 为空 | 升级后目标通知发送仍默认关闭/空规则 |
| channel rate limit | enabled=true，20 messages / 60s | 可沿用为建议初值，但持久 job/lease 语义优先 |
| log retention | 90 天且最多 10,000 条 | 沿用为目标默认，后台批处理而非 insert 同步删除 |
| queue attempts | 5 次；第一次失败入队 60s；之后 60×2^(attempt-1)，最大 3600s | 目标可复用 SmsRelayed delivery 的 30s 初值；两者必须明确区分为“参考事实/目标默认” |
| queue worker | 每 5 秒取最多 20 条并逐条 spawn | 目标为 Notify + next deadline + 30s safety scan + bounded concurrency |
| queue status | pending/scheduled/retrying/sending/sent/failed/cancelled；schema 还有未执行的 expires_at | 目标增加 expired，并真正执行 expires_at/最大年龄 |
| log status | success/failed/queued/quiet_hours/no_available_channel 等发送结果 | 目标与 event status 分离，枚举化 outcome |
| resource monitor | 60s interval；CPU/memory 5 bad/2 good，temperature 1/2，disk 1/1，interface/connectivity 3/2 | 仅是参考事实；目标阈值以第 5 节 registry/condition 配置为准 |

## 3. SmsRelayed 现有能力与约束

### 3.1 EventBus 与 SSE

当前 `src/events.rs` 提供进程内 Tokio broadcast EventBus，容量为 256；`AppEvent` 覆盖 message.created、message.updated、message.deleted、已读状态变化、conversation 更新、config.saved 和 service.restart_scheduled。发布方不等待消费者，慢消费者可能收到 lag error。

当前 `/api/events` 是受会话保护的 SSE 端点，按 `AppEvent::name()` 输出命名事件，前端 `subscribeEvents` 使用 `EventSource` 订阅并在认证失效时重新检查 session。该流没有 replay/cursor，重连期间可能丢事件；当前 AppEvent 某些 payload 含完整 `Message` 或手机号，不能直接作为外部系统通知 payload。

系统事件必须在 EventBus 之前经过“结构化、脱敏、持久化”边界：SSE 只发送安全的 SystemEventView，不把内部 `Message` 或转发正文复用到通知事件中。事件写入成功后再广播；广播失败不能回滚数据库写入。

### 3.2 Delivery worker、转发和重试

当前 delivery worker：

- 使用 `DeliveryWakeup` 唤醒新投递，配合精确 retry deadline 和 30 秒安全扫描。
- 以 90 秒 lease claim 投递，按配置的 concurrency（1 至 16）处理，当前默认 2。
- 初始重试 30 秒，指数退避上限 3600 秒，并基于 delivery id 和 attempt 产生确定性 jitter。
- 单条投递最大生命周期 24 小时；无效 deadline、消息不存在或超过最大年龄会成为永久失败。
- 2xx 成功；408、425、429、5xx、连接/超时等为 transient；其他 4xx 和 3xx 为 permanent（各 provider/webhook 保留其当前明确分类）。
- 每次尝试只记录安全的 outcome、error_code、耗时和 dispatch delay，不记录手机号、正文或密钥。

当前 forwarding profile 以 `channel.name` 标识，配置位于 `[channels]`，启用 profile 位于 `[forward].enabled`；配置保存采用原子写入和 0600 权限，`redacted_summary` 会隐藏 token、secret、key、URL 等敏感值。通知渠道应复用这一套 profile key、校验和 dispatcher，不复制 profile 配置。

当前系统没有 notification queue、system event 表、通知规则、通知日志和 Notification Center；`forward_attempts` 只描述 SMS delivery 的 provider 尝试。现有 SMS API 的 `Idempotency-Key` 只保证 outbound message 创建语义，不能直接代替通知 job 的幂等。

## 4. 目标事件模型

### 4.1 事件信封

建议引入版本化的 `SystemEvent`，所有字段均为可序列化的稳定 API 字段：

| 字段 | 说明 |
| --- | --- |
| `event_id` | UUID/ULID，单次事件的不可变 id |
| `schema_version` | payload 版本，初始为 1 |
| `occurred_at` | 来源发生时间，UTC RFC3339 |
| `recorded_at` | SmsRelayed 落库时间，UTC RFC3339 |
| `category` | 稳定分类，例如 `delivery`、`modem`、`network` |
| `code` | 稳定的 namespaced code，例如 `delivery.profile_unhealthy` |
| `severity` | `info`、`warning`、`critical` |
| `status` | 事件语义状态，见下文 |
| `condition_key` | 同一故障条件跨事件的稳定 key |
| `entity_type` / `entity_key` | 作用对象及脱敏标识，例如 profile、worker、interface |
| `correlation_id` | 关联 delivery、message 或操作的内部 id；外部输出时可保留非敏感 opaque id |
| `summary` | 已脱敏、长度受限的人类可读摘要 |
| `details` | 仅包含 allowlist 数值和枚举的 JSON object |
| `fingerprint` | 对 code、entity、condition、规范化 details 的哈希，用于去重 |
| `source` | `delivery_worker`、`api`、`modem_monitor`、`config` 等来源 |

`details` 只允许计数、阈值、状态、延迟、HTTP status、safe error code、队列长度、资源百分比等字段；事件生产者不得把 SMS body、完整 phone number、认证头、token、webhook URL、IMSI/ICCID/EID 全值写入 `summary` 或 `details`。

### 4.2 状态和严重度

事件状态与通知投递状态严格分离：

- `triggered`：条件从 normal/pending 进入 active；同一 active 周期只生成一次首发事件。
- `recovered`：此前已持久化为 active 的同一 `condition_key` 恢复到 normal；只有确认过 active 才允许生成恢复事件。
- `succeeded`：一次性操作成功，例如服务启动、profile 检查成功；不代表某个故障恢复。
- `failed`：一次性操作失败或单条消息永久失败；不自动生成 recovered。
- `changed`：配置、网络状态或策略发生变化，但不是故障条件。
- `suppressed`：事件已记录但被规则、去抖或 quiet policy 抑制；这是审计/内部状态，不表示 provider 已发送。

严重度只有三档：`info` 用于状态和操作记录，`warning` 用于需要关注但服务仍可用的退化，`critical` 用于服务不可用、数据风险或持续失败。严重度由 code 默认提供，规则可以设置最低严重度但不能把 critical 降级为 info。

### 4.3 建议分类与 event code

首版 registry 至少包含下列 code。code 本身表示条件，不为恢复另造 `*_recovered` code；恢复通过相同 code 加 `status=recovered` 表示。

| 分类 | code | 默认严重度 | 触发/恢复语义 |
| --- | --- | --- | --- |
| `delivery` | `delivery.profile_unhealthy` | warning | profile 连续 transient failure 达阈值；后续成功达到恢复阈值时 recovered |
| `delivery` | `delivery.message_failed` | critical | 单条 delivery 永久失败；记录关联 id，不自动恢复 |
| `delivery` | `delivery.profile_missing` | critical | 配置引用不存在或启动校验失败；配置修复并成功投递后 recovered |
| `delivery` | `delivery.worker_unhealthy` | critical | worker 连续 claim/处理错误达到阈值；worker 正常 drain 后 recovered |
| `delivery` | `delivery.queue_backlog_high` | warning | backlog 持续超过阈值；降到恢复线并保持时间后 recovered |
| `sms` | `sms.persistence_failed` | critical | 接收短信无法持久化；数据库恢复后 recovered |
| `modem` | `modem.unavailable` | critical | modem 缺失或不可用达到去抖阈值；健康探测达标后 recovered |
| `modem` | `modem.registration_degraded` | warning | 搜网/注册持续退化；注册稳定后 recovered |
| `network` | `network.interface_error_spike` | warning | 接口错误速率超过阈值；窗口恢复正常后 recovered |
| `network` | `network.connectivity_failed` | warning | 连续探测失败；连续成功后 recovered |
| `security` | `security.login_failure_burst` | warning | 匿名/脱敏实体在窗口内失败次数达到阈值；冷却窗口无失败后 recovered |
| `config` | `config.validation_failed` | warning | 配置保存/加载校验失败；有效配置成功生效后 recovered |
| `service` | `service.started` | info | 服务或 worker 启动成功，状态为 succeeded |
| `service` | `service.restart_scheduled` | info | 重启计划已接受，状态为 succeeded |
| `resource` | `resource.disk_low` | critical | 磁盘低于下限；高于恢复线并稳定后 recovered |
| `resource` | `resource.memory_high` | warning | 内存高于上限；低于恢复线并稳定后 recovered |
| `resource` | `resource.temperature_high` | warning | 温度高于上限；低于恢复线并稳定后 recovered |

后续 modem/eSIM 能力可以按同一 registry 增加 code。事件 registry 必须由后端作为 API 数据源返回 `code`、label、category、default severity、支持状态和 schema version；前端禁止复制一份可能漂移的静态列表。若需要兼容 SimAdmin 导入，可接受旧的 `modem_missing_threshold` 等别名并在保存时规范化为新 code。

## 5. 去抖、阈值、冷却与恢复

### 5.1 通用条件状态

每个 `(code, entity_type, entity_key)` 维护一个持久化 condition：`normal`、`pending`、`active` 或 `unknown`，并保存 first_seen、last_seen、failure_count、success_count、last_notified_at、next_repeat_at 和状态版本。

规则/registry 的条件参数包括：

- `window`：统计窗口，例如 5 分钟。
- `trigger_count`：窗口内或连续观察达到多少次进入 active。
- `trigger_duration`：条件至少持续多久才进入 active。
- `recover_count` / `recover_duration`：恢复所需的连续成功次数或稳定时长。
- `cooldown`：同一 active 周期首发后的最短再次通知间隔。
- `repeat_interval`：仍 active 时是否周期性重复提醒；默认 info 不重复、warning 60 分钟、critical 30 分钟。
- `recovery_hysteresis`：恢复阈值与触发阈值之间的间隔，防止临界值来回抖动。

默认去抖为“3 次连续观测或 30 秒，以 code 的数据源能力较早满足者为准”；高风险 code 必须明确覆盖默认值，不能由调用方临时自定义含义。

### 5.2 首版默认阈值

| 条件 | 触发默认值 | 恢复默认值 |
| --- | --- | --- |
| profile transient failure | 同一 profile 5 分钟内连续 3 次 | 2 次连续成功，或 5 分钟内成功率回到健康线 |
| worker unhealthy | 5 分钟内 3 次 worker/claim 错误 | 1 个完整成功 drain 周期 |
| queue backlog high | backlog > 50 且持续 5 分钟 | backlog < 10 且持续 2 分钟 |
| login failure burst | 5 分钟内 5 次 | 5 分钟无新失败；不暴露用户名 |
| modem unavailable | 90 秒内 3 次失败观测 | 2 次连续健康观测 |
| disk low | 可配置容量百分比低于下限且 2 分钟稳定 | 高于下限 + 5 个百分点且 2 分钟稳定 |
| memory/temperature high | 连续 3 次或持续 30 秒超过上限 | 低于上限 - 5 个百分点并稳定 2 分钟 |

一次性 `failed` 事件（例如某条消息永久失败）不应因为后续另一条消息成功而生成恢复；只有定义了 condition_key 的聚合条件才有 recovered。服务重启后，若数据库中 condition 为 active，应恢复计数状态；启动时不因一次健康采样发送虚假的 recovered，须满足该 code 的 recover 条件。

## 6. 规则与现有 forwarding profile 复用

### 6.1 规则模型

建议在现有 TOML 配置中增加 `[notifications]` 区块，规则至少包含：

```toml
[notifications]
enabled = true

[[notifications.rules]]
id = "delivery-ops"
name = "转发异常"
enabled = true
event_codes = ["delivery.profile_unhealthy", "delivery.worker_unhealthy"]
min_severity = "warning"
statuses = ["triggered", "recovered", "failed"]
profile_keys = ["telegram.ops", "bark.admin"]
quiet_policy = "defer"
cooldown_seconds = 1800
```

规则还可以提供 category、entity type/key、标签或 safe-details 条件，以及标题/正文模板。事件 code 是主匹配条件；空 `event_codes` 不得隐式匹配全部 critical，若需全量订阅必须显式设置 `all_events=true` 并要求管理员确认。

规则的 `profile_keys` 必须使用现有 `channel.name` 形式，如 `telegram.main`、`webhook.ops`。保存时校验 profile 存在、启用状态、渠道类型和当前配置合法；禁用 profile 不创建新的发送任务。一个事件命中多条规则时，使用 `(event_id, rule_id, profile_key)` 去重，而不是把多个规则合并成一个不可追踪任务。

### 6.2 兼容既有短信转发

`[forward].enabled` 继续只表示新短信的转发目标，不改变现有 `deliveries` 行为。通知规则可引用同一 profile，但不读取或修改 `[forward].enabled` 的语义。为方便部署，可以提供“从当前 enabled profiles 创建系统通知规则”的显式操作；数据库/配置迁移不得默默给用户发送新的系统告警。

通知 dispatcher 复用现有 profile 的认证、请求构造、超时和 `ForwardOutcome` 分类；规则层只负责事件筛选、模板和 job，不能在规则配置中再次放 token、secret、URL 或 header。

### 6.3 quiet hours 与模板

quiet hours 由规则定义时区，默认使用服务器配置时区并在 API 中明确返回。推荐策略：

- info：默认 `drop`，仍记录 `suppressed`。
- warning：默认 `defer` 到安静时段结束，并保留 `not_before`。
- critical：默认 `bypass`；也可由管理员显式改为 defer。

模板只允许安全变量，如 `event.code`、`event.category`、`event.severity`、`event.status`、`event.summary`、`entity.masked_key`、`details.failure_count`、`correlation_id`。不得提供 `message.body`、完整号码、profile credential、原始 provider response 等变量。自定义 JSON 必须在保存和发送前验证为 object/array，并再次执行敏感字段扫描和大小限制。

## 7. 数据模型与迁移

### 7.1 与现有表的边界

现有 `messages`、`deliveries`、`forward_attempts`/`forward_attempt_samples`、出站幂等表继续负责短信业务；不要把系统通知插入要求 `message_id` 的 SMS delivery 表。可以复用现有 worker 的 lease、退避、唤醒和 dispatcher 抽象，但通知需要独立 payload 和状态表。

### 7.2 建议新增表

#### `system_events`

保存不可变事件历史：

```text
id TEXT PRIMARY KEY
schema_version INTEGER NOT NULL
occurred_at TEXT NOT NULL
recorded_at TEXT NOT NULL
category TEXT NOT NULL
code TEXT NOT NULL
severity TEXT NOT NULL
status TEXT NOT NULL
condition_key TEXT NULL
entity_type TEXT NULL
entity_key TEXT NULL
correlation_id TEXT NULL
source TEXT NOT NULL
summary TEXT NOT NULL
details_json TEXT NOT NULL DEFAULT '{}'
fingerprint TEXT NOT NULL
created_at TEXT NOT NULL
```

建议建立 `(created_at DESC, id DESC)`、`(code, status, created_at)`、`(condition_key, created_at)` 和 `fingerprint` 索引。若来源有天然事件 id，可增加 `source_event_id` 唯一约束，避免重启或重连重复写入。

#### `event_conditions`

保存跨重启阈值状态：

```text
condition_key TEXT PRIMARY KEY
code TEXT NOT NULL
entity_type TEXT NOT NULL
entity_key TEXT NOT NULL
state TEXT NOT NULL
failure_count INTEGER NOT NULL DEFAULT 0
success_count INTEGER NOT NULL DEFAULT 0
first_seen_at TEXT NULL
last_seen_at TEXT NULL
last_notified_at TEXT NULL
next_repeat_at TEXT NULL
state_version INTEGER NOT NULL DEFAULT 1
updated_at TEXT NOT NULL
```

更新必须与生成 `triggered`/`recovered` 事件在同一 SQLite transaction 内完成；状态机使用乐观版本或写事务，防止两个 worker 同时把同一条件重复触发。

#### `notification_jobs`

这是通用通知队列，而不是 SMS delivery 的别名：

```text
id TEXT PRIMARY KEY
event_id TEXT NOT NULL REFERENCES system_events(id)
rule_id TEXT NOT NULL
profile_key TEXT NOT NULL
status TEXT NOT NULL
title TEXT NOT NULL
body TEXT NOT NULL
summary TEXT NOT NULL
idempotency_key TEXT NOT NULL UNIQUE
attempt_count INTEGER NOT NULL DEFAULT 0
manual_retry_count INTEGER NOT NULL DEFAULT 0
next_attempt_at TEXT NOT NULL
expires_at TEXT NOT NULL
lease_owner TEXT NULL
lease_until TEXT NULL
last_error_code TEXT NULL
last_error_at TEXT NULL
created_at TEXT NOT NULL
updated_at TEXT NOT NULL
```

状态为 `pending`、`scheduled`、`sending`、`retrying`、`sent`、`failed`、`cancelled` 或 `expired`。索引至少覆盖 `(status, next_attempt_at)`、`(profile_key, status, next_attempt_at)` 和 `(event_id, rule_id)`。

#### `notification_attempts` 与 `notification_logs`

`notification_attempts` 记录每一次 provider 尝试：job id、attempt number、started/completed、latency、dispatch delay、outcome 和 safe error code。`notification_logs` 是面向 UI 的摘要视图：event id、job id、event type/status、rule/profile、结果、summary、message 和 created_at。二者都不能保存完整 body、手机号、credential 或 provider 原始响应；如果需要审计请求，保存 body 的字段白名单摘要或 hash。

### 7.3 迁移要求

- 按当前 storage migration 方式新增幂等 migration；建议同时使用 SQLite `user_version` 或现有 `meta` key 记录 `system_event_notifications_v1`，不能依赖“表不存在”作为唯一版本判断。
- migration 在事务中创建新表、约束和索引；重复执行必须安全。失败时整组回滚，不影响现有 SMS 表。
- 不回填历史系统事件，不改变现有 `messages`、`deliveries`、`forward_attempts` 的状态；新功能首次升级默认 `notifications.enabled=false`、规则为空或仅建立未启用模板，确保升级不会意外发通知。
- 配置读取应接受没有 `[notifications]` 的旧 TOML，并写回时保留现有 `[channels]`、`[forward].enabled` 和未涉及字段。配置版本递增后仍通过现有原子写入和 0600 权限保存。
- 系统事件 registry 的版本与数据库 schema 独立；删除/改名 code 必须保留 alias 和迁移说明，不能复用旧 code 表示新语义。

## 8. API 与 SSE

所有接口复用现有会话认证、CSRF/配置并发控制和统一错误格式；日志、事件和队列响应加 `Cache-Control: no-store`，不因业务错误统一返回 HTTP 200。

目标成功响应直接返回资源 JSON，不使用 SimAdmin envelope。目标错误严格沿用 SmsRelayed 的现有格式：

```json
{
  "error": {
    "code": "notification_job_conflict",
    "message": "notification job is currently sending"
  }
}
```

`message` 必须已脱敏；客户端只按 HTTP status 和 `code` 分支。未知 provider 文本、URL、响应 body 和 channel secret 都不能进入该结构。

### 8.1 事件 API

- `GET /api/system-events`：按 `category`、`code`、`severity`、`status`、`entity_type`、`entity_key`、`from`、`to`、`cursor`、`limit` 查询；limit 缺省 50、范围 1..200。默认按 `(occurred_at DESC,id DESC)`，返回 `{ "items":[SystemEventView], "next_cursor":string|null }`。cursor 是 opaque、带查询排序边界的编码，参数组合改变后不能复用。
- `GET /api/system-events/{id}`：`200 SystemEventView`，details 仍是 allowlist；不存在为 `404 system_event_not_found`，不能返回原始消息或渠道正文。
- `GET /api/system-events/registry`：`200 {schema_version,registry_version,categories,codes}`，每个 code 含 label、category、default severity、supported statuses、condition defaults 和 deprecated aliases，供前端生成选择器。

### 8.2 通知配置、规则和测试

- `GET /api/notifications/config`：`200 NotificationSettingsView`，只含 enabled、retention、worker limits 等非 secret 设置和当前 config revision；profile 列表只返回 key/type/enabled/safe label。
- `PUT /api/notifications/config`：body 为完整 `NotificationSettingsInput`，要求 `If-Match`；成功 `200` 返回新 view 和 `ETag`。缺 header 为 `428 precondition_required`，revision 不符为 `412 config_revision_conflict`；校验 code、状态、严重度、profile key、阈值、模板和 quiet hours。
- `GET /api/notifications/rules`：`200 {items:[NotificationRuleView]}`。`POST` body 不带 id，成功 `201` + Location；`GET/PUT /{id}` 返回/替换单条规则；`DELETE /{id}` 成功 `204`。规则修改/删除不修改历史 event/job/log。
- `POST /api/notifications/test/{profile_key}`：body `{ "title"?:string }`，不接受任意 body/JSON；成功发送为 `200 {test_id,outcome:"sent"}`，已入队为 `202 {test_id,job_id,outcome:"queued"}`，永久配置错误为 4xx。测试 job 标记 `is_test=true`，不改变 condition。

### 8.3 队列和日志

- `GET /api/notifications/queue`：按 `status`、`profile_key`、`event_code`、cursor、limit 分页，返回 `{items,next_cursor}`；item 只有 title/summary，不返回渲染后的完整 body，另含 next retry、attempt count、manual retry count、lease 是否 active 和 safe error code。
- `POST /api/notifications/queue/{id}/retry`：body `{ "reason":string }`（1..200、审计用但需脱敏）；成功 `202 NotificationJobView`。保留 attempts，增加 manual retry count；`sending` 为 `409 notification_job_conflict`；sent/expired 必须创建关联的新 job，并返回新 id。
- `DELETE /api/notifications/queue/{id}`：只允许取消 pending/scheduled/retrying，成功 `204`；sending 为 409，sent/failed/expired 为 `409 invalid_job_transition`。
- `POST /api/notifications/queue/retry-all`：body 为显式 filter 和 `confirm=true`，`202 {operation_id,matched}`；`POST /clear` 同样要求 filter/confirm，只取消可取消 job。批量操作不能触碰 sending。
- `GET /api/notifications/logs`：支持 event/rule/profile/outcome/date/query/cursor/limit，返回 `{items,next_cursor}`；query 只能搜 safe summary/code/name。
- `POST /api/notifications/logs/clear`：body `{filter,confirm:true}`；成功 `200 {deleted,retention_policy}`；非法/过宽请求为 400，普通清理不能删除 event/condition/job。

### 8.4 SSE 扩展

保留 `/api/events` 的现有客户端事件名和认证行为，新增安全事件名：

- `system.event.created`：SystemEventView，包含 event id、code、severity、status、masked entity、summary 和 occurred_at。
- `notification.queue.changed`：job id、状态、attempt、next retry、safe error code 和 summary。
- `notification.log.created`：日志 id、job/event/rule/profile 引用和结果。

事件落库后才广播；为弥补当前 broadcast 无 replay 的限制，客户端收到 SSE 重连或 lag 后按最后 cursor 调用 REST 增量查询。SSE payload 不复用当前包含完整 `Message` 的 AppEvent payload。

## 9. 前端 Notification Center

新增 `/notifications` 页面，沿用 SimAdmin 的信息架构但使用 SmsRelayed 的 profile 模型：

1. **概览/事件时间线**：按严重度、状态、分类和时间过滤；显示首次发生、最近发生、恢复时间、实体脱敏标识和关联 job。
2. **规则**：从 `/api/system-events/registry` 读取 code 分组；支持 code、分类、最低严重度、状态、实体条件、阈值、恢复、quiet policy、模板和 profile 多选。profile 选择器显示现有 `channel.name`，不显示密钥。
3. **队列**：显示 pending/retrying/sending/failed/expired，展示 next retry、尝试次数、safe error code；支持逐项重试/取消和批量操作。
4. **日志**：展示事件、规则、profile、结果和摘要，可跳转到事件详情；支持保留天数、最大条数和清理操作。
5. **渠道测试**：复用现有 profile test API，明确标记为 test，不把测试结果当作系统恢复。

页面使用 SSE 实时刷新，断线时按 cursor/时间窗口补偿并退回有限频率轮询。空规则、无可用 profile、quiet defer 和 suppressed 必须在 UI 中明确显示，而不是伪装为发送成功。

## 10. 队列、重试、幂等与日志清理

### 10.1 入队与幂等

事件检测、condition 状态变更、命中规则和创建 job 在一个 transaction 内完成。job 的 `idempotency_key` 为稳定哈希：

```text
hash(event_id, rule_id, profile_key, rendered_schema_version)
```

同一 source event 重放、SSE 重连或 worker 崩溃都只能得到一个 job。provider 可能不支持幂等，因此系统仍采用 at-least-once 发送；对 Webhook 可提供 `X-Notification-Idempotency-Key`，并在文档中要求接收端去重。不能宣称“网络重试绝不重复”。

### 10.2 worker 与重试

通知 worker 复用现有 delivery worker 的可靠性策略：`Notify` 唤醒、精确 next-at、lease、并发上限、30 秒安全扫描、确定性 jitter 和 24 小时最大年龄，但抽象出不依赖 `message_id` 的 `NotificationPayload`。不要把系统通知 job 强行塞入当前 SMS `deliveries`。

- claim 只允许 pending/scheduled/retrying 且 next_attempt_at 到期的 job，并原子设置 sending + lease。
- lease 过期可被其他 worker 收回；完成时校验 lease owner，失去 ownership 的旧 worker 不得覆盖新结果。
- 复用现有 ForwardOutcome 分类：2xx 成功；408/425/429/5xx、超时、连接和 transport 为 transient；明确的无效 profile、认证配置错误、其他 4xx/3xx 为 permanent，错误码必须经过 safe normalization。
- 默认 5 次自动尝试、30 秒起步、指数退避上限 3600 秒、最大年龄 24 小时；critical 规则可以配置最大尝试，但必须有上限。
- manual retry 不清零 attempt_count；它只增加 `manual_retry_count` 并记录审计日志。已 sent/expired 的 job 不得原地重发，必须由管理员明确创建新的 retry job 并保留原 job 关联。
- 采用 profile 级 rate limit 和 bounded concurrency；不能像 SimAdmin 一样每轮对所有 due 项无界 spawn。

### 10.3 日志与清理

事件、condition、job、attempt 和 UI log 分层保存：

- system event 默认保留 90 天，critical 事件可配置更长保留；事件清理前保留计数和 last-state 所需的 condition。
- notification logs 默认保留 90 天或 10,000 条，以较严格的配置上限为准；attempts 可采用更短保留期。
- sent/failed/cancelled/expired job 在保留期后批量删除；sending 任务永不被普通清理删除。
- 清理由受控的后台维护任务按批次执行，并报告删除数量和耗时；不要在每次 insert 的同步路径执行大量 DELETE。
- 清理操作写一条不含敏感信息的 admin audit entry；UI 显示实际删除数量和过滤条件。

### 10.4 崩溃、竞态与恢复矩阵

| 故障/竞态 | 持久状态 | 恢复动作 | 幂等边界 |
| --- | --- | --- | --- |
| producer 在 event transaction 提交前崩溃 | 无 event/condition/job | source 若可重放则用 `source_event_id` 重交；否则不补造 | transaction 全回滚 |
| event/condition 已提交、SSE 广播前崩溃 | event/job 存在 | worker 正常 claim；UI 用 cursor 补齐 | SSE 不是真相源，不重复 event |
| 两 producer 同时更新同 condition | 只有一个 state_version 获胜 | loser 重新读取并重算 transition | condition transaction + version 防双 triggered/recovered |
| worker 发送前崩溃 | sending + lease | lease 到期后 reclaim | attempt number/lease owner 条件更新 |
| provider 已接收、worker 记 sent 前崩溃 | sending + lease，外部结果 unknown | at-least-once retry可能重复；携带 notification idempotency header | 不承诺 exactly-once；接收端按 key 去重 |
| 人工 retry 与 worker sending 竞态 | sending | API 409；用户等 lease 结束 | 不清零 attempt、不覆盖 owner |
| rule/profile 在 job 排队后被禁用/删除 | job 仍在 | 发送前重新校验；标 failed/cancelled 的 safe code，不自动改投其他 profile | job 绑定原 rule/profile revision |
| quiet defer 期间条件 recovered | trigger job 仍 scheduled | 策略明确：默认取消未发 trigger，并只发送 recovered 摘要或合并成一次“已恢复” | `(condition active cycle,rule,profile,status)` key |
| 数据库不可用 | 不得只靠内存发送 | producer 返回/记录受控 failure；恢复后从 durable source 重建可重建事件 | 没有 durable event 就不创建外部通知 |
| 服务重启 | active condition/job 从 DB 恢复 | reclaim expired lease、恢复 next deadline；不重发 sent | state/job 均持久化 |

## 11. 隐私与敏感信息脱敏

- 系统事件生产者只能提交 allowlist details；在数据库写入、SSE、REST、日志和通知模板出口重复执行 redaction，不能只信任调用方。
- 手机号默认只显示末两位或不可逆稳定 hash；profile key 可以显示，因为它本身不含 credential；用户名、邮箱、IMSI、ICCID、EID、设备序列号等统一显示类型名加末四位或 hash。
- URL 只显示 scheme/host 的安全摘要，去掉 path 中的 token、query、签名和 webhook secret；HTTP header 永不进入事件/日志。
- provider 错误只允许现有 safe error code、HTTP status 和长度受限的非原始摘要；不得记录 reqwest error 中可能包含 URL 的完整字符串。
- 标题/body 模板变量不得访问短信正文和完整号码；自定义 JSON 做字段名/值递归扫描、大小限制和控制字符清理。
- API/日志/前端均不返回 `[channels]` 中的 secret、token、key、完整 webhook URL 或密码。配置写入继续使用当前原子写入和 0600 权限。
- 不把 `MessageCreated(Message)` 这类内部完整 payload 直接转发到外部渠道；SMS 通知若未来需要，必须先生成单独的脱敏摘要事件。

## 12. 分阶段实施

### Phase 0：契约和迁移

- 冻结 event registry、状态/严重度枚举、脱敏 allowlist、配置 schema 和迁移编号。
- 明确从 `[forward].enabled` 引用 profile 的校验规则；默认关闭新通知发送。
- 增加 schema、API、事件和日志的测试样例，不改现有业务行为。

### Phase 1：事件模型与持久化

- 实现 registry、SystemEvent envelope、condition state machine、SQLite migration 和 safe view。
- 接入已有 delivery worker、config、service、auth、resource 等可获得的观测点。
- 将安全事件写入 DB，并通过受保护 SSE 发布；提供事件查询 API。

### Phase 2：规则与通用通知队列

- 增加 notifications 配置、规则匹配、阈值/恢复、quiet policy、profile 校验和原子入队。
- 提取可复用的 queue worker core，复用现有重试/lease/dispatcher；实现 notification jobs、attempts、logs 和幂等。
- 增加队列/日志 API、重试/取消、维护清理和 metrics。

### Phase 3：Notification Center 前端

- 实现概览、事件时间线、规则、队列、日志和渠道测试页面。
- 使用后端 registry，SSE + cursor 补偿，不在前端维护 code 副本。
- 完成空状态、权限、脱敏、quiet defer、失败和恢复展示。

### Phase 4：统一与加固

- 在不改变既有语义的前提下，让 SMS delivery 与 notification queue 共享 lease、退避、错误分类、指标和维护框架。
- 增加故障注入、升级回滚、重复投递、慢消费者、provider 超时和数据库重启测试。
- 评估是否加入可选的发送确认、按 profile 熔断和更多 modem/eSIM/resource code。

### 12.5 实现前必须关闭的决策项

| ID | 需决策事项 | 本 spec 默认建议 | 未决时的安全行为 |
| --- | --- | --- | --- |
| N1 | notification rules 存 TOML 还是 DB | 保持配置型 rules 在 TOML，并以 revision 快照进 job；事件/condition/job 存 SQLite | 只开放整份 config API，不做双写 |
| N2 | active condition 在 quiet hours 内恢复时如何处理 deferred trigger | 取消 trigger job，发送一条 recovered/短暂故障摘要 | 不发送已经过时的 trigger |
| N3 | sent/expired 的 manual retry 是否原地还是新 job | 新 job，保留 `retry_of_job_id` | 禁止重试 terminal job |
| N4 | critical 的 quiet policy 是否允许 drop | 只允许 bypass/defer，禁止 drop | bypass 并记录审计 |
| N5 | 系统通知是否支持自定义 JSON | 首版关闭；后续用渠道类型 schema + allowlist | 仅发送内置 text payload |
| N6 | 是否兼容 SimAdmin HTTP 200 envelope | 可提供短期 adapter，但目标 router 使用真实 status | 不提供 adapter，前端随目标 API 一起重建 |
| N7 | event retention 后 correlation/job 如何展示 | job/log 保留 event tombstone 的 code/time/hash | UI 显示“事件已按保留策略清理”，不伪造详情 |

## 13. 验收与测试标准

### 13.1 单元测试

- registry code 唯一、版本和 alias 规范化；未知 code 有安全 fallback 但不能绕过规则。
- category/code/severity/status/entity 规则匹配、最低严重度和多规则去重。
- 连续失败、窗口计数、持续时间、去抖、cooldown、repeat、恢复阈值和滞回；边界时间使用可控 clock。
- 重启后 active condition 延续，normal condition 不产生虚假 recovered；一次性 failed 不产生恢复。
- quiet policy、模板 allowlist、JSON schema、敏感字段扫描和 provider safe error normalization。
- 幂等 key 稳定；相同 event/rule/profile 不重复入队，不同 rule/profile 不误合并。

### 13.2 集成与可靠性测试

- 从空库和现有库执行 migration，重复执行不报错；现有 SMS 表、投递、配置和 outbound idempotency 行为不变。
- event + condition + job transaction 在提交前崩溃时全部回滚，提交后 worker 重启能继续处理。
- 两个 worker 并发 claim 同一 job 只能有一个 owner；lease 过期、取消竞态和旧 owner 完成均有覆盖。
- 按当前分类验证 2xx、408/425/429/5xx、其他 4xx/3xx、timeout、DNS/connect、profile missing 的成功/重试/永久失败结果。
- 验证 30 秒安全扫描、精确 retry wakeup、bounded concurrency、最大年龄和退避 jitter；模拟 provider 重复接收并验证 idempotency key。
- 日志、事件和 job 清理按批次执行，不删除 active/sending 数据，完成后仍能查询 condition last-state。

### 13.3 API、SSE 与前端验收

- 未认证用户不能读取事件、规则、队列或日志；响应不包含 secret、token、完整号码、短信正文、完整 URL 或原始 provider body。
- 查询过滤、cursor 分页、ETag 冲突、非法阈值、未知 profile、未知 code 和模板敏感变量返回明确 4xx 错误。
- 事件先落库再 SSE；SSE lag/断线后前端能够通过 cursor 补齐，不重复显示同一 event/job。
- 从触发、排队、重试、发送成功、永久失败到恢复的完整链路可在页面按 event id/job id 追踪。
- 规则保存后只引用现有 profile；禁用 profile、删除规则、quiet defer、suppressed、manual retry 和清理操作均有清晰状态。

### 13.4 验收场景

至少演示以下场景并保留测试结果：

1. 同一 profile 连续三次 transient failure 只产生一次 `delivery.profile_unhealthy` triggered 通知，恢复后产生一次 recovered 通知。
2. 一次永久失败只进入 `delivery.message_failed` 和 notification log，不因下一条短信成功而误发 recovered。
3. worker 重启、数据库重启或 SSE 断线后，active condition、未完成 job 和事件时间线均可恢复。
4. quiet hours 内 warning 被 defer、critical 按策略 bypass、info 被 suppressed；三者在日志中可区分。
5. 同一个 source event 被重复提交时只存在一个 `(event, rule, profile)` job，provider 重试仍记录每次 attempt。
6. 规则引用 `telegram.main`/`webhook.ops` 时直接使用现有 profile；前端/API/日志任何位置都看不到对应 token 或 webhook secret。

## 14. 参考实现来源索引

行号固定到 `eb6f497ad332e59f1f3899acc3b2b9427661fe5c`。后续参考仓变更时按符号重新核验，不应只机械平移行号。

| 主题 | 相对 `/tmp/SimAdmin` 的来源（符号/行号） | 支撑的事实 |
| --- | --- | --- |
| notification config schema/default | `backend/src/config.rs: NotificationEventType` 319–328；`NotificationRule` 469–501；`NotificationChannelInstance` 503–515；rate/log defaults 517–577；`NotificationConfig` 580–629、1005–1013 | 类型、matcher/rule/channel、20/60 限流、90 天/10k、v2 空默认 |
| system event registry | `backend/src/system_event.rs: category/severity/status/codes` 13–94；`SYSTEM_EVENT_DEFINITIONS` 96–443 | 编译期 code/label/default-enabled 列表 |
| SystemEvent wire shape | `backend/src/system_event.rs: SystemEvent` 461–510 | 无 id/correlation/details/fingerprint 的实际字段 |
| emitter gating/login threshold | `backend/src/system_event.rs: SystemEventEmitter` 519–611 | rule gate、直接通知、5 分钟 5 次、内存窗口/冷却 |
| rule code match | `backend/src/notification.rs: system_event_enabled` 495–505；`rule_matches` 2198–2227 | 启用 rule 决定 event 是否继续，system event 精确 code 匹配 |
| route/quiet behavior | `backend/src/notification.rs: route_event_for_rule` 704–830 | channel fanout、quiet 直接跳过、disabled/missing log |
| DDNS threshold | `backend/src/notification.rs: ddns_failure_threshold_pending` 2229–2244 | failure_count modulo threshold |
| queue enqueue/worker | `backend/src/notification.rs: enqueue_notification` 1059–1089；`run_queue_worker` 1091–1112；`process_notification_queue_item` 1114–1207；`retry_backoff_seconds` 2325–2328 | 5 次、5 秒 poll、batch 20、spawn、60–3600s、无 lease |
| synchronous log cleanup | `backend/src/notification.rs: record_notification_log_raw` 904–949 | 每次 insert 后执行 retention/max delete |
| notification tables | `backend/src/db.rs` 452–511 | log/queue columns、body/error、expires_at 未进入 worker predicate |
| queue mutations | `backend/src/db.rs: retry_notification_queue_item` 1321–1335；`retry_all...` 1349–1362；`clear...` 1364–1373；mark methods 1424–1498 | manual retry 清零、sending 可被 retry/cancel、无 attempt history/lease |
| notification API routes | `backend/src/main.rs` 839–878 | config/test/log/queue method/path |
| notification handlers | `backend/src/handlers.rs` 3911–3975、3979–4030；`backend/src/notification_queue.rs` 13–119 | HTTP 200 envelope、query/default、test data.success |
| resource condition monitor | `backend/src/system_event_monitor.rs` 11–101、173–445 | 60 秒周期、进程内 AlarmCounter、阈值和 recovered code 分离 |
| connectivity dependency | `backend/src/system_event_monitor.rs: ping_connectivity` 447–459 | 固定 `ping/ping6` 目标和 1 秒 timeout |
| frontend code drift | `frontend/src/pages/notifications/systemEventModel.ts` 13–102 | 前端静态 registry，缺 backend 的 profile download success/failure code |
| frontend sensitive variables | `frontend/src/pages/notifications/systemEventModel.ts` 104–116 | 模板提供本机号码/运营商变量 |
| Notification Center UI | `frontend/src/pages/NotificationCenter.tsx` 47–166、196–331、396–447；`frontend/src/pages/notifications/NotificationQueueIndicator.tsx` 21–55 | 日志/规则/渠道 tabs、queue drawer、poll/retry/clear |
| frontend API calls | `frontend/src/api/current.ts` 924–990 | config POST、test path、logs offset、queue mutations |
