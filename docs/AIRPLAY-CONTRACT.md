# AirPlay 多入口接入合约

## 2026-10-07 字段混音能力

P02 增加快照 `runtime_epoch` 及相应 capability，V2 command 可携带 `runtime_epoch/credential_id`；在管理员认证后、回执查找和版本比较前验证。当前客户端以字段 patch 表达用户输入。对具有运行代次条件、但没有 patch 能力的全量接口，后台执行只读同会话刷新，核对 source/session/stream_epoch，叠加用户字段，生成同一快照 revision 的完整 command；Desktop 在实际发送前冻结该 command。重试使用原 command_id/revision/triple，不重新拼接；回复中的 before 取自这次只读刷新，Undo 仅恢复原修改字段。缺少必需 runtime 条件的组合返回 `upgrade_required`，不通过丢弃条件字段静默写入。

V2 快照的 `capabilities` 声明 `patch_mix_source`。`PatchMixSource` 携带明确的 `source_id/session_id` 和可选 `gain_db/muted/solo`；至少提供一个字段，gain 须为有限的 −96…12 dB。未提供的字段由服务端从当前锁定来源及会话补齐，保持未修改字段；gain/mute 按来源持久化，solo 仅作用于该会话。单独 solo 修改不写 profile。身份、revision 和当前会话校验均在修改及保存前完成，旧 session 返回 `session_changed`。

旧 `MixSource` 保持全量 triple 语义。客户端仅在能力已声明时使用 patch；旧服务器须刷新同一会话后叠加用户修改字段，并以该快照的 revision 提交，冲突后由用户核对重试。当前批次已提供服务端能力，Desktop 调度及兼容接线继续由 P02 跟踪。

版本：macOS 开发 Alpha，2026-10-02。代码入口为 `apps/hub/src/airplay.rs`、`airplay_api.rs`、`admission.rs` 与 `crates/identity/src/airplay_profile.rs`。本合约描述已实现接口；验收记录见 [AIRPLAY](AIRPLAY.md)，协议帧见 [IPC 合约](AIRPLAY-IPC-CONTRACT.md)。四路数字源通过不表示四台 Apple 实机、发现隐藏、视频伴音或发布验收通过。

## 身份、入口和发现

Hub 独占实体输出、worker 与发现。AirPlay 默认关闭；启用 1–4 个稳定入口，每入口一个 worker，分别拥有 listener、控制管道、媒体 IPC、PEM 临时文件与运行目录。入口 1 延续旧 Hub 派生接收 UUID/device ID、发现名称和密钥引用；新增入口使用独立 OS 随机 Ed25519 私钥和持久 UUID。停止接收、改网卡、重启不重建身份。

入口配置保存为 `airplay/receiver.json` v2，秘密只在相邻 `.credentials`，运行 PEM 当前用户 0600、退出删除。最多 64 个来源与 256 个 `(receiver_id, source_id)` 绑定，撤销记录占限额。已验证来源的 ID 为 `sha256:` 加规范 Base64 解码后 32 字节公钥的 SHA-256；名称、IP 或 MAC 不授予权限。一个来源配对某入口不代表其他入口已获信任；相同硬件使用不同公钥时显示为不同来源。

worker ready 且允许新连接后，Hub 发布 `_raop._tcp` 和 `_airplay._tcp` 服务对。各入口使用独立 hostname、device ID、实例名与实际端口。发布器使用隔离 `mdns_sd_scoped` fork，链路本地 IPv6 地址只从所属接口发布；原生房间发现仍单独存在。

入口状态分别报告 `ready`、`active` 和 `discovery_state`（`published/withdrawing/hidden/publishing/error`）、`visibility_reason`。活动会话的 grant 确认后撤下本入口服务对；暂停、静音、FLUSH 不恢复公告。清理完成、输出可用且容量允许后以原身份重新发布；容量满时也隐藏空闲入口。撤公告由独立非实时任务等待本地注销确认，有界重试，不关闭媒体或全局 Bonjour。旧 goodbye 重发按实际服务代际检查，不能撤销新注册。

