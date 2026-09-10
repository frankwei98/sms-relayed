# 功能 4：自动化调度中心

状态：设计规格，未实现

## 0. 证据边界与阅读约定

本文核对的 SimAdmin 基线是 `/tmp/SimAdmin` 的 clean `main@eb6f497ad332e59f1f3899acc3b2b9427661fe5c`。除非另有说明：

- 标题或段落中写明“SimAdmin 已确认”的内容，表示可由该提交的代码直接证明，是兼容性参考，不自动成为 SmsRelayed 的目标行为。
- “重建契约”“必须”“应”表示本文为 SmsRelayed 规定的目标语义；它可以有意修正参考实现的并发、幂等、安全或恢复缺陷。
- “建议”“推荐默认值”“可选”表示尚可在实现评审时调整；调整后必须连同 API schema、migration 和测试 fixture 一起固化，不能让前后端各自猜测。
- 本文不要求复刻 SimAdmin 的 HTTP 200 错误 envelope、固定北京时间、进程内去重或直接 Modem 发送路径。需要兼容旧客户端时，应增加显式 compatibility adapter，不能污染下述新领域模型。

## 1. 背景与设计结论

本规格把 SimAdmin 的自动化中心迁移为适合 SmsRelayed 的“持久化调度 + 既有业务 worker 执行”模型。调度器只负责判断何时运行、创建一次执行记录、取得资源准入并调用领域 handler；它不复制短信发送、转发、Modem 恢复或消息恢复逻辑。

核心结论如下：

1. 所有自动发送必须进入现有 `Messaging` 出站状态机，不能直接调用 ModemManager 或另建一条发送路径。
2. 调度执行与短信消息是两个不同的实体。一次执行可以关联一个出站 `message_id`，但不能用“插入一条消息”代替执行记录。
3. `Pending`、`uncertain`、`unknown` 是“结果尚未确定”，不是失败；任何超时或进程重启都不得据此盲目重发。
4. 转发 worker、出站短信 worker、Modem/eSIM 恢复和设备重启必须通过明确的资源准入协议协作，不能靠任务名称或 UI 约定避免冲突。
5. 运行日志是审计事实，内存中的轮询去重只能是优化，不能是正确性保证。

## 2. 目标与非目标

### 2.1 目标

- 提供固定时间、固定间隔和手动触发三种统一模型。
- 在服务重启、调度器重启、网络抖动、Modem 重连和时钟变化后保持可解释、可恢复的执行状态。
- 为每次执行提供持久化的 run ID、状态、尝试、触发来源、调度时刻、锁冲突、错误码和关联消息。
- 复用 SmsRelayed 已有的短信幂等、出站 lease、Modem 对象恢复、转发 lease 和 retention 语义。
- 通过 HTTP API、SSE 和前端 Automation Center 管理任务、立即执行、取消、查看运行记录和调度健康状态。
- 让自动任务、人工发送、转发 worker 和 Modem/eSIM 恢复在并发下有确定的行为。
- 为后续安全地加入基带重启、eSIM 切换和设备重启建立资源锁、确认和恢复基础。

### 2.2 非目标

- 第一阶段不提供任意 shell/脚本执行、任意 D-Bus 方法调用或用户自定义插件。
- 不替换 `Messaging`、`DeliveryWorker`、`InboundWorker`、出站恢复逻辑或 retention worker。
- 不把短信转发 profile 当作自动化通知渠道，也不把自动化结果发送成短信形成自触发循环。
- 不实现 SimAdmin 的 Hub 中央调度、设备本地 fallback 或多节点 leader 选举；SmsRelayed 当前按单进程、单 Modem 实例设计。
- 不承诺“发送超时后自动再发”；无法证明未发送时，必须停在未确定状态。
- 不在第一批任务中默认启用设备重启、Modem reset、基带重启或 eSIM profile 切换。

## 3. 参考实现与现状差异

### 3.1 SimAdmin 的触发模型

SimAdmin 当前的 scheduler 每 30 秒轮询一次配置：

- 固定时间任务以星期一至星期日和 `HH:MM` 匹配，使用固定的北京时间 `+08:00`；`fixed_last_run` 只在内存中防止同一分钟重复触发。
- 间隔任务以该任务最后一条执行日志的时间为基准；没有日志时立即执行。失败日志也会成为下一次间隔的基准。
- 任务配置只有 fixed 和 interval，没有持久化的 manual trigger 类型；手动测试通过另一个 API handler 后台启动。
- handler 通过 `AutomationTaskHandler` trait 注册，现有类型包括 `restart_baseband`、`reboot_device`、`backup_data`、`send_sms`。
- scheduler 为每个任务直接 `tokio::spawn`，没有统一的任务锁、资源锁、重叠策略或持久化调度 slot。
- 每次执行写入 SQLite automation log，状态只有 success/failed；随后构造 `AutomationEvent`，交给通知规则、quiet hours、限流和通知队列。
- 默认执行超时为 60 秒，短信的随机延迟还会叠加在等待时间上；设备重启 handler 在真正重启前就返回成功，因此日志可能早于实际结果。
- `/automation/test/{task_id}` 与 scheduler 执行路径没有统一的并发控制，可能和定时执行同时运行。
- Hub 在线时由 Hub 负责调度；设备本地任务在 Hub 离线超过 fallback 时间后恢复，Hub 规则不会下发并保存到设备本地。

这些行为适合作为产品交互参考，但不能原样复制：固定 offset、内存去重、以最后日志推导 interval、直接 spawn 和“重启已成功”都会在 SmsRelayed 的未知发送状态、设备重启和恢复场景下产生错误结论。

#### 3.1.1 SimAdmin 已确认的配置与 HTTP 形状

参考实现把全部任务作为一个配置对象整体读写；没有单任务 CRUD、revision、run resource、取消或 scheduler health API：

```json
{
  "enabled": true,
  "tasks": [
    {
      "id": "task-...",
      "name": "...",
      "enabled": true,
      "trigger": {
        "type": "fixed",
        "config": { "weekdays": [1, 2, 3, 4, 5, 6, 7], "times": ["04:00"] }
      },
      "action": { "type": "restart_baseband", "config": null }
    }
  ]
}
```

