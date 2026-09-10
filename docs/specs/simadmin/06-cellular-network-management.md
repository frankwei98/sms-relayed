# 功能 6：蜂窝网络管理

状态：仅规格，不包含实现。

本文定义 SmsRelayed 对蜂窝网络管理的产品边界、适配边界和分阶段交付方案。结论先行：SmsRelayed 是短信 relay，不是 CPE/热点控制器；应先补齐可验证的只读诊断和用户意图控制，再谨慎增加驻网、APN 与射频高级操作。SimAdmin 的全部网络/CPE 功能不应无条件迁移。

## 0. 证据边界与阅读约定

本文核对的 SimAdmin 基线是 `/tmp/SimAdmin` 的 clean `main@eb6f497ad332e59f1f3899acc3b2b9427661fe5c`。

- “SimAdmin 已确认”是该提交的真实兼容行为，包含其同步长请求、HTTP 200 error envelope、进程内状态和潜在 secret/path 暴露；它不是 SmsRelayed 的推荐安全边界。
- “重建契约”“必须/禁止”是 SmsRelayed 的目标语义；“建议/推荐”是实现前仍需固化的选择。
- 本文明确不提供 SimAdmin endpoint 的原路径兼容。若确需兼容旧 UI，应在受保护 adapter 中翻译为 `/api/cellular` operation，不允许旧 handler 直接控制 modem。

## 1. 目标

### 1.1 产品目标

- 在不破坏收发短信的前提下，提供当前 modem、SIM、驻网、运营商、RAT、信号和可用小区信息。
- 让用户明确控制“是否允许蜂窝数据”和“是否允许漫游数据”，并保证后台恢复逻辑不能违背这些明确意图。
- 在硬件和 ModemManager 能力已确认时，提供可观察、可超时、可回滚的运营商扫描/注册与 APN 应用。
- 为射频模式、频段选择和真实小区锁定定义严格的 capability gate；能力不存在时返回不支持或未知，不伪造成功。
- 将 ModemManager、D-Bus、NetworkManager、QMI 和 AT 的职责分层，使 vendor-specific 能力不会污染核心短信路径。

### 1.2 非目标

- 不把 SmsRelayed 变成完整 CPE：不迁移 WLAN 配置/扫描、DDNS、路由、防火墙、热点、eSIM/LPA、通话、OTA、设备自动化、系统重启等 SimAdmin 功能。
- 不因“蜂窝数据关闭”而关闭整个 modem 或影响 SMS/IMS 注册；数据面和短信/控制面必须分离。
- 不提供任意 `mmcli`、`qmicli`、AT 命令执行接口，不把 modem path、设备节点或 shell 参数直接暴露给 HTTP 请求。
- 不仅凭 UI 或进程内状态声称“已锁定小区”；没有真实底层写入和重启后验证时，cell lock 必须标记为不支持。
- 不在本功能中修改、清空或接管宿主机 iptables/ip6tables、默认路由或 NetworkManager 之外的主机网络策略。
- 不默认启用自动驻网、自动数据连接、自动漫游、频段锁定、小区锁定或 watchdog 射频循环。

### 1.3 术语

| 术语 | 本规格中的唯一含义 |
| --- | --- |
| modem | ModemManager 枚举且通过现有 fingerprint/path 校验的设备实例 |
| radio | modem 的无线电启用状态；不等同于蜂窝数据 bearer |
| registration | 3GPP home/roaming/searching 等驻网观测值；不等同于数据连通 |
| bearer/data plane | 提供 IP 数据连接的 MM/NM 对象和接口；SMS 可在数据关闭时继续工作 |
| desired | 用户/配置要求系统维持的策略值 |
| observed | 从 MM/NM/QMI/系统读回、带采样时间的事实 |
| enforcement | desired 是否已应用：`satisfied|applying|blocked|failed|unknown` |
| capability | `supported|unsupported|unknown|in_memory_only`，并带 adapter/source；不能由 UI 推断 |
| operation | 长写操作的持久资源，拥有 ID、generation、状态、deadline 和审计 |
| generation | desired/operation 的单调版本，防止晚到结果覆盖更新意图 |
| read-back | 写操作后从独立的 observed source 再读取；调用返回 `Ok` 本身不算 read-back |
| airplane intent | 用户要求关闭 radio 的持久负向意图；比 watchdog 恢复优先 |
| APN configured/applied/verified | 分别表示保存的目标、live profile/bearer 值、实际数据连接验证，不可合并为一个 bool |

## 2. SimAdmin 的实际 API 与能力基线

以下是对 `/tmp/SimAdmin` 实际代码、Bruno 请求和前端的归纳，不代表 SmsRelayed 的承诺 API。

### 2.1 实际 HTTP API

| API | 实际能力 | 重要行为/限制 |
| --- | --- | --- |
| `GET /api/network` | 当前注册状态、运营商、技术、信号 | 主要是 ModemManager 标量属性映射；包含注册状态和 RAT |
| `GET /api/cells` | serving/neighbour 小区 | 优先 QMI NAS，再尝试 ModemManager `GetCellInfo`，最后用 `mmcli` location/signal；不同 modem 输出差异很大 |
| `POST /api/cell-monitor/start`, `/stop` | 开关小区/信号监测 | 通过 `mmcli --location-enable-3gpp` 和 `--signal-setup=5`；用进程内 atomic 防重复，不能保证硬件长期提供数据 |
| `GET /api/network/signal-strength` | 0–100 信号质量 | 不是 RSRP/RSRQ/SINR；数值可能来自 ModemManager `SignalQuality` |
| `GET /api/location/cell-info` | serving/neighbour 的位置/小区摘要 | 从小区信息和 MCC/MNC 组合，字段随 modem 能力而变 |
| `GET /api/network/interfaces`、`/connection-addresses` | 主机接口、地址和流量信息 | 偏 CPE/主机网络观察，不等同于 modem 注册状态 |
| `GET /api/data`、`POST /api/data` | 通过 NetworkManager 开关数据连接 | 维护 `data_user_disabled` 和持久化 `data_enabled`；会同步 NM autoconnect；不会清理宿主机 iptables/ip6tables |
| `GET /api/roaming`、`POST /api/roaming` | 漫游数据策略 | 持久化 `roaming_allowed`；活动数据连接会按 APN 断开/重连；不等同于强制取消网络注册 |
| `GET /api/airplane-mode`、`POST /api/airplane-mode` | modem enable/disable 形式的飞行模式 | `enabled` 实际映射为 modem disabled；有过渡态重试和最长约 45 秒等待 |
| `GET /api/network/operators` | 当前/已缓存运营商 | 读取 `AvailableNetworks` 等 ModemManager 数据 |
| `GET /api/network/operators/scan` | 运营商扫描 | ModemManager `Scan` 请求约 45 秒，再约每 2 秒轮询 `AvailableNetworks`，总计约 20 秒缓存轮询；前端提示可能需要约 2 分钟 |
| `POST /api/network/register-manual` | 按 MCCMNC 手动注册 | 请求如 `{"mccmnc":"46001"}`；`Register` 约 45 秒，错误时可能执行 radio recovery |
| `POST /api/network/register-auto` | 自动注册 | 使用空 MCCMNC 调 `Register`；不会给调用方一个独立的长任务资源 |
| `GET /api/apn`、`POST /api/apn` | APN 配置和 bearer/APN context | 配置保存与活动 bearer 更新混在同一 handler；保存成功但 bearer 更新失败时仍可能返回 success 并附带 `bearer_update_error` |
| `GET /api/radio-mode`、`POST /api/radio-mode` | `auto`、`lte`、`nr` | 读取 `SupportedModes`/`CurrentModes`，映射后调用 `SetCurrentModes`；handler 不显式重新注册、read-back 或回滚，底层 modem 可能自行掉线/重注册 |
| `GET /api/band-lock`、`POST /api/band-lock` | LTE/NR 频段选择 | 读取 `SupportedBands`/`CurrentBands`，调用 `SetCurrentBands`；空数组表示使用全部支持频段；不能保证每个 modem 的物理 band 映射一致 |
| `GET /api/cell-lock`、`POST /api/cell-lock`、`POST /api/cell-lock/unlock-all` | 显示/修改 LTE 或 NR 的 ARFCN/PCI | **仅修改 `CellLockStore` 的内存 tuple，不调用 ModemManager、QMI 或 AT；重启进程即丢失，也不改变 modem** |

