# AirPlay worker 控制 IPC v2 / PCM v1

实现：`crates/airplay-ipc`，crate 名 `neonmix-airplay-ipc`。本合约冻结进程间字节布局与拒绝规则；它不表示 Apple 客户端互通、Speaker 能力或视频伴音同步已经通过实机验收。

## 通道与所有权

Hub 独占每入口 worker 的生命周期与准入权限。每个入口有独立的父子进程私有 stdin/stdout JSONL 控制管道与媒体连接；连接对象绑定 receiver UUID 和 worker generation，不从 PCM payload 推断入口。最多四个 worker，控制不排在媒体队列后。

2026-10-02 macOS 更新：当前 Hub 通过 `local::Listener` 在项目内创建私有 Unix socket，startup `media_address` 为 `unix:/absolute/path`。Hub 验证对端 UID 和启动的 worker PID，worker 验证 0600 socket 所有者、对端 UID 与父 Hub PID。控制管道、能力令牌与二进制 framing 保持；loopback TCP 只保留于 worker 独立协议探针，不是当前 macOS Hub 的回退。其他平台的本地传输不属于这次 macOS 验证。

2026-10-03 修复：Windows worker 接入 Hub 的 `\\.\pipe\NeonMix.Airplay.v1.…` 字节模式 NamedPipe。发送 token 前检查父进程 PID、创建时间、同用户 SID 及受保护的 owner-only DACL；Hub 继续验证实际 worker PID/SID。客户端禁止服务器 impersonation，采用非阻塞写，一整包限时 250ms，停止标志在写入间隙检查。macOS 上 Windows 分支语法检查与 IPC 交叉 Clippy 通过，Windows 原生运行仍待复验。

Hub 收到 `admit_request` 后，profile/Engine 锁竞争只延后处理，不立即作授权拒绝；从控制读取线程收取事件起计 400ms 处理期限，为 worker 原有 500ms 等待留下余量。重试不阻塞媒体泵、不刷新截止时间，授权前重新检查当前策略与代际；过期请求不能占用名额。已失效的 start/grant 在等待 profile 前丢弃。接收入口的 `admission_denials` 仅累计固定拒绝原因，不含来源身份、密钥或 PIN。

独立探针的 TCP 媒体端口仅绑定回环地址，不监听 LAN。worker 从匿名 stdin startup 配置获得临时 `media_address` 与至少 256 bit 随机 `ipc_token`；禁止把 token 写到 argv、诊断或普通配置文件。每次 worker 启动重新生成。媒体流首先发送 `ipc_token` ASCII 加 LF，Hub 使用有界读取（最多 256 bytes，含 LF）及有限超时进行校验，成功后才解析二进制数据。错误 token、过长握手、超时与连接终止均关闭该连接，不产生媒体授权。token 是连接能力凭证，不能取代会话身份/epoch 检查；同用户父进程本身被控制不在此隔离模型内。Hub 必须限制同时待验证的连接数，媒体 worker 只能占用一个活动连接。

控制管道与媒体通道只能由专用非实时线程读写，音频 callback 不运行 JSON、IPC、阻塞或分配。发送与读取都设置有限期限；媒体队列满、媒体消费者持续阻塞、控制管道失效时结束 AirPlay 输入并报告故障。不得扩展为无界缓存，也不得拖停原生 Sender。具体线程、socket 超时与队列限额由接入实现提供，单包限额由本合约固定。

## PCM 二进制布局

所有数字字段使用 little-endian。固定 header **112 bytes**，紧接 `payload_bytes` 个音频字节；没有外层长度前缀、C 结构体内存布局或跨进程指针。最长包 **3952 bytes**。支持任意 1–480 帧，不将解码回调长度固定为 480；较大回调由 worker 拆包。