`trigger` 只接受 `fixed {weekdays,times}` 或 `interval {interval_value,interval_unit}`。`action` 只接受 `restart_baseband`、`reboot_device {delay_seconds}`、`backup_data {components,storage.local_dir}`、`send_sms {phone_number,content,random_delay_seconds?,retry_limit?}`。Rust 默认只有 `automation.enabled=true,tasks=[]`；业务范围和字符串格式没有统一后端 validation 层。

| 方法与路径 | 已确认请求 | 已确认成功 data | 已确认失败兼容语义 |
| --- | --- | --- | --- |
| `GET /api/automation/config` | 无 | 上述完整配置 | 通常仍是 HTTP 200 envelope |
| `POST /api/automation/config` | 完整 `AutomationConfig` JSON；覆盖整个数组 | `{}` | 保存失败仍为 HTTP 200，envelope `status:error` |
| `GET /api/automation/logs` | query：`type,status,q,start_date,end_date,limit,offset`；limit 默认 100，DB 层 clamp 为 1..200，offset 最小 0 | `{logs,total}`；倒序 | 查询失败仍为 HTTP 200 error envelope |
| `POST /api/automation/logs/clear` | 可选 JSON：`type,status,start_date,end_date` | `{deleted}` | 同上 |
| `POST /api/automation/test/{task_id}` | 无 body 要求 | 立即返回 `{}`，只表示后台 future 已 spawn | task 不存在仍 HTTP 200 error；没有 run ID |

通用 envelope 是 `{status,message,data}`；因此兼容客户端必须先检查 body 中的 `status`，不能只检查 HTTP status。SmsRelayed 的重建 API 不沿用这一歧义。

#### 3.1.2 SimAdmin 已确认的精确执行边界

- fixed 匹配时先把 `task_id -> YYYY-MM-DD HH:MM` 写进进程内 map，再 spawn handler；同一分钟内即使 handler 失败也不会由 scheduler 再触发。进程重启会丢失 map，同一分钟内重启可能再运行一次。
- interval 从最后一条已完成 automation log 的北京时间字符串计算。没有日志、时间无法解析时立即触发；未知 unit 被当作 180 天。run 执行期间尚无新日志，因此一个运行超过 30 秒的 interval task 可能被重复 spawn。
- scheduler 首次评估发生在启动约 30 秒后，之后每 30 秒扫描。它没有 durable slot、scheduler lease、全局并发上限、per-task mutex 或资源锁。
- scheduler 仅在全局 enabled、task enabled 且本地拥有 Hub 调度权时运行；手动 test 路径不检查全局 enabled、task enabled 或 Hub ownership，也不复用 scheduler 的 `execute_task` 准入检查。
- handler 外层 deadline 是 `60 + configured_random_delay_seconds` 秒，不是“实际随机延迟 + 60 秒”；超时详情固定写“超过60秒限制”。日志只产生 `success|failed`，没有 queued/running/unknown。
- `send_sms` 支持的模板变量只有 `{{时间}}` 和 `{{随机字符串}}`；发送前随机等待为 `[0,random_delay_seconds)`，总尝试次数是 `retry_limit + 1`，失败间隔固定 5 秒。它直接调用 ModemManager，之后另行插入 SMS 行，不具备稳定幂等 key。
- `reboot_device` 只 spawn 延迟重启 future 就返回 `Ok(())`，所以 run log 可在机器真正重启或失败前写 success。`backup_data` 同步生成本地 ZIP；`restart_baseband` 复用当前数据/漫游/APN 状态。
- 每次完成后先插入 SQLite `automation_logs`，再同步调用 automation notification；通知失败只 warning，不改变业务 log 的 success/failed。

#### 3.1.3 SimAdmin 已确认的前端行为

- 新任务默认 action 为 `restart_baseband`；设备重启延迟 5 秒；短信随机延迟 120 秒、retry limit 3；fixed 默认为每天 04:00；interval 默认为 180 天。
- task ID 由浏览器时间戳和 `Math.random()` 生成。新增、编辑、启停和删除都会重发完整任务数组；保存时前端强制全局 `enabled=true`，没有 ETag 或并发编辑保护。
- “测试执行”拿到 HTTP 响应即显示“指令已下发”，1.5 秒后刷新配置/日志；长任务可能尚无日志，页面也没有可继续跟踪的 run ID。
- 卡片状态来自最近 100 条日志在前端按 task ID 聚合；因此历史超过窗口或并发执行时，卡片不是可靠的当前运行状态。

### 3.2 SmsRelayed 当前运行能力

`src/runtime.rs` 当前启动并行的长期工作包括：

- `InboundWorker`：从 Modem 接收短信，持久化 inbound 消息并用 dedupe key 抑制重复；插入时可在同一事务创建 forwarding deliveries。
- `DeliveryWorker`：只负责把 inbound 消息投递到配置的 forwarding profiles。每条 delivery 有数据库 lease、owner token、重试时间、attempt sample 和 terminal 状态；转发语义是 at-least-once，网络超时可能导致重复转发。
- `Messaging` outbound worker：处理创建、准备 Modem SMS 对象、发送、恢复和出站事件发布。
- retention worker：约每 6 小时运行；无论消息 retention 是否启用，都会清理超过 30 天且已解除关联的 outbound idempotency key。
- HTTP API、SSE EventBus、Modem 状态/动作和 service restart control。

当前没有 automation 配置、automation 表、scheduler lease 或通用 automation notification。`AppEvent` 目前包含消息、配置保存和 service restart 事件；SSE 是进程内 broadcast 的实时视图，不是可靠审计存储。

### 3.3 短信幂等与未确定状态

现有 `Messaging::send` 会先持久化出站消息并返回 `Sent`、`Failed` 或 `Pending`。出站状态机包含 `created`、`prepared`、`send_started`、`uncertain`、`unknown`、`complete` 等阶段，并用 owner/lease 防止多个 worker 同时推进同一消息。

- Web API 的 `/api/messages/send` 要求 `Idempotency-Key`。同 key 且 phone/body/source 相同会复用同一消息；同 key 不同请求会返回冲突。
- Modem 已接受请求但本地没有可靠结果时，消息保持 `sending`，出站恢复会检查同一个 Modem SMS 对象；不会盲目创建第二个对象重发。
- `Pending` 的含义是 outcome unresolved；CLI 也会在存在未确定出站消息时提醒用户，不应直接再次发送。
- 服务重启恢复时，尚未准备 Modem 对象的创建阶段可标记为中断失败；已准备、已开始、uncertain 或 unknown 的消息必须先 reconcile，无法证明未发送就保持 unknown/pending。
- 删除规则禁止删除大多数仍在发送中的消息；unknown 是可供人工处理的特殊状态。消息删除后，关联 idempotency key 可能暂时保留，但超过保留窗口或关联消息不可用后，旧 key 不保证可 replay。

