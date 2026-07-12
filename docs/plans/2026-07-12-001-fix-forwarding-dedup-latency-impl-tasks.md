# 重复转发修复与延迟状态页：实现任务

## 功能简述

本次工作解决两个相关问题：ModemManager 重复上报同一短信时会重复入库并产生重复通知；用户无法判断各转发 profile 的请求延迟、超时与重试情况。

完成后，modem 入站事件具有持久化幂等性，每个 profile 持久化最近 5 次真实转发尝试，并通过受认证的 API 与独立 `/forwarding` WebUI 页面展示。

系统继续采用“至少一次”下游投递语义。入口重复必须被消除；第三方已经收到请求但响应丢失的情况仍允许重试，页面需把两次 attempt 如实展示。

## 总体解题思路

1. 使用不含 SMS object path 的稳定摘要识别同一入站短信，避免 ModemManager 重连或重编号后重复入库。
2. 在 SQLite 事务和部分唯一索引处执行最终幂等判定，不能只依赖内存集合。
3. 仅在真实调用转发 profile 时用单调时钟计时；每次 retry 是独立样本。
4. 完成 delivery 时同时写入并裁剪样本；即使 lease token 已失效，也必须保留已经发生的网络请求样本。
5. API 只返回 profile、时间、耗时、结果与标准化错误码，禁止返回号码、正文、令牌、命令或 provider 响应。
6. WebUI 不虚构健康阈值，只陈列最近结果和原始延迟，使用手动刷新。

## Rust 行为规格（测试已先写）

`src/storage.rs` 中新增的红灯测试是 Rust 接口的权威规格。实现者不得为了变绿而弱化断言或删除测试；如果发现契约无法安全实现，应停止并报告冲突。

### 入站接口

- `NewMessage::modem_inbound(phone_number, body, timestamp, modem_sms_path, modem_fingerprint)` 创建带持久化去重键的 modem 入站消息。
- 去重键为 SHA-256：对版本标识、modem fingerprint、timestamp、phone number、body 做长度分隔编码后哈希；SMS object path 仅保存在消息中，不进入摘要。
- `MessageStore::insert_inbound_message_with_deliveries` 返回 `InboundInsertResult::Inserted(Message)` 或 `InboundInsertResult::Duplicate(Message)`。
- 重复调用返回同一 canonical message，不新增 message、delivery 或事件。
- timestamp 不同时必须视为不同短信；object path 改变但其余稳定字段相同时必须视为重复。
- runtime 只有在 `Inserted` 时发送 `MessageCreated`；`Duplicate` 返回成功，不能触发 `handle_incoming_sms` 的持久化重试循环。

### 延迟样本接口

- `ForwardAttemptOutcome` 至少包含 `Success`、`TransientFailure`、`PermanentFailure`，并可安全序列化给 API。
- `NewForwardAttemptSample` 包含 `profile_key`、可空 `delivery_id`、`attempt_number`、`started_at`、`completed_at`、`latency_ms`、`outcome`、可空 `error_code`。
- 持久化返回的样本提供 `is_retry()`，定义为 `attempt_number > 1`。
- `record_forward_attempt` 和 `list_forward_attempts(profile_key, limit)` 是存储行为边界；查询按 `completed_at DESC, id DESC` 返回。
- 每个 profile 只保留最近 5 条完成样本；一个 profile 的裁剪不能影响另一个 profile。
- `complete_delivery_with_attempt` 在同一事务中记录/裁剪样本，并按现有 lease token 条件完成 delivery。lease 不匹配时返回 `false`、保持 delivery 状态不变，但样本仍存在。
- delivery worker 使用 `Instant` 测量实际 `forward_to_profile` await；不为 `message_not_found` 或 `profile_missing` 生成样本。
- HTTP timeout 使用 `http_timeout`；Shell timeout 必须从其他 shell failure 中区分为 `shell_timeout`。

### SQLite 迁移

- 为 `messages` 增加可空 `inbound_dedupe_key` 和部分唯一索引。
- 旧数据库迁移必须在事务中完成并可重复执行。
- 对历史 modem inbound 计算摘要；每组重复消息只给最早 ID 回填键，其余历史行保留且键为 `NULL`，不得删除用户消息或重置 delivery 状态。
- 新增 `forward_attempt_samples` 表和 `(profile_key, completed_at DESC, id DESC)` 查询索引。
- 样本不使用会被 message retention 级联删除的强制外键。

### API 规格

- 新增受现有 session middleware 保护的 `GET /api/forwarding/attempts`。
- 返回 `generated_at` 和 `profiles`；每个 profile 包含 `profile_key`、`enabled`、最多 5 条 newest-first samples。
- samples 包含 `attempt_number`、`is_retry`、`started_at`、`completed_at`、`latency_ms`、`outcome`、`error_code`。
- 当前启用但无样本的 profile 返回空数组；仍有样本的停用 profile 返回 `enabled: false`。
- 启用 profile 按配置顺序，历史 profile 随后按 key 排序。

## 实现任务

### Task 1 — 单 Agent：完成端到端实现

