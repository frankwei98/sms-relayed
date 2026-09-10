# 功能 3：Modem watchdog 与完整故障恢复流程

状态：Spec（本文件只定义方案，不包含实现）
范围：SmsRelayed 的 ModemManager、D-Bus SMS 接收、短信重扫、可选数据连接恢复，以及与 systemd 的协作。

参考基线：`/tmp/SimAdmin` 的 clean `main@eb6f497ad332e59f1f3899acc3b2b9427661fe5c`。后续参考实现变化不自动改变本规格。

### 文档判读规则

- **已确认**：能由上述固定提交中的代码直接证明，重建兼容层时应原样理解。
- **目标契约**：SmsRelayed 重建后的必需行为；可有不同内部实现，但外部可观察结果必须一致。
- **重建建议**：为补齐参考实现的安全性、幂等性或可运维性而提出，不是 SimAdmin 已有能力。
- **需决策**：参考实现没有唯一答案；实现前必须选定并记录，不能由代码作者临时猜测。

## 1. 目标与非目标

### 1.1 目标

1. 在不丢失入站 SMS、不重复发送出站 SMS、不过度重启硬件的前提下，发现并恢复以下故障：
   - ModemManager D-Bus owner 消失或重启；
   - Modem 对象暂时消失、对象路径重新编号或路径漂移；
   - modem 处于长时间 searching、enabled/idle、connecting 或 disconnecting；
   - D-Bus SMS 订阅断开、启动期间漏掉的 SMS 未进入本地数据库；
   - SIM、射频或基带恢复后，SMS worker 没有重新绑定；
   - 在明确启用数据恢复策略时，数据连接未恢复。
2. 将恢复动作分成由轻到重的阶梯，并对每一级设置超时、冷却、退避、次数上限和人工接管状态。
3. 将“运行时 SMS 目标”和“允许执行控制动作的 modem 目标”分开，利用已登记的 modem identity/fingerprint 防止路径漂移后误操作另一台 modem。
4. 为健康接口、受保护 API、事件流、日志和指标提供可诊断但不泄露 SMS 内容、电话号码、SIM/设备标识或原始命令输出的状态。
5. 保留用户的手动禁用和飞行模式意图；watchdog 不得在用户明确关闭 modem 或数据连接后偷偷重新打开。

### 1.2 非目标

- 不把 watchdog 变成通用主机监控器；CPU、内存、磁盘、温度、iptables 和互联网连通性不属于本功能的自动恢复范围。
- 不自动删除 ModemManager 缓存、不自动操作 remoteproc、不自动 kill 任意进程、不刷新或清空主机防火墙。
- 不把低信号强度、长时间没有新 SMS 或单次 mmcli 超时直接判定为硬件故障。
- 不在本功能中重新设计 SMS 转发、投递重试或认证体系；允许为恢复 lease、用户意图、熔断和审计增加独立表，不改写既有消息/投递表语义。
- 不承诺恢复运营商网络、SIM 欠费、PIN/PUK 锁定、基站覆盖不足、物理 USB/电源故障或运营商侧短信延迟。
- 不在第一阶段自动启用 SIM power-cycle、USB reset、基带 remoteproc 重启或设备重启；这些动作必须是显式配置、受限的 systemd 辅助服务或人工操作。

## 2. 参考实现：SimAdmin 已确认行为

以下是对固定参考提交的现状提取。阈值、路由、状态码和副作用属于兼容事实；本文后续标为“目标契约/重建建议”的部分则不要求照抄。SimAdmin 启动后等待 5 秒，无配置开关地启动 15 秒周期的 `data_connection_watchdog`；所有计数器、冷却时间与“问题是否活跃”标记均只在该进程内存中保存。

### 2.1 检测指标、阈值与恢复动作

| 指标 | SimAdmin 判定 | 恢复动作 | 备注 |
| --- | --- | --- | --- |
| 找不到 Modem 对象 | 连续 3 次扫描约 45 秒 | mmcli --scan-modems，发出扫描事件 | 发现失败缓存 30 秒，并用互斥避免并发扫描 |
| 找不到 Modem 对象 | 连续 5 次约 75 秒 | systemctl restart ModemManager | ModemManager 重启冷却 300 秒；冷却内只记录，不重复重启 |
| enabled 但长期 idle | 连续 8 次约 120 秒 | 射频 disable/enable cycle | 计数归零后执行；该分支没有复用 searching 的 300 秒冷却 |
| searching | 连续 4 次约 60 秒 | 自动注册运营商一次 | 自动注册请求超时 45 秒，轮询缓存 |
| searching | 连续 8 次约 120 秒 | 射频 cycle，并允许重新注册 | 射频恢复冷却 300 秒 |
| connecting/disconnecting | 连续 6 次约 90 秒 | 射频 cycle | 用于处理卡住的状态迁移；该分支也没有共享 300 秒冷却 |
| 已注册但数据未连接 | 数据激活重试冷却 120 秒 | 尝试 NetworkManager activation | watchdog 不刷新主机 iptables；只读诊断已有规则 |
| 注册/基带恢复 | 状态恢复后 | 可选 NM 重新连接，并发送恢复事件 | 状态恢复通过轮询确认，不依赖固定长 sleep |

SimAdmin 还保留了以下较重动作：基带软重启会串行执行 RF disable、等待约 5 秒、RF enable，再轮询搜索和注册（约 45/60 秒）；eSIM/profile 恢复会停 ModemManager、选择首个可用 QMI 设备节点、对 SIM 做 power off/on、再启动 ModemManager 并等待 modem 重新枚举（最长 15 秒）。它在 profile/baseband 恢复后请求检查未同步 SMS，并依赖持久化去重减少重复和漏收；但 resync 请求通道是无界内存队列，没有 operation id、完成状态或持久化恢复。

### 2.2 已确认的 HTTP 兼容契约

SimAdmin 暴露 `POST /api/baseband/restart` 与 `GET /api/baseband/restart/status`。POST 是同步长请求：handler 等 `restart_baseband` 整体结束才响应；成功与失败都使用 HTTP 200，JSON 包装统一为 `{ "status": "ok|error", "message": string, "data"?: T }`。状态端点同样总是 HTTP 200。

`restart/status` 返回的是进程级单例 `steps`、`running` 和 `current_registration`，没有 operation id 或调用者隔离。开始一次基带重启会清空旧步骤并设置全局 `running=true`；进程退出会丢失所有进度。eSIM profile enable 的异步后台任务也复用这套全局进度，因此并发调用可能覆盖/混合观察结果。参考实现没有通用 watchdog 状态 API、恢复历史 API、恢复取消 API 或幂等键。

兼容适配器若必须服务旧前端，应保留上述路由、HTTP 200 envelope 与全局进度的读取形状，但其内部必须转接到目标 coordinator，并在 `message` 中提供稳定错误码映射。新客户端不得继承该语义，必须使用第 7.3 节的真实 HTTP 状态码、operation/epoch 与幂等接口。

### 2.3 已确认的用户意图、并发和外部命令局限