| Offset | Bytes | 字段 | v1 值 / 含义 |
|---:|---:|---|---|
| 0 | 4 | magic | ASCII `NMAM` |
| 4 | 2 | ipc_version | `1` |
| 6 | 2 | header_bytes | `112` |
| 8 | 4 | payload_bytes | `frame_count × 2 × 4`，最大 `3840` |
| 12 | 4 | flags | bit 0：discontinuity；其他位为 0 |
| 16 | 8 | session_id | Hub 授予的非零 u64 |
| 24 | 8 | stream_id | Hub 授予的非零 u64 |
| 32 | 8 | stream_epoch | Hub 授予的非零 u64 |
| 40 | 8 | format_epoch | Hub 授予的非零 u64 |
| 48 | 8 | sequence | 本 context 内单调递增 u64，可从 0 开始 |
| 56 | 8 | source_sample_position | 原输入采样时间线的展开 u64 位置 |
| 64 | 8 | presentation_time_ns | 第一帧应出声的 Unix 时间，非零 u64 ns |
| 72 | 8 | mapping_id | Hub 授予的非零时钟映射 ID |
| 80 | 8 | uncertainty_ns | 时间映射误差估计 u64 ns |
| 88 | 4 | source_rate | 原输入采样率，8000–384000 Hz |
| 92 | 2 | frame_count | normalized PCM 帧数，1–480 |
| 94 | 1 | channels | `2`：interleaved L/R |
| 95 | 1 | pcm_format | `1`：IEEE-754 float32 LE |
| 96 | 4 | protocol_gain | 线性 f32，有限且处于 0–1 |
| 100 | 1 | gain_applied | `0` 或 `1` |
| 101 | 1 | clock_domain | `1`：Unix wall clock ns |
| 102 | 2 | reserved | `0` |
| 104 | 4 | pcm_rate | `48000` Hz |
| 108 | 4 | reserved | `0` |
| 112 | 可变 | PCM | f32 LE，按 L0/R0/L1/R1 顺序 |

`source_rate` 与 `pcm_rate` 分开：44.1 kHz 来源转换为 48 kHz 时保留原来源位置，不能用 normalized 帧数直接累加原采样位置。source position 经 RTP 回绕展开，转换后的若干小块允许共享来源位置，但不能倒退。PCM 中 NaN/Inf 无条件拒绝；幅度有限但超出 ±1 可以进入后续 limiter，不在 IPC 改写音频。

`protocol_gain` 保存 Apple 单路音量。`gain_applied=0` 时 Hub 对本路应用一次；为 1 时 PCM 已带此增益，Hub 不重复应用。房间总控、Mute/Solo 仍由 NeonMix 控制。未知 gain 布尔值、时钟域、PCM 格式、保留位和版本都拒绝，不猜测兼容。

`presentation_time_ns` 绝不是进程内 `Instant` 纳秒。worker 将上游协议时钟转换为 Unix 墙钟；Hub 使用自己采样的墙钟/单调钟锚点映射到输出 deadline，并纳入待播队列、设备延迟和映射误差。墙钟跳变、休眠、输出重开或映射失效时 Hub 撤销旧 `mapping_id`，清旧队列并重新授予 context。worker 所填 uncertainty 只是估计；未经物理测量不能把它作为音画同步精度证据。接入层另外限制过早、迟到及不可信 uncertainty；二进制解析器仅验证字段形式。

此字段始终保存来源的协议目标；低延迟模式只在 Hub ingress 的新 epoch 固定提前目标，原始 IPC PTS 不改。`synchronized` 模式保留该目标。模式、120ms 余量及可见诊断口径见 [AirPlay 行为合约](AIRPLAY-CONTRACT.md)。

## 控制 JSONL

UTF-8，每行一个 JSON 对象，以单个 LF 结束。**最大 16384 bytes，包含 LF**；不接受 CRLF、截断行、多个 JSON 值或未知 typed message 字段。字段中的换行必须 JSON 转义，不能插入新的控制行。`read_control` 在读到 LF 前同样执行限额，`write_control` 在序列化期间执行限额，超长消息不会向目的管道写入部分内容。消息总体受限并不免除 Hub 对名称、公开密钥、PIN、端口和功能位的业务验证。

控制与 PCM 分别版本协商：startup 和 `ready` 必须同时为 `control_version=2`，PCM header 仍为 `ipc_version=1`。版本不匹配只令该入口失败，不回退到没有连接归属的 v1 事件。正式 Rust DTO 在 `crates/airplay-ipc/src/lib.rs`；worker C++ 实现在 `apps/airplay-worker/main.cpp`。

