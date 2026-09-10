# 功能 5：数据备份与恢复

> 本文是设计规格，不包含实现代码。目标产品为 SmsRelayed，SimAdmin 的现有备份能力作为交互和组件化设计的参考基线。

## 0. 证据边界与阅读约定

本文核对的 SimAdmin 基线是 `/tmp/SimAdmin` 的 clean `main@eb6f497ad332e59f1f3899acc3b2b9427661fe5c`。

- “SimAdmin 已确认”表示该提交的实际兼容行为；它可能包含安全或原子性缺口，只供识别旧包/旧交互，不能被当作推荐实现。
- “重建契约”“必须/禁止”表示 SmsRelayed 的目标语义；“建议/推荐”仍需在实现前固化为 schema、默认值和测试 fixture。
- portable archive、same-device snapshot 和 pre-restore snapshot 是三个不同安全域。正文提到“备份”时如未说明，默认指 portable archive；不得把逻辑 ZIP、SQLite 快照和回滚点互换概念。
- 本功能不承诺读入 SimAdmin ZIP。若未来要兼容，应作为只读 importer 单独标识 `source_format=simadmin-v1`，先转换为 SmsRelayed staging model，不能直接调用 live import。

## 1. 目标与非目标

### 1.1 目标

本功能需要为 SmsRelayed 提供一套可审计、可预览、可回滚的数据保护流程：

- 按组件选择备份内容，支持精简备份、完整备份和同机灾备快照。
- 在任何写入前完成归档格式、版本、组件、大小、哈希、权限和语义校验。
- 恢复必须经过“上传/选择、校验与冲突策略、执行”三步，用户在真正写入前能看到影响范围。
- 每一次恢复前自动生成完整的 pre-restore snapshot，并在验证失败或服务重启失败时使用它回滚。
- 让消息正文、API 凭据、转发渠道 secret、eSIM Matching ID 等数据有明确的默认处理方式，避免凭据泄漏到归档、日志、预览和错误信息。
- 兼容现有 SQLite WAL、迁移、消息去重、出站发送状态、投递 lease 和至少一次投递语义。
- 让 API、CLI、前端和自动化任务共享同一个恢复协调器、操作状态和失败语义。

### 1.2 非目标

- 不备份或回滚 ModemManager、调制解调器硬件、SIM/eSIM 运营商侧状态，也不执行 eSIM profile 下载、安装、启用或删除。
- 不把自更新下载的二进制、systemd/procd 单元、操作系统包、内核或 /etc 外部文件纳入可移植数据归档；归档只记录构建版本和 commit，不能代替 OTA 二进制回滚。
- 不恢复活动认证 session；恢复后必须重新认证。
- 不提供任意 SQL 导入、任意路径写入或将归档中的部署路径直接覆盖到目标机器的能力。
- 第一阶段不承诺 WebDAV、对象存储、多机实时复制或跨设备自动同步；本地目录和浏览器下载是首选范围。
- 不承诺短信发送或渠道通知的 exactly-once。现有系统是 at-least-once，恢复期间必须优先避免盲目重发而不是伪造 exactly-once。

### 1.3 术语

| 术语 | 本规格中的唯一含义 |
| --- | --- |
| archive | 带 manifest 和逻辑组件的可移植 `.srb`；不含原始 SQLite schema |
| snapshot | 同设备、一致性的完整 SQLite + 配置 + journal 回滚 artifact；不作为普通下载文件 |
| pre-restore snapshot | 每次 destructive apply 前自动创建的完整 snapshot，供该 operation 回滚 |
| staging | 校验/解密/迁移的私有临时区；未 commit 的内容绝不能出现在备份库列表 |
| component | 有独立 schema version、分类、摘要和 import adapter 的逻辑数据集合 |
| manifest | 描述格式、来源、组件和摘要的元数据；它不是数据来源签名本身 |
| merge | 按组件业务键幂等合并并报告冲突，不清空整个组件 |
| replace | 只替换明确选择的逻辑组件；不替换当前 sqlite_schema、session 或本机部署路径 |
| quarantine | 数据已导入但 worker 不得 claim/send 的恢复状态 |
| preview token | 绑定 archive hash、目标 revision/schema、选择与确认摘要的一次性/短期授权，不是 session |
| operation journal | 持久记录 restore/rollback 阶段与恢复材料的非敏感日志，供崩溃恢复 |

## 2. SimAdmin 参考基线

### 2.1 组件清单

SimAdmin 当前使用 ZIP 容器、manifest.json 和 components/{component}.json。格式版本为 1，应用标识为 simadmin，组件级记录数和 SHA-256 写在 manifest 中。组件清单如下：

| 组件 key | 内容边界 | 默认精简 | 完整备份 | 敏感性与恢复备注 |
| --- | --- | :---: | :---: | --- |
| config | 设备网络、漫游/数据/APN、工作模式、eSIM 基础配置、版本通知和备份配置 | 是 | 是 | 包含配置类敏感字段；不是整份 AppConfig |
| sms | 短信方向、号码、正文、时间、状态、PDU 等 | 是 | 是 | 含正文和个人数据 |
| notification_config | 通知配置、Webhook 和渠道规则 | 是 | 是 | 前端标注隐私，当前后端敏感标记仍较宽松 |
| notification_logs | 通知事件、规则、渠道和消息记录 | 否 | 是 | 可能包含正文或渠道标识 |
| notification_queue | 队列状态、重试次数、错误、标题、正文和过期时间 | 否 | 是 | 恢复时不能无条件重新发送 |
| automation_config | 自动化任务定义、计划和动作 | 是 | 是 | 恢复前需要校验动作权限 |
| automation_logs | 自动化运行历史 | 否 | 是 | 只恢复审计历史，不重新执行 |
| sim_cache | SMSC、号码、IMSI/ICCID 等缓存 | 是 | 是 | 设备相关，跨设备恢复需谨慎 |
| esim_cache | eSIM profile/eUICC 缓存，包括 Matching ID 等元数据 | 是 | 是 | 只恢复缓存，不改变 eUICC |
| auth | 安全配置和 auth_config；不包含 session | 否 | 否 | 必须显式勾选，导入会清理当前 session |

默认精简集合是 config、sms、notification_config、automation_config、sim_cache、esim_cache。完整集合包含除 auth 外的其余九个组件；SimAdmin 的“full”并不自动包含 auth。组件可自由选择，full/slim 只是预设，不应绕过敏感数据确认。

### 2.2 校验和导入流程

SimAdmin 当前已确认的校验包括以下内容：

- ZIP 使用 `enclosed_name()` 接受相对安全路径；无法 enclosed 的条目会被跳过而不是令整个包失败。manifest 必须存在。重复同名条目会以后读到的内容覆盖内存 map 中先前内容；未在 manifest 声明的额外条目不会被拒绝。
- 校验 app、format_version、非空组件列表、组件文件存在性。
- 对每个组件计算 SHA-256，并核对 manifest 中的记录数和文件摘要。
- 预览返回文件名、备份类型、格式版本、生成时间、版本、敏感标记、组件数量和警告。
- 上传 preview/apply 的压缩 body 上限为 50 MiB；本地文件 apply 没有同一 body limit，解压后总量、压缩比、单条目大小和组件字段长度没有独立硬上限。当前没有整体签名/整体摘要，也没有归档级加密。

前端是三步恢复界面：

1. 上传或选择本地文件，读取 manifest 并校验。
2. 选择组件和 merge/replace 冲突策略；auth 需要单独确认。
3. 执行导入，显示进度和日志；导入 auth 后提示当前 session 将失效。

SmsRelayed 应保留这套用户心智模型，但把“模拟进度”改为真实操作状态，把敏感字段和跨文件原子性补齐。

### 2.3 pre-restore snapshot 与清理参考