- “关闭蜂窝数据”会写入配置，并在进程启动时初始化 `data_user_disabled`；飞行模式请求仅存于 `AtomicBool`，重启即丢失。二者都不是持久化的 modem 级 `user_disabled` 状态。
- watchdog 只在全局 `BASEBAND_RESTART_RUNNING` 为 true 时暂停；eSIM/profile power-cycle、人工 data/airplane 操作、SMS 扫描、NetworkManager activation 与 watchdog 动作之间没有一个共同 recovery lease。
- `run_recovery_command` 用于 `mmcli --scan-modems`、`systemctl restart ModemManager` 等动作，但没有 timeout；它把 stdout/stderr 直接变成成功或错误字符串，调用者可能继续写入日志/事件。
- `find_modem_path`/QMI 节点选择偏向排序后的首个候选，未按持久化 fingerprint 绑定动作设备。参考实现的 `serial::with_serial` 只有一把进程内异步互斥锁，不能防多进程或 systemd 脚本竞态，也不持久化 owner/deadline。
- watchdog 会读取 iptables 规则数量用于诊断，但明确不 flush；数据恢复通过 NetworkManager，120 秒内避免再次 activation。只有 searching 射频恢复与 modem 缺失 restart 有 300 秒冷却，idle/transition 射频 cycle 没有同等护栏。
- SMS listener 注册 `Messaging.Added` 后做 initial scan，此后每 15 秒再扫描并处理无界 resync 请求；每次 scan 串行遍历所有 SMS path。它没有 owner-change 专用状态、run deadline、候选上限、结果计数或显式完成通知。

### 2.4 SimAdmin 的 systemd 辅助服务

`scripts/simadmin-modem-recovery.service` 是一次性服务：

- After=network.target ModemManager.service；
- Type=oneshot、ExecStart=/usr/local/bin/simadmin-modem-recovery.sh、RemainAfterExit=yes；
- 脚本启动后等待约 60 秒；用 mmcli -m 0 检查 modem；发现无 modem、状态为 failed，或近期日志包含 UimUninitialized 时进入救援；
- 救援阶段停止 ModemManager、停止 qmi-proxy、清理 /var/lib/ModemManager 下的子项、触发 udev、启动 ModemManager 并等待；
- 若仍失败，再尝试停止/启动名称包含 mss/modem 的 remoteproc，随后再次启动 ModemManager。

这些行为暴露出 SmsRelayed 需要避免的风险：脚本固定使用 modem 0，缓存清理具有破坏性，二阶段失败仍可能返回 0，RemainAfterExit=yes 只覆盖启动时而非持续 watchdog，也没有和应用内动作共享锁。因此本功能只吸收“启动后健康检查”和“二阶段恢复”的思路，不直接复制脚本。

## 3. SmsRelayed 当前能力与缺口

### 3.1 当前健康判定

src/api/health.rs 的公开 /api/health 同时检查 SQLite 和 ModemService::public_health。ModemService 当前通过 mmcli 取得 modem 快照：单次命令超时 5 秒，健康结果缓存 5 秒；刷新并发时可在最多 30 秒内返回旧快照。公开结果只有 status、首个 reason 和 checked_at。

src/modem.rs 的健康分类主要是：

- 配置路径不能解析：Error/modem_path_unresolved；
- modem disabled：Degraded/modem_disabled；
- SIM 未 ready：Degraded/sim_not_ready；
- Messaging 不可用：Degraded/messaging_unavailable；
- failed、locked、unavailable 等状态：Degraded/modem_state_unhealthy；
- 只有状态、SIM、enabled 和 Messaging 条件满足时才为 Ok。

这是一份 dashboard/health 快照，不是故障恢复判定：它不包含 D-Bus owner 是否仍存活、SMS worker 是否在消费信号、最近一次 SMS snapshot/resync 是否成功、数据连接/路由/互联网状态、恢复次数或用户禁用意图。

### 3.2 ModemService、路径和动作能力

- ModemService 封装 mmcli 能力探测、JSON/text fallback、状态解析、Messaging/SIM 补充查询、IMS 探测和公开健康缓存。
- action_lock 使用 try-lock，当前保护 enable/disable/reset 之间的 mmcli 控制动作；reset 有每 session 60 秒限流。
- ModemTargets 已把 runtime_path 和 action_path 分开。SMS worker 可以仅绑定 runtime path；控制动作必须使用 verified action path。
- 路径解析使用精确路径匹配，不使用前缀匹配；一个候选路径可作为运行时目标，但只有已登记 identity/fingerprint 或明确验证的路径才可作为控制目标。
- modem identity 使用 domain-separated SHA-256 fingerprint 持久化；检测到配置路径上的 identity 变化时会撤销 verified action target，并保存 mismatch/quarantine 状态。
- 当前 run_action 能执行 mmcli 的 enable、disable、reset，但没有 watchdog action、systemd restart、扫描、恢复 epoch 或恢复次数账本。

### 3.3 D-Bus 与 SMS worker 能力

src/dbus/connection.rs 提供共享状态的 system-bus connection cache，并能在失败连接仍为当前连接时丢弃它。当前入站实现 src/dbus/inbound.rs 在每次订阅中建立 system D-Bus connection：

- 在解析 ModemManager owner 后订阅 NameOwnerChanged、该 modem 的 Messaging.Added 和 InterfacesRemoved；
- 先 Messaging.List，再读取 SMS State，将已有的 received SMS 作为启动 snapshot；
- owner 变化、被监视 modem object 被移除或 signal stream 结束时终止本次订阅，让上层重连；
- 添加信号会按 sender、modem path 和 received 标志过滤。

src/inbound.rs 的 InboundWorker 当前具备：

- modem 解析失败时 5 秒起步、最多 60 秒的指数重连退避；
- 订阅建立后先处理 snapshot，并继续消费 live Added；snapshot 与 live 信号重叠时由持久化去重；
- 最多 16 个并行入站处理任务；SMS properties 最多重试 5 次；Text 为空时每 100 ms 轮询，最多 600 次（约 60 秒）；
- 持久化失败采用 100 ms 起步、最多 30 秒的持续退避；
- 每 60 秒刷新 runtime identity，支持 runtime-only/action-only/separate targets，并在 identity mismatch 时隔离控制动作；
- SMS 入库和 delivery 创建在 Messaging/Store 中保持幂等；outbound worker 有 durable lease，重启后可恢复过期 lease。

src/runtime.rs 启动 inbound、delivery、retention、outbound worker 和 API；任一受监督 worker/API 退出会使 forwarding runtime 退出。当前没有独立 modem watchdog，也没有在 ModemManager 重启后统一暂停/恢复所有 worker 的协调层。

### 3.4 当前缺口

1. 没有周期性、带迟滞和熔断的 modem watchdog。
2. 没有应用层对 mmcli --scan-modems、ModemManager restart、udev 或 systemd helper 的编排。
3. inbound worker 的重连会自然触发 snapshot，但没有明确的“恢复后重扫”接口、状态、事件和结果计数。
4. action_lock 没有覆盖 D-Bus resubscribe、SMS resync、systemd restart、outbound SMS 临界区和未来的数据动作；存在竞态风险。
5. 健康缓存可能暂时陈旧，且没有 worker liveness、SMS freshness 或恢复阶段信息。
6. 当前没有数据连接控制器；SmsRelayed 不能据此安全地实现 SimAdmin 的 NetworkManager data watchdog。
7. API 只有受保护的 modem status/enable/disable/reset；EventBus 当前只有消息、配置和服务事件，没有恢复事件。
8. 配置只有固定 app.modem_path 和独立的 [monitoring] Sentry 开关；配置变更要求服务重启。