`hidden` 仅代表 Hub 本地公告状态，不保证 Apple 列表缓存已经消失。缓存或直接地址连接仍须通过 owner 与配额检查；不能抢占活动来源。真实 Apple 撤公告不中断播放、观察设备列表消失/恢复的 P95 目标仍待验证。

## 音频能力

保持经典 UDP 音频/NTP、44.1kHz PCM/ALAC/AAC/AAC-ELD 输入和 48kHz stereo F32 输出，协议兼容型号 `AppleTV3,2`、features `0x481C5A00`。不声明 AirPlay 2 buffered audio、PTP、多房间、视频、照片、HLS 或镜像。视频/屏幕请求在协议层拒绝，worker 不自行打开实体声卡。原生 Sender 与所有 AirPlay 来源进入同一个 Mixer。

## 准入、配额与会话

多入口模式满足“活动输入 + 建立中预留 ≤4”，原生和 AirPlay 共用 Hub 权威准入状态。每入口最多一个 owner，每个已验证 source ID 最多一个活动/预留会话；闲置公告不占 lane。配额中包含已进入建立阶段的原生会话。媒体构建、凭证写入与发现等待不应持有 Engine 全局锁；锁外准备后必须核对原授权和 reservation，不能把迟到结果用于替代来源。

AirPlay 预留期限 5 秒；worker 请求等待最多 500ms。过期后先关闭旧媒体门并回收实际 producer，才能释放 lane。session/stream ID 在房间媒体命名空间内检查冲突；重连使用新会话。先安装 context，有效首包在有界 Ingress 等待；正确 `grant_applied` 后才进入 Mixer。未确认、旧代际、错连接、错 session/epoch 或迟到事件不能提交/结束新会话。

兼容单路模式保留既有上限：AirPlay 活跃或预留时最多一路原生输入；没有 AirPlay 时不额外降低原生内部槽位上限。必须显式配置 `multi_receiver=true` 才启用四路总容量，不以内部 16 槽位宣称 16 路 AirPlay 支持。

PIN 首次配对窗口 10 分钟、每窗口最多 5 次挑战。Hub 持有窗口剩余时间与尝试数，入口进程重启不能刷新未到期预算。重新开放窗口是明确管理员动作。Hub 全局同时最多 4 个首次挑战，每入口最多 1 个；挑战租约最长 60 秒且不超过 PIN 窗口。租约、连接、请求和信任代际都须匹配才能完成注册。已验证配对与会话准入分开；配对成功不承诺仍有播放容量。公钥注册必须匹配 connection/request/trust generation，不能由迟到注册解除撤销。

## `/v2/airplay` API

GET/POST 复用现有 HTTPS、固定证书与 Bearer 验证。读状态返回 `revision/enabled/multi_receiver/receivers[]/sources[]/sessions[]/capacity`。`sources` 为持久来源，`sessions` 为当前活动流，连接数只统计后者。容量包括 `limit/active/reserved/available`。入口 `configured_enabled` 表示持久配置是否选用，`enabled` 表示当前运行状态；降低入口数量保留身份，不应按 receivers 长度推断启用数量。来源公钥原文、凭证引用与秘密不进入公开 API；PIN 只在管理员的入口状态中出现，session、诊断和普通成员快照不含 PIN。

POST 使用闭合 schema：

```json
{"command_id":"UUID","expected_revision":27,"operation":{"action":"mix_source","source_id":"sha256:…","session_id":123,"gain_db":-6.0,"muted":false,"solo":false}}
```