### 3.4 retention 与 API 能力

SmsRelayed 使用 SQLite/WAL、外键、状态约束和事务。消息 retention 按 `max_age_days` 分批清理 terminal 消息，但只要存在 pending/in_flight/retry_wait 的 forward delivery 就保留消息；发送中的消息不会被当作普通 terminal 数据清理。当前配置使用 TOML，HTTP 配置变更通过 revision/ETag 防止覆盖并可能需要 service restart。

现有 API 能力包括：

- `/api/messages`、`/api/conversations`：消息和会话查询、读取、收藏、删除。
- `/api/messages/send`：带 idempotency key 的发送。
- `/api/events`：SSE 消息/配置/服务事件。
- `/api/config`、`/api/status`、`/api/health`、`/api/service/restart`。
- `/api/modem/status`、`/api/modem/enable`、`/api/modem/disable`、`/api/modem/reset`；Modem 动作已有 `action_in_progress`、reset rate limit、确认和同源防护。
- `/api/forwarding/attempts`：forwarding profile 最近尝试样本。

因此，automation 应增加编排 API，而不是把任务数组塞入现有 `/api/config` 或复制 `/api/messages/send` 的实现。

## 4. 推荐领域模型

### 4.0 术语

| 术语 | 重建契约中的唯一含义 |
| --- | --- |
| task definition | 可编辑、带 revision 的长期计划与 action 定义；不是一次执行 |
| slot | scheduler 根据某个 task revision 算出的一个应触发 UTC instant |
| run | 对一个 slot 或一次 manual 请求的持久化执行实例，拥有不可变 run ID |
| attempt | run 在取得准入后的一次 handler 尝试；retry 会新增 attempt，不新增同一 slot 的 run |
| trigger source | `scheduled|manual|recovery`；manual 是 run 来源，不是持久 schedule 类型 |
| waiting | 外部 durable worker 已接受工作、结果尚未 terminal |
| unknown | 外部副作用是否发生无法证明；既不是 failed，也不能自动重试 |
| blocked | 当前不满足资源、维护窗口或 capability 准入；必须带稳定 reason code |
| skipped | 策略明确决定不执行该 slot，例如 misfire/overlap=skip；不是异常失败 |
| resource lease | orchestrator 对共享资源的有期限所有权；不同于 Messaging/DeliveryWorker 自己的数据行 lease |
| action fingerprint | task revision、规范化 action type/params 和 actor category 的稳定摘要，用于检测幂等 key 参数漂移 |

### 4.1 TaskDefinition

任务定义是用户管理的长期对象，建议字段如下：

```text
TaskDefinition
  id: opaque UUID/string, immutable
  revision: monotonically increasing integer
  name: display name
  enabled: bool
  timezone: IANA timezone, explicit per task
  trigger: Fixed | Interval
  action: registered action payload
  policy:
    max_concurrency = 1 by default
    overlap = skip | coalesce | queue, default skip
    timeout
    retry policy
    dangerous confirmation/maintenance-window policy
  created_at, updated_at: UTC RFC3339
  deleted_at: optional UTC RFC3339, preferably soft delete
```

Manual 不是 task definition 的第三种永久 schedule，而是一个合法的 `trigger_source=manual`。同一个 task 可以被固定/间隔调度，也可以被 API 手动运行；两者必须进入同一 run admission 和锁路径。

### 4.2 Trigger

#### Fixed

```text
Fixed {
  timezone: "Area/Location",
  weekdays: [1..7],
  local_times: ["HH:MM"],
  misfire: skip | fire_once | next_valid_time
}
```

固定时间按照 task timezone 的民用时间计算，但 `scheduled_for` 永远保存为 UTC instant。一个实际触发 slot 使用 `(task_id, task_revision, scheduled_for)` 唯一约束，不能用进程内 HashMap 作为唯一去重手段。

#### Interval

```text
Interval {
  every: positive duration,
  anchor: created_at | explicit_start,
  misfire: skip | fire_once
}
```

间隔是经过时间的 duration，以 UTC instant/单调时钟计算，不从“最后一条日志”推导；失败、跳过或锁冲突是否推进计划由持久化的 `scheduled_for` 和 misfire policy 决定。默认 anchor 为任务创建/启用时刻，停机期间不补发一串历史 interval，只按 `fire_once` 最多补一个。

#### Manual

`POST /api/automation/tasks/{id}/runs` 创建一次 run。请求可以带调用方 idempotency key、参数覆盖和 `dry_run`（只对支持的 handler 有效），但不能绕过 task 的 schema、资源锁和危险动作保护。

### 4.3 时区、DST 和时间存储

- 全部数据库时间使用带 `Z` 或明确 offset 的 RFC3339；UI 可按浏览器时区展示，但不改变任务语义。
- 每个固定时间任务必须保存 IANA timezone，不能只保存 `+08:00`。全局默认建议为 `Etc/UTC`，创建任务时前端应明确显示并要求确认；使用中国时间时保存 `Asia/Shanghai`，不能保存 `+08:00` 代替它。
- DST 春季跳过的 local time 默认 `skip` 并记录 `dst_nonexistent`；`next_valid_time` 必须显式选择，不能静默提前或延后。
- DST 秋季重复的 local time 默认只触发一次，使用较早的有效 instant，并记录 `dst_ambiguous`；如确实需要两次，任务必须选择 `both_occurrences`，且 slot key 包含 offset。
- interval 不受夏令时前进/回拨影响；固定时间才受 wall-clock 规则影响。
- 系统时钟回拨、NTP 校时或睡眠唤醒不能导致同一 slot 重跑。scheduler 每次计算应记录 `observed_at`、`scheduled_for` 和 misfire 原因，避免以当前时间直接覆盖事实。

### 4.4 Execution/Run

每次触发创建一个持久化 `AutomationRun`：