## 4. 建议的整体设计

### 4.1 核心原则

- “可观察”与“可恢复”分离：每次 probe 先记录证据，只有达到连续阈值才执行动作。
- “运行时可消费”不等于“可以控制”：路径漂移时可以安全接收 SMS，但未经 identity 验证不能 enable/disable/reset 或重启硬件。
- 恢复动作必须可取消、可超时、可回滚到安全状态；最重动作默认关闭。
- 恢复成功必须同时满足 modem 状态稳定、D-Bus subscription 已恢复、SMS reconcile 完成；单次 mmcli 成功不等于服务恢复。
- 用户意图优先级高于自动恢复：手动 disable、airplane mode 和“禁止数据自动连接”是硬 inhibit。

### 4.2 状态机

~~~text
BootGrace
    └─ probe owner/path/identity
       ├─ UserDisabled/AirplaneInhibit ───────────────┐
       ├─ healthy + subscription healthy → Healthy     │
       └─ evidence threshold → Degraded                │
              └─ S0 D-Bus rebind + SMS reconcile       │
                   ├─ success → Healthy                │
                   └─ threshold → S1 modem rescan      │
                        ├─ success → AwaitingStable    │
                        └─ threshold → S2 MM restart   │
                             ├─ re-enumerated → S3 SMS  │
                             │   reconcile → Healthy   │
                             └─ failed/backoff ────────┤
                                  ├─ allowed → S3 RF   │
                                  │   cycle → reconcile│
                                  └─ circuit open →    │
                                      ManualRequired   │
~~~

状态定义：

| 状态 | 含义与进入条件 | 自动动作 |
| --- | --- | --- |
| BootGrace | 启动或 ModemManager 重启后的观察窗口 | 只 probe，不重启硬件 |
| Healthy | modem、owner、worker subscription 和最近 reconcile 均成功 | 正常工作 |
| Degraded | 有限的连续失败，但尚未达到恢复阈值 | 轻量 probe、重连、记录事件 |
| Recovering(stage) | 持有恢复 lease，正在执行一个阶梯动作 | 暂停同类 watchdog 触发，保留入站/出站协调 |
| AwaitingStable | 动作完成但尚未通过稳定窗口 | 轮询路径、状态、owner、SMS reconcile |
| UserDisabled / AirplaneInhibit | 用户明确禁用 modem 或开启飞行模式 | 禁止自动 enable、RF cycle、数据 reconnect；仍可报告状态 |
| ManualRequired | 熔断、identity mismatch、路径多义、硬恢复需要人工确认 | 不再自动升级；提供诊断和受保护人工动作 |

UserDisabled 和 AirplaneInhibit 是硬状态，覆盖所有自动恢复分支。identity mismatch 不是普通 degraded，而是控制动作 quarantine；runtime-only SMS 可以继续工作。

### 4.3 探测周期与证据

推荐默认值：

- 基础周期 15 秒；每轮增加 0--2 秒确定性抖动，避免多个实例同时调用 mmcli。
- 启动和每次 ModemManager restart 后先进入 60 秒 BootGrace。
- 每轮低成本检查 D-Bus owner、对象管理器中的 modem 数量、runtime/action target、modem state、SIM/Messaging 可用性和 inbound subscription liveness。
- mmcli 完整 status 使用现有 5 秒命令超时，建议至少 30 秒缓存；健康 API 可返回最近快照，但必须标出 checked_at 和 stale。
- 不以“最近没有收到 SMS”作为故障指标。SMS liveness 由 owner、signal stream、List/GetAll 成功和 resync 结果判定。
- 计数器按故障原因分开；连续两次完整健康 probe 后清零。一次成功的 scan/restart 只清除“对象缺失”计数，不直接宣告 SMS 或数据恢复。

### 4.4 恢复阶梯、阈值与退避

以下为 SmsRelayed 建议的初始策略，数字必须可配置并写入事件/指标；它们不是当前实现行为。

| 阶段 | 触发 | 动作 | 单次超时/完成条件 |
| --- | --- | --- | --- |
| S0 rebind | owner 变化、subscription 终止、路径暂时不可用 | 丢弃失效 connection，重新解析 owner/path，重新订阅并做 bounded SMS reconcile | 新 owner、已验证 runtime target、snapshot 完成 |
| S1 scan | 无 modem path 连续 3 次（约 45 秒） | 调用 mmcli --scan-modems 或等价受限 D-Bus scan；不重启服务 | scan 5 秒，随后最多等待 30 秒重新枚举 |
| S2 MM restart | 无 modem path 连续 5 次（约 75 秒），且不在冷却/熔断 | 请求受限 systemd helper 重启 ModemManager；不在应用内直接执行任意 systemctl | helper 45 秒；owner/path/resubscribe/reconcile 完成后才算成功 |
| S3 RF cycle | searching 连续 8 次、enabled/idle 连续 8 次或 transition 连续 6 次；仅 verified action target | disable/enable 或等价受限动作，随后重新注册并 reconcile | 90 秒；成功需连续两次稳定 probe |
| S4 data reconnect | 已注册但 data bearer 未连接，且用户允许自动连接 | 仅调用受限 data adapter/NM activation 一次；不删 profile、不改防火墙 | 120 秒冷却；以 bearer/IP/route 证据确认，不只看 modem state |
| S5 hard recovery | S2/S3 反复失败，且显式开启并通过人工/策略授权 | SIM power-cycle、remoteproc 或专用硬件脚本 | 默认关闭；每次都需单独审计、超时和人工可见结果 |

searching 连续 4 次（约 60 秒）可以先做一次非破坏性的 operator re-register；它不能和 RF cycle 同一轮并发。S3 触发后至少冷却 300 秒。S2/S3 使用指数退避：5、10、20、30 分钟封顶；默认每小时最多 3 次 MM restart/RF cycle 合计。达到上限后进入 ManualRequired，至少经过 30 分钟观察窗口和一次明确人工确认才可清除熔断。

恢复动作只有在完成该阶段的后置验证后才算成功。动作超时、owner 未恢复、路径多义、identity mismatch 或 SMS reconcile 失败均不得自动把状态标成 Healthy。

### 4.5 并发、互斥与恢复 lease

引入一个逻辑上的 RecoveryCoordinator（名称仅为设计概念）：

1. 所有 watchdog、人工 modem API、D-Bus resync、mmcli scan、ModemManager restart、RF cycle、SIM power-cycle 和未来 data action 先取得同一全局 lease；同一时间最多一个 state-changing recovery。
2. lease 携带 recovery_epoch、stage、owner、deadline、reason。lease 失效后只能由 coordinator 恢复，不允许遗留任务继续执行下一阶段。
3. 现有 ModemService.action_lock 应成为该协调层的一部分，而不是另设一把只保护 mmcli 的锁；API 在冲突时返回 409，watchdog 在冲突时跳过本轮并记录一次受抑制事件。
4. ModemManager restart 前暂停新的 SMS subscription 建立、mmcli action 和 data activation；等待正在执行的 outbound SMS 到达可恢复边界。对已进入 sending/未知结果的短信不得因恢复自动重发，交由 durable lease/人工确认处理。
5. restart 后先恢复共享 system-bus owner/connection，再恢复 inbound subscription，最后运行 snapshot/reconcile；不能让旧 connection 或旧 owner 的 SMS handle 跌落到新 session。
6. watchdog probe 可以并发读取缓存，但不能在 recovery lease 持有期间触发第二个动作。人工 disable/airplane 请求优先级高于自动恢复，必须使当前 epoch 的后续 enable/data reconnect 失效。