SimAdmin 的统一 response 大多是 HTTP 200 加 `{status,message,data}` 或 `{status:error}`。这适合其现有 CPE UI，但不适合直接作为 SmsRelayed 新控制 API 的错误契约：SmsRelayed 应使用语义化 HTTP 状态码，并区分“不支持”“正在执行”“执行失败”和“结果不确定”。

#### 2.1.1 SimAdmin 已确认的请求/响应字段

下表补充旧客户端真正依赖的 JSON。所有路由位于认证 middleware 后；表中的业务错误通常仍是 HTTP 200，调用方必须检查 envelope `status`。

| 方法与路径 | request | success `data` |
| --- | --- | --- |
| `GET /api/network` | 无 | `{operator_name,registration_status,technology_preference,signal_strength,mcc?,mnc?}` |
| `GET /api/cells` | 无 | `{serving_cell:{tech,cell_id,tac},cells:[...]}`；字段缺失多以空串/0 表达 |
| `POST /api/cell-monitor/start|stop` | 空 JSON 可选 | `{}`；重复 start/stop 也 success |
| `GET /api/radio-mode` | 无 | `{mode,technology_preference,supported_modes}` |
| `POST /api/radio-mode` | `{"mode":"auto|lte|nr"}` | `{}`；只表示 `SetCurrentModes` 返回成功 |
| `GET /api/band-lock` | 无 | `{locked,supported_lte_fdd_bands,supported_lte_tdd_bands,supported_nr_fdd_bands,supported_nr_tdd_bands,lte_fdd_bands,lte_tdd_bands,nr_fdd_bands,nr_tdd_bands}` |
| `POST /api/band-lock` | 上述四个 current 数组 | `{}`；四数组全空会把 current 设置为 supported 全集 |
| `GET /api/network/operators/scan` | 无；注意是 GET，不是 POST | `{operators:[{path,name,status,mcc,mnc,technologies}]}` |
| `POST /api/network/register-manual` | `{"mccmnc":"46001"}` | `{}`，message 为 Registration started |
| `POST /api/network/register-auto` | `{}` | `{}` |
| `GET /api/apn` | 无 | `{contexts:[{path,name,active,apn,protocol,username,password,auth_method,context_type}]}`；**password 会原样返回** |
| `POST /api/apn` | `{context_path,apn?,protocol?,username?,password?,auth_method?}` | `{}` 或 `{bearer_update_error}`；后者仍是 success envelope |
| `GET/POST /api/data` | POST `{"active":bool}` | `{active}` |
| `GET/POST /api/roaming` | POST `{"allowed":bool}` | `{roaming_allowed,is_roaming}` |
| `GET/POST /api/airplane-mode` | POST `{"enabled":bool}` | `{enabled,powered,online}` |
| `GET /api/cell-lock` | 无 | `{any_locked,rat_status:[{rat,rat_name,enabled,lock_type,pci?,arfcn?}]}` |
| `POST /api/cell-lock` | `{rat:12|16,enable,lock_type?,pci?,arfcn?}` | `{success:true}`，仅改内存 |
| `POST /api/cell-lock/unlock-all` | `{}` | `{success:true}`，仅清内存 |

参考实现没有 operation ID、generation、If-Match、确认字段或统一 mutation conflict 响应。scan 在普通 GET 中最长可等待约 65 秒；register 在普通 POST 中等待最多 45 秒或更久的 recovery。SmsRelayed 推荐的 `POST + 202 operation` 是有意不兼容的新契约。

### 2.2 实际后端适配

- ModemManager D-Bus 是主要控制面：`Modem3gpp.Scan/Register`、`Modem.Enable/Disable`、`Modem.SetCurrentModes/SetCurrentBands` 以及相关属性读取。
- 数据连接由 NetworkManager profile 和 `nmcli` 管理。APN 会写入 GSM profile；漫游禁止时使用 home-only 等策略。
- 小区诊断会优先尝试 QMI NAS，再回退到 ModemManager/`mmcli`；这只是“读到什么就展示什么”，不保证所有 RAT 或邻区字段都有值。
- `mmcli` 同时被用作发现、监测启停和 fallback/recovery。长时间 Scan/Register、modem 重建和设备重枚举均可能超时或改变 object path。
- watchdog 周期约 15 秒，会检查 modem、注册、数据和 host firewall 规则；在允许恢复时可能断开/重连、循环 radio、扫描 modem 或重启 ModemManager。

#### 2.2.1 SimAdmin 已确认的默认值、写入顺序和并发

| 项 | 已确认行为 |
| --- | --- |
| config defaults | `data_enabled=false`、`roaming_allowed=true`；APN 空、protocol=`dual`、username/password 空、auth=`chap` |
| startup | 从 `data_enabled` 构造进程内 `data_user_disabled`；airplane intent 总从 false 开始；无条件调用 `ensure_nm_modem_profile`；2 秒后 init data；5 秒后启动 15 秒 watchdog |
| mutation serialization | radio/band/data/register/airplane/APN bearer 等多条路径使用全局 `with_serial` mutex；不是 per-modem；operator scan 和部分 monitor/read 路径不使用同一锁 |
| data off/on | 先执行 NM live activate/deactivate，成功后保存 `data_enabled` 和内存 flag；再 detached spawn 更新 NM autoconnect，不等待结果 |
| roaming | 先保存 `roaming_allowed`；若 observed data active，则断开再重连。重连失败不会回滚刚保存的 policy |
| airplane | `enabled=true` 时在调用前先把进程内 intent 置 true，成功后 read current state；intent 不持久化。启用失败后该 true 仍可被 watchdog继续执行 |
| APN | 先合并并保存配置，再按客户端提供的 `context_path` 写 live bearer；live 失败仍返回“config saved” success 和原始 `bearer_update_error` |
| radio/band | method 返回即 success；没有 handler 级 read-back、rollback、generation 或 registration verification |

`GET /api/data` 在内存 user-disabled 为 true 时直接返回 `active=false`，不读实际 NM/MM；否则以 modem state `>= connected` 推断 active。保存 data policy 失败发生在 live 操作之后，可造成 live 已改变、配置未改变。APN GET 会从 bearer/config 回填并返回 password；APN POST 的 object path 来自客户端，后端直接构造 D-Bus proxy，未绑定到已验证 modem。这些都是重建时必须消除的安全/一致性缺口。