SimAdmin 在 apply 前写一个逻辑 ZIP 到 `pre-restore` 目录，但已确认它并不是“当前所选组件”的镜像：它固定包含默认六组件 `config,sms,notification_config,automation_config,sim_cache,esim_cache`，只有本次显式导入 auth 时才额外包含 auth。它不包含 notification logs/queue 或 automation logs。代码在 apply 失败后不会自动导回该文件；前端可以让用户以后选择文件，以 replace 模式再次 apply。正常备份和 pre-restore 使用两个目录但复用同一套清理设置，默认保留期 7 天、最多 10 个文件；文件名使用 `simadmin-backup-` 前缀和 `.zip` 后缀。自动化任务也可生成普通本地备份。

该机制有三个需要在 SmsRelayed 中修正的地方：

- 只保存所选组件不能保证跨配置文件和 SQLite 的整体回滚，因此 SmsRelayed 的 pre-restore 必须是完整当前状态。
- 直接 fs::write 以及先提交数据库、再写配置会造成半成功状态；SmsRelayed 必须使用临时文件、fsync、操作日志和回滚恢复。
- pre-restore 是逻辑 JSON 归档而不是 SQLite 物理克隆；SmsRelayed 需要同时支持可移植逻辑归档和停止服务后的同机数据库快照。

### 2.4 SimAdmin 已确认的真实 HTTP 契约

所有下列路由都位于认证 middleware 后。除 ZIP 下载成功外，大部分业务失败仍返回 HTTP 200，错误位于 `{status:"error",message,...}` envelope；这是旧兼容事实，不是重建目标。

| 方法与路径 | 实际请求 | 实际成功结果/重要限制 |
| --- | --- | --- |
| `GET /api/backup/options` | 无 | format version、默认组件、每组件 count、服务端绝对 local/pre-restore 路径 |
| `GET /api/backup/config` | 无 | 规范化后的 `BackupConfig` |
| `POST /api/backup/config` | `{"config": BackupConfig}` | 覆盖配置；无 revision/If-Match |
| `POST /api/backup/export` | `{"components":[...]}` | 同步在内存构建完整 ZIP，`application/zip` 下载 |
| `POST /api/backup/export-local` | 同上 | 同步写入 server local dir，返回 `file` |
| `POST /api/backup/data/clear` | `{"components":[...]}` | 直接清空/重置 live 组件；不是 restore 子步骤 |
| `POST /api/backup/import/preview` | body 是原始 ZIP bytes | 同步解析，返回非 token 化 preview |
| `POST /api/backup/import/apply?mode=merge&components=a,b` | body 是原始 ZIP bytes；`mode` 默认 merge；components 是逗号字符串 | 空 components 自动选 archive 中除 auth 外全部；同步 pre-backup + apply，无 operation ID |
| `GET /api/backup/files` | 无 | `{backups,pre_restore}`，两个目录的文件摘要 |
| `GET /api/backup/files/{filename}/preview` | path filename | 重新读取/解析本地 ZIP |
| `POST /api/backup/files/{filename}/apply?mode=...&components=...` | query 同上传 apply | 同步 apply；没有 body/preview token |
| `GET /api/backup/files/{filename}` | 无 | 普通备份和 pre-restore 都可下载 |
| `DELETE /api/backup/files/{filename}` | 无 | 普通备份和 pre-restore 都可直接删除 |

本地 filename 必须以 `simadmin-backup-` 开头、`.zip` 结尾，字符限 ASCII 字母数字、`-_.`；查找顺序先普通目录再 pre-restore。配置中的 `storage.local_dir` 只要非空就直接作为路径使用，没有允许根、symlink、owner 或 mode 校验。

### 2.5 SimAdmin 已确认的 apply 顺序与恢复缺口

1. 解析 ZIP、校验 manifest/app/version、所选组件存在、每组件 SHA-256 与 record count。
2. 同步生成上述固定默认集合的 pre-restore ZIP。
3. 先在单个 SQLite transaction 中 import 所选 DB 组件并 commit。
4. 再通过 `replace_config` 保存合成后的配置。
5. 只有选择 auth 时才清空 session。

没有 worker quiesce、SQLite connection swap、operation lock/journal、post-import `integrity_check/foreign_key_check`、service restart/health check 或自动 rollback。若第 3 步 commit 后第 4 步失败，数据库已改变而配置未改变。merge/replace 只影响行表的删除策略；config、notification config、automation config 都直接用归档值覆盖相应分区，不因 mode 不同而做字段级 merge。notification queue 的 status/retry 时间会原样导入，因此可能被现有 worker 后续消费。

### 2.6 SimAdmin 已确认的默认值与组件细节

| 项 | 实际默认/规范化 |
| --- | --- |
| backup enabled | `false` |
| components | `config,sms,notification_config,automation_config,sim_cache,esim_cache` |
| schedule | mode `manual`；weekdays 1..7；time `04:00`；interval `7 days` |
| cleanup | retention 启用、7 天；max-files 启用、10 个；规范化范围分别 1..3650、1..1000 |
| storage | `/opt/simadmin/backups` |
| upload compressed limit | 50 MiB |
| format/app | `format_version=1`，`app="simadmin"` |
| filename | `simadmin-backup-{full|slim}-YYYYMMDD-HHMMSS.zip`，秒粒度，已存在时会覆盖 |

`BackupConfig.schedule,last_run_at,last_run_key` 在该提交中只有配置/前端存储，没有独立 backup scheduler 消费；真正的定时备份来自 automation action `backup_data`。因此不能仅凭保存 backup schedule 就宣称会自动运行。

`full` 的判定是选择了除 auth 外的九个组件；额外选择 auth 不改变 full。`sensitive()` 只把 config、sms、sim_cache、esim_cache、auth 标为敏感，notification config/log/queue 和 automation config/log 即使实际字段含 webhook、body、detail，也会被标为非敏感。这是参考实现的分类缺口。

归档由整个组件 JSON 的 SHA-256 和 record count 保护；manifest 本身、组件顺序/整体 ZIP、额外文件不在签名边界。导出逐组件分别锁数据库连接读取，不保证多个组件来自同一 SQLite snapshot；ZIP 完整构建在内存中。local 写入使用 `create_dir_all + fs::write`，没有 `.partial`、`create_new`、fsync、原子 rename 或权限固化，写完即按 mtime 清理。

已确认 merge 去重键包括：SMS 的 direction/phone/content/timestamp/PDU；notification/automation log 的全部选定字段；notification queue 的 status/event/summary/channel/rule/title/body/next_attempt/created_at。SIM/eSIM cache 使用 identity/ICCID/cache key upsert。replace 会先 DELETE 组件对应表，再逐行插入；这与 SmsRelayed 推荐的 quarantine、状态合法性和跨组件外键处理不是同一语义。

## 3. SmsRelayed 当前结构

### 3.1 配置文件

当前配置由 AppConfig 以 TOML 保存，默认路径为 /etc/sms-relayed/config.toml。主要分区为：

| 分区 | 当前内容 | 备份边界 |
| --- | --- | --- |
| app | device_name、modem_path | device_name 可移植；modem_path 是部署路径，不直接覆盖 |
| sms | storage 忽略策略、验证码关键词等 | 可移植 |
| forward | enabled、profile 列表 | 可移植，但 profile 的 secret 分离 |
| delivery | 并发数、渠道超时等 | 可移植；运行中的 lease 不属于配置 |
| channels | Bark、Telegram、WeCom、DingTalk、Lark、Webhook profile | profile 名称和非 secret 参数可备份，secret 单独保护 |
| api | enabled、bind、port、IPv6、trusted proxy、password、database_path | password 不进入普通归档；bind/port/path 视为本机部署策略 |
| http | 请求超时等 | 可移植 |
| retention | 消息保留期和批量大小 | 可移植 |
| monitoring | 监控设置 | 可移植，但不得泄漏 secret |

渠道 secret 包括 Bark key、Telegram bot_token/chat_id/API base 中的敏感部分、企业微信 secret、钉钉 secret、Lark webhook/secret，以及 Webhook URL、body、headers 中可能出现的 token、Bearer 值或个人信息。当前配置摘要会对这些字段做 redaction，但配置文件中的 api.password 仍是可读 TOML 值，因此备份不能把“当前配置文件可读”误当成“可公开归档”。