### 4.6 目标持久化模型与进程恢复

这是 **目标契约**，不是 SimAdmin 现有 schema。使用 UTC RFC 3339 时间或整数 epoch 毫秒，SQLite 写入必须和对应状态跃迁在同一事务中。建议增加：

~~~sql
CREATE TABLE modem_recovery_state (
  modem_fingerprint TEXT PRIMARY KEY,
  watchdog_state TEXT NOT NULL,
  current_epoch INTEGER NOT NULL DEFAULT 0,
  intent_generation INTEGER NOT NULL DEFAULT 0,
  modem_user_disabled INTEGER NOT NULL DEFAULT 0,
  airplane_inhibit INTEGER NOT NULL DEFAULT 0,
  data_user_disabled INTEGER NOT NULL DEFAULT 0,
  circuit_open_until TEXT,
  consecutive_missing INTEGER NOT NULL DEFAULT 0,
  consecutive_searching INTEGER NOT NULL DEFAULT 0,
  consecutive_idle INTEGER NOT NULL DEFAULT 0,
  consecutive_transition INTEGER NOT NULL DEFAULT 0,
  last_probe_at TEXT,
  last_healthy_at TEXT,
  last_resync_at TEXT,
  last_resync_status TEXT,
  updated_at TEXT NOT NULL
);

CREATE TABLE modem_recovery_attempts (
  epoch INTEGER PRIMARY KEY,
  modem_fingerprint TEXT NOT NULL,
  idempotency_key_hash TEXT,
  trigger_kind TEXT NOT NULL,
  reason_code TEXT NOT NULL,
  stage TEXT NOT NULL,
  status TEXT NOT NULL,
  intent_generation INTEGER NOT NULL,
  started_at TEXT NOT NULL,
  deadline_at TEXT NOT NULL,
  finished_at TEXT,
  result_code TEXT,
  counters_json TEXT NOT NULL DEFAULT '{}',
  UNIQUE(modem_fingerprint, idempotency_key_hash)
);
~~~

要求：

1. 原始 IMEI/IMSI/ICCID、电话号码、SMS 内容、D-Bus path、stdout/stderr 不进入这两张表；`modem_fingerprint` 使用现有 domain-separated digest，幂等键只保存 digest。
2. action lease 由 `status='running'`、`epoch`、`deadline_at` 和事务性 compare-and-set 表示。若未来允许多个实例，共享 SQLite 只能协调同一数据库；特权 helper 仍须由 systemd/OS 级锁防跨进程竞态。
3. watchdog 的短期连续采样可留在内存，但用户意图、circuit、当前 epoch、动作次数窗口和最近 resync 结论必须落盘。进程重启不能重置小时上限或解除 inhibit。
4. 启动时把 deadline 已过或 owner 已消失的 `running` attempt 标为 `interrupted`，进入 BootGrace，重新 probe 和 reconcile；不得盲目重放 RF cycle、MM restart、SIM power-cycle 或 outbound SMS。
5. 仍在 deadline 内的旧 attempt 也不能被新进程视为本进程持有；先通过 helper/owner/target 证据判定外部动作是否结束，再标 `interrupted` 或 `unknown_result`。未知结果进入 AwaitingStable/ManualRequired，不直接升级。
6. 每个新人工请求带 `Idempotency-Key`。同一 fingerprint、相同 key 与相同规范化 body 返回原 epoch；相同 key 配不同 body 返回 409 `idempotency_conflict`。自动 watchdog 使用由故障类别、采样窗口和 intent generation 派生的内部 key。
7. 保留最近至少 1,000 次 attempt 或 30 天（先到者），清理不得删除当前 running、ManualRequired 的根因或仍用于小时限流的记录。实际保留值属于第 9.5 节待决策项。

## 5. ModemManager 重启与路径漂移

### 5.1 重启语义

ModemManager 重启只保证服务进程重启，不保证 modem 硬件、USB port、object path 或 bearer 不变。实现必须：

- 监听 NameOwnerChanged，把旧 owner、旧 object path 和旧 SMS handles 标成 stale；
- 清空 runtime target，重建 system D-Bus connection，重新获取 owner 和 modem 列表；
- 不使用 /Modem/0 假设，不用路径前缀匹配，也不把 mmcli 输出里的 id 当作稳定 identity；
- 在 action target 上重新校验持久化 fingerprint。只有 exact enrolled match 或显式验证的配置路径可控制；
- runtime 只找到一个但 identity 暂不可读时，可以临时 runtime-only 接收 SMS，但不得自动控制它；多 modem 且无法区分时不绑定 runtime，也不执行 action；
- old path 恢复为同一 fingerprint 后清除 mismatch quarantine；不同 fingerprint 时保留 quarantine 并进入 ManualRequired。

### 5.2 systemd 辅助服务边界

推荐新增一个受限的 privileged helper（例如 sms-relayed-modem-recovery.service），由 coordinator 通过明确参数/本地 IPC 请求，不从业务代码拼接任意 shell 命令。服务应满足：

- Type=oneshot，单次 TimeoutStartSec，失败返回非零；不使用会掩盖失败的无条件 exit 0；
- 由 systemd 控制 StartLimitIntervalSec/StartLimitBurst，与应用的每小时熔断双重保护；
- 只允许列明的动作：restart ModemManager、可选 udev trigger、可选已知 modem 的有限恢复；不允许任意路径删除、任意进程 kill 或 remoteproc glob；
- 不默认清理 /var/lib/ModemManager。若未来必须清理，必须是单独人工确认的服务、明确备份/恢复策略和 dry-run 结果；
- 记录结构化的 stage/result/exit code，不记录命令行、完整 journal、SMS 内容、电话号码、IMSI/IMEI 或原始 mmcli 输出；
- helper 只负责特权动作，健康判定、路径验证、SMS reconcile 和用户意图仍由 coordinator 负责。

### 5.3 外部命令、设备与权限契约

下表是目标实现的依赖边界。可用 D-Bus API 时优先 D-Bus；命令必须通过参数数组调用，不经过 shell。

| 能力 | 依赖 | 最低权限/目标约束 | 默认 timeout | 缺失或失败时 |
| --- | --- | --- | --- | --- |
| owner/path/state probe | system D-Bus，`org.freedesktop.ModemManager1` | 只读；runtime/action target 分离 | 单轮 5 秒 | 记 probe 证据；未达阈值不动作 |
| modem scan | `mmcli --scan-modems` 或等价 D-Bus | coordinator lease；不接受用户传入参数 | 5 秒 + 30 秒枚举窗口 | 记 `scan_failed`，留在 Degraded |
| MM restart | 固定 systemd unit/helper | helper 只允许 `ModemManager.service` | helper 45 秒 + BootGrace | 503/`helper_failed`；不直接进入 Healthy |
| RF cycle | ModemManager Enable(false/true) | 仅 exact verified action target | 整体 90 秒 | 停止本 epoch，按次数/冷却决定 ManualRequired |
| SIM power-cycle | 可选 `qmicli` 与已验证 control port | 默认关闭；设备节点必须从 verified modem 映射，不扫描后取首个 | 每步 20 秒 | 停止并人工接管；仍尝试把 MM 恢复为 running |
| data reconnect | 可选 NetworkManager D-Bus/`nmcli` adapter | 独立 data intent 与已登记 profile | 30 秒，120 秒冷却 | 只标 data degraded，不升级硬件恢复 |
| SMS reconcile | ModemManager Messaging D-Bus + SQLite | 不需 root；复用 Store 幂等事务 | 总时限由配置，建议 120 秒 | `partial`/`failed`，保持非 Healthy |