operator scan 会吞掉 `Scan` 的 D-Bus error/45 秒 timeout，随后轮询 `AvailableNetworks` 约 20 秒，最后可能把旧 cached/current 列表作为 success 返回。manual/auto register 未在 handler 校验 MCCMNC；特定 QMI internal error 会自动 disable/enable radio 并等待 searching/ready，recovery 成功可使 HTTP 返回 success，但不证明已注册到请求的 manual network。

cell monitor 使用 `mmcli -m any --location-enable-3gpp`、`--signal-setup=5` 和对应 stop 命令；进程内 atomic 只防同进程重复按钮。start 的第一步成功、第二步失败时 location 可能仍启用；进程重启会丢失 active 标记。

### 2.3 硬件和 ModemManager 限制

ModemManager 只暴露 modem 固件和插件能够表达的能力。以下情况均属正常失败/未知，不应通过猜测补齐：

- modem 未暴露 `Modem3gpp`、`SupportedModes`、`SupportedBands`、`CurrentBands`、`GetCellInfo` 或对应的 writable method。
- 同一型号在不同固件、USB composition、MBIM/QMI/串口模式下的字段和 writable capability 不同；5G NR band、NSA/SA 和 LTE fallback 也不能从 RAT 名称推断。
- `SetCurrentModes`/`SetCurrentBands` 可能接受调用但需要重新注册，甚至导致 modem 长时间无服务；当前网络支持不代表该 band 组合可用。
- 运营商扫描、手动注册和启停 radio 都可能耗时几十秒，期间短信 D-Bus 对象、bearer、QMI port 或 modem object path 可能暂时消失。
- 小区锁定通常是 vendor/QMI/AT 专有能力，ModemManager 通用接口并不保证存在；即便 modem 支持，锁定可能是一次性固件状态、重启丢失或与 band/radio 配置相互覆盖。
- APN 的真实生效点可能是 NetworkManager profile、ModemManager bearer 或厂商拨号器；只改其中一层不能声称数据连接已经采用新 APN。

### 2.4 已知的“仅内存态” cell lock 限制

SimAdmin 的 `CellLockStore` 只有 `lte: Option<(arfcn,pci)>` 和 `nr: Option<(arfcn,pci)>`，由 `Arc<Mutex<...>>` 持有。`/api/cell-lock` handler 只调用 store 的 `apply/status/unlock_all`，完全没有底层锁网调用；Network 页面仍然把它展示为 cell lock 按钮。这是 UI/状态语义错误，而不是可移植的实现参考。

SmsRelayed 必须遵守以下边界：

1. 不复制这个 store 作为“锁定功能”。
2. 如果尚无真实 adapter，API 只能返回 `capability: "unsupported"` 或 `501 Not Implemented`，明确说明“未向 modem 写入”。
3. 若未来增加真实锁定，必须记录 adapter、RAT、ARFCN、PCI、持久性和重启后 read-back 结果；仅内存的 desired state 只能叫“待应用配置”，不能叫“已锁定”。

### 2.5 已确认的参考前端行为与误导风险

以下只是兼容性事实，不是目标 UX：

- Network 页进入小区页签时调用 monitor start，按用户选择的刷新间隔轮询；切换页签/卸载时 fire-and-forget stop。两个页面实例或快速切换会互相停止共享 monitor，后端也没有订阅者引用计数。
- operator scan 由浏览器发起一个同步 `GET`，按钮一直等待该请求；manual/auto register 成功后延迟 3 秒刷新，而不是轮询注册 operation。
- radio mode POST 返回后，前端立即把选择值写成本地 current mode，并在 3 秒后刷新 band；没有以服务器 read-back 覆盖成功文案。
- band 选择在成功后写入浏览器 `localStorage`，空数组被当作“取消限制”；本地选择不是设备 desired/observed truth，也不会在另一浏览器同步。
- cell lock handler 只改内存，但前端显示“已锁定/已解除”并在 2 秒后刷新，造成硬件已生效的错觉。
- APN GET 返回的 username/password 被直接填入表单；保存时又把 password 作为普通 JSON 字段发回。返回成功后页面统一显示“已保存”，不会把 `bearer_update_error` 解释为 saved-not-applied。

重建可以复用“页签按需加载、后台暂态错误不打扰用户”的交互意图，但不得复用上述 secret 回显、乐观成功、localStorage truth 或同步长请求模式。

## 3. SmsRelayed 当前状态与缺口

### 3.1 当前已有能力

- `src/modem.rs` 的 `ModemService` 通过 `mmcli` 查询 modem JSON，旧版不支持 JSON 时回退到文本；现有 `ModemStatus` 包含 enabled、state、SIM state、号码、运营商名称、信号质量和 access technologies。
- 现有动作只有 enable、disable、reset。动作使用 try-lock 串行化；reset 按 session token 限流；执行前要求已验证的 modem path，并检测 path drift/设备 fingerprint，避免对错误的重枚举 modem 执行危险动作。
- `src/dbus/*` 是短信收发路径：使用 ModemManager SMS/Messaging D-Bus、信号订阅、连接缓存和 5–30 秒级超时；没有 Modem/Modem3gpp/NetworkManager 的通用网络控制抽象。
- `src/modem/ims/*` 的 native QMI 只读探测 IMS/NAS 能力，选择唯一或 primary QMI port，探测总期限约 5 秒；探针会拒绝歧义端口、权限失败和 proxy 不可用，不执行通用 NAS 写操作。
- API 路由只有受 session 保护的 `/api/modem/status`、`/enable`、`/disable`、`/reset` 等；受保护路由还包括短信、配置、转发和 service 控制。
- TOML 配置目前有 `[app]` 的 `modem_path`、API session/password、SMS/转发等配置，没有蜂窝数据、漫游、APN、注册、radio、band 或 cell lock 配置。
- 前端 Status 页的 Modem panel 展示健康、SIM、运营商、信号、access technology、短信能力，并提供启停/确认 reset；没有 SimAdmin 那种 Network/CPE 页面。现有测试覆盖 mmcli JSON/text、disabled/SIM missing、path drift、QMI port selection、IMS probe、D-Bus SMS owner change/超时等。

### 3.2 缺口与风险

| 能力 | 当前状态 | 不能直接假设 |
| --- | --- | --- |
| 注册 | 只有 modem state/access technology 的粗粒度信息 | `state=registered` 不是 MCC/MNC、home/roaming 或可用数据的完整证明 |
| 运营商 | 只有 operator name 的诊断字段 | 没有 scan、manual register、auto register job |
| 信号/小区 | 只有 `signal_quality` 百分比；IMS QMI 不是 cell API | 没有 RSRP/RSRQ/SINR、PCI、ARFCN、邻区或 serving-cell 模型 |
| 数据 | 无 NetworkManager/ModemManager bearer 控制 | 不能提供 data switch 或 watchdog reconnect |
| 漫游 | 无策略模型 | 不能从 operator name 或注册状态推导允许漫游 |
| APN | 无 APN 配置/验证 | 不应把 APN credential 塞进现有 modem status |
| 飞行模式/radio | modem enable/disable 与 radio mode 尚未抽象 | 不能把 disable 直接当作可恢复的 airplane policy |
| band lock | 无 `SupportedBands`/`SetCurrentBands` 适配 | 不应按 band number 猜测 ModemManager ID |
| cell lock | 无底层接口 | 不能复制 SimAdmin 的内存态假实现 |
| watchdog | 没有蜂窝恢复循环 | 新 watchdog 若越权，会破坏用户数据/漫游意图和短信稳定性 |