```text
AutomationRun
  id: UUID
  task_id, task_revision
  trigger_source: scheduled | manual | recovery
  scheduled_for: nullable UTC instant
  state: queued | running | waiting | succeeded | failed |
         timed_out | cancelled | skipped | blocked | unknown
  attempt, max_attempts
  idempotency_key / action_fingerprint
  linked_message_id: optional
  resource_snapshot: names acquired or conflict reason
  started_at, finished_at, next_retry_at
  error_code, redacted_detail
  cancellation_requested
```

`waiting` 表示 handler 已提交到既有 durable worker，正在等待可观察的 terminal 结果；`unknown` 表示无法判断外部副作用是否发生；二者都不能被 UI 误显示为失败。`skipped`/`blocked` 必须记录是 disabled、重复 slot、锁冲突、维护窗口、Modem 不可用还是 misfire。

`AutomationRunAttempt` 保存每次准入和 handler 尝试的开始/结束、超时、错误码、重试决策和锁信息。日志可以被 retention 清理，但 run ID、关联消息 ID 和最终状态必须足以解释一次动作。

### 4.5 Handler 注册与契约

保留 SimAdmin `AutomationTaskHandler` 的注册思想，但 handler 应遵守统一异步契约：

```text
validate(params, capabilities) -> normalized params
resources(params) -> ordered resource set
execute(ctx {run_id, deadline, cancellation, store, messaging, modem})
  -> Succeeded | Waiting(link) | Failed(code) | Unknown(code)
```

handler 不自行创建 scheduler task、不直接写 automation log、不直接发通知、不自行实现无限重试。所有结果、取消、超时、attempt 和事件由 orchestrator 统一落库。短信 handler 只能依赖 `Messaging` 接口；retention handler 只能调用现有 store retention；Modem handler 只能依赖带 action gate 的 Modem service。

## 5. 调度、锁、并发与故障语义

### 5.1 Scheduler loop 与 durable lease

- scheduler 可以使用短轮询加精确 next wake，但每轮必须从数据库读取 enabled task 和持久化 next slot；30 秒轮询只能作为安全扫描。
- 进程启动时生成 owner UUID，在 `scheduler_leases` 取得带过期时间的单实例 lease；失去 lease 后停止领取新任务，仅允许已提交的业务 worker完成恢复。
- 领取 slot 使用 SQLite `BEGIN IMMEDIATE`/条件更新和唯一约束。先写 queued run，再提交；commit 成功才可 spawn handler。
- 重新启动时，过期的 scheduler lease、queued run 和 running run 都要被扫描。queued 可重新排队；running 不能直接改成 failed，应依照 handler 类型标记 `interrupted`/`unknown` 或进入 reconcile。
- 不使用无限 `tokio::spawn`。scheduler 有全局 max running、每 task max concurrency 和资源锁；排队、跳过和阻塞都写 run。

### 5.2 资源锁

建议使用统一的资源名称和固定获取顺序：

1. `system.lifecycle`：设备/系统重启和服务生命周期。
2. `modem.recovery`：Modem/eSIM 恢复、profile 切换、基带恢复。
3. `modem.control`：enable、disable、reset、基带控制。
4. `modem.sms`：通过 Modem 发出站短信。
5. `storage.maintenance`：长时间或批量数据库维护；实际 SQLite transaction 必须短，不能持有它等待网络或 Modem。

锁是带 owner、过期时间和诊断信息的 lease。任何 handler 只能声明自己需要的集合；禁止先取得低序锁再等待高序锁。`modem.sms` 的并发默认 1，人工发送和自动发送共用同一准入队列；这不改变现有 outbound worker 的 durable lease，只限制 Modem 交互。

同一 task 默认禁止重叠。固定任务默认 `skip`，interval 默认 `coalesce` 一个待运行 slot；用户若选择 queue，必须有上限，队列满时记录 blocked。手动执行不能绕过同 task 的重叠策略。

### 5.3 超时、取消与重试

- 每个 run 有排队超时、handler deadline 和整体超时；超时后发送取消通知，handler 必须尽力停止等待。
- 取消只取消“尚未产生外部副作用”的工作。短信已进入 `send_started` 后不能因为 HTTP timeout/cancel 停止并重发，必须等待 `Messaging` reconcile；run 进入 waiting/unknown。
- Modem reset、基带操作和设备重启不能在不确定阶段自动重试。取消窗口只存在于危险动作 commit 前。
- handler 错误需分类为 `validation`、`blocked`、`not_attempted`、`transient`、`permanent`、`unknown`。只有明确 `not_attempted` 或可证明无副作用的 transient 才可自动重试。
- retry 使用持久化次数、指数退避和确定性 jitter，并有总时限；不能同时叠加 SimAdmin 式 handler 内部盲重试与 scheduler 重试。
- 对短信，首选一次 `Messaging::send` 并等待/关联出站消息；其内部的 finalizer、outbound worker 和恢复机制是发送重试边界。已知 failed 后若要重新发送，应创建新的显式 action attempt，并先记录前一次已确定终止，不能复用一个可能已发送的操作。

### 5.4 幂等

- scheduled run 的唯一键为 `(task_id, task_revision, scheduled_for)`；同 slot 再次被扫描时返回原 run。
- manual run 支持 `Idempotency-Key`；同 key 必须绑定 canonical task revision、action 参数和 actor category，不同参数返回 409。
- 短信 action 为每个 run/step 生成稳定的 outbound idempotency key，并传给 `Messaging`。重启、scheduler 重试或客户端重复请求都只能得到同一 `message_id`，不得创建第二条短信。
- automation run 的 key 生命周期不能短于它的 pending/unknown 处理窗口；消息 retention 删除关联消息后，不应假装旧 key 仍可 replay，而是向 UI 返回 replay unavailable，需要人工创建新的 run。
- retention、日志清理和 schedule 计算都必须是可重复的。备份类动作若以后加入，应使用 snapshot ID、临时文件和原子 rename；不能通过“再次执行”覆盖未知的半成品。

### 5.5 设备重启和其他危险动作

设备重启、Modem reset、基带重启和 eSIM profile 切换属于不可逆或会中断通信的动作：