helper 的标准输出只能返回有上限的结构化结果，例如 `{stage,result,exit_code,duration_ms}`；应用丢弃或摘要化额外输出。设备节点、unit 名、远程处理器路径和缓存目录不得来自 HTTP body。生产部署必须用 systemd `ProtectSystem`、`NoNewPrivileges`（在所需特权允许的范围内）、能力白名单和明确的 polkit/sudo 边界；不允许把 Web 服务整体以不受限 root 身份运行。

## 6. 短信漏收、重扫与数据连接风险

### 6.1 SMS resync contract

每次以下事件发生后必须执行一次 bounded resync：ModemManager owner 变化、modem object 重建、inbound subscription 重连、S2/S3/S5 成功或恢复流程中收到明确的“SMS service ready”信号。

resync 顺序：

1. 先建立新 owner 的 signal match，再调用 Messaging.List，避免 List 期间新到达的 Added 丢失。
2. 对 snapshot 过滤 State=received，复用现有 SMS properties retry、空 body poll、ignore_storage 和 16 个并发上限。
3. snapshot 与 live signal 允许重叠；Store 的 modem fingerprint/dedupe namespace 和事务性入库是唯一去重依据，不用“本次 resync 已见路径”的内存集合代替持久化去重。
4. 给 resync 设置候选数量、总时限和逐条错误上限；单条 properties 失败不能阻塞后续 SMS。失败候选应在下一轮 reconciliation 重试，并把未完成数量计入健康状态。
5. 不删除 modem SMS storage，不因为空 body 立即丢弃；达到现有约 60 秒 body poll 上限后记录脱敏错误码，保留下一轮重扫机会。

resync 结果至少分为 completed、partial、failed，并带 snapshot_count、persisted_count、duplicate_count、deferred_count、failed_count；只记录计数，不记录正文、号码或完整 SMS object path。

### 6.2 数据连接风险

SmsRelayed 当前没有 SimAdmin 那样的 NetworkManager data controller，ModemManager connected 也不等价于已有 IP、默认路由或互联网可用。因此：

- 第一阶段 watchdog 不自动触碰数据连接；只报告 modem registration 和 SMS health。
- 后续若加入 data adapter，必须有独立的 data_user_intent、airplane_inhibit、bearer/profile identity 和 activation lease；不能把 modem watchdog 的 RF cycle 当作数据恢复的默认动作。
- ModemManager restart 可能使 bearer、接口名、IP、路由和 DNS 短暂消失；恢复顺序应是 modem registered → bearer present → IP/route confirmed → optional application probe，并且每一步都有超时。
- 不删除或重建用户的 NM profile，不刷新全局 iptables/ip6tables，不以一次 ping 失败触发 modem restart。
- 数据 activation 期间 SMS worker 仍应保持可重连；activation 失败只进入 data degraded，不提升为硬件 recovery，除非同时有独立的 modem/object evidence。

### 6.3 用户手动禁用语义

- 手动 disable 成功后立即持久化 user_disabled=true，自动 enable、RF cycle、MM restart 后的自动 enable 和 data reconnect 全部禁止；允许只读 probe、事件和手动查询。
- 手动开启飞行模式是更高优先级的硬 inhibit；退出飞行模式不会自动恢复用户此前明确禁用的 modem。
- 手动 enable 只有在 action path verified 且动作成功后才清除 user_disabled；失败不改变原意图。
- 手动 reset 不自动清除 disable/airplane 意图，也不自动启动数据连接；它应与 recovery lease 冲突时返回 409。
- 用户在恢复进行中执行 disable/airplane 时，coordinator 必须递增 intent generation，使当前 epoch 的后续 enable、RF cycle 和 data activation 完成后不得继续下一步；能安全取消的动作立即取消，不能取消的动作只等待其 deadline，不再升级。

## 7. 配置、事件、API 与观测性

### 7.1 配置建议

建议新增独立 [watchdog]，不要复用当前用于 Sentry 的 [monitoring]：

~~~toml
[watchdog]
enabled = false
probe_interval_secs = 15
startup_grace_secs = 60
scan_after_missing_probes = 3
modemmanager_restart_after_missing_probes = 5
action_cooldown_secs = 300
max_recovery_actions_per_hour = 3
sms_resync_enabled = true
sms_reconcile_interval_secs = 300
auto_operator_reregister = true
auto_radio_cycle = true
auto_data_reconnect = false
allow_sim_power_cycle = false
allow_remoteproc_recovery = false
~~~

字段要求：

- 所有周期、阈值、上限均校验非零且有合理上限；restart_after >= scan_after；
- allow_sim_power_cycle、allow_remoteproc_recovery 和未来的 cache cleanup 必须分别授权，不能由一个 force=true 总开关覆盖；
- 默认先以 observer-only/enabled=false 灰度，确认指标和 fault injection 通过后再启用自动 S1/S2；
- 配置变更沿用当前“保存后重启服务”语义；运行中的 user_disabled、airplane 和 recovery epoch 不得因普通配置 reload 被清除；
- 配置文件继续按现有权限保护；敏感的 helper 凭证/IPC token 不进事件和日志。

### 7.2 事件模型

扩展 EventBus/SSE 事件，事件 payload 只含稳定枚举、计数和时间：

- modem.watchdog_state_changed；
- modem.probe_failed / modem.probe_recovered；
- modem.recovery_started / modem.recovery_step / modem.recovery_succeeded / modem.recovery_failed；
- modem.path_rebound / modem.identity_quarantined；
- modemmanager.scan_requested / modemmanager.restart_requested / modemmanager.restarted；
- sms.resync_started / sms.resync_completed / sms.resync_partial / sms.resync_failed；
- data.reconnect_suppressed / data.reconnect_started / data.reconnect_recovered / data.reconnect_failed；
- modem.recovery_manual_required 和 modem.recovery_inhibited。

每个事件带 recovery_epoch、stage、reason_code、attempt、duration_ms、status 和必要的计数。恢复事件必须有对应的 recovered/succeeded 或 manual-required 终点，避免只有“触发”没有结论。

### 7.3 API 与健康响应

以下是 **目标契约**。除 `GET /api/health` 外均沿用现有认证/CSRF 保护。成功体直接返回资源 JSON，不使用 SimAdmin envelope；错误体沿用 SmsRelayed 已有形状：

~~~json
{ "error": { "code": "recovery_busy", "message": "another recovery action is running" } }
~~~

错误 `message` 只用于展示，客户端只按稳定 `code` 分支。不得把底层命令输出拼入 message。

#### 7.3.1 查询恢复状态

`GET /api/modem/recovery` 返回 HTTP 200：