## 4. 推荐的能力分层

能力必须按“可观察性”和“破坏半径”分层；每一层都要先完成 capability discovery 和 read-back，再允许下一层。

### L0：只读诊断（默认、优先交付）

建议提供统一 snapshot，至少包括：

- modem identity 的非敏感摘要、已验证 object path、ModemManager/plugin/tool capability；
- SIM ready/locked/missing、modem state、radio enabled/disabled；
- registration state、MCC/MNC、operator name/code、home/roaming、RAT/access technology；
- `SignalQuality` 以及可用时的 RSRP/RSRQ/SINR、serving cell、neighbour cells、PCI、ARFCN、band、TAC/CID；缺失字段返回 `null` 与 reason；
- data bearer/profile 的状态、接口和 IP 摘要，但不返回 APN 密码或完整 IMSI/ICCID/号码；
- APN 的有效来源和脱敏值；`configured`、`applied`、`verified` 必须分开；
- capability flags：`supported`、`unsupported`、`unknown`、`in_memory_only`，以及 adapter/source 和采样时间。

小区监测应是按需、只读、有限频率的操作；默认不常驻启用 QMI/AT polling，避免与 SMS/IMS QMI port 争用。

### L1：低风险用户策略开关

低风险层只包含与短信 relay 直接相关的策略：

- **蜂窝数据允许/禁止**：禁止数据必须断开当前数据连接、关闭相关 NM autoconnect，并持久化为最高优先级的用户意图；不能关闭 radio，也不能阻止短信接收/发送尝试。
- **允许/禁止漫游数据**：禁止漫游时阻止/断开 roaming bearer，但保持 modem 注册能力；不能用“当前不是 roaming”替代策略值。
- **飞行模式**是用户可操作但高影响的显式开关，不应称为低风险自动控制。它可以关闭 modem radio，但必须确认、展示长任务状态，并禁止 watchdog 自动重新启用。

L1 不包括后台自动切换 radio、band、cell 或自动替用户打开数据/漫游。

### L2：网络注册

- 先提供当前/缓存运营商列表的只读查询，再提供显式触发的 scan。
- 手动注册要求合法 MCCMNC、二次确认、长任务 operation id 和完成后的 registration read-back；自动注册是显式用户操作，不是默认后台策略。
- scan/register 期间标记网络控制面为 busy，不能并发执行 airplane、radio、band、APN apply 或另一注册操作。
- 如果 operator scan/register 可能影响短信可达性，UI 必须先说明；失败不应自动扩大为 baseband reset，除非另有用户确认和独立 recovery policy。

### L3：APN

- APN 是持久化 desired config、NetworkManager profile/bearer 的 live config 和实际已连接验证结果三者的组合。
- 先支持读取/校验/保存和显式 apply；对活动 bearer 使用新 APN 前应捕获旧 profile，应用后验证 bearer 的 APN、接口和连接状态。
- 用户名、密码、认证方式、IP protocol 必须有 schema 校验；密码只接受写入/保留语义，不在 GET、错误、事件、日志或前端回显。
- APN 应用失败不能返回“已成功应用”；最多返回 `saved_not_applied`，并提供 operation status 与旧配置/恢复结果。
- 不为短信 relay 默认创建数据连接；是否管理数据连接必须由显式 feature gate 和用户策略决定。

### L4：射频、频段和小区锁定

- **radio mode**（auto/LTE/NR 等）属于高影响操作，只能使用 modem 报告的 supported/current modes；切换后必须等待并验证注册，失败时尽力恢复此前 mode。
- **band lock** 只有在 `SupportedBands`、`CurrentBands` 和 `SetCurrentBands` 全部可用且物理 band 映射经过 adapter 验证时才开放。空集合的“解锁全部”必须被明确定义并 read-back，不能把“用户没有选择”误当作安全默认。
- 选择频段时至少保留用户明确允许的 fallback 或要求用户确认“可能完全失去服务”；核心产品不提供自动 band optimizer。
- **cell lock** 默认不提供写操作。只有完成真实 QMI/vendor/AT adapter、读回、重启持久性和解锁路径验证后才可进入实验性/高级功能；否则只展示发现到的 serving cell。

## 5. 适配边界：ModemManager、D-Bus、NetworkManager、QMI、mmcli、AT

### 5.1 ModemManager D-Bus：首选抽象

生产控制应优先经 D-Bus adapter，按 object path 和接口 capability 调用，而不是拼接命令行：

- `org.freedesktop.ModemManager1.Modem`：状态、启用/禁用、signal/access technology、`SupportedModes`/`CurrentModes`；
- `Modem3gpp`：registration、operator、`AvailableNetworks`、scan/register；
- `Modem.Simple`/Bearer：只在明确确认 ModemManager bearer 是本项目数据路径时使用；
- `SupportedBands`/`CurrentBands`/`SetCurrentBands`：仅在三者和 modem plugin 语义一致时使用；
- `GetCellInfo` 等只读接口：作为 cell source 之一，不承诺所有字段。

每个 adapter 应在启动和 modem 重枚举后重新发现 capability，缓存的 unsupported 不能永久阻止后续 modem；对象路径必须由现有 path verification/fingerprint 机制保护。

### 5.2 NetworkManager：数据面边界

NetworkManager 负责主机连接 profile、autoconnect、bearer activation、接口和地址。蜂窝控制层只通过受控 adapter 改变目标 GSM profile，不直接编辑任意连接、不修改路由/防火墙。没有 NetworkManager 或无可验证 GSM profile 时，数据/APN 写操作返回 unsupported/unknown；不能因为 modem 已注册就声称“数据已连接”。

### 5.3 `mmcli`：诊断和兼容 fallback

`mmcli` 可以用于 modem discovery、健康诊断、版本兼容 fallback 或明确隔离的 recovery，但不是新控制 API 的默认实现层。所有调用都必须：

- 使用已验证的 modem id/path；
- 固定参数 allowlist、独立超时和输出解析；
- 不记录完整命令、手机号、IMSI、APN 密码、SMS body 或原始 QMI/AT payload；
- 对长操作返回 operation 状态，而不是占住普通 HTTP 请求；
- 与 D-Bus adapter 共享同一 per-modem mutation lock。

### 5.4 QMI/qmicli：能力专用 adapter

当前 SmsRelayed 的 native QMI 是 IMS/NAS 只读探针，使用 QMI proxy、5 秒 deadline、primary/唯一 QMI port 选择，并明确拒绝歧义 port。它不能直接升级为通用网络写入层。未来若需要 QMI cell/band/lock：

- 必须另建 capability-specific adapter 和 modem/firmware compatibility matrix；
- 与 IMS probe 隔离 client、锁和错误码，避免 QMI CID/port 争用影响短信；
- qmicli 仅作为可选外部 fallback，不能成为所有发行版的硬依赖；
- 不能因为某个 QMI NAS 查询成功就推断 band/cell lock 写能力存在。

### 5.5 AT：最后的 vendor adapter

AT 只允许在明确型号/固件/串口映射的 adapter 内部使用。命令必须静态 allowlist、参数范围校验、串口互斥、响应 parser 和超时；禁止 HTTP 传入原始 AT。AT 失败或响应不确定时，状态标为 uncertain，不继续发送破坏性命令。没有可维护的兼容矩阵就不纳入核心阶段。