配置保存已有较好的原子写入基础：创建 0600 临时文件，写入并 sync_all，通过 rename 替换目标，再同步父目录；临时文件失败时由 Drop 清理。备份实现应复用这一语义，并对用户配置的备份目录同样强制 0700/0600、拒绝 symlink，而不能只依赖默认 /etc/sms-relayed 目录。

### 3.2 SQLite、迁移和派生数据

默认数据库路径为 /etc/sms-relayed/sms-relayed.sqlite。打开数据库时启用 foreign_keys、WAL、busy timeout；主库、-wal、-shm 文件均通过 no-follow 方式打开并限制为 0600。任何物理复制都必须把 WAL 一并纳入一致性边界，不能只复制主库文件。

主要表和数据性质如下：

| 数据 | 当前结构或用途 | 备份策略 |
| --- | --- | --- |
| messages | inbound/outbound 消息、phone_number、body、timestamp、source、read_at、favorite_at、error、dedupe key、outbound phase、lease owner/deadline | 主要业务数据；portable 归档按 typed record 导出 |
| outbound_idempotency | key、request_hash、message_id、created_at | 同机恢复可选；跨设备导入需防止 key 冲突 |
| forward_deliveries | 每个 message/profile 的 pending、in_flight、retry_wait、succeeded、permanent_failed、attempt、next_attempt、lease 和错误 | 状态与消息一致性重要；活动 lease 不能原样恢复 |
| forward_attempt_samples | 最近的渠道尝试、耗时、调度延迟、结果和错误码 | 审计历史；不重新执行 |
| conversation_pins | 会话置顶 | 用户状态，可选恢复 |
| conversation_summaries | 会话计数、未读数、最后消息 | 派生数据，导入后由消息和触发器重建/校验 |
| meta | modem_fingerprint、modem_dedupe_namespace、runtime_modem_fingerprint、mismatch 标记等 | 设备绑定；portable 默认不导出 |
| auth_sessions | token_hash、credential_proof、expires_at | 永不导出或导入 |
| auth_credential_state | salt、verifier | 与本机 .auth-key 绑定；portable 不导出 |
| SQLite indexes/triggers | 去重、会话汇总、状态约束和外键 | 不作为数据组件导入，由当前程序和 migrations 重建 |

src/storage/migrations.rs 会逐步增加 favorite_at、inbound_dedupe_key、outbound phase、dispatch_delay_ms、唯一约束、meta/outbox 初始化以及 conversation summary 回填。备份不能直接替换 sqlite_schema、user_version 或表文件；必须在当前程序打开并运行现有迁移后，以组件适配器逐行导入。

### 3.3 认证与 session

认证入口为 /api/auth/login、logout、me，session 通过 HttpOnly cookie sms-relayed-session 传递，默认有效期 7 天，且有失败限流和最多 256 个 session 的约束。SessionStore 使用数据库路径同目录的 .auth-key（32 字节、0600），数据库只保存 token_hash、credential_proof 和过期时间；auth_credential_state 保存 PBKDF2-HMAC-SHA256 派生的 verifier 和 salt。

密码变化会删除 auth_sessions。由于 credential proof 依赖本机 .auth-key，单独导入数据库里的 auth_credential_state 不能可靠迁移认证能力。设计原则是：

- 不备份 token 明文、token_hash、credential_proof、auth_sessions、.auth-key。
- 不在预览、操作日志、错误响应、CLI 输出中显示密码或 token。
- 任意恢复完成后清理本机所有 session；如果配置中的 API 密码发生变化，要求使用新密码重新登录。
- .auth-key 保留在本机，不从 portable 归档导入；如果执行凭据重置，可主动轮换它并重新生成 credential state。

### 3.4 消息、出站发送和转发投递

messages 的状态包括 received、sending、sent、failed；出站阶段还包括 created、prepared、send_started、uncertain、unknown、complete。uncertain/unknown 表示 modem 可能已经发送，不能因为恢复而自动再发一条短信。

forward_deliveries 采用 pending、in_flight、retry_wait、succeeded、permanent_failed，使用 lease_at/lease_token 做 compare-and-swap；DeliveryWorker 会恢复过期 lease、按 30 秒安全扫描和下次重试时间调度。forward_attempt_samples 只记录尝试历史。持久化层还提供 outbound idempotency、inbound dedupe、消息保留和 active delivery 保护。

因此：

- portable 归档应保存消息和投递历史，但不能把某个 worker 的 lease token 当成可迁移凭据。
- in_flight 的转发必须在恢复时转为“待人工确认”或安全的 retry_wait，不能在没有明确选项时立即重发。
- sending、uncertain、unknown 的出站短信默认进入 quarantine/review，不自动调用 modem。
- 恢复完成后是否恢复投递队列必须是显式策略；默认 history-only，用户确认后才允许 resume。

### 3.5 自更新结构

src/update.rs 会下载校验文件和平台二进制，在目标文件同目录生成临时文件，流式计算 SHA-256，校验版本/commit，fsync、chmod 0755 后以 rename 替换旧二进制，再请求 systemd 或 OpenWrt procd 重启。失败时保留旧二进制，重启失败时会明确告知“二进制已更新但服务未重启”。

备份只在 manifest 记录 app_version、build_commit、数据库/配置 schema 版本和架构；不打包当前或旧二进制，不覆盖 update 临时文件，也不把“恢复数据”描述成“回滚版本”。如果旧版本无法读取新组件，必须在预览阶段拒绝或明确提示需要先安装兼容版本。

## 4. 推荐归档格式

### 4.1 容器与编码

推荐使用带 .srb 后缀的 ZIP 容器，格式标识为 sms-relayed-backup。选择 ZIP 是为了沿用 SimAdmin 的本地库和下载体验；ZIP 只负责容器，不使用传统弱 ZIP 密码。条目使用 UTF-8、固定路径、固定排序和固定换行，便于重现摘要和离线校验。

建议布局如下：

    manifest.json
    components/config.public.toml
    components/messages.ndjson
    components/forwarding.state.ndjson
    components/forwarding.attempts.ndjson
    components/ui.state.json
    components/idempotency.ndjson
    components/secrets.enc                 # 仅显式导出并加密时存在
    components/personal.enc                # 需要保护短信正文时存在

NDJSON 适用于大规模消息和投递记录：一行一个版本化记录，允许流式导入并设置行数、字段长度和总大小限制。小型配置和 manifest 使用 UTF-8 TOML/JSON。portable 归档不包含原始 SQLite 文件。

同机灾备快照是另一种 artifact，可以使用相同 manifest，但增加：

    snapshot/database.sqlite
    snapshot/config.toml.enc
    snapshot/operation.json

database.sqlite 必须由 SQLite online backup 或等价的一致性快照生成，而不是在 WAL 工作期间直接复制主库。快照只用于同机回滚，默认不提供下载为 portable 归档。

### 4.2 manifest、版本和 schema

manifest 至少包含以下字段：

    {
      "format": "sms-relayed-backup",
      "format_version": 1,
      "manifest_schema": 1,
      "app_version": "...",
      "build_commit": "...",
      "created_at": "...",
      "kind": "slim|full|snapshot",
      "scope": "portable|same_device",
      "source": {
        "config_schema": 1,
        "sqlite_schema": 7,
        "config_revision": "..."
      },
      "components": [
        {
          "id": "messages",
          "schema_version": 1,
          "path": "components/messages.ndjson",
          "encoding": "ndjson",
          "records": 123,
          "bytes": 45678,
          "sha256": "...",
          "classification": "personal",
          "required": false
        }
      ],
      "encryption": {
        "mode": "none|envelope",
        "algorithm": "XChaCha20-Poly1305",
        "kdf": "Argon2id",
        "salt": "...",
        "payload": "..."
      }
    }

规则如下：