startup 包含 `control_version`、`worker_generation`、`trust_generation`、`pairing_remaining_ms`（0–600000）、`pairing_attempts`（0–6，6代表已耗尽）、`receiver_uuid`、`media_address`、`ipc_token`、稳定 `device_id`、项目内 `keyfile`、`name`、四位字符串 `pin`、`rtsp_port`、五个 context ID、`known_client_keys`、`blocked_client_keys` 与 `pairing_allowed`。最多 64 个规范 Base64 公钥，known/blocked 合计计数；接收器发现公钥仍为 hex，不能混用编码。PIN、token、keyfile 内容不得进入诊断。

### Hub → worker

以下 `generation` 指 `worker_generation`。涉及活动流的命令必须同时匹配连接和已安装 context；错误或迟到命令不能作用于替代来源。

| type | 必需字段 | 含义 |
|---|---|---|
| `pairing_admit` | generation、`trust_generation/connection_id/pairing_request_id/allowed/attempts` | 答复首次 PIN 挑战预算与并发名额申请 |
| `pairing_window` | generation、`trust_generation/pin/remaining_ms/attempts` | 空闲入口安装较新配对窗口，需同代信任快照先安装；旧窗口不能重放 |
| `admit` | generation、`connection_id/request_id/allowed` | 答复唯一正在等待的已验证请求 |
| `grant` | generation、`connection_id/request_id`、五个 context ID | 安装 Hub 授予的新媒体身份和 epoch；worker 确认后才能解锁媒体 |
| `allow` | generation | 更新入口允许状态；不恢复旧 grant |
| `disconnect` / `revoke` | generation、`connection_id/session_id/stream_epoch` | 只处理匹配 owner，清旧 PCM/context |
| `trust_update` | generation、`trust_generation/known_client_keys/blocked_client_keys/pairing_allowed` | 安装较新授权快照；同代/旧代不可重新开放已撤销信任 |
| `stop` | 无 | 由该入口 owner 关闭监听、清媒体并退出，不停止其他 worker |

### Worker → Hub

`provenance` 为 `worker_generation/connection_id/request_id`；`active context` 为 `session_id/stream_epoch`。事件是观察结果，不能自己产生或升级授权。

| type | 字段 | 归属 |
|---|---|---|
| `ready` | `control_version/worker_generation/port/public_key/features` | 实际监听和纯音频能力确认；未匹配前不发布 |
| `pairing_request` | `worker_generation/trust_generation/connection_id/pairing_request_id` | 申请首次配对挑战名额，不占媒体 lane |
| `pairing_ended` | 同 `pairing_request` | 释放匹配挑战名额，不能释放替代连接 |
| `pairing_window_applied` | `worker_generation/trust_generation` | 新窗口已安装，Hub再发布可见PIN状态 |
| `pairing_attempt` | `worker_generation/attempts` | Hub 单调保存首次挑战次数，进程重启不绕过窗口 |
| `pairing_pin` | `worker_generation/pin` | 仅管理员交互展示 |
| `admit_request` | provenance、`trust_generation/client_public_key/device_id/name` | 已验证公钥申请 Hub 配额 |
| `session_started` | provenance、active context、`trust_generation/client_public_key/device_id/name` | 初次 grant 前 context 为 0；仅凭已匹配预留推进，不视为授权 |
| `registered` | provenance、`pairing_request_id/trust_generation/client_public_key/device_id/name` | PIN 注册；媒体准入前 request ID 可为 0，仍须匹配连接和当前信任代际 |
| `grant_applied` | provenance、五个 context ID | Hub 核对完整 grant 后开媒体门；迟到确认无效 |
| `session_ended` | provenance、active context | 仅结束对应会话 |
| `format` | provenance、active context、`codec/source_rate/source_frame_count` | 已协商音频格式观察 |
| `volume` | provenance、active context、`volume_db` | 单路协议音量观察 |
| `flush` | provenance、active context、`reason` | reason 为 `protocol/stream_setup/timestamp_jump`（缺省 protocol），本路停发等待新 epoch |
| `fatal` | `message` | 固定脱敏失败类别，结束该入口 |