## 6. 用户意图、watchdog 与自动恢复优先级

### 6.0 SimAdmin 已确认的 watchdog 基线（不作为目标）

参考 watchdog 每 15 秒一轮：找到 modem 并成功读取 state 后，先检查进程内 airplane intent，再检查 `data_user_disabled`；二者会阻止该分支的 reconnect/register/radio-cycle。若 modem discovery 本身失败，则代码不会先检查这两个 intent，连续 3 次会执行 `mmcli --scan-modems`，连续 5 次可 `systemctl restart ModemManager`，recovery cooldown 300 秒。它只检查并记录 iptables 是否有规则，不再自动 flush。

在未阻断时，searching 连续 4 轮会 auto register，8 轮会 radio cycle；enabled-idle 8 轮或 transition 6 轮会 radio cycle；data activation retry cooldown 120 秒。`roaming_allowed=false` 会传给 NM connection policy，但本身不阻止 auto register/radio recovery。计数、cooldown、airplane intent 全在内存中，进程重启即丢失。

因此下述优先级是 SmsRelayed 的**重建契约**，不是对参考实现已经满足的声明。特别是 modem missing recovery 也必须受持久 airplane/maintenance intent 和 operation generation 约束；不能因为 object 消失就绕开用户负向意图。

用户明确的负向意图必须比可用性自动化更强。建议把“观测状态”和“desired policy”分开持有，优先级如下：

1. **飞行模式意图**：用户要求关闭 radio 时，任何 watchdog、数据连接、驻网、scan/register、band/radio recovery 都不得自动重新启用 modem。
2. **用户禁止数据**：断开数据并关闭 autoconnect；watchdog 不得 reconnect、Simple.Connect 或修改为启用。radio/注册可继续为短信服务工作。
3. **用户禁止漫游数据**：不允许 roaming bearer；若当前处于 roaming 且数据已连，断开数据；不得通过强制打开 roaming 或切换注册来绕过策略。
4. **正在执行的显式用户操作**：在不违反上述负向意图的前提下，scan/register/APN/radio/band 操作独占控制锁；完成后必须 read-back。
5. **watchdog 只读观察与有限恢复**：只在无阻断意图、无显式操作、超过去抖阈值时运行；第一阶段只记录诊断、刷新状态或提示用户。
6. **自动数据连接/自动注册**：默认关闭；只有配置和用户都明确开启时才可执行。

用户重新打开数据或允许漫游，只表示恢复许可，不表示立即 radio cycle、强制注册或立刻消费漫游流量；恢复动作应按正常冷却和显式策略执行。

任何控制 API 的返回都应同时报告 `desired`、`observed`、`enforcement` 和 `last_operation`。例如数据关闭但 NM 断开尚未完成时，状态应是 `desired=disabled, observed=disconnecting, enforcement=blocked`，而不是简单 `false`。

## 7. 状态机、超时、并发与回滚

### 7.1 分离的状态机

不要用一个字符串覆盖所有状态。建议至少分成四个正交维度：

```text
modem:    unavailable -> discovered -> disabled
                     -> enabling -> enabled/searching
                     -> registered_home | registered_roaming
                     -> disabling -> disabled

data:     unknown -> disconnected -> connecting -> connected
                                      \-> disconnecting -> disconnected

policy:   airplane_requested, data_user_disabled, roaming_allowed

operation: queued -> running -> verifying -> succeeded
                                      \-> failed | timed_out | uncertain
                                      \-> rolled_back | cancelled
```

`registered_home`/`registered_roaming` 是网络观测值，不能替代 `roaming_allowed` 策略；`connected` 是已验证数据面，不等于 modem state 高于某个数字。modem object removal、ModemManager owner change、设备重枚举统一进入 `unavailable/unknown`，并使当前 operation 进入 `uncertain`，不能猜测成功。

### 7.2 建议超时和轮询

这些是 API/测试契约的初始预算，实际 adapter 可按 capability 调整，但不得无限等待：

| 操作 | 单步/总预算建议 | 超时后的行为 |
| --- | --- | --- |
| D-Bus 属性读取 | 2–5 秒 | 返回 stale/unknown，保留采样时间和 reason |
| modem enable/disable、airplane | 单次 5 秒、总计 45 秒 | 重新读取状态；不确定时保留安全的用户意图并提示 |
| operator scan | `Scan` 45 秒，随后最多 20 秒缓存轮询 | operation `timed_out`/`uncertain`，不自动重试多次 |
| manual/auto register | `Register` 45 秒，验证注册最多再 60 秒 | 不自动 reset；用户另行确认 recovery |
| NM APN/data apply | 每个步骤 15–30 秒，总计 60 秒 | 读取旧 profile/live 状态，执行回滚或返回 uncertain |
| cell/signal snapshot | 5–10 秒 | 缺失字段为 null；不阻塞状态页 |
| operation 总期限 | 建议 120 秒硬上限 | 取消后禁止晚到的结果覆盖新 desired generation |

### 7.3 并发与取消

- 每个 modem 一个 mutation lock；airplane、data policy apply、scan/register、APN、radio、band、cell lock 互相声明冲突关系。
- 只读 snapshot 可并发，但要有短缓存/请求合并，避免前端轮询风暴；cell monitor 最多一个实例。
- 冲突请求返回 `409 operation_in_progress` 和当前 operation id，不排队执行未知数量的破坏性操作。
- 每个 desired policy 和 operation 带单调递增 generation；晚到的 D-Bus/QMI/NetworkManager结果只能更新同 generation，不能覆盖用户新选择。
- 网络控制锁不得包住短信发送的整个生命周期；SMS D-Bus owner change、对象消失和发送结果 unknown 必须沿用现有短信恢复语义。
- 取消只取消等待/后续步骤，不能假定 modem 已回到旧状态；取消后仍必须 read-back。

### 7.4 回滚与不确定结果

- APN、radio mode、band selection：先读取旧 live state 和旧持久化配置，应用后 read-back；失败时尽力恢复旧值，并分别报告 `rolled_back` 或 `rollback_failed`。
- 配置写入采用现有安全的临时文件、权限和 revision/precondition 思路；不能出现 live 已切换而旧凭据丢失的情况。
- 数据禁止和飞行模式是安全负向意图：底层断开/禁用失败时也保留阻断 intent，禁止 watchdog 趁失败间隙恢复连接；状态显示 `uncertain` 并要求用户明确重试或解除意图。
- 如果 modem 在应用中消失，不能立即写入“成功”；重连后必须用 fingerprint/object path 重新发现并验证，无法验证时提供人工恢复提示。
- band/cell 操作只要没有硬件 read-back，就不得从命令返回值推断已生效；cell lock 不支持时不产生“回滚成功”的假事件。

## 8. 权限、安全与隐私