**执行方式：** 在同一个 Agent 任务和同一份上下文中完成。先使用 GLM 阅读计划、检查范围并复述实现约束；用户确认后切换到 DeepSeek，由 DeepSeek继续实现 Rust 后端与 WebUI。

**交付内容：**

- 实现红灯测试规定的 storage 类型、方法、schema 与迁移。
- 修改 runtime 使用幂等入站接口并只为新消息发事件。
- 在 delivery worker 中测量并持久化每次真实 profile attempt。
- 区分 Shell timeout 安全错误码。
- 新增 authenticated forwarding attempts API 及 Rust route tests。
- 为迁移、并发重复、runtime 不发重复事件、HTTP/Shell timeout 与 API shape 补充必要测试。
- 新增 `/forwarding` TanStack Router route、API types/helper、状态 panel 和主导航入口。
- 每个 profile 显示 enabled/disabled、最近结果、最近 5 次耗时、完成时间、attempt、Retry 标记和安全错误码。
- 提供 Loading、Error、Empty、Refreshing 状态与手动 Refresh；不自动轮询，不给出自创健康阈值。
- 延迟小于 1000ms 显示毫秒，否则显示秒。
- 使用已有 Table、Badge、Button，不新增或手抄 shadcn 组件。
- 添加组件测试，并用项目生成器更新 route tree。

**提交策略：**

- 所有工作留在当前功能分支，不切回 `main`。
- 先提交 Rust 幂等、样本、API 与测试，形成一个可追溯的后端提交。
- 再提交 WebUI、route tree 和前端测试，形成一个独立前端提交。
- 不改写、压缩或 amend 本文与红灯测试所在的基线提交。
- 每次提交前只 stage 属于该逻辑单元的文件，并在提交后确认工作区剩余修改符合预期。

**验收标准：**

- `cargo test` 全绿且 `cargo fmt --check` 通过。
- `pnpm test`、`pnpm check`、`pnpm build` 通过。
- `pnpm generate-routes` 后生成文件与 route 一致。
- 同一入站事件重复两次只产生一条消息和每 profile 一条 delivery。
- 每 profile 样本最多 5 条，lease 丢失仍记录真实 attempt。
- API 未认证返回 401，且不暴露敏感信息。
- 空样本、停用历史 profile、成功、失败、超时和 retry 均有测试。
- 页面及 DOM 不包含号码、短信正文、令牌或 provider 原始错误。

## 单一 Agent Prompt

你在仓库 `SmsRelayed` 的当前功能分支中负责完成“重复转发修复与延迟状态页”。这是一个 Agent 任务，用户会在同一任务里切换 GLM 与 DeepSeek，所有模型共享当前上下文、工作区和 git 历史。

第一阶段使用 GLM，只做理解与检查：完整阅读根目录 AGENTS.md、`docs/plans/2026-07-12-001-fix-forwarding-dedup-latency-impl-tasks.md`、`src/storage.rs` 中新增的红灯测试，以及相关 storage/runtime/delivery/API/frontend 现有模式。然后用简洁清单复述目标、关键接口、迁移风险、隐私边界、提交拆分和验收命令。不要修改文件，不要运行实现，不要创建提交。复述结束后明确输出“计划已理解，可以切换 DeepSeek”，并停下来等待用户切换模型。

切换 DeepSeek 后继续同一任务，不要重新规划或要求用户重复上下文。实现本文 Task 1 的全部端到端范围：持久化入站幂等、每 profile 最近 5 次 forwarding attempt、delivery 计时与 Shell timeout、安全 authenticated API，以及独立 `/forwarding` WebUI。

`src/storage.rs` 中已有红灯测试是 Rust 公共行为的基线规格。不得删除、跳过或弱化断言；先逐个实现接口使其变绿，再补迁移、并发重复、runtime 事件、HTTP/Shell timeout、API auth/shape 和前端状态测试。保持现有 Tokio/zbus、rusqlite transaction、lease token、固定并发与至少一次重试语义。迁移必须兼容已填充数据库并保留历史消息。不得记录、返回或渲染号码、短信正文、token、命令、provider response 或原始敏感错误。

所有工作保留在当前功能分支。完成 Rust 后端并通过 `cargo test` 与 `cargo fmt --check` 后，创建一个后端逻辑提交；完成 WebUI、生成 route tree 并通过 `pnpm generate-routes`、`pnpm test`、`pnpm check`、`pnpm build` 后，创建一个前端逻辑提交。提交时精确 stage 文件，不 amend 或改写已有基线提交，不 push。最后报告提交 hash、改动摘要、完整命令结果，以及 provider 响应丢失下仍存在的至少一次投递边界，然后等待 Codex 验收。

## 最终验收顺序

1. GLM 完成理解检查后，用户在同一 Agent 任务中切换到 DeepSeek。
2. DeepSeek 完成后端与前端逻辑提交，但不 push。
3. 回到本任务，由 Codex 检查分支提交、diff、接口契约、迁移安全、隐私边界和测试质量。
4. Codex 运行 Rust/前端完整验证，并给出返工项或验收结论。