- format_version 是归档协议的大版本，manifest_schema 是 manifest 本身版本，component schema_version 独立演进，不能混用 SQLite user_version。
- app_version/build_commit 仅用于兼容性提示，不作为可信安全边界。
- manifest 必须列出每个条目的路径、编码、记录数、字节数、SHA-256、组件 schema 和数据分类。
- manifest 只包含计数、摘要和非敏感版本信息；不包含密码、token、短信正文、完整电话号码、Webhook header 或 Matching ID。
- 加密归档的 manifest 可以明文保留组件 id、分类、密文大小和摘要，但不能泄漏正文或 secret；需要密码解密后才能解析受保护组件。
- 可选地增加 Ed25519 签名或外部签名文件，签名覆盖规范化 manifest、条目摘要和格式版本。加密的 AEAD tag 是机密性和完整性保护，但不替代来源签名。

### 4.3 SmsRelayed 组件边界

建议把可移植内容拆成以下组件，组件 id 一旦发布后只追加字段，不复用已有 id 表示不同语义：

| 组件 id | 包含 | 默认策略 |
| --- | --- | --- |
| config.public | 非 secret 的 app、sms、forward、delivery、channels、http、retention、monitoring；profile 名和运行开关保留 | 精简和完整均可选 |
| messages | 消息正文、号码、方向、状态、时间、来源、阅读/收藏状态；portable 时清理 modem_sms_path | 精简默认选中，正文视为 personal |
| forwarding.state | message/profile 关系、成功/永久失败/待处理状态、次数、错误摘要和下一次时间 | 精简默认选中；活动 lease 不原样导入 |
| forwarding.attempts | provider attempt 历史和调度延迟 | 完整默认选中 |
| ui.state | 会话置顶等非消息派生 UI 状态 | 完整默认选中 |
| idempotency | outbound idempotency key、request_hash、message 关联 | 仅同设备或显式选中；跨设备默认跳过 |
| secrets.enc | API password、APN password、渠道 key/token/secret、Webhook 敏感 URL/body/header 等 | 永不默认；必须加密并二次确认 |
| personal.enc | 被用户要求保护的短信正文和号码等个人数据 | 当归档离开本机时推荐/可强制加密 |
| device.meta | modem fingerprint、dedupe namespace、runtime mismatch 等设备绑定信息 | 仅 same_device snapshot |

conversation_summaries、索引和触发器不作为独立导入数据；它们由当前迁移和重建流程生成。auth_sessions、.auth-key、auth_credential_state 不属于任何 portable 组件。

配置中的 database_path、modem_path、API bind/port 和 trusted proxy 等部署策略不能由 portable 归档覆盖。若需要同机完整回滚，snapshot 可以保存原值，但恢复前仍需确认目标路径与当前安装一致。

### 4.4 精简、完整和快照

- 精简备份：config.public、messages、forwarding.state；默认不含尝试样本、idempotency、device.meta、secrets.enc。适合日常本地备份和消息迁移。
- 完整备份：全部安全 portable 组件，包括 config.public、messages、forwarding.state、forwarding.attempts、ui.state，以及用户明确允许的 idempotency。完整不等于包含 secrets 或 session。
- 凭据包：secrets.enc 的显式附加选项，不受 full 预设自动启用；必须输入归档密码并在执行前再次确认。
- 同机灾备快照：保存完整当前 SQLite 一致性快照、完整配置和操作元数据，包含回滚所需的本机 secret，但权限为 0600、不可通过普通下载接口导出；可使用安装级本地密钥加密。

## 5. 敏感数据策略

### 5.1 凭据和转发渠道 secret

普通 portable 归档中必须删除或替换以下值：

- api.password、APN username/password 中的 password；
- Bark key、Telegram bot_token 和相关 token、企业微信 secret、钉钉 secret、Lark secret；
- Webhook URL 中的 token、Authorization/Bearer header、body 中的固定 secret，以及任何等效的自定义 header；
- .auth-key、auth_credential_state 和 session 数据。

非 secret 字段可以保存 profile 名称、启用状态、超时、HTTP method、content type、非敏感规则和已 redacted 的 endpoint。恢复后保留 profile 但将缺失 secret 的 profile 标为 needs-credentials，并禁止 worker 发送，直到用户补齐。

如果用户选择导出 secrets.enc：

- 使用应用层 envelope encryption；推荐 Argon2id 派生密钥、随机 salt、随机 nonce 和 XChaCha20-Poly1305 AEAD。不能使用传统 ZIP 密码。
- 密码只能来自交互式输入、受限权限的文件或 stdin，不能放入 URL、命令行参数、操作日志或前端 localStorage。
- 预览只显示“包含凭据、需要密码”，不显示字段值；错误信息只报告解密/认证失败，不回显密文内容。
- 导入 secret 前要求显式授权，并记录组件 id、操作结果和操作者，不记录 secret 本身。

### 5.2 session 与认证状态

归档导入不接受 session。每次恢复，包括只恢复 messages 的恢复，都应在 commit 后清理本机 auth_sessions，并使当前浏览器 cookie 失效；API 需要返回 session_invalidated=true，前端转到登录页。

同机 snapshot 回滚也不恢复旧 session。回滚 auth_credential_state 时必须用当前本机 .auth-key 重新同步；如果不匹配，删除旧 credential state 并按当前配置密码重新生成。凭据恢复后的唯一可接受结果是“旧 session 全部失效、用户重新登录”，不能尝试让旧 cookie 继续有效。

### 5.3 短信正文、号码和投递日志

短信正文、完整号码、Webhook body 和错误文本均按 personal 数据处理：

- 预览只显示组件名称、记录数、时间范围、大小和冲突数，不显示正文、完整号码或 provider response。
- 本地文件权限为 0600；归档要离开本机、通过浏览器下载或被标记为 personal 时，默认启用 personal.enc 或要求用户确认未加密导出。
- 诊断日志只记录 operation_id、组件 id、计数和错误码；不记录正文、完整号码、归档密码或渠道响应。
- metadata-only 是可选的合规导出模式，保留方向、时间、状态和稳定内部 id，但不包含 body/phone_number；metadata-only 归档不能声称可以完整恢复短信。

### 5.4 eSIM Matching ID

SmsRelayed 当前没有 SimAdmin 的 eSIM profile 管理表，但 SimAdmin 的 esim_cache 包含 matching_id、SMDP、ISDP AID、ICCID 等高敏感缓存。若未来导入这类组件：

- Matching ID、SMDP 地址、ISDP AID、ICCID/IMSI 以及 raw eUICC 缓存都视为 credential/activation data，默认不导出。
- 可移植归档只保留“存在缓存”和非敏感统计；完整缓存只能进入 secrets.enc 或同机加密 snapshot。
- 恢复缓存永远不触发 eSIM 操作；跨设备恢复默认丢弃 device.meta 和 eUICC 关联，必须由用户在目标设备重新确认。
- 前端预览只显示“eSIM 敏感数据已省略/已加密”，不显示 Matching ID 的部分掩码也不应写入普通日志。

## 6. 导出、校验和原子写入

### 6.1 导出一致性

导出需要通过 BackupCoordinator 获取只读一致性视图：

1. 锁定备份/恢复操作，暂停会改变归档边界的清理和自动备份。
2. 配置通过安全打开和稳定 revision 读取；如果在读取期间发生保存，重试或失败，不拼接两份配置。
3. SQLite 使用只读独立连接或 SQLite online backup 获取一致视图，不长期占用业务 writer。必要时在同一快照上读取 messages、deliveries、attempts、UI 状态和 idempotency。
4. 按固定排序流式生成组件，写入临时归档，计算组件摘要和 manifest。
5. 关闭并 fsync 临时文件，重新验证后以原子 rename 放入备份目录，再 fsync 目录。任何 .partial 文件都不能出现在文件列表或下载接口中。

### 6.2 归档校验

导入前和导入过程中都执行校验：