~~~json
{
  "watchdog_state": "healthy",
  "stage": null,
  "epoch": 42,
  "intent_generation": 7,
  "user_inhibit": { "modem_disabled": false, "airplane": false, "data_disabled": true },
  "target": { "runtime_available": true, "action_verified": true, "quarantined": false },
  "evidence": {
    "reason_code": null,
    "consecutive_missing": 0,
    "consecutive_searching": 0,
    "consecutive_idle": 0,
    "consecutive_transition": 0,
    "last_probe_at": "2026-08-24T01:02:03Z",
    "last_healthy_at": "2026-08-24T01:02:03Z",
    "stale": false
  },
  "cooldown_until": null,
  "circuit_open_until": null,
  "last_attempt": { "epoch": 41, "trigger": "watchdog", "stage": "s1_scan", "status": "succeeded", "result_code": "modem_reappeared" },
  "last_resync": { "status": "completed", "snapshot_count": 2, "persisted_count": 1, "duplicate_count": 1, "deferred_count": 0, "failed_count": 0, "finished_at": "2026-08-24T00:59:00Z" }
}
~~~

时间字段均为 UTC RFC 3339；未知值为 `null` 而非空串。状态枚举必须来自 4.2/6.1，额外字段可向后兼容增加，既有字段不得换义。

#### 7.3.2 异步操作

所有 POST 接受 `Idempotency-Key` header（1--128 个可打印 ASCII 字符），成功受理返回 HTTP 202：

~~~json
{ "operation": { "epoch": 43, "kind": "sms_resync", "status": "queued", "status_url": "/api/modem/recovery/operations/43" } }
~~~

| 路由 | 请求 JSON | 作用与限制 |
| --- | --- | --- |
| `POST /api/modem/recovery/resync-sms` | `{ "reason": "manual" }` | 仅 reconcile，不操作 RF/MM；相同幂等键返回原 operation |
| `POST /api/modem/recovery/actions` | `{ "action": "scan|restart_modemmanager|radio_cycle", "reason": "manual", "confirm": true }` | 人工执行列明 stage；`radio_cycle` 与 `restart_modemmanager` 必须 `confirm=true`，且 target verified、无 inhibit |
| `POST /api/modem/recovery/clear-quarantine` | `{ "confirm": true, "expected_fingerprint_digest": "..." }` | 只在当前读取 fingerprint 与期望 digest 完全相同时清除 identity quarantine/circuit；不 reset、不 enable、不清用户意图 |

`GET /api/modem/recovery/operations/{epoch}` 返回 200，体为 `{ "operation": { "epoch", "kind", "stage", "status": "queued|running|awaiting_stable|succeeded|partial|failed|interrupted|manual_required", "reason_code", "started_at", "deadline_at", "finished_at", "result" } }`。不存在或不属于当前实例授权范围返回 404 `operation_not_found`。异步失败仍通过该资源的 HTTP 200 + `status=failed` 表示；只有读取 operation 本身失败才用非 2xx。

统一状态码与错误码：

| HTTP | 稳定 code | 条件 |
| --- | --- | --- |
| 400 | `bad_request` / `confirmation_required` | JSON、枚举、幂等键格式错误或危险动作未确认 |
| 401/403 | `unauthorized` / `forbidden` | 未认证、CSRF/权限不足 |
| 404 | `operation_not_found` | epoch 不存在或不可见 |
| 409 | `recovery_busy` | 另一个 state-changing epoch 正在运行 |
| 409 | `target_unverified` / `identity_mismatch` | action target 未验证或 digest 不匹配 |
| 409 | `user_inhibited` / `idempotency_conflict` | 用户意图禁止动作，或 key 被不同请求复用 |
| 429 | `recovery_rate_limited` / `circuit_open` | stage 冷却、小时上限或熔断 |
| 503 | `watchdog_disabled` / `helper_unavailable` | 功能关闭或受限 helper/依赖不可用 |
| 504 | `helper_timeout` | 请求在受理前需要的 helper handshake 超时；已受理 operation 的后续超时写入其状态 |

保留现有受保护接口 `GET /api/modem/status`、`POST /api/modem/enable|disable|reset`，但接入 coordinator：disable 成功事务性写用户意图；enable 成功才清除；reset 与 active recovery 冲突返回 409。是否让这些旧接口也强制 `Idempotency-Key` 属于第 9.5 节决策，未决前不得静默重试 state-changing POST。

`GET /api/health` 继续公开，但在现有 service/modem 字段内只增加非敏感摘要：`watchdog_state`、`last_probe_at`、`last_healthy_at`、`consecutive_failures`、`current_stage`、`recovery_epoch`、`last_resync_at`、`resync_status`、`stale` 和 `user_inhibited`。公开响应不得返回 raw D-Bus path、IMSI/IMEI、电话号码、operator、SMS body、mmcli stdout/stderr 或 shell command。

### 7.4 指标与日志脱敏

建议指标：

- sms_relayed_modem_probe_total{result,reason}；
- sms_relayed_modem_recovery_total{stage,result}；
- sms_relayed_modem_recovery_duration_seconds{stage}；
- sms_relayed_modem_recovery_circuit_open；
- sms_relayed_modem_path_rebind_total{verified}；
- sms_relayed_sms_resync_total{result}、sms_relayed_sms_resync_messages_total{result}；
- sms_relayed_inbound_subscription_reconnect_total{reason}；
- sms_relayed_data_reconnect_total{result}（仅实现 data adapter 后）。

指标 labels 必须来自有限枚举，不能把 path、phone number、IMSI、IMEI、operator 或 SMS body 作为 label。日志只允许记录 stage、reason_code、epoch、attempt、duration_ms、有限的 modem_slot/fingerprint hash 和结果；禁止记录 SMS body、电话号码、SIM/设备标识、完整 object path、原始 mmcli/qmicli/AT 输出、journal 文本和带 secret 的命令行。Sentry 继续使用固定 error code 和现有 scrubber，不把底层错误字符串直接作为事件 payload。

### 7.5 UI 行为

这是目标管理界面的最小行为，不限定组件库：

1. 状态卡同时显示 modem、watchdog、SMS subscription/resync 与用户 inhibit；`stale=true` 时明确显示“数据可能过期”和最后探测时间，不能继续显示绿色 Healthy。
2. 当前 operation 用阶段时间线展示 queued/running/awaiting stable/终态，页面刷新后通过 `status_url` 恢复。SSE 只用于加速，断线后以 2 秒轮询 active operation、稳定时 15 秒轮询恢复状态；连续错误使用 2/4/8/15 秒退避。
3. “重扫短信”是低风险独立按钮；重复点击复用同一幂等键并禁用按钮，直到 operation 终态。UI 只展示计数，不列出 SMS 内容、号码、object path 或 fingerprint。
4. “扫描 modem”“重启 ModemManager”“射频 cycle”“清除隔离”按风险分组。后三者必须二次确认，确认框说明可能造成短时离线；clear quarantine 要求用户确认当前设备摘要，但 UI 不回显完整标识。
5. 409 busy 时跳转/附着到服务器返回的 active epoch（若响应提供）；429 显示 `Retry-After`/冷却截止时间；503/504 显示依赖不可用，不提供无限自动重试。
6. UserDisabled/AirplaneInhibit 时控制按钮禁用并解释来源。关闭飞行模式不得在 UI 里隐式打开此前关闭的 modem/data；手动 enable 成功前开关保持原意图状态。
7. 兼容 SimAdmin 页面若轮询 `/api/baseband/restart/status`，adapter 可从 coordinator 投影最近 active epoch；新 UI 只使用 operation API，避免看见另一个调用者的全局步骤。