| action | 定向字段 | 行为 |
|---|---|---|
| `configure` | `receiver_count` 1–4、`multi_receiver` | 运行中可增加入口，保留其他会话；缩减时被移除入口须无活动/预留与配对挑战；多余身份保留 |
| `enable` / `disable` | 无 | 开启配置允许的入口 / 关闭全部入口；不自动恢复来源媒体 |
| `enable_receiver` / `disable_receiver` | `receiver_id` | 保存目标入口的启用配置，并改变它的运行状态 |
| `pair_receiver` | `receiver_id` | 为配置启用且空闲的目标入口开放首次配对窗口；运行入口无需重启，等待Applied后显示PIN；占用时拒绝 |
| `rename_receiver` | `receiver_id/name` | 入口停止且无claim时保存发现名，非空白、无控制字符、≤50 UTF-8字节且本profile唯一；身份/密钥不变，下次启动生效 |
| `alias_source` | `source_id/alias` | 设置≤128 UTF-8字节非空白无控制字符别名，或null清除；不合并身份/授权 |
| `mix_source` | `source_id/session_id/gain_db/muted/solo` | 范围 −96…+12dB；trim/mute 持久，Solo 只属于会话 |
| `disconnect_source` | `source_id/session_id` | 结束该来源并禁止它自动重连，须明确重新允许 |
| `revoke_source` | `source_id/session_id`（离线为 null） | 撤销来源在所有入口的配对；活动来源必须匹配会话 |
| `allow_source` | `source_id` | 解除播放禁止；不能恢复已撤销配对 |
| `repair_source` | `source_id/receiver_id` | 来源无活动会话、目标入口配置启用且空闲时丢弃旧绑定并重新开放 PIN 配对；不恢复旧授权 |
| `playback_mode` | `source_id/mode` | 来源无活动会话时保存 `low_latency` 或 `synchronized` |

全部写入仅 Admin。AirPlay revision 独立于原生 revision；电平/诊断不递增控制 revision。进程内最多 128 个 command ID 回执；同 ID 同完整请求返回原回执，同 ID 不同内容拒绝，不跨重启重放媒体动作。

持久写入在单独 profile 锁下准备和落盘。提交后若原会话已结束，返回 `outcome=saved_session_ended`，保存结果保留但不操作新会话；多步入口创建部分完成或文件已发布但同步未知时，不返回成功回执；HTTP 503 固定为 `profile_durability_unconfirmed`，保留原 command ID、完整请求、候选配置与原 owner。重试原请求只同步该候选；创建中断后复用私有日志中的接收器身份，已完成配置的同步重试不再创建 receiver/key。未知期间拒绝新配置和新配对/准入，旧完成回执仍可无副作用重放。

恢复期间使用旧权限与候选权限的交集：已发布 revoke/disconnect/disable 保持拒绝并独立关闭 gate/lane，不受 Mixer 配置队列容量阻塞；Allow/Repair/Enable、打开配对窗口与新 worker 注册信任，均等待成功同步。状态提供 `persistence_pending`，管理员可读 `pending_command_id`；权限投影不冒充候选已保存。`POST /v1/persistence/recover` 以该 ID 作为 `request_id`，携当前 `runtime_epoch`，重查当前 Admin 后确认同一候选；配对注册 worker 在发布未知后停止，确认返回 `saved_worker_stopped`，不重启媒体。正常命令恢复保留原 owner 条件，不能操作替换会话；停止中的 owner 不因恢复而再次启动。

Mixer 配置队列暂满时，已持久化回执可返回 `media_pending=true`，由 owner 重试最新配置。客户端分别展示待确认的保存结果和媒体进度；事件/config 版本分域及完整 desired/applied 仍待 P07。

错误包括 `receiver_busy/room_capacity_full/source_already_active/source_blocked/pairing_revoked/stale_revision/session_changed/output_unavailable/worker_unavailable/upgrade_required` 及有界参数/配置错误。这些是 NeonMix API 错误，不承诺 Apple 原生提示显示相同文案。旧 `/v1/airplay` 在兼容单路模式保留，在多路模式返回升级要求，不把无目标操作映射到第一路。

## 调音、时间与资源

每 session 独立 Ingress、8×480 帧短 SPSC、lane、protocol gain、连续 sinc、RTP/PTS 锚点和统计，共享实体输出呈现时钟。单路 flush/seek/重复 SETUP 只重建该路 epoch；暂停不靠无声超时释放 owner。公共输出丢失、休眠或墙钟映射失效可能影响所有路，应在诊断中与单 worker 故障区分。