- 归档总大小、解压后总大小、组件数、记录数、单行长度、字段长度和压缩比都有硬上限；manifest 不能让程序预先分配不受限内存。
- 条目路径只能匹配 manifest 中声明的固定前缀和文件名；拒绝绝对路径、..、重复条目、符号链接、目录伪装、未知必需条目和超出备份根目录的路径。
- 校验 format、manifest schema、组件 schema、app id、条目编码、记录数、字节数、SHA-256 和可选签名。
- 每条记录按当前 schema 解码并进行类型、枚举、时间范围、长度、外键引用和业务状态校验。错误要指出组件、记录序号和错误码，但不泄漏字段值。
- SQLite 变更完成后执行 integrity_check、foreign_key_check、关键计数检查和派生 summary 校验；配置导入前执行 AppConfig validate。
- 解密时先验证 AEAD tag，再解析明文；密码错误和篡改统一为不可恢复的校验错误，不尝试部分导入。

校验结果应区分 fatal、warning 和 informational。fatal 不允许进入第三步；warning 必须在最终确认页再次显示。

### 6.3 原子写入和操作日志

恢复涉及配置文件、SQLite 和服务进程，不能假设文件 rename 与数据库 commit 是一个操作。推荐使用恢复 journal：

- 每个操作生成不可预测 operation_id，在私有 staging 目录创建 operation.json，记录阶段、源归档摘要、目标组件、snapshot_id 和清理状态；不写入 secret/body。
- 先完成 pre-restore snapshot，再对暂存数据库和配置执行导入、迁移、完整性检查。
- 数据库变更使用单个事务；配置写入使用现有 prepare_secure_write 的 temp + sync_all + rename + parent fsync。
- replace、snapshot rollback 或涉及 schema/配置变化的操作优先在停止服务的 maintenance window 内进行。关闭 SQLite 连接并处理 WAL 后，再以可恢复的 rename/交换替换目标文件。
- merge 也必须在单一数据库事务内完成；如果配置随后提交失败，用 journal 和 snapshot 恢复数据库，不能返回成功。
- 只有数据库、配置、校验和服务启动后的健康检查全部通过，journal 才标记 committed；否则进入 rollback-required，保留 snapshot 和诊断信息。

## 7. 恢复语义

### 7.1 三步恢复

#### 第一步：上传/选择与预览

用户可上传归档、从本地备份库选择文件，或选择 snapshot。服务端先执行大小、路径、manifest、摘要、解密和记录语义校验，返回：

- 归档类型、scope、生成时间、来源版本、schema；
- 可用组件、分类、记录数、大小、时间范围和是否需要密码；
- 目标当前 schema/版本的兼容结果；
- 预估磁盘空间、是否需要停机/重启、是否会清理 session；
- fatal/warning 列表和 preview_token。

此阶段禁止写业务数据库、配置、session 或 worker 状态，只允许创建受限 staging 文件。

#### 第二步：组件、冲突和安全策略

用户选择：

- 要恢复的组件；
- merge 或 replace；
- metadata-only 或包含正文；
- 是否解密并导入 secrets；
- forward delivery 采用 history-only、quarantine 或显式 resume；
- 设备绑定数据是否仅限 same_device；
- 是否接受 service restart 和 session invalidation。

preview_token 绑定归档 SHA-256、当前 config revision、当前数据库 schema、组件选择和确认摘要；归档或目标发生变化时 token 失效，必须重新预览。

merge 的定义是按业务键去重并保留当前数据；replace 的定义是对所选组件的源数据做整体替换，但仍不删除未选择的组件，也不能删除当前运行所需的 schema、索引、触发器和认证保护。replace 前必须成功创建完整 snapshot。

#### 第三步：执行

用户确认后服务端重新校验 preview_token、权限、磁盘和当前版本，创建 pre-restore snapshot，然后进入 maintenance。返回 operation_id，前端通过轮询或 SSE 获取真实阶段：

    validating -> snapshotting -> quiescing -> applying -> verifying
    -> restarting -> healthy -> committed

失败时状态为 failed 或 rolled_back，并返回 snapshot_id、恢复建议和是否需要重新登录。不会用前端计时器伪造百分比。

### 7.2 pre-restore snapshot

每次 apply 都自动创建一个完整的当前状态 snapshot，即使用户只选择恢复 messages。snapshot 至少包含：

- 一致性的 SQLite 数据库；
- 当前完整配置的受保护副本；
- snapshot manifest、数据库 schema、config revision、服务版本、权限和创建时间；
- rollback 所需的 operation journal。

snapshot 写入私有目录的 .partial 目录，文件 0600、目录 0700，全部 fsync 后原子 rename。snapshot 成功后才能对 live 状态做 destructive replace。snapshot 不是普通可下载文件；API 只返回 id、状态、大小和时间。

回滚必须按 snapshot 的整套配置和数据库恢复，不能只把原来选择的组件再次 import。回滚成功后清理 session，重新启动服务并进行健康检查；回滚本身失败时保留 snapshot、当前文件和 journal，禁止自动删除任一份可用于人工恢复的副本。

### 7.3 merge/replace 的组件语义

- config.public：merge 只更新归档明确包含且用户确认的字段，保留本机部署路径、API bind/port、database_path 和当前缺失的 secret；replace 也不能覆盖这些部署策略。
- messages：以稳定消息 id、inbound dedupe key 或组合业务键做幂等去重；同 id 内容冲突不静默覆盖，记录冲突并要求 replace 或人工处理。portable 归档中的 modem_sms_path 清空。
- forwarding.state：以 message_id/profile_key 为键合并；导入前清空 lease owner/token/lease_at。in_flight、retry_wait、pending 默认 quarantine，不自动发出。
- forwarding.attempts：只追加或按稳定 attempt id 去重，永不触发发送。
- ui.state：按用户状态的业务键合并；conversation_summaries 始终从消息重建。
- idempotency：只有 same_device 或明确选择时恢复；跨设备冲突保留当前 key 并报告冲突，不能把不同 request_hash 合并。
- secrets.enc：逐字段显式替换，导入后将受影响 profile 标为待验证，并通过 test/health 检查确认，不在恢复阶段发送真实通知。
- device.meta：只允许同机 snapshot；portable 归档不恢复 modem fingerprint、dedupe namespace 和 mismatch 标记。

## 8. 恢复期间的服务与 worker 协调

当前 runtime 会同时启动 modem inbound worker、outbound worker、DeliveryWorker、retention worker、HTTP API，并使用 Store、EventBus 和 DeliveryWakeup。恢复必须增加一个共享的 Backup/Restore Coordinator，状态至少为 idle、preparing、quiescing、applying、verifying、restarting、failed。

协调规则：

- 同一时间只允许一个 backup/restore/rollback；清理任务和自动化备份在 maintenance 期间暂停。
- 第二步完成后禁止新的短信发送、消息删除、配置保存、渠道配置变更和 delivery claim；API 对这些操作返回明确的 maintenance 错误。
- 停止接收新的 modem 事件，等待当前事务结束；inbound worker、outbound worker、DeliveryWorker 停止 claim 新任务并在有界时间内 drain。无法 drain 的调用必须按现有 uncertain/lease 语义记录，不假装成功。
- 在数据库交换前关闭所有 SQLite 连接、checkpoint/保存 WAL，并等待 worker 退出；不能让旧 Store 持有被 rename 的文件句柄。
- 配置变更当前需要重启才能进入运行态，因此恢复 config、channel、delivery、API 或数据库路径相关设置后必须安排受控重启。若重启失败，操作不算 committed，按 snapshot 回滚或标记人工恢复。
- 重启后重新运行 migrations、完整性检查、SessionStore password synchronization 和过期 delivery lease recovery。默认不唤醒 quarantine 的投递；用户选择 resume 后才调用 DeliveryWakeup。
- restore 完成后恢复 inbound、outbound、delivery、retention 和自动备份，并发布一个不含敏感数据的 Config/Backup 状态事件。