- 第一批默认不开放；必须有全局 `allow_dangerous_actions=false`、任务级 enabled、维护窗口、冷却时间、明确 confirmation phrase 和 authenticated actor。
- 执行前写入 durable plan/commit marker，阻止新的短信和 Modem destructive action；没有 commit 的任务可以取消，已有 commit 的任务不可撤销。
- `reboot_device` 不得在发出 reboot command 后立即记 success。成功需要 post-boot marker、服务重新启动并恢复数据库后确认；否则为 `unknown`/`interrupted`，等待人工检查。
- reboot 前不得尝试“为了完成任务”重发所有 pending SMS。出站恢复仍按已有 Modem 对象和 unknown 规则处理。
- 恢复后清理过期 lock，检查本次 generation、Modem fingerprint/path 和未完成 run；任何旧 owner 不能继续操作新 Modem。

## 6. 第一批适合 SmsRelayed 的任务

第一阶段只交付低风险、可用既有状态机解释的任务：

### 6.1 `send_sms`

支持 fixed、interval、manual。参数为目标、正文模板和可选业务 metadata；模板只允许白名单变量（例如计划时间、run ID 的短 hash），不允许执行表达式或读取任意文件。

执行流程：

1. 校验目标和正文大小，在创建 run 前完成脱敏/规范化。
2. 取得 `modem.sms` 准入；通过 `Messaging::send` 创建带稳定 idempotency key 的 outbound message。
3. run 保存 `linked_message_id`，等待 `MessageUpdated` 或重新从 store 查询。
4. `sent` 映射为 succeeded，已知 `failed` 映射为 failed；`Pending`、`sending`、`uncertain`、`unknown` 映射为 waiting/unknown，禁止自动盲重发。

自动任务不能直接复制 SimAdmin `send_sms` 中的随机延迟、逐次 Modem 直发和无限外层 retry。若需要发送节流，应在 `modem.sms` admission queue 中实现并记录排队时间。

### 6.2 `run_retention`

支持 fixed、interval、manual。它调用既有 `MessageStore::run_retention`，沿用 terminal message、active forwarding delivery 和 batch size 规则，不删除 pending/unknown outbound。该任务只取得短时 `storage.maintenance`，不持有锁等待网络，不暂停 DeliveryWorker。

automation run 的保留策略独立于消息 retention：默认可沿用 90 天/最多 10000 条，但 automation log 的清理不能改变短信 idempotency 或出站消息安全窗口。

### 6.3 `reconcile_outbound`

可选地提供只读/低风险手动任务，调用现有出站恢复/状态检查，报告仍为 sending/unknown 的 message ID 数量和错误码，不创建发送动作。它不能替代启动时的 outbound worker，也不能由 interval 高频运行。

### 6.4 后续任务

以下任务在资源协作和恢复验收完成后再开放：

- `modem_reset`/`restart_baseband`：需要 `modem.recovery + modem.control`，与现有 Modem action gate 合并。
- `esim_profile_switch`：需要 Modem generation、profile 验证和恢复后 capability 检查。
- `device_reboot`：需要 `system.lifecycle`、计划/commit/post-boot 机制和人工确认。
- `backup_data`：只有在确认 SmsRelayed 的数据库/配置备份边界、敏感数据保护和原子写入后再加入；不能照搬 SimAdmin 的本地备份路径而暴露 password、auth key 或短信正文。

## 7. 如何避免与现有 worker 和人工操作冲突

| 参与者 | 统一入口/资源 | 自动化规则 | 其他规则 |
| --- | --- | --- | --- |
| 人工 Web/CLI 发送 | `Messaging`、`modem.sms` | 自动任务不得抢占或另建 Modem 发送路径 | CLI 无 key 时仍由现有 pending 提示保护；Web key 冲突按现有 409 语义处理 |
| 自动发送 | `Messaging`、`modem.sms` | 使用稳定 outbound idempotency key；等待出站 terminal 或标记 unknown | scheduler timeout 不取消已开始的副作用 |
| Inbound/forwarding worker | `forward_deliveries` lease、网络 profile | automation 不调用 DeliveryWorker，也不把 forward delivery 当短信发送结果 | at-least-once 和网络重复转发保持原语义；retention 不删 active delivery |
| Modem/eSIM recovery | `modem.recovery`、`modem.control` | recovery 期间阻止新的 destructive task 和新的 Modem SMS admission | 已有 outbound 仍由 Messaging reconcile；不因 recovery 失败而重发 unknown |
| `/api/modem/*` 人工动作 | 现有 Modem action gate | automation 必须使用同一个 gate；`action_in_progress` 转为 blocked/409 | reset 的确认、限流、同源检查不能被 scheduler 绕过 |
| 设备/服务重启 | `system.lifecycle` | 先 durable plan，再 commit；阻止新的 destructive action | service restart 与 device reboot 状态分开记录；不提前写 succeeded |
| retention/DB maintenance | `storage.maintenance`、短 SQLite transaction | 不等待 Modem/HTTP，不改消息发送状态机 | automation log 单独 retention；使用 SQLite 事务和 batch |

锁冲突只会导致 `blocked`/`skipped` 或可观测排队，不会让 handler 私自绕过锁。手动操作和自动操作的 priority 可区分，但必须公平并设置最大等待时间，防止 interval 任务饥饿人工发送，也防止连续人工操作永久饿死定时任务。

## 8. 配置与数据库持久化

### 8.1 TOML 全局配置

建议只在现有 TOML 增加全局策略和安全默认值，不把完整任务数组存进 `/api/config`：

```toml
[automation]
enabled = true
default_timezone = "Etc/UTC"
max_running = 2
sms_lane_concurrency = 1
run_retention_days = 90
run_retention_max_entries = 10000
allow_dangerous_actions = false
```

现有配置保存采用 revision/ETag、原子安全写入并可能需要 restart；automation 任务的 CRUD 和启停应实时生效，不能要求用户通过保存整个 AppConfig 来修改任务。全局策略变更若需要 restart，API 必须明确返回，不得静默使用旧配置。

### 8.2 SQLite 表建议

通过 schema migration 增加以下表和索引：

- `automation_tasks`：任务定义、revision、trigger/action/policy JSON（或规范化列）、timezone、enabled、soft delete、created/updated。
- `automation_runs`：run ID、task revision、trigger source、scheduled_for、state、attempt、idempotency key、linked message ID、deadline、next retry、redacted detail、时间戳。
- `automation_run_attempts`：每次尝试、错误码、等待/锁诊断、开始/结束和 retry decision。
- `automation_schedule_slots` 或 `automation_runs` 上的唯一索引：保证 task revision/scheduled slot 只产生一个 run。
- `automation_scheduler_leases`：单实例 owner、lease expiry、last heartbeat、schema version。
- 可选 `automation_event_outbox`：需要可靠外部通知时持久化事件；不能只依赖 `EventBus` broadcast。