worker 等待准入最多 500ms；Hub 媒体预留 5 秒。首次 PIN 挑战通过 `pairing_request/pairing_admit` 单独申请，全 Hub 最多 4 个、每入口 1 个，挑战租约最多 60 秒且不越过 PIN 窗口；registered 必须匹配有效 pairing request，不能仅凭公钥名字登记。相同已验证来源不能同时抢占其他入口。活动 owner 的重复 SETUP 继续属于原连接；其他连接即使发现缓存仍在，也不能覆盖 decoder/context。registered 携带的旧 trust generation 不能反向恢复被撤销身份。

控制和媒体通道没有先后保证。Hub 安装只属于预留的校验 context，确认前的有效首 PCM 只进入有界待播；`grant_applied` 全字段匹配后才提交到 Mixer。超时关闭 gate、清队列并释放原预留。flush 产生新 epoch，也必须完成新 grant 确认，不复用旧确认。

每入口 owner 有独立控制/媒体执行单元，控制事件按有界批次消费；发现注销在另外的非实时任务等待。实时 callback 不执行 JSON、IPC、等待或堆分配。

## 失效与解析顺序

Hub 先撤销本地媒体 grant 并清空待播/Mixer 本路队列，再发送 disconnect/revoke/stop。收到 flush 或格式/时间断点时同样先让旧 context 失效，再递增相应 epoch、安装新映射与 grant。epoch 不回绕；耗尽时结束会话重建 ID。普通 PCM、discontinuity flag 和 worker 事件均不能自行提升 epoch。

接收端先读固定 header；校验 magic、版本、格式、身份字段与 `payload_bytes == frame_count × 8`，成功后才读取至多 3840 bytes 的负载。`read_packet` 使用固定栈缓冲，不按未知长度分配。完整 decode 也拒绝尾随字节、截断负载、非有限 samples 与非法增益。解码失败关闭连接，禁止在错误流中扫描 magic 重新同步。

`MediaGuard` 使用 Hub 已安装的 `MediaGrant` 严格匹配 session/stream/两个 epoch/mapping，拒绝未授权与在途旧包。sequence 严格递增，允许缺口供接入层计数；source position 不下降，原采样率变化必须获得新 context。新的 grant 必须由 Hub 生命周期逻辑授予，不能反复安装相同 context 来清除反重放状态。

## 验证与边界

macOS Rust IPC 测试覆盖固定二进制 offset、原采样率/Unix 时间、边界/截断/非有限数据、grant/epoch/反重放、JSONL 限额、v2 来源上下文与旧事件拒绝。worker 协议探针另验证连接归属、重复 SETUP、配对及媒体输出；多入口数字源经真实 Hub 和 CoreAudio 的证据单独保存，不能用 Rust parser 测试替代 Apple 客户端互通。

```sh
tools/dev cargo test -p neonmix-airplay-ipc -p neonmix-airplay-adapter -p neonmix-hub --locked
tools/dev python3 apps/airplay-worker/probe.py
tools/dev python3 tools/airplay_multi_source_probe.py --sources 4 --output coreaudio:BlackHole2ch_UID
```

## 控制 ID 与诊断补充

二进制身份字段仍为 u64。当前 C/C++ JSON 桥的已授权 ID、连接与代际限定为 `1..2^53-1`，Hub 生成的 session/stream ID 也在此范围。仅文档明确的 pre-grant session/epoch 与注册前 request ID 可以为 0；越界或负值拒绝，避免 libplist 对大数饱和导致上下文不一致。Unix ns 和源采样位置通过二进制媒体传递，不截断到此范围。

开启显式开发 trace 后，可有 `protocol {method,route,status}` 与 `protocol_detail {stage,a_bytes,proof_bytes,reason}`。只接受固定方法/路由类别、HTTP/RTSP 状态和 SRP 字节长度/失败枚举，最近记录上限 32；不带原始 URL、头、body、密钥、公钥负载、PIN 或 IP。正常运行默认关闭。