默认低延迟在每 epoch 首包按 `max(协议目标−接收时间−设备输出积压−120ms,0)` 固定提前 PTS；同 epoch 保持采样间隔，不逐包按到达时间调速。`synchronized` 保留协议目标。120ms 是提交设备前的接收余量，设备输出积压另计，不是端到端延迟承诺。首包之前读取原生回调的 playback−callback 时钟差；同 epoch 内设备积压变化不重排已有 PCM。设备偏好优先于入口默认值，活动来源不能切换播放方式。

首块 PCM 已提前进入 Mixer 时，等待起播的游标按输出采样网格推进；相对于最新回调观测不超过 1ms 的相位差交由现有 SRC servo 连续纠正，避免回调边界的小抖动跳过首块样本。负向抖动时，起播相对最新原始观测最多提前 1ms。首次取得 PCM 时已经迟到，或等待期间观测与采样网格偏差超过 1ms，仍按实际观测重新定位并记录跳帧。换代、通道重用及断流重获时间线会清除旧等待锚点；不改变发送方 PTS、待播上限或短队列容量。

协议音量只应用一次，有效增益为 `protocol_gain × input_trim × master_gain`。Solo 覆盖全部原生/AirPlay 输入；静音或被 Solo 排除仍消费时间线。不按来源数量自动归一化，沿用 limiter 和渐变。断开清理 Solo，不污染下一台占用同 lane 的设备。

每路待播最多 4 秒且 2MiB，四路合计 8MiB；这不包含 worker/GStreamer 进程全部内存，也不是进程 RSS 保证。PCM 包 ≤3952 bytes，控制行 ≤16KiB。媒体或控制堵塞不能扩大无界缓存、拖停其他来源或使实时 callback 执行锁等待、IPC、分配/析构。

## 保存与验证边界

profile v2 闭合 schema、UUID 引用、私有权限和原子替换沿用 ADR-014。v1 迁移保留入口 1 身份/密钥/名称/配对/拒绝/模式；旧全局禁止播放投影为所有旧来源的blocked状态，v2全局允许字段归真，仍须管理员逐来源Allow才能恢复已知来源；撤销记录保持撤销，迁移不启动接收。未知版本、损坏或缺失秘密失败关闭。迁移不自动开启接收。离线 `neonmix-airplay-profile migrate/export-v1` 使用 Hub owner 锁，导出当前入口 1 与所有适用全局撤销；不能直接恢复旧快照。

macOS 两路/四路加密数字源、动态独立 lane、同输出电平、逐路断开/worker 退出隔离和身份恢复有短程证据。尚未完成四台真实 Apple 来源、观察设备隐藏/恢复、跨平台运行、健康网络 8 小时/24 小时资源长测、物理音画 P95 或正式分发。UI、协议注销确认或合成数字通过均不能替代这些验收。

新身份创建使用同目录 `receiver.json.pending` 私有操作日志，先原子保存原profile/目标profile/新入口UUID和key引用（不含秘密），再写指定引用的独立随机密钥、读回、原子发布profile，核对后清日志。create/add重试及停止接收的启动迁移可恢复同一待完成身份；常规load保持只读，未完成日志阻止其他save覆盖。恢复不删除秘密；profile冲突、已发布身份缺失密钥或日志损坏均报错。profile缺失而.credentials非空、又无日志时返回credential_profile_missing，不生成替代身份。强制结束子进程的journal/secret/profile三个阶段均有恢复测试。

定时 Mixer 在已经开始播放后耗尽 PCM 时增加 `underrun_frames` 与对应 `underrun_frames_by_lane`；等待未来首个 PTS 的静音不算欠载。`timed_late_frames` 仍只记录重新获得时间线时跳过的采样，二者不可互相代替。Worker 队列失败分别公开 `decoder_input_queue`（压缩输入）、`pcm_output_queue`（IPC 发送待发）、`decoder_metadata_queue`（时间映射元数据），不导出原始错误字符串。