## 8. 避免 watchdog 反复重启的安全护栏

1. 启动 grace、连续失败阈值、恢复后稳定窗口和原因分离计数；单次超时不能升级。
2. 每个 stage 独立 cooldown；S2/S3 共享小时级 token bucket，并在进程重启后由 systemd StartLimit 再兜底。
3. 一个全局 recovery lease、一个 recovery epoch、每步 deadline；禁止重入、并发 enable/disable、并发 resync 和 recovery/action 交叉执行。
4. identity mismatch、多 modem 歧义、action path 未验证、SIM locked/PUK、用户 inhibit 和 helper 返回未知结果时立即停止升级，进入 ManualRequired。
5. 只读 probe 不触发 RF；不以低信号、没有新 SMS、单次 mmcli 失败或数据库短暂错误作为重启证据。
6. ModemManager restart、RF cycle、SIM power-cycle 的成功判据必须包含 owner/path 重建、状态稳定和 SMS reconcile；失败不清零计数、不重复执行同一阶段。
7. 默认不做 cache cleanup、remoteproc、USB reset、设备 reboot；任何启用都必须是独立配置、独立计数和人工可见事件。
8. systemd helper 使用最小权限、明确参数白名单、非零失败码、Timeout、StartLimit 和审计日志；不得用无条件成功掩盖恢复失败。
9. watchdog 自身异常、数据库不可用或 API 重启不能改变用户禁用意图；watchdog 退出时也不得遗留一个继续 enable modem 的 detached task。
10. 对出站 SMS 保持至少一次和“未知结果不盲重发”语义；恢复流程不得为了清空队列而重试未知发送。

### 8.1 失败、恢复和幂等矩阵

| 故障点 | 必须记录的终态 | 自动后续 | 客户端/运维可见结果 |
| --- | --- | --- | --- |
| probe 单次超时 | `degraded` + 原因计数 1 | 下轮再 probe，不执行动作 | 状态 stale/原因码；无告警风暴 |
| scan 失败/超时 | attempt `failed:scan_failed` | 保留 missing 计数；达到 S2 阈值且未限流才升级 | operation/事件有终点，不含 stderr |
| MM restart helper 非零 | `failed:helper_failed` | 不假定 MM 已停止或已启动；probe 外部状态，必要时 ManualRequired | 503（同步受理失败）或 operation failed |
| helper 超时/连接断开 | `unknown_result`/`awaiting_stable` | 只观察 owner/path，不重复发同一动作 | 相同幂等键返回原 epoch |
| restart 后 path 改变且 identity 相同 | `awaiting_stable` → reconcile | 更新 runtime/action binding；稳定两次后成功 | path 不对外泄露，事件为 path_rebound |
| path identity 不同/不可验证 | `manual_required` + quarantine | runtime-only 可继续；禁止 RF/reset/hard recovery | 409 `identity_mismatch`/`target_unverified` |
| subscription 已恢复但 resync partial | recovery 不得 Healthy | 下个 bounded reconcile 窗口重试 deferred | 计数与 `partial`，不泄露消息 |
| 数据库写失败 | attempt 状态不能假成功 | 停止 state-changing 后续；保留/恢复 lease，进入 degraded | 503 `storage_unavailable`，重启后审计 interrupted |
| 恢复中收到 disable/airplane | 当前 epoch `interrupted` 或安全结束后 `inhibited` | intent generation 失效后续 enable/data；不升级 | 新意图立即可见；冲突请求 409 |
| 相同幂等键同请求 | 不创建新 attempt | 返回原 operation（包括终态） | 200（已有终态资源）或 202（仍 queued/running） |
| 相同幂等键不同请求 | 无状态变化 | 不执行 | 409 `idempotency_conflict` |
| 进程在动作中退出 | 启动时旧 attempt `interrupted`/`unknown_result` | BootGrace + probe + reconcile；绝不盲重放 | 历史可查，新的 epoch 需重新受理 |
| circuit/小时上限触发 | `manual_required`，持久化截止时间 | 只 probe，不升级 | 429 + `Retry-After`，UI 显示人工接管 |

## 9. 分阶段交付

### Phase 0：观测与契约

- 增加 coordinator 状态、probe reason、worker liveness、resync 计数和脱敏日志，但默认 observer-only；
- 定义 ModemTargets、recovery epoch、user intent 和事件/API schema；
- 为时间、command runner、D-Bus source、systemd helper 建立可注入接口。

### Phase 1：安全重绑与 SMS reconcile

- 将 owner/path 变化统一送入 coordinator；
- 实现新 owner 订阅、List snapshot、live signal overlap、持久化去重和 resync 结果；
- 健康接口增加 stale/worker/resync 摘要；不做 ModemManager restart。

### Phase 2：扫描与 ModemManager restart

- 实现 S1 scan 和受限 systemd helper 的 S2 restart；
- 完成 path re-enumeration、fingerprint 验证、runtime/action 隔离、冷却/熔断；
- 通过 fault injection 验证 restart 期间 SMS、outbound lease 和用户 inhibit。

### Phase 3：RF 与人工恢复控制

- 在 verified action path 上实现 searching/idle/transition 的 RF cycle；
- 增加受保护 recovery status/resync/quarantine API、冲突码和审计事件；
- 继续默认关闭 S5，只有明确运行环境和硬件测试通过后才允许开启。

### Phase 4：可选数据和重硬件恢复

- 在独立 data adapter 下实现 bearer/IP/route 证据、用户意图和一次性 reconnect；
- 如确有硬件需求，再设计独立 SIM/remoteproc helper；不把它们并入普通 S2/S3 计数器。

### 9.5 需决策项（实现前关闭）

| ID | 决策 | 推荐默认 | 未决定时的安全行为 |
| --- | --- | --- | --- |
| W1 | observer-only 观察期多长、何时启用 S1/S2/S3 | 现场至少 7 天；S1、S2、S3 分批开放 | `enabled=false`，只观测 |
| W2 | 单机还是多实例部署 | 单实例 + systemd 级互斥 | 不允许自动特权动作 |
| W3 | recovery attempt 保留期/上限 | 30 天或 1,000 条，先到者 | 不清理当前/限流所需记录 |
| W4 | 既有 enable/disable/reset 是否强制幂等键 | state-changing POST 全部要求 | 客户端不得自动重试 |
| W5 | clear-quarantine 是否只清 circuit，还是也允许重新登记 identity | 首版只清 circuit；identity 重新登记走独立人工流程 | identity mismatch 保持隔离 |
| W6 | S5 是否允许、具体硬件和设备映射方式 | 默认全部关闭 | 返回 503 `helper_unavailable`，不降级为首个设备 |
| W7 | 数据健康的权威证据与所有权 | 第一阶段不实现 data adapter | 只报告 registration/SMS，不自动连数据 |
| W8 | resync 候选上限、总时限、周期 | 建议 1,000 条、120 秒、300 秒 | 超限为 partial，下一窗口继续 |
| W9 | 公开 `/api/health` 暴露的恢复粒度 | 仅有限枚举、时间和计数 | 敏感/高基数字段一律省略 |