对现有 at-least-once 投递语义的说明必须出现在 API 和前端：恢复可能导致已成功但归档未记录的渠道通知重试，也可能导致不确定的出站短信保持人工确认；系统不能用“恢复成功”承诺外部 provider 没有重复。

## 9. 清理策略

备份和快照使用独立策略、目录和锁：

| 类型 | 推荐默认值 | 清理原则 |
| --- | --- | --- |
| 正常 portable 备份 | 30 天、最多 20 个、可选总容量上限 | 只清理已校验且非活动文件；至少保留最近一个有效 full |
| pre-restore snapshot | 至少保留最近 3 个，最短 14 天 | 当前 operation、最近一次失败操作和最后一个 healthy snapshot 不得删除 |
| .partial/staging | 24 小时后 | 只清理没有 active journal 的孤儿目录；操作进行中不清理 |
| 无法校验的归档 | 不自动纳入保留计数 | 移到 quarantine 或标记 invalid，保留足够时间供诊断 |

清理流程只有在新备份完整写入、fsync、重新校验且登记成功后才能删除旧文件。按时间、数量、容量排序时不能删除最后一个有效恢复点；删除失败不应删除数据库或配置。清理和 restore 使用同一把 operation lock。

## 10. API 设计

SmsRelayed 当前没有 backup API；建议新增受现有认证 middleware 保护的接口：

| 方法 | 路径 | 作用 |
| --- | --- | --- |
| GET | /api/backup/options | 返回组件、版本、大小上限、存储能力、加密能力和当前空间 |
| GET | /api/backup/config | 返回备份目录、计划、保留期、数量/容量上限；secret 永不返回 |
| PUT | /api/backup/config | 使用 revision/If-Match 保存计划和清理配置 |
| POST | /api/backup/export | 创建 portable export operation；完成后 result 给出一次性下载资源或 local file_id |
| POST | /api/backup/export-local | `/export` 的兼容便捷入口；在受限本地库生成归档，返回同一种 operation |
| POST | /api/backup/import/preview | 上传归档，返回 preview_token 和非敏感摘要 |
| POST | /api/backup/import/apply | 携带 preview_token、mode、components 和安全策略执行恢复 |
| GET | /api/backup/operations/{id} | 查询真实阶段、进度、warning、snapshot_id 和最终状态 |
| GET | /api/backup/files | 列出已校验的 portable 归档和 snapshot 摘要 |
| GET | /api/backup/files/{id}/preview | 重做本地文件预览 |
| GET | /api/backup/files/{id} | 下载 portable 归档；snapshot 默认禁止下载 |
| DELETE | /api/backup/files/{id} | 仅删除用户明确指定的非保护归档 |
| GET | /api/backup/snapshots | 列出可回滚的 snapshot |
| POST | /api/backup/snapshots/{id}/rollback | 需要二次确认，执行完整 snapshot 回滚 |
| GET | /api/backup/health | 返回目录权限、空间、锁和最近一次操作状态 |

具体约束：

- 所有接口都返回 operation_id 或 request_id；错误响应使用稳定错误码，正文不进入错误消息。
- export/apply 支持 request idempotency key，重复请求不能生成两个并发 restore。
- apply 必须要求 preview_token、显式 components、mode 和确认摘要；replace、secret、resume、rollback 需要更强的确认。
- 认证 session 在 commit 后失效；正在执行 restore 的请求可以完成，但后续 polling 应允许使用短期 operation token 或重新登录后的只读查询。
- 上传上限和解压后上限均在 HTTP 层和归档层执行，不能只依赖 body size。
- 不复用 SimAdmin 当前“HTTP 200 + error envelope”造成的歧义；失败应使用合适的 4xx/5xx，同时返回 operation 状态。

### 10.1 重建请求/响应 schema

export 请求必须明确安全域，`full` 不隐式加入 secret：

```json
{
  "kind": "full",
  "scope": "portable",
  "components": ["config.public", "messages", "forwarding.state"],
  "destination": "download",
  "personal_data": "encrypted",
  "include_secrets": false,
  "expected_config_revision": "..."
}
```

接受后返回 `202`：

```json
{
  "operation": {
    "id": "...",
    "kind": "export",
    "state": "queued",
    "phase": "validating",
    "created_at": "...Z"
  }
}
```

operation 成功后 `result` 至少包含 `artifact_id,manifest_sha256,size,expires_at,download_path?`，客户端再从受保护、同源且短期有效的下载资源取文件。这样大归档、客户端断线和本地导出使用同一事实源；不得在内存中一次性构建整个 archive。若实现确需同步 stream，必须另写兼容 endpoint，响应带 request ID，不能与异步 operation 的语义混用。

上传 preview 使用 `Content-Type: application/octet-stream`、受限 body 和经过清理的 `X-Backup-Filename`；服务端流式写 staging，成功返回：

```json
{
  "preview_token": "...",
  "archive": {
    "sha256": "...",
    "format_version": 1,
    "scope": "portable",
    "created_at": "...Z",
    "source_version": "..."
  },
  "components": [
    {"id":"messages","schema_version":1,"records":123,"bytes":4567,"classification":"personal","compatible":true}
  ],
  "impact": {
    "restart_required": true,
    "session_invalidation": true,
    "required_free_bytes": 123456,
    "quarantined_deliveries": 2
  },
  "fatal": [],
  "warnings": []
}
```

apply 不再重新上传 ZIP；它用 JSON 引用 preview：

```json
{
  "preview_token": "...",
  "mode": "merge",
  "components": ["messages", "forwarding.state"],
  "delivery_policy": "quarantine",
  "import_secrets": false,
  "accept_restart": true,
  "confirmation": {"archive_sha256_prefix":"12ab34cd","replace":false}
}
```

成功接受返回 `202` operation。相同 `Idempotency-Key` 和 canonical body 返回同一 operation；不同 body 返回 `409 idempotency_conflict`。preview token 必须单次绑定 archive hash、目标 config revision/database generation、actor 和已确认选项；过期或目标变化返回 `409 preview_stale`，不自动重新解释用户选择。

operation 查询的固定状态集合为：

```text
queued -> validating -> snapshotting -> quiescing -> applying
       -> verifying -> restarting -> healthy -> committed
任意非 terminal -> cancelling -> cancelled（仅尚未 destructive commit 时）
任意阶段 -> rolling_back -> rolled_back
rolling_back 失败 -> recovery_required
校验/准备失败 -> failed
```

每次响应返回 `state,phase,phase_started_at,completed_units,total_units,warnings,snapshot_id,result,error,session_invalidated`。百分比只能由真实单位推导；不知道 total 时返回 null。`committed,rolled_back,failed,cancelled,recovery_required` 是 terminal；`failed` 仅表示 live 状态未变或已安全处理，若 live 状态不确定必须使用 `recovery_required`。

### 10.2 重建错误契约

```json
{
  "error": {
    "code": "archive_checksum_mismatch",
    "message": "backup validation failed",
    "request_id": "...",
    "retryable": false,
    "details": {"component":"messages","record_index":null}
  }
}
```

| HTTP | code 示例 | 语义 |
| --- | --- | --- |
| 400 | `invalid_json`,`invalid_archive`,`missing_preview_token` | 请求/容器不可解析 |
| 401/403 | `authentication_required`,`confirmation_required`,`snapshot_download_forbidden` | 身份、安全域或确认不足 |
| 404 | `artifact_not_found`,`snapshot_not_found`,`operation_not_found` | opaque ID 不存在 |
| 409 | `operation_in_progress`,`preview_stale`,`idempotency_conflict`,`restore_conflict`,`maintenance_active` | 目标或并发状态冲突 |
| 413 | `archive_too_large`,`expanded_size_limit`,`compression_ratio_limit` | 压缩/解压限制 |
| 422 | `unsupported_format`,`unsupported_component_schema`,`semantic_validation_failed`,`incompatible_target` | 可解析但不可应用 |
| 507 | `insufficient_storage` | 预检或写入空间不足 |
| 503 | `worker_quiesce_failed`,`restart_failed`,`dependency_unavailable` | 服务协调失败；是否已回滚从 operation 查询 |