- 所有新蜂窝 API 默认加入现有 protected router，要求有效 session；写操作要求 `application/json`、same-origin/CSRF 防护和明确确认字段。
- 继续使用已验证 modem path、path drift 检查和 identity fingerprint；绝不接受客户端任意 object path、`/dev/*`、NM connection name 或命令片段。
- scan/register、airplane、radio mode、band lock、APN apply、cell lock 等操作应有速率限制；reset、baseband recovery 等高影响动作必须独立确认且不能被 watchdog隐式触发。
- 日志、事件、错误和前端响应不得泄露 APN password、用户名（除非必要且脱敏）、IMSI、ICCID、完整号码、QMI/AT 原始 payload、完整 cell location 或 SMS body。现有 public health 的最小披露原则继续适用。
- API 错误只返回稳定 code 和可行动的摘要；底层 stderr、命令行、D-Bus object path 和 vendor 错误写入受控 debug 日志时也必须脱敏。
- 不提供任意 shell/AT 代理，不清空 host firewall，不修改与短信 relay 无关的接口/路由。
- 单用户产品当前不必引入复杂 RBAC，但应保留 operation audit：谁、何时、请求哪种 capability、结果是 succeeded/failed/uncertain；不记录 secret 内容。

## 9. API、配置与前端方案

### 9.1 推荐 API 契约

建议新增独立命名空间 `/api/cellular`，不要把 CPE 端点原样复制到 `/api/modem`。推荐形态如下：

| 资源 | 读/写 | 说明 |
| --- | --- | --- |
| `/api/cellular/status` | GET | 单次完整 snapshot：modem/registration/data/policy/capability/source/采样时间 |
| `/api/cellular/cells` | GET | serving/neighbour；按 capability 返回字段，不能保证有 cell lock |
| `/api/cellular/operators` | GET | 当前/缓存列表 |
| `/api/cellular/operators/scan` | POST | 返回 `202 {operation_id}`，不在普通请求中阻塞 45–120 秒 |
| `/api/cellular/registration` | POST | `{mode:"auto"}` 或 `{mode:"manual",mccmnc:"..."}`；显式确认、返回 operation |
| `/api/cellular/operations/{id}` | GET | `queued/running/verifying/succeeded/failed/uncertain/rolled_back` 及安全错误摘要 |
| `/api/cellular/data` | GET/PUT | desired policy 与 observed data 状态分开；PUT 要求 `{enabled,confirm?}` |
| `/api/cellular/roaming` | GET/PUT | policy 与 live registration 分开；禁止漫游不必 deregister |
| `/api/cellular/airplane` | GET/PUT | 明确高影响确认；不可由 watchdog 覆盖 |
| `/api/cellular/apn` | GET/PUT | GET 脱敏；PUT 支持保留密码和显式 apply/test operation |
| `/api/cellular/radio-mode` | GET/PUT | 只呈现 ModemManager 实际 supported modes |
| `/api/cellular/bands` | GET/PUT | 只有 capability gate 通过才允许写；提供旧值、fallback 和 read-back |
| `/api/cellular/cell-lock` | GET/PUT | capability 为 `unsupported` 时返回 501/明确状态；不提供 SimAdmin 内存态写成功 |

新 API 建议使用 `200`（读/同步完成）、`202`（已接受的长 operation）、`400/422`（输入或 capability 参数错误）、`401/403`（权限/确认）、`409`（并发冲突）、`501`（未实现/不支持）、`503`（依赖不可用）、`504`（adapter 超时）。所有长操作都必须有可轮询的 operation resource。

#### 9.1.1 重建 snapshot schema

`GET /api/cellular/status` 返回一个自洽采样；空值和不支持必须显式，不能沿用 SimAdmin 的空串/0：

```json
{
  "sampled_at": "...Z",
  "generation": 12,
  "modem": {"state":"registered_home","radio_enabled":true,"fingerprint":"short-redacted"},
  "registration": {
    "state":"home","operator":{"name":"...","mccmnc":"46001"},
    "rat":["lte"],"reason":null
  },
  "signal": {"quality_percent":72,"rsrp_dbm":null,"rsrq_db":null,"sinr_db":null,"source":"modemmanager"},
  "data": {
    "desired":"disabled","observed":"disconnected","enforcement":"satisfied",
    "profile_id":"redacted-stable-id","interface":null
  },
  "policy": {"airplane_requested":false,"roaming_allowed":false,"auto_register":false,"auto_reconnect":false},
  "capabilities": {
    "operator_scan":{"state":"supported","adapter":"modemmanager"},
    "band_write":{"state":"unknown","adapter":null},
    "cell_lock":{"state":"unsupported","adapter":null}
  },
  "last_operation_id": null
}
```

敏感身份只返回产品确实需要的脱敏/稳定摘要；完整 IMSI、ICCID、号码、APN secret、D-Bus path 和 `/dev` 节点不属于此响应。若某 source 读取失败，保留该字段 `null`、`sampled_at` 和稳定 `reason`；不得用上次值冒充新采样，缓存值必须带 `stale=true` 和原采样时间。

#### 9.1.2 重建写请求与 operation

所有 mutation 要求 `Idempotency-Key`、JSON、expected generation；已处于目标且 read-back 可证明时可同步返回 `200`，否则返回 `202`：

```json
{
  "operation": {
    "id": "...",
    "kind": "registration.manual",
    "generation": 13,
    "state": "queued",
    "phase": "admission",
    "requested_at": "...Z",
    "deadline_at": "...Z"
  }
}
```

推荐 request body：

| API | body |
| --- | --- |
| `PUT /data` | `{"enabled":false,"expected_generation":12,"confirm":true}` |
| `PUT /roaming` | `{"allowed":false,"expected_generation":12}` |
| `PUT /airplane` | `{"enabled":true,"expected_generation":12,"confirmation":"ENABLE AIRPLANE MODE"}` |
| `POST /operators/scan` | `{"expected_generation":12}` |
| `POST /registration` | `{"mode":"manual","mccmnc":"46001","expected_generation":12,"confirm":true}` 或 mode auto |
| `PUT /radio-mode` | `{"mode":"lte","expected_generation":12,"fallback":"previous","confirm":true}` |
| `PUT /bands` | `{"bands":[{"rat":"lte","number":3}],"unlock_all":false,"expected_generation":12,"fallback":"previous","confirm":true}` |

MCCMNC 必须是 5 或 6 位 ASCII 数字并在 server-side schema 校验；band/mode 只能取本次 capability snapshot 暴露的 canonical ID。客户端不能提交 MM numeric band ID、object path、profile name 或 adapter 名称。

APN 使用 secret 三态，GET 永不回传 secret：

```json
{
  "apn":"internet",
  "protocol":"ipv4v6",
  "username_update":{"mode":"keep"},
  "password_update":{"mode":"replace","value":"..."},
  "auth_method":"chap",
  "apply":true,
  "test_connectivity":false,
  "expected_generation":12,
  "confirm":true
}
```

`mode` 只允许 `keep|replace|clear`，只有 replace 接受 value。APN GET 返回 `username_present/password_present` 和 configured/applied/verified 三层状态，不返回原值。operation 保存 params redaction 后的 fingerprint；同 idempotency key/同 body 返回同一 operation，不同 body 返回 409。

operation 状态固定为：

```text
queued -> running -> verifying -> succeeded
                    \-> rolling_back -> rolled_back
                    \-> failed | timed_out | uncertain | rollback_failed
queued/running -> cancelling -> cancelled（仍需 read-back）
```

详情返回 `state,phase,generation,capability,adapter,started_at,deadline_at,finished_at,previous_safe_summary,observed_after,error,rollback`。`uncertain/rollback_failed` 禁止 watchdog 自动采取扩大的恢复动作；必须等待同 generation reconcile 或人工选择。

#### 9.1.3 重建错误 envelope