## 10. 验收标准

### 10.1 功能验收

- 健康 modem 在至少 24 小时运行中不触发 recovery；健康 API 的 stale 行为符合缓存契约。
- owner 变化、ModemManager 重启、InterfacesRemoved 和 signal stream 结束都能重建 subscription；新旧 owner 的 SMS handle 不交叉读取。
- 启动 snapshot、resync snapshot 与 live Added 重叠时每条 SMS 只入库和创建 delivery 一次；不同 SMS object path 的同一短信仍按持久化 dedupe 规则去重。
- modem path 从 /Modem/0 漂移到其他编号时，已登记 fingerprint 可恢复 runtime/action；未验证或 identity 不同只能 runtime-only/quarantine，不能执行控制动作。
- 3 次缺失只 scan，5 次缺失才可能 restart；冷却、小时上限、systemd StartLimit 和 ManualRequired 均可观察且生效。
- searching/idle/transition 阈值和 S3 冷却生效；单次 probe、低信号或无新 SMS 不会触发 RF cycle。
- ModemManager restart 后 SMS reconcile 完成才报告 recovered；partial/failed resync 不得伪装成 healthy。
- 手动 disable、airplane 和禁止 data reconnect 始终阻止自动 enable/RF/data action；手动 enable 的语义和失败回滚符合本 spec。
- outbox 在 recovery 期间不产生重复发送；未知发送结果保留 durable lease/人工处理路径。
- 日志、事件、指标、Sentry 和公开 API 均不含 SMS body、号码、SIM/设备标识、原始路径或命令输出。

### 10.2 故障注入测试

必须使用 fake runner/clock/D-Bus source/systemd helper；CI 不执行真实 ModemManager restart、cache 删除、remoteproc 或设备重启。至少覆盖：

1. mmcli 缺失、5 秒超时、JSON 不支持、status parse 失败和短暂恢复；
2. D-Bus 无法连接、owner 变化、owner 不变的 NameOwnerChanged、stream end、InterfacesRemoved、错误 sender/path 的噪声 signal；
3. 旧 path 消失后新 path 同编号/不同编号、单候选、多候选、identity 暂不可读、fingerprint mismatch 和 fingerprint 恢复；
4. scan 成功但 object 延迟出现、scan 连续失败、restart helper 超时/非零/返回未知结果；
5. searching、enabled/idle、connecting/disconnecting 各自在阈值前后变化，验证不会提前或重复 RF cycle；
6. snapshot 与 live Added 同一 SMS、重启期间到达 SMS、空 body 延迟、properties 五次失败、16 个并发满载和第 17 条 backpressure；
7. D-Bus subscription 恢复但 snapshot 部分失败、重复 resync 请求、resync 与恢复动作并发；
8. outbound SMS 在 prepare/send/未知结果时触发 restart，验证不会盲目重发；
9. recovery 进行中手动 disable、airplane、enable、reset，验证 lease、generation、409 和用户意图；
10. 连续恢复失败触发 300 秒冷却、指数退避、小时 token bucket、circuit open 和 ManualRequired，并验证恢复后稳定窗口才清零；
11. 可选 data adapter 的 bearer/IP/route 缺失、NM activation 超时、用户禁用和恢复后一次 reconnect；
12. 进程重启、数据库临时不可用、API 客户端断开和 watchdog task cancellation，验证无 detached recovery action 和无丢失的用户 inhibit。

## 11. 交付结论

SmsRelayed 已经有较可靠的 D-Bus owner 重连、启动 SMS snapshot、并发限制、持久化去重和 path/identity 隔离基础；缺少的是统一 coordinator、连续证据驱动的恢复阶梯、ModemManager/systemd 边界、恢复后 SMS reconcile、数据连接所有权和可观测状态。本功能应先交付 Phase 0/1，再在故障注入和现场观察通过后逐步开放 S2/S3；S5 永远保持显式授权和人工可见。

## 12. 参考实现来源索引

全部位置均相对于 `/tmp/SimAdmin`，基线为 `main@eb6f497ad332e59f1f3899acc3b2b9427661fe5c`；行号用于定位，若移植提交应同时以符号名核对。

| 事实 | 来源（路径 + 符号/行号） |
| --- | --- |
| watchdog 启动延迟、15 秒周期、data/airplane flags | `backend/src/main.rs` `data_user_disabled`/`airplane_mode_requested` 约 328–329 行；watchdog spawn 约 425–467 行 |
| 阈值和冷却常量 | `backend/src/modem_manager.rs` `MODEM_SCAN_THRESHOLD` 等约 56–66 行；`TRANSITION_STUCK_THRESHOLD`/`ENABLED_IDLE_RECOVERY_THRESHOLD` 约 6246–6272 行 |
| watchdog 全流程、内存计数与恢复分支 | `backend/src/modem_manager.rs` `data_connection_watchdog` 约 6246–6686 行 |
| iptables 只读诊断、基带运行时暂停 | `backend/src/modem_manager.rs` `data_connection_watchdog` 约 6273–6300 行 |
| searching/idle/transition 射频 cycle 与不一致冷却 | `backend/src/modem_manager.rs` `data_connection_watchdog` 约 6330–6535 行 |
| scan、MM restart 与 300 秒冷却 | `backend/src/modem_manager.rs` `data_connection_watchdog` 约 6585–6677 行 |
| 无 timeout 的命令 runner 与有限时 runner | `backend/src/modem_manager.rs` `run_recovery_command`/`run_recovery_command_owned` 约 5763–5810 行 |
| 进程级全局 restart progress | `backend/src/modem_manager.rs` `reset_baseband_restart_progress`/`get_baseband_restart_progress` 约 5829–5875 行 |
| 同步基带 restart API、成功失败均 HTTP 200 | `backend/src/main.rs` routes 约 682–687 行；`backend/src/handlers.rs` `restart_baseband_handler`/`get_baseband_restart_status_handler` 约 2406–2441 行 |
| profile 恢复停启 MM、QMI power、15 秒重新枚举 | `backend/src/modem_manager.rs` `power_cycle_sim_for_profile_switch_inner` 约 5916–6131 行；`find_qmi_device_path`/`qmicli_sim_power` 约 2218–2275 行 |
| 全局进程内 D-Bus/AT 互斥 | `backend/src/serial.rs` `DBUS_LOCK`/`with_serial` 约 1–29 行 |
| 数据用户意图持久化、飞行请求仅内存 | `backend/src/main.rs` 约 328–329 行；`backend/src/handlers.rs` `set_data_status` 约 2337–2404 行、airplane handlers 约 2520–2601 行 |
| 无界 resync channel、SMS marker/持久化去重 | `backend/src/sms_listener.rs` `sms_resync_channel`/`request_scan` 约 34–59 行；`sms_marker`/`process_sms_path` 约 78–103、192–289 行 |
| initial/poll/requested SMS scan 与 15 秒轮询 | `backend/src/sms_listener.rs` `scan_sms_paths`/`start_sms_listener` 约 297–337、400–535 行 |
| 启动救援的固定 modem 0、缓存删除、remoteproc、无条件成功 | `scripts/simadmin-modem-recovery.sh` 约 1–95 行 |
| oneshot + RemainAfterExit、无 Timeout/StartLimit | `scripts/simadmin-modem-recovery.service` 约 1–11 行 |