写 task、创建 scheduled run、更新 slot 状态必须在短事务中完成。所有 JSON 进入数据库前由 handler registry 做 schema validation；禁止将未校验的 action type 直接反序列化后执行。

任务删除默认 soft delete：新 slot 不再触发，历史 run 和审计仍可查询。硬删除必须有明确管理 API，并且不能删除仍有 waiting/unknown run 的 definition metadata。

automation log retention 单独按天数和最大条数批量清理，保留 non-terminal run，或先转换为 `interrupted/unknown` 再按审计策略处理。不得复用消息 retention 的删除条件清理 automation run。

## 9. API 与前端交互

### 9.1 API

所有 automation endpoint 需要现有 session authentication；修改任务、手动运行、取消和危险动作还要通过 JSON content type、同源和 CSRF 等价保护。建议 API：

```text
GET    /api/automation/status
GET    /api/automation/tasks
POST   /api/automation/tasks
GET    /api/automation/tasks/{id}
PUT    /api/automation/tasks/{id}          If-Match: task revision
POST   /api/automation/tasks/{id}/enable
POST   /api/automation/tasks/{id}/disable
DELETE /api/automation/tasks/{id}           soft delete
POST   /api/automation/tasks/{id}/runs      manual trigger, 202 + run_id
GET    /api/automation/runs                 filters/pagination
GET    /api/automation/runs/{run_id}
POST   /api/automation/runs/{run_id}/cancel
POST   /api/automation/preview              next slots/DST/misfire preview
```

约定：

- 创建 manual run 支持 `Idempotency-Key`；同 key 重复请求返回原 run，参数冲突返回 409。
- task 更新使用 revision/ETag；旧 revision 返回 412，避免两个浏览器覆盖任务。
- 立即执行返回 202 和 run ID，而不是声称已发送；前端通过详情查询/SSE 展示 queued、waiting 或 unknown。
- 列表分页、按 task/state/source/time/error code 过滤；默认不返回 phone number、正文、完整 idempotency key 或 access token。
- `/api/automation/status` 返回 scheduler owner 状态、最近 heartbeat、queued/running/blocked 数量、下一次 wake 和 recovery barrier；不把“API 可用”当作 scheduler 健康。

#### 9.1.1 重建 API 的最小 JSON 契约

任务 create/update 的 action 和 trigger 必须是 discriminated union；未知字段是否拒绝必须统一，本文规定服务端对 write request 使用严格 schema、拒绝未知 action/trigger/policy 字段：

```json
{
  "name": "daily-report",
  "enabled": true,
  "trigger": {
    "type": "fixed",
    "timezone": "Asia/Shanghai",
    "weekdays": [1, 2, 3, 4, 5, 6, 7],
    "local_times": ["04:00"],
    "misfire": "skip"
  },
  "action": {
    "type": "send_sms",
    "params": { "phone_number": "+...", "body_template": "..." }
  },
  "policy": {
    "overlap": "skip",
    "max_concurrency": 1,
    "timeout_seconds": 120,
    "max_attempts": 1
  }
}
```

创建成功返回 `201` 和完整 task（含 `id,revision,created_at,updated_at,next_scheduled_for`）。更新要求 `If-Match: "<revision>"`，成功返回新 revision；创建/更新时不得把 phone/body 原文复制到通用 audit log。

manual run 请求与响应：

```http
POST /api/automation/tasks/{id}/runs
Idempotency-Key: <opaque 1..128 bytes>
Content-Type: application/json

{"expected_task_revision":7,"dry_run":false}
```

```json
{
  "run": {
    "id": "...",
    "task_id": "...",
    "task_revision": 7,
    "trigger_source": "manual",
    "scheduled_for": null,
    "state": "queued",
    "attempt": 0,
    "linked_message_id": null,
    "created_at": "...Z"
  }
}
```

首次接受返回 `202`；同 key、同 fingerprint 返回原 run（`200` 或 `202` 取决于是否 terminal，并始终带相同 ID）；同 key 不同 fingerprint 返回 `409 idempotency_conflict`。不允许通过 manual body 覆盖 phone/body 等 action 参数，除非该 handler 的 schema 显式声明可覆盖字段，并把覆盖值纳入 fingerprint。

run 详情至少返回 `state,state_reason,state_history,attempts,resources,linked_message_id,cancellation_requested,started_at,finished_at,next_retry_at,error`。`POST .../cancel` 接受可取消请求时返回 `202`；已 terminal 返回当前 run（幂等）；外部副作用不可撤销时返回 `409 effect_already_started`，但可以把“停止等待”作为单独、明确的操作，不能把它叫撤回短信。

#### 9.1.2 重建错误 envelope

所有错误使用合适 HTTP status 和稳定、无敏感内容的结构：

```json
{
  "error": {
    "code": "operation_in_progress",
    "message": "automation resource is busy",
    "request_id": "...",
    "retryable": true,
    "details": { "resource": "modem.sms", "run_id": "..." }
  }
}
```

| HTTP | 稳定 code 示例 | 场景 |
| --- | --- | --- |
| 400 | `invalid_json`,`missing_idempotency_key` | 请求不可解析/缺 header |
| 401/403 | `authentication_required`,`confirmation_required`,`dangerous_action_disabled` | 身份或危险动作保护 |
| 404 | `task_not_found`,`run_not_found` | opaque ID 不存在；不泄漏敏感详情 |
| 409 | `idempotency_conflict`,`operation_in_progress`,`effect_already_started`,`replay_unavailable` | 并发/幂等/副作用边界 |
| 412 | `revision_mismatch` | If-Match 或 expected revision 过期 |
| 422 | `invalid_trigger`,`invalid_timezone`,`invalid_action_params`,`unsupported_action` | schema/capability 校验 |
| 429 | `rate_limited` | 手动或危险操作限流 |
| 503 | `scheduler_unavailable`,`modem_unavailable`,`maintenance_active` | 依赖或恢复 barrier |

handler 的 `failed|unknown|timed_out|blocked` 是已创建 run 的业务状态，通常通过 run resource 返回，不把底层 D-Bus stderr 直接变成 HTTP message。SSE payload 只携带 ID、状态和安全 code；详情由受保护的 run API 读取。

### 9.2 前端 Automation Center