```json
{
  "error": {
    "code":"operation_in_progress",
    "message":"cellular mutation is busy",
    "request_id":"...",
    "retryable":true,
    "details":{"operation_id":"...","resource":"modem.control"}
  }
}
```

| HTTP | code 示例 | 场景 |
| --- | --- | --- |
| 400 | `invalid_json`,`missing_idempotency_key` | transport/request 格式 |
| 401/403 | `authentication_required`,`confirmation_required`,`policy_forbidden` | 身份、确认、负向 intent 阻断 |
| 404 | `operation_not_found` | opaque operation ID |
| 409 | `operation_in_progress`,`generation_mismatch`,`idempotency_conflict`,`modem_identity_changed` | 并发/晚到/重枚举 |
| 422 | `invalid_mccmnc`,`invalid_apn`,`unsupported_selection`,`no_safe_fallback` | 输入可解析但不合法 |
| 429 | `rate_limited`,`cooldown_active` | scan/register/high-impact 限流 |
| 501 | `capability_unsupported` | 确认不支持；cell lock 默认使用此语义 |
| 503 | `modem_unavailable`,`networkmanager_unavailable`,`dependency_unavailable`,`maintenance_active` | 外部依赖或维护 barrier |
| 504 | `adapter_timeout` | 请求尚未创建外部副作用；若副作用可能已发生，应创建 operation 并标 `uncertain`，不能只返回 504 |

`UnknownMethod/Unsupported` 映射 capability unsupported；`AccessDenied` 映射依赖权限错误；`NoReply/owner change/object removed` 通常映射 operation uncertain。底层 stderr、D-Bus path、APN 值、raw QMI/AT 不进入 error response。

### 9.2 推荐配置

建议增加独立且默认关闭的配置段；名称可在实现前按现有 TOML 风格定稿：

```toml
[cellular]
enabled = false                 # 未显式开启时只保留现有 modem/SMS 功能
manage_data = false             # 不默认接管 NetworkManager 数据连接
data_enabled = false            # 用户意图；实现时要区分“未设置”和“明确关闭”
roaming_allowed = false         # 默认不替用户产生漫游数据；按部署需求显式开启
auto_register = false
auto_reconnect = false
watchdog_recovery = false
```

- `enabled=false` 时不得自动创建 NM profile、开启数据、扫描运营商或写 radio/band/cell。
- APN 放在 `[cellular.apn]` 或同等子段，密码沿用现有 0600 配置文件保护；配置 summary、API GET 和事件全部脱敏。
- airplane、radio、band、cell lock 的持久化必须分别定义“desired”和“last applied”；在 capability 未确认前不持久化伪造的 cell lock。
- 用户负向意图（数据关闭、漫游关闭、飞行模式）应持久化并在服务重启后先恢复为阻断策略，再开始任何自动恢复。
- 配置 revision/precondition 与 live apply 必须有明确顺序和回滚规则，不能采用“先返回成功、后台随便尝试”的语义。

### 9.2.1 推荐持久化与重启恢复

这是目标设计，不是 SimAdmin 已有行为：

- `cellular policy` 使用原子替换的持久化配置保存 `enabled/manage_data/data_enabled/roaming_allowed/airplane desired`、APN secret 引用和单调 `revision`；`observed` 状态不写成 policy。
- 长操作写入 `cellular_operations`（或等价持久层），至少包含 `id/type/state/target_fingerprint/redacted_params_hash/idempotency_key/config_revision/created_at/started_at/deadline_at/finished_at/error/rollback_state`。不得持久化 APN secret、原始命令或完整身份标识。
- 服务启动时把遗留 `queued/running/cancelling` 标为 `reconciling`，重新发现 modem 身份并 read-back；能证明目标已达成则 `succeeded`，能证明未执行则 `failed`/按策略重试，无法证明则 `uncertain`。不得盲目重放 radio/band/APN/register。
- 操作保留期、事件保留期和幂等键 TTL 必须成为显式配置/常量并有测试；在实现前需冻结具体默认值。过期清理不能删除仍在运行、uncertain 或等待人工处理的记录。
- capability/cache 可留在内存，但必须绑定 modem fingerprint、D-Bus owner generation 和采样时间；重枚举或 owner change 立即失效。前端 localStorage 只能保存展示偏好，不能保存硬件 truth。

### 9.3 推荐前端

- 第一阶段把蜂窝诊断放在现有 Status/Modem 区域或一个显式的 Advanced Cellular 页面，不复制 SimAdmin 的 CPE Dashboard、WLAN 和系统控制入口。
- Dashboard/Status 只显示紧凑摘要：注册、运营商、RAT、信号、home/roaming、数据 policy/observed、SMS 可用性；cell 详情按需展开。
- 数据/漫游开关显示 pending、enforcement、最后操作和失败原因；不可用/未知/不支持必须有文字说明，不使用乐观 UI 永久覆盖服务器状态。
- 飞行模式、手动注册、APN apply、radio/band 操作都先确认影响，显示预计可能断网/短信延迟，提交后轮询 operation；页面离开后操作仍可观察。
- APN 密码输入支持“保持现有密码”，保存后不回显；错误中不展示 bearer secret。
- band UI 只列出服务器 capability 返回的实际支持集合；不使用 localStorage 作为 desired/hardware truth，不把空选择隐式当作危险锁定。
- cell lock 控件在真实 adapter 未交付前应隐藏或明确禁用，并显示“当前产品未向 modem 写入小区锁定”；不保留 SimAdmin 那种误导性成功按钮。

## 10. 测试矩阵

| 层级 | 必测场景 | 验收重点 |
| --- | --- | --- |
| 只读解析 | ModemManager JSON/text、缺少接口、SIM missing、disabled、无信号、旧版 mmcli | 字段为 null/unknown 且 reason 清楚；不把 fallback 当完整能力 |
| object path/身份 | modem 重枚举、owner change、path drift、多个 modem、fingerprint 不匹配 | 不能对未验证路径执行写操作；晚到结果不覆盖新状态 |
| D-Bus adapter | 属性读取、Enable/Disable、Scan/Register、Supported/Current Modes/Bands、UnknownMethod/AccessDenied/NoReply | 错误分类、deadline、read-back、断线重连；无 SMS deadlock |
| NetworkManager | profile 缺失、APN 应用、autoconnect、连接/断开超时、漫游 home-only、旧 profile 恢复 | data off 永不 reconnect；APN 的 configured/applied/verified 不混淆；iptables 不变 |
| QMI | 唯一/primary/歧义 port、proxy 不可用、权限、CID/timeout、IMS probe 与 cell adapter 并发 | IMS 只读探针不被新控制污染；不因 NAS 成功宣称 lock capability |
| AT/vendor | 允许型号、非法参数、错误响应、设备消失、超时、未识别固件 | 无任意命令注入；uncertain 时停止并提示，不能继续破坏性操作 |
| 用户策略 | data off、roaming off、airplane on 与 watchdog、自动 reconnect、显式 register/APN 并发 | 优先级严格满足第 6 节；安全负向 intent 不被恢复逻辑覆盖 |
| 状态机 | searching、registered、roaming、connecting、disconnecting、owner change、cancel、late result | desired/observed/enforcement/operation 四维一致，超时不假成功 |
| 回滚 | band/radio/APN 应用失败、配置 commit 失败、live apply 成功但 read-back 失败 | 可恢复则恢复旧值；不可恢复返回 uncertain 和操作建议 |
| API/安全 | 未登录、跨源、缺 content type、缺确认、重复 operation、速率限制、501 unsupported | HTTP status/错误 code 正确；secret、号码、IMSI/ICCID、raw command 不泄露 |
| 配置 | 缺少新段的旧 TOML、默认关闭、0600、revision 冲突、重启恢复负向 intent | 旧配置兼容；默认不接管 CPE 数据；写入具备原子性 |
| 前端 | loading/pending、操作超时、服务端状态覆盖 optimistic state、unsupported cell lock、APN 密码保持 | 不误导、不丢状态、不展示 secret；长操作可轮询 |
| 硬件实验室 | 至少一款 LTE、带 NR 的 modem、不同 MM/plugin/firmware、无 NM、无 QMI、AT-only | 每个 capability 有矩阵与 known limitation；破坏性用例需人工确认和恢复步骤 |