错误 `details` 只允许组件 ID、记录序号、限制和稳定冲突计数；禁止包含原始行、正文、号码、secret、解密材料或 provider response。

## 11. CLI 流程

在现有 setup、run、send、update、config check/show 基础上增加：

    sms-relayed backup create --output PATH --kind slim|full
    sms-relayed backup preview PATH
    sms-relayed backup verify PATH
    sms-relayed backup list
    sms-relayed backup cleanup
    sms-relayed restore preview PATH
    sms-relayed restore apply PATH --mode merge|replace --components LIST
    sms-relayed restore snapshots list
    sms-relayed restore snapshots rollback SNAPSHOT_ID
    sms-relayed restore status OPERATION_ID

规则如下：

- restore 默认要求交互式确认；非交互执行必须同时传 --yes，并且 replace、secret、resume、rollback 还要有对应确认选项。
- 归档密码从 TTY、stdin 或 0600 的 passphrase file 读取，禁止使用命令行明文参数；进程列表和 shell history 不能看到密码。
- 在服务运行时，CLI 通过 coordinator 请求 maintenance；离线模式要求明确说明服务已停止并检查数据库路径，不能静默覆盖正在使用的 SQLite。
- backup create 可在运行时生成一致性 portable 归档；物理 snapshot 和 restore 必须具备关闭 worker/重启服务的能力，否则只做 preview，不执行 destructive apply。
- CLI 输出只显示组件、计数、版本、摘要、operation/snapshot id 和错误码，不显示短信正文、号码、secret 或 provider response。

## 12. 前端流程

前端可沿用 SimAdmin BackupRestore.tsx 的三个区域，但按 SmsRelayed 结构调整：

### 12.1 备份页

- 展示 config.public、messages、forwarding.state、forwarding.attempts、ui.state、idempotency、secrets 和 snapshot 的组件卡片。
- 每张卡片显示分类、记录数、预计大小、是否包含正文、是否设备绑定；secret/personal 使用显眼的隐私提示。
- local、download、未来 WebDAV 分开显示；local path 由后端提供且只读，前端不能让用户构造任意路径。
- 自定义文件名必须由后端校验并真正采用；文件名只允许安全字符，禁止覆盖已有文件和目录穿越。

### 12.2 恢复页

1. “上传/校验”：拖拽或选择文件，显示 manifest、摘要、版本、兼容性、空间和 warning；不显示正文。
2. “冲突策略”：选择组件、merge/replace、正文模式、secret 解密、投递 history-only/quarantine/resume；replace 和 credentials 有额外确认。
3. “执行”：显示 snapshot 创建、worker quiesce、应用、校验、重启和健康检查的真实状态；成功显示 rollback snapshot id，失败显示自动回滚结果。

页面必须在执行前明确：

- 将创建完整 pre-restore snapshot；
- 服务会进入 maintenance 并可能重启；
- 所有 session 会失效；
- active/unknown delivery 和 modem outbound 不会被无条件重发；
- 未包含在归档中的渠道 secret/eSIM 数据需要重新配置。

本地库页区分 portable archive、pre-restore snapshot 和 invalid/quarantine 文件。snapshot 只提供预览、回滚和受控删除，不提供普通下载。成功恢复后前端清除旧缓存并跳转登录页；进度不能用固定延时模拟。

## 13. 失败、回滚、磁盘和权限

### 13.1 失败矩阵

| 失败点 | 预期结果 |
| --- | --- |
| 上传、路径、大小、摘要、解密或语义校验失败 | 不接触 live 配置/数据库；删除 staging 或进入 quarantine |
| snapshot 创建失败 | 不开始 restore，保留原 live 状态，报告空间/权限/SQLite 错误 |
| worker 无法 quiesce | 超时并取消操作；不替换数据库，不删除旧 snapshot |
| 数据库事务失败 | 事务回滚，配置临时文件不 commit，live 状态不变 |
| 配置 commit 失败 | 使用 journal/snapshot 恢复数据库，保留失败诊断，不能返回部分成功 |
| 重建派生数据或 integrity_check 失败 | 停止重启流程，使用 pre-restore snapshot 回滚 |
| 服务重启/健康检查失败 | 标记 failed/rollback，保留 old database、new staging、snapshot 和 journal，提供人工恢复命令 |
| 自动回滚失败 | 不继续尝试覆盖；进入 recovery-required，保留所有副本并提示管理员停止服务后离线处理 |
| 清理失败 | 只报告清理失败，不影响已 committed 的数据；至少保留当前恢复点 |

任何失败日志都必须可由 operation_id 关联，但不得包含正文、号码、secret、密码或完整 provider 错误响应。恢复前后都要保留 config revision、数据库 schema 和归档摘要，便于审计。

### 13.2 磁盘空间预检

在上传写入、snapshot、解压和最终归档四个阶段分别检查 statvfs。所需可用空间至少按以下保守值估算：

    required_free =
      max(2 * archive_uncompressed_limit,
          current_db + current_wal + current_shm + current_config)
      + 20% headroom
      + 64 MiB

实际值应取 manifest 声明值、HTTP 上限和当前文件大小的最大值；压缩比异常时立即拒绝。空间不足必须在第三步之前报告，不能先删除旧备份来“腾空间”后再尝试；只有用户明确执行清理且仍保留最后一个有效恢复点时才允许清理。

### 13.3 权限和路径

- 默认 backup root 建议为 /var/lib/sms-relayed/backups，目录 0700、服务用户拥有；config、database、snapshot 和归档文件 0600。
- 创建目录、临时文件和目标文件时使用 create_new、O_NOFOLLOW、CLOEXEC，拒绝 symlink、硬链接替换和跨目录 rename。
- 用户提供的 local_dir 必须 canonicalize 到允许根目录内，不能等于 config、database、运行时 socket 或系统二进制目录。
- 文件写完后 fsync 文件和父目录；崩溃遗留的 .partial 只能在没有 active journal 且超过 TTL 后清理。
- CLI 若无权限必须报告需要的用户/目录权限；不能自动 chmod 用户已有的任意目录，也不能以 root 将归档写到宽泛路径。

## 14. 分阶段交付

### Phase 0：协议和协调基础

- 固化 manifest、组件 schema、分类、错误码、preview_token 和 operation 状态机。
- 实现 BackupCoordinator、operation lock、maintenance 状态和敏感日志约束。
- 为 config 与 SQLite 建立一致性读取、空间预检、权限检查和 staging 目录规范。

### Phase 1：可移植备份与只读预览

- 交付 slim/full portable ZIP、config.public、messages、forwarding.state、manifest、SHA-256、verify 和本地库。
- 交付 API export/preview、CLI create/preview/verify、前端第一、二步。
- 完成正文 metadata-only、redaction、WAL 一致性读取和自动清理。

### Phase 2：逻辑恢复与 pre-restore snapshot

- 交付 merge/replace、typed import、SQLite 事务、配置安全写入、派生数据重建和完整 pre-restore snapshot。
- 交付 quiesce/restart/health-check、失败自动回滚、snapshot 列表和 rollback API/CLI。
- 前端交付第三步真实进度、session 失效和登录跳转。

### Phase 3：投递、凭据和设备绑定

- 交付 forwarding.attempts、idempotency、quarantine/resume 策略。
- 交付 secrets.enc、personal.enc、passphrase 流程、凭据重置和 profile needs-credentials。
- 明确 same_device snapshot 与 portable restore 的 device.meta/eSIM 行为。

### Phase 4：运营和增强

- 交付计划备份、容量保留策略、自动化任务、可选签名、审计查询和监控指标。
- 评估 WebDAV/对象存储；在远端存储前增加传输加密、端点认证和远端原子提交。
- 评估跨版本离线恢复工具，但不把版本回滚与数据回滚混为一体。

## 15. 验收标准

功能完成至少满足：