前端应分为任务、运行记录和调度状态三个视图：

- 任务卡显示 enabled、action、trigger timezone、下一次 UTC/local 时间、DST/misfire 提示、上次 run 状态和当前锁冲突。
- 编辑器提供 fixed、interval、manual run 入口；固定时间使用 IANA timezone 选择器，显示下一个实际 instant；interval 明确 anchor 和停机补发策略。
- “立即执行”走与 scheduler 完全相同的 run API；按钮显示 run ID 和“已排队/等待发送”，不在 1.5 秒后自行猜测结果。
- 运行详情显示 trigger source、scheduled_for、attempt、linked message ID、state history、锁名、错误码和 redacted detail。phone/body 只在已有消息详情权限范围内展示，不复制到 automation log。
- 取消按钮根据状态显示：queued 可取消，waiting/已开始的短信只能请求取消等待而不能撤销外部发送；unknown 需要人工 resolve/inspect，不提供“再次发送”快捷按钮。
- 危险动作创建/启用/手动运行均需二次确认、影响说明、维护窗口和 confirmation phrase；默认隐藏或 disabled。
- SSE 重连后必须用 cursor/时间范围重新拉取 run 列表，不能把 broadcast 丢失误判成没有执行。

## 10. 日志与通知

### 10.1 结构化日志

使用 tracing/结构化字段记录：`run_id`、`task_id`、`task_revision`、`trigger_source`、`action_type`、`state`、`error_code`、`scheduled_for`、`resource`、`attempt`、`duration_ms`、`linked_message_id`。允许记录目标的不可逆短 hash 作为排障关联，但默认不记录完整号码。

严禁在 automation log、普通日志、SSE automation payload 或 notification body 中记录完整 phone number、短信正文、auth password、session token、idempotency key 原文或 webhook secret。错误信息也要经过 redaction，不能把第三方响应中的正文带回日志。

### 10.2 AppEvent/SSE

扩展 `AppEvent`/SSE 的建议事件：

```text
AutomationTaskChanged
AutomationRunChanged { run_id, task_id, state, source, linked_message_id, error_code }
AutomationSchedulerChanged { lease_state, next_wake, blocked_count }
```

事件是实时 UI 提示，不是事实来源；事实来源仍是 SQLite run/attempt。事件 payload 使用枚举 state 和 redacted detail，不能直接序列化整个 handler 参数。

### 10.3 外部通知

第一阶段至少提供日志和 SSE。若增加 webhook/通知规则，应单独定义 automation notification profile/outbox：

- 不复用 `forward_deliveries`，不把 automation result 当 inbound SMS forward。
- 不通过 Modem 发通知短信，避免与 `modem.sms` 冲突和自动化自触发。
- 通知失败只能重试通知 outbox，不得改变 task run 的业务结果。
- 通知模板只允许 task name、run ID、action type、state、error code、time、message ID 等安全变量。
- 支持 quiet hours、rate limit、失败重试和通知 retention，但这些策略不能删除业务 run。

## 11. 分阶段交付

### Phase 0：契约与安全基线

- 定义 task/action/trigger/run state、错误码、资源名、状态转移图和 redaction 规则。
- 增加 schema migration 方案、handler registry、统一 run context 和测试 clock。
- 明确 scheduler lease、slot 唯一性、锁顺序和 `Pending/unknown` 的产品文案。

### Phase 1：持久化与低风险手动任务

- 实现 task/run/attempt 表、manual run API、审计查询和 SSE 事件。
- 实现 `send_sms` 与 `run_retention` handler，但短信必须走 `Messaging`，retention 必须走现有 store。
- 加入统一资源准入、同 task 不重叠、run idempotency、超时/取消状态。
- 前端完成任务 CRUD、立即执行、运行详情和敏感字段隐藏。

### Phase 2：fixed/interval durable scheduler

- 实现 IANA timezone、DST、misfire、interval anchor、schedule slot 唯一索引和重启恢复。
- 增加 scheduler status、blocked/skipped 原因、preview API、run retention。
- 以安全扫描兜底但不依赖固定 30 秒轮询保证正确性。

### Phase 3：只读恢复与 Modem 协作

- 增加 `reconcile_outbound` 手动任务。
- 把 `/api/modem/*` 的 action gate 与 automation resource coordinator 统一，验证 recovery barrier、Modem generation 和人工动作冲突。
- 仅在 Phase 1/2 的未知状态、锁和恢复测试通过后，评审基带/Modem reset 任务。

### Phase 4：危险动作

- 在 feature flag 默认关闭的前提下，加入 eSIM/profile switch、基带重启和 device reboot。
- 实现 durable plan/commit、维护窗口、确认短语、冷却、post-boot marker、恢复后的 run reconcile 和明确的 unknown UI。
- 评审是否需要独立的 automation notification outbox；不引入 Hub/multi-node，除非产品范围另行批准。

## 12. 验收标准

### 调度正确性

- 同一 task revision 的同一 fixed slot 无论 scheduler 扫描多少次、进程重启多少次都只创建一个 run。
- interval 以 anchor 和 duration 计算；失败、blocked、停机补发符合声明的 misfire policy，不会因历史 log 缺失而立即风暴执行。
- fixed 任务在 DST 缺失/重复时间的行为与配置一致，所有 UI 和日志能看到 UTC instant 与 local label。
- 禁用、删除、修改 revision 后，旧 queued slot 不会以新定义静默执行。

### 发送与恢复

- 同一 manual key、scheduler slot、服务重启重试和重复点击只产生一个关联 outbound message。
- `Messaging` 返回 `Pending` 或检测到 `unknown` 时，run 显示 waiting/unknown，不自动创建第二条消息。
- Modem recovery、进程崩溃、lease 过期和 Modem path 变化后，outbound worker 仍按既有对象 reconcile；automation 不绕过它。
- 人工发送和自动发送共用 `Messaging` 与 `modem.sms` lane，不能同时对同一 Modem 发出两个未经准入的操作。

### Worker 与危险动作隔离

- automation 不会创建、claim、complete 或重试 `forward_deliveries`；DeliveryWorker 可继续独立转发 inbound 消息。
- retention 不会删除有 active forwarding delivery 或 sending outbound 的消息；它也不会持有锁等待网络/Modem。
- Modem/eSIM recovery 与自动/人工 Modem control 互斥；冲突返回可解释的 blocked/409，不是超时后隐式重试。
- device reboot 在 command 发出前有 durable plan，在重新启动并确认 post-boot 前不会记录 succeeded；重启中断的 run 可被审计和恢复。