`timed_late_frames_by_lane` 按物理 lane 生命周期累计，不因来源、session 或 epoch 重用清零；对应 `last_timed_late_*` 保存最近跳帧的 stream、epoch、输出帧、首块 PCM 目标时间及输出呈现时间，供定位而非多字段原子事件快照。每个 receiver 公开当前 worker 的 `max_loop_gap_ns`、`max_control_ns`、`max_pcm_ns`，分别观察循环间隔、控制处理和 PCM 交接的最大耗时。多路探针在稳态以及每个 baseline、fault、recovery 窗口均要求聚合跳帧和存活 lane 欠载 / ingress late 无增量；聚合跳帧不能只按存活 lane 推导。


## P07 AirPlay 命令 v3 与版本域

已初始化的 `/v2/airplay` 快照提供 `command_version=3`、`config_revision`、`event_sequence`、当前 `runtime_epoch`，capabilities包含version_domains。revision继续等于旧运行期序号，未重新解释。未初始化时保留旧v2引导快照，首个合法v2写入可加载/创建私有profile；随后重新取快照使用v3。只读请求仍不创建凭证。Native版本另外在native_versions中标示，容量是两域的实时观察，不承诺跨域原子配置。

v3 Command正文声明command_version=3，携同runtime_epoch/credential_id，省略expected_revision。持久操作用expected_config_revision，涉及会话、入口运行/空闲、容量或配对窗口的操作还用expected_event_sequence；Mix/含gain或mute的Patch同时检查两域，Solo-only只检查运行期。AliasSource/AllowSource无需让无关健康变化阻塞配置CAS。v2正文省略新版本标记和新条件，继续required expected_revision的原CAS含义；新旧条件混用或缺必要条件返回upgrade_required，不忽略条件。缓存重放和未知候选重试仍在新CAS前结束，原ID换正文拒绝。

profile增加持久config_revision。旧v2资料缺字段时导入基线1，不改变身份/密钥/最新限制；新字段要求新版reader，旧严格reader不能原位读取，回退仍须停止下使用最新trust导出v1，不能恢复过时授权。新身份staging journal为v2，支持既有v1 journal恢复；每次实际身份/配置变更前进其持久版本，组合配置可跳多个版本，但同步同一已赋号候选不再递增。不将counter字段排除在journal一致校验或替换结果之外。

快照state是版本化业务投影：configuration为已确认配置，receivers/sessions/effective_sources是当前业务状态及即时限制，persistence_pending区分未确认保存。未知阶段configuration/config_revision保持原已确认值，文件中候选版本可以已可见；已发布限制仍立即有效。原根receivers保留旧兼容观察字段，包括PCM统计、PIN和剩余时间；这些采样及跨Native域的capacity不进入AirPlay业务state。GET和新命令CAS前对当前业务投影与已发布状态对账，未显式递增的discovery等业务变化也取得新event_sequence；采样统计不推进它。序号耗尽不wrap，快照将event_sequence置null并标snapshot_required，新的写入拒绝，限制/停止仍执行。

Desktop普通字段调度按最新确认快照构造v3条件；Repair/Revoke等确认框保存打开时的原命令/ID/条件，确认时不补后来版本。未知Flight继续原envelope对账。缺Patch能力的兼容解析只接受v2，不把v3条件替换成legacy条件。队列满、FIFO授权及desired/applied进度仍是剩余P07验收。


P07 AirPlay终态操作在apply_action前撤销对应lane原子授权；ingress gate与Mixer FIFO分别关闭，worker队列失败也不重新允许旧PCM。media_pending采用callback的desired/applied，而非仅mixer_dirty（enqueue成功尚不等于回调应用）。保存与应用通过media_application分开报告，完整历史重放保留原回执；实时确认读取新观测。实际软件矩阵使用另一Native来源持续渲染，未知Native Revoke亦保留另一Timed来源。