1. 可以生成 slim、full 和 same-device snapshot；manifest 的格式、组件、schema、记录数、字节数和 SHA-256 可独立验证。
2. 任意不完整、篡改、越界、重复、超限或版本不兼容的归档都在写入 live 状态前失败。
3. 三步恢复中，第一步无副作用，第二步能看到组件和警告，第三步创建完整 pre-restore snapshot 并报告真实 operation 状态。
4. merge/replace 对消息、投递状态、attempt、UI 状态和 idempotency 的语义符合本规格；派生 summary、索引和触发器由当前 migrations/重建生成。
5. 恢复期间没有新的 worker claim、modem send、消息删除或配置并发写入；恢复后服务能重启、健康检查通过并恢复正常 worker。
6. 数据库事务失败、配置写入失败、重启失败和 post-check 失败均能保持 live 状态不变或自动完整回滚，并保留可用 snapshot。
7. portable 归档不含 session、.auth-key、auth credential state、普通 secret、Matching ID 或设备 namespace；正文和号码遵守明确的加密/确认策略。
8. 任何 API、CLI、前端和日志输出都不泄漏密码、token、secret、完整电话号码、短信正文或 provider response。
9. 默认权限、symlink 防护、fsync、空间预检和清理保护满足本规格，崩溃不会产生可被误识别为完整备份的文件。
10. 现有旧数据库经 migrations 后仍能备份、预览和恢复；自更新成功后，备份/恢复不改变二进制替换和重启语义。

## 16. 测试要求

### 16.1 单元和属性测试

- manifest canonical serialization、组件排序、记录数、字节数、SHA-256、签名和 schema 版本兼容。
- ZIP 路径穿越、绝对路径、重复条目、symlink、目录伪装、截断文件、超大字段、压缩炸弹和未知组件。
- config/public redaction、secret envelope 加解密、AEAD 篡改、密码错误、passphrase 不进入日志。
- typed record 的枚举、时间、长度、外键、状态迁移和冲突检测。
- 不同 schema fixture 的兼容迁移、未知 optional component warning 和未知 required component fatal。

### 16.2 SQLite 与一致性集成测试

- WAL 写入期间导出，验证主库、WAL、SHM 的一致性和独立只读连接不阻塞业务 writer。
- 并发 inbound、outbound、delivery claim、retention 与 export/restore 的锁竞争。
- merge 去重、replace、外键、触发器、conversation summary 重建、favorite/pin/read 状态和 idempotency 冲突。
- pending、retry_wait、in_flight、succeeded、permanent_failed、expired lease 以及 sending/uncertain/unknown 的恢复策略。
- 现有 migrations 从旧 schema 到当前 schema 的备份和恢复；database path、modem path 和设备 meta 不被 portable 归档误改。

### 16.3 崩溃、回滚和服务协调测试

- 在 validating、snapshotting、quiescing、数据库 commit、配置 rename、verify、restart 各阶段注入进程崩溃或断电。
- 验证重启后 journal 能识别未完成操作，并选择提交、回滚或 recovery-required；不能把 partial 文件当作有效备份。
- 验证 worker 不会在 maintenance 中 claim/send，lease 和 quarantine 状态符合策略，恢复后 resume 必须显式开启。
- 验证 pre-restore snapshot 能完整恢复配置、数据库、派生数据和服务健康状态；回滚后 session 仍全部失效。
- 验证 systemd/procd 重启成功、失败和二进制已更新但服务未重启的组合不会导致错误的“恢复成功”。

### 16.4 安全、CLI、API 和前端验收

- 文件/目录权限、父目录 fsync、nofollow、硬链接替换、非服务用户访问和自定义目录越界测试。
- API preview/apply token 过期、并发操作、权限不足、断点上传、大小上限、正确 HTTP 错误码和敏感字段不回显。
- CLI 非交互确认、stdin/passphrase file、--yes、服务运行/停止、磁盘不足和命令行进程列表检查。
- 前端三步流程、组件选择、warning 阻断、真实进度、replace/secret/resume 二次确认、snapshot rollback、session 失效和登录跳转。
- 端到端验证：生成归档、复制到另一临时实例、预览、恢复消息/历史、重新填写渠道 secret、启动 worker，并检查不发送不应发送的短信或通知。

## 17. 实现前必须固化的决策

以下仍是 SmsRelayed 重建建议，不是 SimAdmin 已确认行为：

- `.srb` 是否就是 ZIP v1、canonical JSON/TOML/NDJSON 的字节规则、未知 optional component 的保留/忽略规则，以及 signature 是否在 v1 强制。
- portable personal 数据是“下载时强制加密”还是允许二次确认后的明文；Argon2id 参数、passphrase 最短策略和 key rotation 流程。
- archive/upload/expanded/单组件/单记录/字段/压缩比的具体硬上限。不能只沿用参考实现的 50 MiB 压缩 body。
- snapshot 本地加密密钥来源、backup root、总容量上限、最小健康回滚点数量和管理员恢复手册。
- message 的跨实例稳定业务键。当前 SmsRelayed 内部 ID、inbound dedupe key 和归档 source ID 的冲突优先级必须用 fixture 证明。
- `forwarding.state` 的 quarantine 表达方式：独立 restore quarantine 表、delivery 新状态还是 generation barrier；不得复用一个现有可 claim 状态后仅靠内存阻止。
- config.public 的字段 allowlist、哪些部署字段只可 same-device 恢复、secret 缺失时 profile 的 `needs_credentials` 持久化位置。
- service quiesce deadline、不可 drain outbound 的 unknown 映射、重启/健康检查预算及 `recovery_required` 的离线修复命令。
- preview/operation token TTL、幂等 key 保留时间、归档下载 TTL，以及 restore 后 polling 如何跨 session invalidation 延续。

未决项必须返回明确的 unsupported/validation error，不能以默认 merge、默认恢复 queue 或明文导出代替产品决策。

## 18. SimAdmin 来源索引

以下均为 `/tmp/SimAdmin` 相对路径，行号对应本文锁定提交：

| 事实 | 来源 |
| --- | --- |
| format/app/path/50 MiB/默认组件、组件路径、敏感标记 | `backend/src/backup.rs:31-151`，符号 `BackupComponent` |
| kind/import mode、manifest/options/preview/apply/local-file response schema | `backend/src/backup.rs:155-301` |
| config/auth 实际导出边界 | `backend/src/backup.rs:303-347,976-1127`，符号 `BackupBaseConfig/AuthBackup/export_component` |
| 真实 backup routes 与 upload body limit | `backend/src/main.rs:898-950` |
| HTTP handlers、raw body + query apply、固定 pre-restore 集合 | `backend/src/backup.rs:360-475,552-714` |
| config 规范化、full 判定、filename、内存 ZIP 构建 | `backend/src/backup.rs:771-970` |
| ZIP parse、checksum/count 校验、preview warning | `backend/src/backup.rs:1202-1334` |
| DB transaction 后配置保存、仅 auth 清 session | `backend/src/backup.rs:1336-1502`，符号 `apply_backup` |
| merge/replace 去重与 queue/cache 导入 | `backend/src/backup.rs:1514-1850` |
| local 任意路径、`fs::write`、mtime cleanup、普通/pre 文件解析与下载/删除解析 | `backend/src/backup.rs:2017-2258` |
| backup 配置默认值和未被独立 scheduler 使用的 schedule 字段 | `backend/src/config.rs:1505-1647`；全库消费者见 `backend/src/automation/tasks/backup_data.rs:23-43` |
| automation backup action 直接写 local archive | `backend/src/backup.rs:2039-2057`；`backend/src/automation/tasks/backup_data.rs:23-43` |
| 前端 restore/backup 模拟日志与定时进度、auth 确认、三块文件库 | `frontend/src/pages/BackupRestore.tsx:477-1058,1523-1710,1770-1925` |
| 前端实际 API body/query | `frontend/src/api/current.ts:1054-1142`；类型见 `frontend/src/api/contracts.ts:1118-1224` |