### API、前端与审计

- 未认证、旧 revision、重复 key、危险动作缺确认、锁冲突都有稳定的 HTTP status/code。
- SSE 丢事件后前端可重新拉取，不会把实时事件当作唯一日志。
- 运行日志可按 task/state/source/time 分页；不存在完整 phone/body/token/key 泄露。
- 任务删除、run retention、服务重启和数据库迁移都保留可解释的最终状态。

## 13. 测试计划

### 单元与性质测试

- fixed slot 去重、时钟回拨、前进、睡眠唤醒、不同 timezone、DST 缺失/重复、misfire。
- interval anchor、coalesce/skip/queue、任务 revision、enabled/delete 竞态。
- SQLite unique slot、scheduler lease、过期 owner、并发 manual 请求、ETag/If-Match 冲突。
- 状态机转移：queued/running/waiting/unknown/failed/cancelled/blocked，禁止非法 terminal 回退。
- 锁的固定顺序、同资源互斥、队列上限、公平性和取消边界。

### 集成与故障注入

- fake `Messaging` 验证同一 idempotency key 只创建一次；模拟 Sent、Rejected、NotAttempted、Unknown、timeout、process crash 和恢复。
- 使用现有 Modem/SMS 测试 double 验证 prepared、send_started、uncertain、unknown 不会盲重发。
- 并发人工发送、automation send、InboundWorker 和 DeliveryWorker；验证转发 attempt/lease 不被 automation 改写。
- 模拟 Modem reset、eSIM recovery、path/fingerprint 改变、旧 owner lease 过期，验证 barrier 和 generation 检查。
- 模拟 SQLite busy、retention 分批、数据库迁移中断、磁盘满和 notification outbox 重试。
- device reboot 以 fake post-boot marker 测试 plan/commit、非成功前置状态、重启后 reconcile 和 cooldown。

### API/前端端到端

- 登录/session、同源保护、权限、创建/编辑/启停/删除、手动运行、取消、重复提交、分页和 SSE 重连。
- 前端按 UTC/local 展示 next run，正确显示 DST warning、waiting/unknown、blocked 原因和危险确认。
- 对日志、SSE、错误响应做敏感字段扫描，确保号码、正文、password、token、secret 和原始 key 不出现。

## 14. 完成定义

只有当 Phase 2 的 durable scheduler、Phase 1 的短信幂等集成、worker 冲突隔离、重启恢复、API/前端审计和上述关键测试全部通过，才可把“自动化调度中心”标记为可用。Modem/eSIM 恢复和设备重启必须作为独立的危险能力继续保持默认关闭，不能因为基础 scheduler 已交付而自动开放。

## 15. 实现前必须固化的决策

以下目前仍是重建建议而非参考实现事实。开始 migration/API 编码前必须写入 schema 常量和测试 fixture：

- `default_timezone` 最终取值，以及旧任务缺 timezone 时是拒绝、迁移为 UTC 还是显式选择部署时区；不得静默使用主机 locale。
- fixed 的 DST ambiguous 默认取早一次还是要求用户选择；是否支持 `both_occurrences`。
- 每种 trigger 的 misfire/overlap 默认值、queue 上限、排队 timeout、全局 max running 和资源公平策略。
- manual `Idempotency-Key` 的最大长度、保留时间、actor category 组成和 terminal replay HTTP status。
- run/attempt/log 的保留天数与条数。文中的 90 天/10000 条是推荐初值，不是已确认现状。
- `send_sms` 对号码规范化、最大正文长度、模板变量白名单和 handler deadline 的具体限制；应直接复用 Messaging 已有校验时只记录引用，不复制两套常量。
- scheduler lease TTL/heartbeat、进程启动 recovery barrier 和 `running` run 针对每种 handler 的 reconcile 映射。

若未做出某项决定，API 必须返回明确的 `unsupported_policy`，不能退回 SimAdmin 的隐式默认或任意字符串处理。

## 16. SimAdmin 来源索引

以下均为 `/tmp/SimAdmin` 相对路径，行号对应本节开头锁定的提交：

| 事实 | 来源 |
| --- | --- |
| 30 秒 loop、北京时间、fixed 内存去重、interval 从最后日志计算、并发 spawn | `backend/src/automation/scheduler.rs:12-125`，符号 `spawn_automation_scheduler` |
| handler 参数、`60 + delay` timeout、success/failed log 与通知顺序 | `backend/src/automation/scheduler.rs:141-245`，符号 `execute_task` |
| handler trait 与四类 registry | `backend/src/automation/traits.rs:5-12`；`backend/src/automation/tasks/mod.rs:10-36` |
| SMS 模板、随机等待、`retry_limit+1`、5 秒重试及直发/插库 | `backend/src/automation/tasks/send_sms.rs:56-149`，符号 `SendSmsHandler::execute` |
| reboot 提前返回、baseband 和 backup handler | `backend/src/automation/tasks/device_reboot.rs:14-35`；`baseband_reboot.rs:15-39`；`backup_data.rs:23-43` |
| config 默认值及 tagged trigger/action schema | `backend/src/config.rs:1446-1503`，符号 `AutomationConfig/AutomationTask/AutomationTrigger/AutomationAction` |
| log 表结构、分页/过滤/last log/retention | `backend/src/db.rs:656-673,2192-2384`，符号 `insert_automation_log/get_automation_logs/get_last_log_for_task/cleanup_automation_logs` |
| 真实 automation 路由 | `backend/src/main.rs:879-897` |
| HTTP 200 error envelope、manual test 绕开 scheduler 准入 | `backend/src/handlers.rs:4223-4445`，符号 `*_automation_*_handler` |
| 前端默认值、校验和浏览器生成 task ID | `frontend/src/pages/automation/AutomationTaskDialog.tsx:42-117,124-175,200-307` |
| 整包配置保存、强制 enabled、测试后 1.5 秒猜测刷新、最近 100 日志聚合 | `frontend/src/pages/AutomationCenter.tsx:73-139,173-245` |
| 前端实际 API 方法/查询参数 | `frontend/src/api/current.ts:1145-1179`；类型见 `frontend/src/api/contracts.ts:1066-1115` |