核心单元测试应使用 fake D-Bus/NetworkManager/adapter，不依赖真实 modem；真实设备测试只作为 opt-in integration/lab suite，不能在普通 `cargo test` 中切换 radio、band、APN 或数据。

## 11. 分阶段交付

### Phase 0：契约与能力发现

- 定义 `CellularSnapshot`、capability、policy、operation、error code 和敏感字段脱敏规则。
- 把 modem path verification、D-Bus owner change、MM/QMI/NetworkManager adapter interface 设计好；不提供新的写操作。
- 增加 fake adapter 和状态机/策略测试。

### Phase 1：只读诊断

- 实现 `/api/cellular/status`、信号/注册/运营商/数据观测和按需 cell snapshot。
- 在现有 Modem 页面展示摘要；缺少 capability 时显示 unknown/unsupported。
- 保证 SMS 收发与 IMS probe 测试不回归。

### Phase 2：用户策略开关

- 先交付 data enabled/disabled 和 roaming allowed/disallowed，明确 NM 数据面与 SMS/注册面分离。
- 加入持久化负向 intent、enforcement/read-back、并发锁和 watchdog 禁止越权测试。
- 飞行模式作为显式高影响操作单独验收，不启用自动恢复。

### Phase 3：运营商扫描与注册

- 长任务 operation API、扫描缓存、manual/auto register、超时/取消/owner change 处理。
- 默认只做显式用户操作；不在后台自动注册或 radio cycle。

### Phase 4：APN

- 配置校验、secret 保护、NM/bearer apply、连接验证和回滚。
- 明确无 NetworkManager、无 bearer 或应用结果不确定时的 API 状态；不影响 SMS relay 默认行为。

### Phase 5：radio mode 与 band lock

- 只在 capability matrix 覆盖的 modem 上开启；默认 advanced/feature flag off。
- 先实现 read-only supported/current，再实现显式写入、fallback、超时、read-back 和解锁全部；不做自动优化。

### Phase 6：真实 cell lock 评估

- 只有在选定硬件上完成 QMI/vendor/AT adapter、持久性、重启验证、解锁和恢复实验后才决定是否交付。
- 若没有跨硬件可维护的真实能力，产品结论应是“不支持 cell lock”，而不是增加一个仅内存态 API。

## 12. 完成标准

本功能只有在以下条件同时满足时才可宣称完成：

- SmsRelayed 默认仍是短信 relay，不默认接管蜂窝数据或 CPE 网络；
- 用户明确关闭的数据/漫游/飞行模式意图在服务重启、modem 重枚举和 watchdog 运行时仍具有最高优先级；
- 所有写操作都有 capability gate、认证、确认、operation 状态、超时和 read-back；失败不会伪装成 success；
- APN、radio、band 的失败路径有可验证回滚或明确 uncertain；
- cell lock 没有真实硬件实现时明确返回 unsupported/in-memory-only，且 UI 不误导；
- 没有修改 host firewall/路由、没有暴露任意命令、没有日志或 API secret 泄露；
- 普通单元/集成测试覆盖上述状态、并发、安全和 fallback 矩阵，真实设备破坏性操作仅在受控实验室执行。

## 13. 实现前必须冻结的决策

下列事项在参考实现中没有可靠答案，不能由实现者暗自选择：

1. 首批支持的 modem/plugin/firmware、NetworkManager 版本和多 modem 选择规则；不在矩阵内的能力是 `unsupported` 还是 feature flag off。
2. `airplane` 在本产品中精确定义为 modem `Enable(false)`、低功耗 radio state，还是平台级飞行模式；它与 SMS relay 可用性的冲突提示。
3. data/APN 的唯一所有者：只管理指定 NM connection、只管理 MM bearer，或两者协调；现有用户 profile 的所有权和回滚边界。
4. 操作 timeout、保留期、幂等 TTL、scan cache TTL、watchdog 阈值和 recovery budget 的生产默认值。
5. cancel 的底层能力：哪些 adapter 可中止，哪些只能停止后续步骤并等待 read-back；`uncertain` 的人工恢复入口。
6. radio/band/cell lock 的产品范围。特别是 cell lock 在没有选定真实 adapter 前必须保持 unsupported。
7. APN secret 使用当前 0600 配置、独立 secret store 还是外部凭据提供者，以及备份/恢复是否包含它。
8. 向已有兼容客户端保留第 2.1.1 节 legacy 路由的期限；目标客户端只使用第 9.1 节 operation API。

## 14. SimAdmin 参考来源索引

以下行号以已审阅基线 `/tmp/SimAdmin` 的 `main@eb6f497ad332e59f1f3899acc3b2b9427661fe5c` 为准；正文已经给出完整契约，索引仅用于复核。

| 主题 | 参考位置 |
| --- | --- |
| 真实受保护路由与 HTTP method（scan 为 GET） | `backend/src/main.rs:510-688`，`device_routes` |
| handler 请求/响应及业务错误 envelope | `backend/src/handlers.rs:1525-1818`、`:2309-2577`，`start_cell_monitor_handler`、radio/band/operator/APN/data/roaming/airplane handlers |
| 全局串行锁 | `backend/src/serial.rs:1-29`，`SERIAL_LOCK`/`with_serial` |
| radio、band、cell monitor、scan/register、APN、airplane 底层行为 | `backend/src/modem_manager.rs:3057-3524`、`:4096-4420`、`:4833-5017` |
| 数据连接与 NetworkManager/profile 行为 | `backend/src/modem_manager.rs:3659-4094`，`set_data_connection_with_apn`/`set_data_connection_inner` |
| watchdog 阈值与恢复顺序 | `backend/src/modem_manager.rs:6246-6590`，`data_connection_watchdog` |
| 内存 data-off/airplane intent 与启动任务 | `backend/src/main.rs:328-470`、`backend/src/state.rs:42-91` |
| cell lock 仅内存 store | `backend/src/cell_lock_store.rs:1-69`，`CellLockStore` |
| 配置字段和默认值 | `backend/src/config.rs:1811-1867`、`:1886-1935`、`:2182-2268`，`ApnConfig`、`AppConfig` 与 network/APN getters/setters |
| 前端请求 method 与类型 | `frontend/src/api/current.ts:536-718`、`frontend/src/api/types.ts` 中 network/cellular request/response types |
| Network 页 scan/register、乐观 radio、band localStorage、cell lock、APN secret 回填、monitor 生命周期 | `frontend/src/pages/Network.tsx:132-149`、`:347-429`、`:499-692`、`:768-821` |
