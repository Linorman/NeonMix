# NeonMix 稳定性改进设计与开发计划

日期：2026-10-07（Asia/Shanghai）。状态：开发中（逐批验收）。适用范围：Desktop、后台、Hub 控制、Mixer、AirPlay、输出绑定和发布流程。

实施记录：[本轮基线、逐项覆盖及证据](evidence/review-stability-20261007/README.md)。勾选仅覆盖已完成的对应部分；平台验收和发布条件独立保留。

下一阶段应优先消除三类用户可见故障：操作落到错误对象、连接成功却持续无声、停止或保存结果不可信。保留现有 Rust 音频核心、独立后台、独立 AirPlay worker 和 Neon Console 界面结构，围绕身份、时间线、生命周期和提交结果补齐契约。大范围重构不应成为修复空列表崩溃、字段覆盖、会话音量残留等问题的前置条件。

本计划覆盖原分析的 **B01 至 B20、三项后续风险，以及产品和发布建议**。每项均对应设计、开发批次和关闭条件。所有 checklist 跟踪实施与验收任务；未勾选不代表未分析，勾选必须有对应代码及证据。实施状态以本轮证据及逐项 checklist 为准。

## 一 依据与当前基线

原分析为 [NeonMix 代码审查与设计改进](/Users/linorman/Downloads/NeonMix_Review_2026-10-05.html)，审查日期为 2026-10-05，固定提交为 `aefb8690320cb114c974be2477b2ebb64dfadf1e`。原文件 SHA256 为 `f6682a65b23fc760af8cc048c2b2b12e1cba2cd4227739b21cd8d6f28b9d09f4`。其中的操作与建议作为分析材料，不构成本次执行代码、安装、发版或修改外部系统的指令。

本次阅读时仓库 HEAD 仍为上述提交，但工作区包含尚未提交的 UI、Windows AirPlay、Hub/Sender 生命周期等修改。下文的“当前机制”依据阅读期间的源码，不是一个已冻结并执行测试的制品快照。后续开发必须先记录 HEAD、工作区差异及参与验收的二进制哈希。

工程范围以 [STATUS](STATUS.md)、[README](../README.md)、[ADR-003](adr/ADR-003-media-mixer-control.md)、[ADR-012](adr/ADR-012-desktop-background.md)、[ADR-014](adr/ADR-014-file-credentials.md) 和 [ADR-015](adr/ADR-015-multi-airplay.md) 为准。

需要在计划中保留的现状有四点：

- 2026-10-07 的 [Windows AirPlay 调查](WINDOWS-AIRPLAY-INVESTIGATION-20261007.md) 已记录 MediaPipe 写入吞吐修复、原生探针和真实单设备播放结果。这不能关闭 B20，也不能关闭历史四路恢复问题。
- 工作区已有 `crates/lifecycle`、托管停止信号及相关调用改动。B08/B09 应沿这条实现继续收敛，先检查实际接线和原生证据，避免建立第二套生命周期机制。
- 当前 UI 已改进部分读取失败处理、布局与交互，但仍可见全局 `queued_write`、不绑定上下文的 Undo、AirPlay 全量混音写入，以及缺失诊断值归零等机制。
- Windows 公用网络规则、Unicode 密钥路径和正常退出已有[专项计划](WINDOWS-AIRPLAY-RELIABILITY-PLAN-20261007.md)。本文复用其安装和平台验收，补齐通用的命令排队、停止确认与状态发布边界。

原分析的 Python 模型、提取函数执行和源码推导用于确定回归方向，不等于真实 Rust Mixer、完整网络会话或声卡实测。原报告中的约 9.979 ms 提前播放是模型结果；后续以真实 Ingress 和 Mixer 的输出时间测试为关闭依据。本次没有执行产品构建、原生 GUI 或音频验收。

## 二 问题覆盖与优先级

保留原报告编号和 P1/P2；R01 至 R03 是本文为原报告“三项后续风险”增加的跟踪编号，不是新增的已确认事故。D01 至 D08 对应下文设计，P00 至 P11 对应开发批次。

| 编号 | 原优先级 | 当前机制及处理方向 | 设计与批次 | 关闭条件 |
|---|---|---|---|---|
| B01 | P1 | `mixer_keys` 仍可在空 lanes 上索引；在共享入口处理空态和失效选择 | D01，P01 | 空房间、末路移除、失效选择均不 panic、不产生无效操作 |
| B02 | P1 | 队列头决定 `timed_mode`；改为控制面显式 lane 绑定 | D02，P03 | 实际 Mixer 在双向模式复用时拒绝旧包并正确播放新流 |
| B03 | P1 | 全局 `Option<Write>` 覆盖其他对象操作；改为有界字段意图队列 | D01，P02 | 慢回复下多通道、多字段修改均得到明确终态 |
| B04 | P1 | AirPlay `MixSource` 携全量 gain/mute/solo；新增字段 patch | D01，P02 | 调 gain 不改变并发更新后的 mute/solo |
| B05 | P1 | dispatch 才读取当前房间；Undo 不携带房间身份 | D01，P02 | 四个切换窗口均无跨房间、跨身份、跨会话误写 |
| B06 | P1 | 未 grant 阶段 context 为零；关闭命令仍按非零 context 比较 | D03，P04 | 准入各阶段取消后双方释放同一 owner，迟到 grant 不复活 |
| B07 | P2 | 协议 gain 的生命周期仍属于 worker | D03，P04 | 新 owner 使用初始 gain；同会话 flush 保留合法音量 |
| B08 | P1 | 停止与长操作共享锁；UI 仍存在双 IPC 失败即成功路径 | D04，P05 | 远端慢操作和连接过载不丢停止；未确认不显示成功 |
| B09 | P1 | 生命周期基础正在改动；跨平台强退回收尚需闭环 | D04，P05 | 后台被强制结束后，所属媒体按期限退出，无误杀 |
| B10 | P1 | 输出设置仍跨 state/profile 两次 replace，并尝试回滚 | D05，P06 | 任一持久边界中断后恢复完整旧值或完整新值 |
| B11 | P2 | `health()` 仍把缺失计数当 0；诊断新鲜度未独立建模 | D07，P08 | 不可用、过期、缺字段均不显示绿色正常 |
| B12 | P2 | DSP reset 清掉 started，使连续断流漏计 | D02，P03 | 已运行 lane 一秒缺 PCM 精确增加 48000 内部帧 |
| B13 | P2 | 管理 CAS 仅比较 revision，重建后可复用数值 | D05，P06 | A 删除、B 重建后，A 的旧管理请求全部冲突 |
| B14 | P2 | 持久版本和事件游标混用；事务期间健康可无事件改变 | D06，P07 | 重启和事务交错后事件副本与权威快照一致 |
| B15 | P2 | UID 级虚拟输出与实例级 owner 不一致 | D04，P09 | 第二实例明确拒绝冲突，不静默借用另一实例资源 |
| B16 | P2 | spawn/运行中可当成功；ready 依赖可被覆盖的最近日志 | D04，P05 | 五类启动夹具均返回正确业务状态 |
| B17 | P2 | 后台传入 `--seconds 86400` | D04，P09 | 桌面持续模式跨 24 小时不自行结束；限时 CLI 行为明确 |
| B18 | P2 | RPATH 正则仍使用 `\S+`，且清理忽略命令失败 | D08，P01/P10 | 含空格路径完整解析，独立检查及重定位启动通过 |
| B19 | P2 | HTTP 准备事务未区分缓存重放与新事务 | D06，P01/P07 | 重放返回原结果，存储、预留、DSP 副作用调用数均为 0 |
| B20 | P1 | Ingress 用已接受帧数连续编号，预取断点会触发整段 reset | D02，P03 | 缺口后的波形不提前，前段有效尾音不被预取丢弃 |
| R01 | 待验证风险 | revoke gate 受 Mixer 配置队列影响 | D06，P07 | 队列满时仍及时关闭终态会话媒体，旧排队 PCM 不继续输出 |
| R02 | 待验证风险 | replace 出错后字节相同即当成功，混淆可读与耐久 | D05，P06 | 明确区分未发布、已发布耐久未确认、已持久化 |
| R03 | 待验证风险 | CLI 反馈环判断只认 loopback，且位于 capture 启动之后 | D07，P09 | 本机 LAN/IPv6 同端点在采集前拒绝，远端同名设备不误拒 |

这些关闭条件需要各自的确定性回归。只做一次四路播放长测，无法覆盖命令串房间、提交中断或 grant 竞态。

## 三 共同设计约束

### 身份和版本各有一个明确用途

| 标识 | 作用域 | 失效条件 | 不应替代的标识 |
|---|---|---|---|
| `hub_id` | 持久房间 | 更换房间身份 | 地址或房间展示名 |
| `credential_id` | 一份控制身份的稳定引用 | 身份替换或撤销 | 凭证文件路径、明文 token |
| `context_epoch` | 当前 UI 控制视图 | 切换房间或身份 | Hub 启动代次 |
| `runtime_epoch` | 一次 Hub 启动 | Hub 重启 | 持久配置 revision |
| `config_revision` | 一份持久配置域 | 持久提交 | 高频诊断序号 |
| `event_sequence` | 同一 runtime 的公开状态流 | 每次公开状态改变递增 | 配置 CAS |
| `session_id`、`stream_epoch` | 一次媒体会话和时间线 | 新会话、时间线重建 | lane 数组下标 |
| `binding_generation`、`output_epoch` | 槽位复用、输出时钟 | lane 重新绑定、输出重开 | 流显示名 |
| `applied_config_sequence` | 音频侧已应用配置 | callback 应用新配置 | HTTP 已接收或已保存状态 |

以上为目标语义，不意味着首批必须统一改完全部协议。原生控制和 AirPlay 目前各有版本域，迁移时明确 `domain`；不得把两个碰巧相等的 revision 当成同一个版本。

### 六条不可破坏的行为

1. 用户意图在创建时绑定对象；地址刷新可以发生，Hub、身份和会话不能在发送时悄悄替换。
2. 原始 PCM 不决定 lane 的持久播放模式。所有数据先验证绑定，再影响 DSP 状态。
3. 音频时间由有效时间坐标决定；丢包、拒包和预取不能压缩媒体时间。
4. “已接收”“已持久化”“已应用”“已停止”分别有证据；超时属于未知结果，不推断失败或成功。
5. callback 维持预分配、有界循环、无锁等待、无文件与网络 I/O、无分配及析构；对象回收留在非实时 owner。
6. 诊断的未知、过期和有效零不同。测试结论绑定平台、制品、来源组合、阶段和观察窗口。

保留现有产品边界：同一后台 Hub/Sender 互斥；关闭 UI 保持媒体；后台异常后不自动恢复播放；不自动换默认声卡；恢复权限后仍需显式 Start；文件凭证继续遵循 ADR-014；不扩展 AirPlay Alpha 协议范围。

## 四 D01 命令意图和桌面交互

涉及 B01、B03、B04、B05。主要代码：[Desktop](../apps/desktop/src/main.rs)、[lanes](../apps/desktop/src/lanes.rs)、[shell](../apps/desktop/src/shell.rs)、[Mixer 键盘入口](../apps/desktop/src/pages/mixer.rs)、[AirPlay API](../apps/hub/src/airplay_api.rs)。

### 意图模型和有界调度

将队列从 `Option<Write>` 改成 `IntentScheduler`，建议在 `apps/desktop/src/intent.rs` 新建纯状态模块。以下是拟议字段，不是当前接口：

```text
ContextKey = {hub_id, credential_id, context_epoch}
TargetKey  = Master | Native(stream_id, session_id, stream_epoch)
                   | AirPlay(source_id, session_id, stream_epoch)
PendingIntent = {intent_id, context, target, patch, created_at, state}
RequestEnvelope = {request_id, runtime_epoch, expected_version, target, patch}
UndoRecord = {context, target, confirmed_request_id, before, after, expires_at}
```

UI 意图与已发网络请求分开。意图在第一次 dispatch 时结合最新的同一对象快照形成请求；一旦进入 `InFlight`，`request_id`、版本、目标、payload 全部冻结。网络重试重放这个请求，不把旧请求的 payload 换成最新 revision。

调度规则：

- 同一 `(context, target, field)` 的连续绝对 gain 合并为最新值。不同通道或不同字段独立保存。
- Mute/Solo 保存用户选择后的布尔值，不保存到服务器执行的“toggle”。这样重试不会反转两次。
- Stop、Disconnect、Revoke、Remove 是有序离散操作，不与推子合并。目标终止意图成为该目标的屏障，取消尚未发出的混音意图。
- 首版每个房间保持一个普通写请求在途，避免扩大服务端 CAS 冲突。字段槽按首次排队顺序公平调度，拖动一个推子不能饿死其他通道。
- 建议最多 128 个未发意图；相同字段可在原槽更新。容量满时拒绝新增并显示“操作过多，请等待”，不静默丢最早操作。本地停止走 D04 独立路径。

命令状态为 `Queued → InFlight → Acknowledged / Conflict / Failed / Unknown`；未发送意图还可进入 `Cancelled`。`Acknowledged` 表示服务端确认该修改，音频应用仍由 `applied_config_sequence` 表示。失败草稿保留并标明原因；取消草稿移除；权威值不能被长期未确认草稿冒充。

### AirPlay 字段 patch

新增明确的 `PatchMixSource` 能力，字段用 `Option<gain_db/muted/solo>`，`None` 表示不修改；至少有一个字段，gain 必须有限且处于现有范围。服务端在同一临界区校验 source/session、权限、版本，并从当前状态补齐其他字段。持久 gain/mute 和会话 Solo 的不同寿命继续保留。

旧全量 `MixSource` 保留原语义，新客户端通过能力协商选用 patch。旧服务端兼容路径只允许：刷新同一会话 → 将用户修改字段叠到该快照 → 使用该快照的 revision 提交；若遇冲突，显示冲突，由用户重试。不能拿旧 triple 配新 revision，也不能在冲突后自动覆盖另一客户端的新值。

### 切换上下文和 Undo

切换房间或身份时递增 `context_epoch`，取消旧上下文未发意图，清理其草稿和 Undo。旧在途请求可能已经执行，不能假称取消成功；其结果只更新原上下文记录，不写入新房间状态。地址发现只能更新相同固定 Hub 身份的连接地址。

只有确认成功的混音修改生成 Undo；保留现有 8 秒窗口，但从确认时开始。`before/after` 取自实际提交成功的那次字段变更；合并后的拖动不能沿用已被取代意图的 inverse。Undo 必须检查同一 context、同一会话，且目标字段当前值仍等于 `after`。随后以最新同对象版本提交只包含相关字段的 inverse；服务器 CAS 负责挡住读取与提交之间的并发变化。字段已变化时显示“此设置已被修改，无法直接还原”。

没有修改其他字段的 Undo 不恢复整个对象快照。对于回复丢失、结果未知的请求，不提前开放 Undo；先用相同 `request_id` 对账。幂等回执过期或 Hub 重启后无法确定因果时，显示未知结果，不自动制造新请求再执行。

### 空列表和键盘行为

在 `mixer_keys` 最前面处理 `lanes.is_empty()`：清空失效选择并返回。非空列表中选择不存在时，方向键可选首路；M/S 不对不存在目标执行。现场、行式 Mixer 和控制台共用约束，保留文本框、IME、Modal 和命令面板对快捷键的屏蔽。

### D01 checklist

- [x] 在现有 egui 测试中覆盖空房间、末路移除、过期 selection 和键盘焦点，不另造只复述 guard 的测试。P01：真实 egui 输入帧覆盖两种布局；Desktop 45 项通过，见 `p01-mixer-unfixed.log` / `p01-desktop-suite.log`。
- [x] 用可控响应夹具延迟第一写，交错 B gain、C mute、B solo，验证所有目标服务端值及草稿终态。P02：实际 Desktop/Authority 状态模块，见 `p02-intent-tests-verified.log`。
- [x] 覆盖“mute→gain”“solo→gain”以及另一客户端更新 mute 的场景。字段 patch 与同会话全量兼容解析分别验收，未知重放不重新解析 triple。
- [x] 在排队、发出、收到回复、Undo 四个阶段切换 A/B 房间，并加入同房间更换身份。运行代次及 session 复用同步失效旧上下文/草稿。
- [x] 记录实际发出的 envelope，断言旧请求未改写目标或 request_id。实际 IPC 回归验证身份替换拒绝、冻结元数据、原 request ID/revision 转发和临时文件清理。
- [x] 验证队列容量、公平性、目标结束屏障、重复点击和网络结果未知。128 槽满时拒绝新增，轮询占用 worker 时不虚构 InFlight；Unknown 仅重放冻结请求。

## 五 D02 Lane 绑定和音频时间线

涉及 B02、B12、B20。主要代码：[Mixer](../crates/audio-core/src/mixer.rs)、[block queue](../crates/audio-core/src/queue.rs)、[Ingress](../crates/airplay-adapter/src/ingress.rs)、[Hub 资源提交](../apps/hub/src/server.rs)。

### 控制面决定播放方式

扩展 `LaneMix` 或加入 `LaneBinding`：`lane_id`、`binding_generation`、`stream_id`、`stream_epoch`、`playback_kind`、`output_epoch`。`playback_kind` 采用 `NativeAdaptive` 或 `Timed`，与 gain/mute/solo 一起通过现有有界配置通道发布。

callback 首先应用控制配置，再按绑定验证队列 generation、stream、epoch、format 和数据类型。`Timed` 要求有效 PTS；`NativeAdaptive` 拒绝 timed 数据。移除 `next_is_timed()` 对持久播放模式的决定权。无效旧包只增加 rejection，不重设模式、时间锚点或增益包络。

绑定变更重置 DSP、预取段和恢复淡入。旧队列元素可以继续占用物理环形槽，但只能被有界清理；每次回调最多扫描现有队列容量，不等待生产者排空。输出重开使用新的 `output_epoch`，全部旧 deadline 和旧原子电平快照失效。

### 时间坐标不能用接收数量替代

在同一 format/mapping epoch 内，Ingress 维护锚点 `(p0, rate, q0, pts0)`。48 kHz 归一化位置按原始源位置计算：

```text
q(p) = q0 + floor((p - p0) * 48000 / source_rate)
gap  = q(next_packet) - previous_normalized_end
```

计算使用足够宽的整数和范围校验；从固定锚点换算或维护余数，不能逐包四舍五入后累加。PTS 映射继续保持一个 epoch 内固定的低延迟平移；源坐标与 PTS 是相互校验的两种信息，不用到包时间替代媒体时间。

归一化 PCM 已经经过 SRC。实施时应先明确 worker header 表示“本包第一个输出样本对应的源坐标”，并覆盖 SRC 延迟与可变分块；若现有字段不足以无歧义表达，先补充内部段元数据或带能力协商的 PCM 版本。不能仅套用上述公式便宣称任意 SRC 分块都正确。

被拒绝或丢失的包不参与已接收帧数累加，但下一包仍保留自己的真实位置。仅 `sequence` 跳号不能直接推断缺多少 PCM；缺口长度以源坐标、格式和 PTS 一致性为依据。mapping/format 改变、位置倒退、超出容差的 PTS 矛盾分别结束旧时间段或拒绝，计数单独记录。

### 分段预取避免丢掉前段尾音

单纯添加 discontinuity flag 不够：当前预取循环在看到断点时会 reset 整个 timed FIFO，可能丢掉尚未播放的前段。建议使用预分配的 PCM 环形区及有界 `SegmentDescriptor` 环形区，描述 `{start, end, pts_anchor, epoch, reason}`。描述符容量须覆盖 FIFO 容许的最小合法包数量，不靠 callback 扩容；不足时由非实时 Ingress 施加背压并计数。

预取到新段时仅入队 descriptor，继续消费旧段。游标到达旧段末尾才处理边界：

- 正向缺口输出与时间长度相同的静音，不把后段往前拼；不为长缺口实际分配海量零样本。
- 同格式的小缺口可保留时钟映射并用零填充边界；大缺口或 seek 在旧段消费完后重建 SRC/servo，等待新段 PTS，再做现有 240 内部帧淡入。
- 后段确实已经晚到时只跳过对应过期样本，并增加 `late_skipped_frames`；不能回退 deadline 或无限补播积压。
- FIR lookahead 不跨段读取未来 PCM。段边界采用零延拓或分段重建，测试补偿已声明的滤波延迟，防止前振铃造成后段信号提前。

首轮可将“保持 DSP 的小缺口上限”设为不超过一个内部块的候选值；它只决定滤波器重置策略，不能改变缺口时长和后段 deadline。最终数值由 44.1/48 kHz、可变分块及波形回归确定并写入音频契约，不能用调阈值让失败用例过关。

### DSP 与计量分离

把状态分成 `BindingState`、`DspState`、`MeasurementState`。`ever_started` 属于当前绑定计量，不随 SRC reset 清空。渲染状态为 `Inactive / Priming / Running / Starved / Stopped`；定时首包等待属于 Priming，已开始后缺失本应存在的 PCM 属于 Starved。

| 计量 | 定义 |
|---|---|
| `underrun_frames` | 活动且已起播的 lane 在应呈现位置缺少 PCM 的 48 kHz 内部帧；每帧最多计一次 |
| `timeline_gap_frames` | 从时间坐标确认的缺口长度；是原因计数，不能与 underrun 相加当总损失 |
| `late_skipped_frames` | 因 deadline 已过而跳过的真实样本 |
| `rendered_pcm_frames` | 实际消费的有效 PCM 帧，静音 PCM 也算有效数据 |
| `render_state` | Mixer 当前消费状态，不从 MediaWorker 最近一次交包推断 |

初始预缓冲不计欠载；明确暂停、停止、移除后的计划静音不计欠载。mute/solo 是混音决策，PCM 健康仍可独立统计，不能因静音隐藏断流。连续断流一秒在 48 kHz 内部域应计 48000 帧，任意实际输出采样率、任意 callback 分块均保持同一口径。恢复重置 DSP，但保留当前绑定累计量。

### D02 checklist

- [x] 通过真实 `Mixer + block_queue` 覆盖 timed→native、native→timed、旧代头、lane remove/reuse 和 output reopen。 P03：`p03-time-release-verified.log`；波形周期 1/127/480/1024，深 backlog 另含 256/1024/2048。
- [x] 通过真实 `Ingress + Mixer` 输入两个可识别波形，A 为 100–110 ms，B 从 120 ms 起；B 不提前、A 尾部完整。 P03：`p03-time-release-verified.log`；波形周期 1/127/480/1024，深 backlog 另含 256/1024/2048。
- [x] 覆盖起播前预取、运行中缺口、拒包、重复包、位置倒退、mapping 切换及 starvation 后恢复。 P03：`p03-time-release-verified.log`；波形周期 1/127/480/1024，深 backlog 另含 256/1024/2048。
- [x] 44.1/48 kHz 和可变 frame_count 的相同媒体时间得到一致位置，无累计舍入漂移。 P03：`p03-time-release-verified.log`；波形周期 1/127/480/1024，深 backlog 另含 256/1024/2048。
- [x] 输出时钟夹具覆盖不同设备周期与积压；按实际呈现时间断言，不按 `sleep` 的执行时刻断言。 P03：`p03-time-release-verified.log`；波形周期 1/127/480/1024，深 backlog 另含 256/1024/2048。
- [x] 原生和 timed 断流精确计数；停止后不累加，恢复后累计值不清零。P03：release 真实 Mixer 覆盖一秒 48000 内部帧、1/127/480/1024 分块、初始 Priming、停止、恢复及有效静音 PCM；见 `p03-components-verified.log`。
- [x] 扩展 `mixer_realtime` 验证新分段、重绑定、缺口和撤销路径均无 callback 分配、释放及无界扫描。 P03：`p03-time-release-verified.log`；波形周期 1/127/480/1024，深 backlog 另含 256/1024/2048。

## 六 D03 AirPlay 准入与会话隔离

涉及 B06、B07。主要代码：[worker](../apps/airplay-worker/main.cpp)、[Hub AirPlay owner](../apps/hub/src/airplay.rs)、[IPC 类型](../crates/airplay-ipc/src/lib.rs)、[AirPlay IPC 合约](AIRPLAY-IPC-CONTRACT.md)。

### 每个建立阶段都可取消

统一 Hub 与 worker 的阶段定义：

```text
Idle → AdmissionPending → AdmittedAwaitGrant → Active → Closing → Idle
               各阶段的 Reset / Disconnect / Revoke 均可进入 Closing
```

会话关联键为 `(receiver_id, worker_generation, connection_id, request_id)`。Hub 分配 session/epoch 后再加上完整 context；未 grant 阶段由关联键证明目标身份，不要求 worker 已持有 Hub 的非零 session。Active 阶段同时核对关联键与 context，禁止全局接受零 context。

取消处理应完成三件事：立即关闭本 owner 的媒体 gate；锁存该建立请求已取消；异步结束对应连接并清理 admission/decoder/pending PCM。关闭确认和重复取消保持幂等。迟到 `admit`、`session_started`、`grant`、`grant_applied` 不得重新打开已取消 owner，也不得结束新 owner。

worker 在 generation 内使用有界的已取消请求记录，超过既定在途/重放窗口后回收；generation 变化使旧记录整体失效。Hub 在 gate 已关闭、lane generation 已失效后可以释放共享容量，不无限等待 worker；异常 worker 则按 D04 的期限回收并显示该入口故障。新入口接入仍须确认自身 owner 已清空。

### 协议音量属于会话

将 `protocol_gain` 与 decoder、context、待发 PCM 放入 SessionState。第一次接受新 owner 时初始化为与 `volume_initial` 一致的 0 dB，即线性 gain=1；新会话不继承前一来源的协议音量。

重复 SETUP 和同 session flush 只重置媒体时间线/decoder 所需部分，保留当前会话的有效协议音量。显式 disconnect、新 owner、trust 撤销清理整个 SessionState。Hub 保存的来源 trim/mute 与源端协议音量是不同层，不互相覆盖。

### D03 checklist

- [x] 在真实状态机接线处设置 barrier，分别于 admit、session_started、grant、grant_applied 前后注入取消。P04：Worker 五个实际接线 barrier，Hub 实际 claim/gate/lane 与迟到事件 barrier；见 `p04-final-barriers.log` / `p04-hub-cancellation-barrier.log`。
- [x] 覆盖重复取消、乱序 grant、新旧 connection/request、worker generation 更换；另一来源持续播放。macOS 双来源 Hub 数字恢复完成四次 disconnect/crash 故障与恢复，见 `p04-two-source-recovery.json`；其他平台及 Apple 仍分项。
- [x] 取消后容量、claim、gate、lane 与 worker owner 收敛，无“Hub 空闲但 worker 占用”。Hub 关闭待确认 owner 保留五秒期限，期间不再发布；worker Closing 拒绝新 SETUP，独立 cleanup 后才空闲。
- [x] 同 worker 依次接入 A/B，A 分别静音、低音量、正常音量，B 未发音量命令仍使用初始值。实际签名 pair-verify、加密 UDP 与 PCM IPC 六项场景通过；见 `p04-session-complete.json`。
- [x] 同会话暂停恢复、flush、重复 SETUP 不重置合法协议音量；完整 worker 探针覆盖，白盒函数测试仅作补充。验收合法重复 stream SETUP；重复 initial key SETUP 保持 Alpha 455 拒绝规则，不扩展协议。

## 七 D04 后台生命周期和本地资源

涉及 B08、B09、B15、B16、B17。主要代码：[daemon](../crates/desktop-service/src/daemon.rs)、[本地协议](../crates/desktop-service/src/lib.rs)、[Hub](../apps/hub/src/server.rs)、[Sender](../apps/hub/src/sender.rs)、[Linux virtual output](../apps/audio/src/virtual_output.rs)。拟议实现应整合工作区已有 `crates/lifecycle`，其未提交状态不视为已验收能力。

### 职责拆分与停止可达性

逐步将 Runtime 拆成四个 owner，不要求一次性移走所有代码：

| owner | 独占资源 | 消息与等待规则 |
|---|---|---|
| `LifecycleManager` | Child、实例 generation、start/stop operation | 不等待远端 HTTP 或持久化；最终回收不随客户端断开取消 |
| `RemoteController` | 认证客户端、远端请求及回执 | 长操作在独立任务运行；完成消息携带原 context |
| `PersistenceCoordinator` | 配置事务和私有文件锁 | 按配置域串行；不持有生命周期锁执行 fsync |
| `StatusPublisher` | 不可变状态快照 | Status 只短时复制，不等待上述长操作 |

停止通路必须从连接接入处就可达。仅给内部 mpsc 队列更高优先级仍可能被普通 IPC 连接占满；建议使用同等级用户认证、限长的独立本地 lifecycle endpoint，或提供等效的独立 accept/配额。它只接受闭合的 Status/Stop/Shutdown，不执行远端命令。

已认证的 Stop 先在 manager 中锁存 `stop_generation` 并返回 `Accepted(operation_id, instance_generation)`，再执行收尾。重复 Stop 合并到同一 operation；Shutdown 拒绝新 Start。较早启动任务晚到时，只能看到更新后的停止 generation 并立即回收刚创建的 Child，不能重新把状态发布为运行。

远端变更已可能提交时不强行取消其业务事务，但其等待不阻止本地媒体退出。Stop 以 owned 子进程的实际生存期为准，远端会话撤销失败另外报告；本地不再采集/发送必须能够独立兑现。

### 启动和退出使用明确状态

```text
Stopped → Starting(operation_id, generation) → Ready
                      └→ Failed(reason)
Starting / Ready → Stopping(operation_id, generation) → Stopped(result)
```

spawn 仅证明进程创建。Hub 的 Ready 至少要求配置装载成功、控制 listener 已绑定、所选实体输出满足当前启动契约；AirPlay 入口就绪单独报告，不能用一个 Hub ready 掩盖入口失败。Sender Ready 要求采集启动、远端 Start 获准及媒体传输准备完成；“尚未收到采样/采样为静音”是后续信号状态，不伪装成启动失败。

Ready 由结构化事件专用字段锁存，携带当前 generation。普通日志不会覆盖它；进程退出必须清掉 Ready。启动接口首版可统一返回 Accepted，再通过 status 查询终态；若保留同步等待，达到等待上限只能返回 Starting 或超时错误，不能返回成功 Ready。

停止结果建议包含 `graceful`、`forced`、`elapsed_ms`、`exit_code`、`cleanup_complete` 和未完成资源列表。每个 owned 资源都要尝试收尾并聚合错误，不能第一个失败就提前 `?` 返回。

采用与 Windows 专项方案一致的初始总预算：停止控制写入 250 ms，Hub 正常收尾 5 s，包含多个 worker 的并发收尾；完整停止最多 10 s，超时进入强制回收并报告结果。新增建议目标：健康本地 IPC 下，接收停止意图和读取 Status 的 P95 均不超过 250 ms，不能受 20 s 远端夹具拖延。以上是待测设计目标，不是已有性能保证。

UI 仅在收到 Completed 或通过可信实例 generation/进程对象确认目标已退出后显示“已停止”。Shutdown 和 Status 都失败时显示“尚未确认后台已退出”；仍可允许用户单独关闭窗口，但不能把该动作显示为音频已停止。

### 后台崩溃的默认策略

选择“后台意外死亡后，本实例媒体退出”，首版不做跨后台收养。这与关闭 UI 保持音频不同：UI 不是媒体 owner。`kill_on_drop` 仅覆盖 Child 被析构的情形，不能作为父进程被强杀后的回收证明。[Tokio process 文档](https://docs.rs/tokio/latest/tokio/process/struct.Command.html#method.kill_on_drop)

- Windows：用私有、不可继承的 Job Object 约束完整媒体进程树；子进程在运行用户代码前加入 job，禁止启动竞态逃逸。关闭最后一个带 `KILL_ON_JOB_CLOSE` 的 job handle 可终止所属进程，但不会替代正常清理。[Microsoft Job Objects](https://learn.microsoft.com/en-us/windows/win32/procthread/job-objects)
- macOS/Linux：以只由后台持有写端的私有父子存活管道作为协作停止入口，EOF 触发收尾；严格限制描述符继承，孙进程不能意外持有父端。仅 EOF 不足以回收完全卡死的子进程，需要独立 watchdog/supervisor 在父退出后对原进程树按期限升级终止，且其验证不依赖媒体主线程继续运行。
- Linux 可补充父死亡信号等平台机制，但必须验证设置与父退出的竞态、worker 后代覆盖及容器/桌面环境行为；不能把单个进程收到信号当作整个树已退出。

回收只作用于创建时拥有的进程句柄/出生身份，不凭磁盘 PID 或进程名全局杀进程。下一次启动在 owner 锁下清理本模块遗留运行文件；配对资料和持久身份不能因崩溃重建。

### Linux 多实例和桌面持续发送

B15 首版采用明确限制：同一 UID 的 NeonMix 虚拟输出只允许一个具备匹配实例身份的 owner。与 owner 握手验证协议、实例 generation、绑定 UUID；另一个 state_dir 请求复用时返回 `ResourceOwnedByOtherInstance`。允许两个 UI 连接同一个后台，但不静默让两个后台共同拥有节点。发现旧 socket 时核验 owner，再按现有私有目录/锁规则清理，不能仅依据路径存在判定可用。

完整的 UID 级共享 owner 服务属于后续扩展，需要租约、统一绑定配置和退出引用计数，不进入本轮关键路径。

B17 将运行期限改为显式的 `RunLimit::UntilStopped | Duration` 或等效 `Option<Duration>`。桌面使用 UntilStopped；实验 CLI 保留显式 `--seconds`，并公开 `UserStopped / DurationElapsed / Failed` 结束原因。不要把 0 暗中用作无限时长，也不靠每 24 小时自动重连维持表面连续。

### D04 checklist

- [x] 20 s 远端变更、普通连接配额耗尽、状态查询风暴期间，Stop 均可登记并最终执行。P05：真实本地 IPC/Child，20 秒 mutation、7 个未完成普通连接及 20 次状态读取，Stop 不到 2 秒完成并回收；`p05-independent-lifecycle-final.log`。Windows/Ubuntu 原生仍分项。
- [x] 客户端在收到回复前断开，Stop 继续；Start/Stop 交错不会出现停止后晚到启动。P05：真实断开 socket、stdin Stop→迟到 Ready、owner 代次状态及 IPC 回归；Desktop 在创建时冻结实例/停止代次，追加矩阵进行中。
- [x] 即刻 exit、永不 ready、迟到 ready、ready 后普通日志、ready 后 exit 五种夹具均准确显示。P05：真实 Child 五场景与真实 IPC Ready→stats 回归通过；`p05-ready-final.log`。独立停止/operation 和三端强退仍待后续。
- [ ] 三端真实独立进程强杀后台，检查 owned 进程树、端口、profile 锁及运行文件；无其他实例误杀。
- [ ] 卡住 worker、堵住控制管道、单个清理失败；其余资源仍回收，正常/强制结果不混淆。
- [ ] Linux 双 state_dir 明确拒绝冲突；退出 A 不破坏独立 B 的其他资源，重启可重新取得 owner。
- [ ] 可控时钟跨 24 小时验证持续/限时模式，再以真实长测观察资源增长。
- [ ] 关闭窗口和 UI 崩溃仍保持媒体；后台退出则回收，两个生命周期分别验收。

## 八 D05 持久化和输出绑定

涉及 B10、B13、R02。主要代码：[私有文件写入](../crates/identity/src/files.rs)、[输出绑定](../crates/output-binding/src/lib.rs)、[Hub identity](../apps/hub/src/identity.rs)、[持久状态保存](../apps/hub/src/server.rs)、后台 HubSettings。

### 先补崩溃恢复 再消除重复权威字段

当前 state 和 profile 都保存输出 ID，且启动严格检查一致。直接删掉其中一份会改变资料格式和旧版本启动行为。首个修复采用受控的可恢复事务日志，后续再迁移到单一权威字段，避免把格式迁移做成 B10 修复的前置条件。

事务日志放在同一私有 profile 目录，使用现有权限检查和稳定 owner 锁；仅容许固定的 state/profile 文件。记录 `transaction_id`、schema、旧/新 revision、两份目标内容及校验和，保持有界。日志可能包含敏感配置，不能进入普通诊断导出、CI 工件或公开证据。

建议流程：

1. 按全局一致顺序获取 profile/state 锁；拒绝另一个正在运行的 owner；先处理未完成日志，再检查当前配置。
2. 完成目标配置校验，写入目标临时文件并同步；写入并持久化 `Prepared` 日志。
3. 持久发布 `CommitDecided` 决定。该决定稳定后采用向前恢复，不再尝试把一份新文件回滚成旧文件。
4. 逐个原子替换 state/profile，同步需要的目录；中断后根据日志补齐目标内容。
5. 读回两份目标配置并校验一致；移除日志并同步目录，再返回持久完成。

日志恢复必须先于现有两文件一致性校验。只有 Prepared 时旧文件尚未被替换，可以清理暂存并保留旧值；已存在有效 CommitDecided 时完成新值。若发布决定的耐久结果不确定，当前进程进入恢复状态并重新同步，不能继续替换文件又向用户宣称没有提交。启动时以实际留存且有效的决定恢复，损坏日志明确失败，不生成新身份。

随后单独版本迁移：令 persistent state 的 `output.id` 成为运行输出唯一权威字段；profile 只保留身份、资料引用和房间元数据。旧字段只由旧版本 importer 读取一次。Room name 与 output 若仍要求原子修改，继续通过上述事务协调，不假称删除重复字段就解决所有跨文件原子性。

### 可见性和耐久性分别报告

将低层 `replace` 的结果由普通 `io::Result<()>` 扩展为具备发布阶段的结果，并保留原始错误：

| 结果 | 内存和运行行为 | 对外结果与恢复 |
|---|---|---|
| `NotPublished` | 保留旧权威状态，释放本次预留 | 明确失败，可安全重新准备 |
| `PublishedDurabilityUnknown` | 文件已可见，不盲目 abort 回旧内存；按新配置投影或进入受限恢复状态 | `DurabilityUnconfirmed`；阻止新的冲突持久写，重试同步并对账 |
| `Durable` | 提交权威状态并触发运行时应用 | 回执标明已保存；音频是否应用另看序号 |

字节相同只能帮助确认内容可见，不能升级成 Durable。对于撤销等限制性修改，只要已发布便保持限制，不因耐久不明重新允许媒体；对于启动等授予能力的修改，耐久未确认前不开放新媒体。防止“为了保证内存磁盘一致”反而重新授权。

三端分别定义实际持久原语和错误分类。当前 Windows 替换策略与 Unix 目录 fsync 不相同，不能以同一个函数名推断完全相同的断电保证。故障注入证明程序恢复路径；若没有掉电测试，不宣称已经证明所有文件系统的掉电耐久性。

### 输出绑定 CAS 包含对象身份

将所有修改接口条件改为 `(expected_output_id, expected_revision)`，覆盖 rename、enable、disable、remove、原生名称同步及后台转发。API/CLI/UI 都必须携带用户读取过的绑定 UUID，不能在服务端收到旧请求后读取“当前 UUID”补上。

现有 `OutputBinding` 已有 `output_id`，可优先只改管理请求和 Store 方法，无须重写文件。删除后重建即使 revision=1，也因 UUID 不同返回 `ObjectReplaced` 或 Conflict。授权 epoch 继续用于 Sender 许可，不替代管理对象身份。

### D05 checklist

- [ ] 在写临时、同步、发布日志、两个 rename、目录同步、日志删除每个边界注入错误和真实进程中断。
- [x] 重启读取走恢复入口，结果为完整旧配置或完整新配置，不能留下半提交无法启动。P06：43 个实际写入进程 SIGKILL 边界、新进程恢复、真实 Hub 产品配置读取和后台 IPC 接线；macOS 文件范围，其他平台/掉电另验。
- [ ] 校验磁盘满、权限失败、损坏日志和未知 schema；不丢原错误、不重建身份。
- [ ] 区分 rename 前失败和 rename 后同步失败；后者不会仅因读回一致变成成功。
- [ ] 耐久不明时重试同一 transaction/request 不产生第二次业务副作用。
- [x] A 删除/B 重建后，A 的 rename/disable/remove/name-sync 旧请求均不能修改 B。P06：Store A/B 同 revision 替换矩阵、真实 CLI dispatch、必填参数与 IPC 冻结转发/回执后同步交错通过；平台 setter/最终包按每平台独立验收。
- [ ] 迁移保留 Hub UUID、证书、receiver 密钥和最新撤销；明确旧版不兼容时的回退限制。

## 九 D06 控制事务和状态复制

涉及 B14、B19、R01，并支撑 B05 的对账。主要代码：[Authority](../crates/control/src/lib.rs)、[HTTP 事务](../apps/hub/src/server.rs)、[控制客户端](../apps/hub/src/control_client.rs)、[媒体 gate](../apps/hub/src/media_worker.rs)。

### 幂等重放在副作用之前结束

将 `prepare_transaction` 返回类型改成 `Replay(StoredResponse)` 或 `Prepared(Transaction)`。在认证和授权有效性检查后，先按身份、runtime、request_id 查已完成结果，再判断是否有其他 pending 事务；已完成重放不应被无关 pending 返回 Busy。

同一个 request_id 还要验证原始请求指纹，包括目标、操作、条件版本和 payload；同 ID 不同内容返回 `RequestIdReused`。新事务才进入容量预留、媒体创建、持久写入和配置发布。HTTP 层对 Replay 立即返回存储的完整业务响应，不再次调用这些副作用。

同 ID 正在执行时返回 `InProgress` 或等待同一结果，不启动第二份事务。完整响应所需参数在准备阶段形成，回执在权威提交的同一串行步骤中发布，之后才向客户端写回复，避免“业务已提交但回执尚未记录”的重放窗口。耐久未确认的请求保存该中间状态，按原 transaction 对账，不当作普通失败重新执行。

原生 `StartResponse` 需要保存第一次返回的 session/endpoint/媒体参数，不根据重放时当前 lane 重建。重放只返回原操作结果，不恢复已经结束的 session。响应可附带单独的当前有效性信息；客户端看到原回执成功仍须核对会话是否活跃。已撤销身份的旧回执不能绕过当前认证重新获得敏感媒体参数。

首版保持有界进程内回执缓存，不承诺跨 Hub 重启 exactly-once。请求带 `runtime_epoch`，旧 runtime 请求在新进程中明确拒绝，要求客户端重新取快照、保留结果未知提示。缓存过期后也不自动重发离散操作；持久 transaction_id 只用于已进入 D05 恢复协议的操作，不能声称覆盖所有历史命令。

### 配置 CAS 与运行时游标分离

新快照至少包含 `{hub_id, runtime_epoch, event_sequence, config_revision, state}`。原生持久配置和 AirPlay 来源配置先保留各自版本域；需要同时读两域时返回各域版本，不伪装成跨域原子快照。

持久配置变更比较对应 `config_revision`。会话混音、Start/Stop 等运行期变化还需要同 runtime 的对象存在性/epoch 校验，以及对应的运行时 CAS 条件；首版可用预期 `event_sequence`，后续若健康事件造成过多无关冲突，再引入对象版本。不能简单把所有请求的 expected_revision 改成配置 revision，使两个运行期写入失去并发保护。

事件协议使用 `(runtime_epoch, event_sequence)`：

1. GET 在同一短临界区复制 state 和游标。
2. WSS 带 epoch 和 after sequence 订阅；同 epoch 且历史仍保留才续传。
3. 跨 epoch、游标超前、历史过旧、事件跳号均返回 `SnapshotRequired`，停止套用增量并重新 GET。
4. 客户端 `apply_event` 仅接受相同 Hub/epoch 且严格下一序号；重复事件可忽略，缺事件不能跳过。
5. 心跳不前进业务状态，不证明副本已同步；高频电平和连续计数留在独立诊断采样，不每个样本制造配置事件。

旧 `revision` 接口通过明确版本/能力协商迁移，不重解释同名字段而让旧客户端静默误用。不能支持可靠续传的旧订阅应明确要求完整快照或升级；轮询读可以保留，但不能宣称已经具备新游标保证。

### pending 事务期间继续发布健康状态

将 Authority 分成持久配置和独立运行状态。PreparedTransaction 保存配置候选及针对原对象的操作，不保存将来覆盖整个 Authority 的旧克隆。设备不可用、会话终止等即时运行变化仍更新实时状态，并递增 event_sequence。

commit 时在短锁内检查 transaction token、原对象身份及适用条件，把持久结果合并到最新运行状态，再发布事件；abort 只丢候选配置，不恢复过去的健康值。已保存的来源偏好即使目标 session 结束也保留，返回“设置已保存，原会话已结束”；不把偏好转成对新会话的临时 Solo 或 Disconnect。

必须覆盖 `available=false → GET → available=true → abort/commit` 交错。中间 GET 和之后事件应用的结果要与权威当前快照一致，同一游标不能对应两个不同的公开状态。

### 撤销 gate 不依赖配置队列

对于提交后进入终态的 session，先使媒体授权 generation 失效并关闭其 gate，再异步尝试 DSP 配置；queue full 只影响配置收敛，不能推迟撤销。Stop/Revoke 等限制性操作在准备阶段也不能被普通 Mixer 容量 preflight 挡住，应与分配新媒体的 Start 路径分开。生产者关闭还不够，已经排队或进入 Mixer FIFO 的 PCM 也必须被拦截。

建议为每个 lane 预分配原子授权 generation，callback 在每个有界渲染块开始时检查绑定值。撤销用 Release 发布失效，callback 用 Acquire 观察；新 session 使用新的 generation，不能重新利用旧 PCM。端点失效不等待文件写入即可阻断运行；持久撤销与耐久不明的限制性行为按 D05 执行。

验收以“失效被观察后的渲染块不再消费旧会话 PCM”为准。正在执行的 callback 和已提交硬件缓冲可能仍有尾部声音，需报告其上界；不能把软件 gate 关闭承诺成物理声卡立即无声。

配置发布分配 `desired_config_sequence`，callback 应用后发布 `applied_config_sequence`。未应用只显示 Pending；停止授权是独立完成项。配置队列反复拥塞应暴露错误和重试次数，不能无限显示已完成。

### D06 checklist

- [x] 第一次成功后注入必失败 save/prepare，再重放；完整响应一致，副作用回调为 0。P01：真实 Hub `execute_native` 回归及 Authority 事务回归通过，见 `p01-replay-after.log`；runtime 和耐久对账待 P07/P06。
- [x] 覆盖 request_id 换 payload、不同身份、缓存淘汰、runtime 更换和 Start 会话已结束。P07：原Actor/HTTP指纹与完整SDK回执回归、v2身份/旧epoch拒绝、128条淘汰后旧Start不能再prepare；见 `p07-complete-replay-matrix.log` 与真实wire重启/重放结果。
- [x] 用 barrier 在 GET 与 WSS 之间重启 Hub；客户端明确重取快照，无幽灵会话。P07：固定release、BlackHole虚拟输出、真实Hub进程退出/恢复及TLS/WSS；旧epoch订阅和旧Start重放拒绝，实际watch客户端两次GET/订阅，新epoch无旧session/stream。见 `p07-control-epoch-final/result.json`。
- [x] 使用真实 `Snapshot::apply_event` 检查事务 commit/abort、健康 false/true、事件丢失和缓冲淘汰。P07：pending 健康实时 publish，提交保留最新状态/事件；10项事务组件回归与真实 Native 保存 barrier/GET handler通过。显式版本分域和WSS重启仍分项。
- [x] Mixer 配置队列满时撤销，检查 ingress gate、FIFO 中旧 PCM 和后续 lane 复用。P07：实际Native Stop/未知Revoke及AirPlay Revoke/Disconnect/Disable→Mixer渲染，另一来源继续；同ID新binding不带旧PCM，retirement尾部也受lease限制。见 `p07-native-fifo-gate-final.log` / `p07-config-progress-core.log`。
- [x] 记录 desired/applied 的推进与超时；界面不把已保存当成已可闻。P07：queue满仍记录desired，同目标retry不增序号，callback Release确认applied；control discard不冒充确认。两秒stalled/失败次数、同epoch新观测、Unknown不重发、过期scope清除及双语/窄窗反馈通过；真实BlackHole callback desired=applied=3。
- [x] 新旧协议混用明确协商或拒绝，不静默丢 patch、epoch 或 UUID 条件。Native v2/AirPlay v3要求各域条件及身份/epoch，legacy数字CAS保持；旧WSS426、混条件/缺条件拒绝，binding缺UUID拒绝；实际wire与IPC矩阵见对应P06/P07证据。

## 十 D07 产品状态和诊断设计

涉及 B11、R03，以及原分析的产品改进建议。沿用 [DESIGN](../DESIGN.md) 和 [UX-CONTRACT](../UX-CONTRACT.md) 的结构、颜色、快捷键与尺寸；本轮改变状态语义和恢复入口，不重做视觉风格。

### 分开控制对象和本机声音任务

顶部显示“正在控制：房间名 · 身份”，取自当前通过固定身份验证的快照；本机 Hub 是否共享和本机 Sender 实际目标另行展示。切换控制房间不改变已经运行的 Sender 目标。未知目标显示“正在确认”，不从当前选中的房间反推。

所有页面共用 D01 的草稿/提交状态：推子旁显示“待提交”“正在应用”或具体失败；不同通道的错误留在对应通道。房间结束或 session 改变后取消旧意图，提示“原会话已结束”。底部消息提供通知，不能成为唯一恢复入口。

### 诊断值带可用性和独立时间

拟议数据类型：

```text
Measurement<T> = Available(value, sampled_at, received_at, runtime_epoch, stream_epoch)
               | Unavailable(reason, last_good)
               | Stale(last_good, age, reason)
```

诊断、控制快照、AirPlay 会话和 Sender 采集各有自己的更新时间。服务端单调时间只在对应 runtime 内可比较，跨机器 UI 的过期判定以本地接收时间加服务端样本年龄为依据，不直接相减两台机器的单调时钟。

必须先校验 `available` 和本磁贴必需字段，再计算健康。可选字段缺失只影响对应项目；不能让一个缺失的 Sender 指标抹掉有效输出诊断。一次失败就将相关采样标记为不可用/过期，可保留 last_good；可以延后重复通知，但不能继续把旧数据画成“刚更新”。

初始建议以超过三个预期采样周期标记 Stale，阈值与实际采样频率同源配置。历史累计错误与近期增量分开：历史曾有 1 次错误不代表现在仍失败；当前无样本也不代表当前错误为 0。收到有效零样本时显示“采样为静音”；收到足够完整、更新鲜且符合相应条件的指标后才显示“正常”。

### 声音路径和恢复入口

| 可观察状态 | 页面表达 | 对应操作 |
|---|---|---|
| 虚拟绑定 enabled，但无采集帧 | “输出入口可用，尚未收到应用音频” | 打开选路由说明、刷新设备 |
| 有采集帧，近期样本为零 | “已收到采样，当前为静音” | 检查应用播放/音量，不报网络故障 |
| 来源 mute 或其他 Solo 排除 | 在该通道显示确切原因 | 根据权限解除本字段状态 |
| 总静音 | 房间核心与总控显示“总静音” | 总控解除 |
| Mixer Starved | “音频供给中断”，显示开始时间和近期缺帧 | 查看来源/网络详情，保留其他正常通道 |
| 实体设备丢失 | “所选输出不可用” | 重试同一设备，或显式选择替代设备 |
| 诊断读取失败或缺必需字段 | “未取得诊断”及上次有效时间 | 重试诊断、展开具体错误 |
| 已保存但音频尚未应用 | “设置已保存，正在应用” | 超时后提供重试/详情，不再次生成新业务操作 |
| 停止尚未确认 | “正在停止”或“停止结果待确认” | 查询同一 operation，保留可达停止入口 |

首次接收流程：选择实体输出 → 用户显式测试音 → 创建/共享房间 → 邀请原生来源或接入 AirPlay → 观察首路信号与输出证据。测试音保持现有低电平与显式设备选择，不自动播放或修改系统默认路由。

首次发送流程：验证目标 Hub 身份 → 确认虚拟输出绑定及权限 → 指导用户设置应用/系统路由 → 显式 Start → 分别观察采集和远端接收。远端诊断无权限或不可用时明确标记，不能用本机发送计数替代远端接收证据。

现有 Mixer 电平已提供近期 Peak/RMS，应该首先纠正绑定和新鲜度，再考虑额外电平。输入信号、混音后数字输出和真实扬声器可闻性分别表达；RTT、FIFO 时间和 callback 推进都不能标成已测端到端延迟。

### CLI 同机反馈环

在 `open_capture` 和 `capture.play` 之前执行判断。先用实际连接的本地/远端 socket 地址及当前本机接口归属确定是否同机，兼容 IPv4、IPv6、scope 和解析后的地址；再验证目标 Hub 固定身份及本机已登记的 Hub 实例，按后端规范化 endpoint ID 比较 capture 与物理输出。

同机且同端点返回明确 `LocalFeedbackLoop`，不同设备或真实远端不拒绝。显示名称相同、地址文本相似都不充分。首版不向 LAN 公布通用机器指纹；若本机身份/接口查询失败，显式返回检查不可用，不以未知当作已经确认安全或已经发现回授。该规则针对 CLI 路径，桌面同实例互斥继续保留。

### D07 checklist

- [x] Available 全零、available=false、缺必需字段、Stale、有效非零错误分别渲染正确。P08：真实Desktop健康解释与双语呈现，历史错误/近期增量、无新采样及有效静音回归通过，见 `p08-desktop-scoped-verified.log`。
- [x] 快照持续成功、诊断持续失败时，诊断时间不被刷新，旧电平不继续伪装新信号。P08：实际PollData/process交错保留last_good时间，历史写入缺样本；现场/行式/控制台/顶栏统一按独立clock读取，见 `p08-desktop-scoped-verified.log`。
- [x] 现场/Mixer/Sender/诊断使用同一状态解释，切换控制房间不改变实际发送目标文字。P08：共享Lane/诊断clock、真实进程日志锁存及房间/身份切换回归；截图同时显示控制Studio B/发送Studio A，`p08-sender-600-zh-CN.png`。
- [ ] 最小 600×440、标准/宽窗、键盘、真实中文 IME、VoiceOver 及平台可用辅助功能分别验收。
- [ ] 本机 LAN、loopback、IPv6 同端点在采集前拒绝；远端同名/不同 ID、同机不同端点允许。
- [x] 脱敏导出仅扩展显式白名单，携带指标可用性和年龄，不带身份、地址、路径或凭证。P08：实际process日志锁存/Status年龄和export_whitelist回归，新增三个数值字段，sender_target及未知键剔除，见 `p08-semantic-tests-final.log`。

## 十一 D08 构建门禁和发布包

涉及 B18 和原分析的 CI 建议。主要代码：[macOS 打包](../tools/package_macos.py)、[通用检查](../tools/check.py)、[CI](../.github/workflows/ci.yml)、[Windows 检查](../.github/workflows/windows.yml)。

### 打包修复和独立验证

RPATH 按 `LC_RPATH` load-command 结构识别，提取完整 `path … (offset N)`，允许空格和 Unicode；不能全局匹配任意 path 行。删除/添加 RPATH 的 `install_name_tool` 返回值必须检查，预期不存在的路径与实际改写失败分别处理。

修改器与发布验证器不共享同一个路径提取实现。发布侧采用独立 Mach-O load-command 读取方式，检查所有主程序、helper、framework、dylib 和插件；允许系统库与包内相对路径，拒绝开发目录绝对引用。同时检查 dylib install name 和依赖引用，避免只验 RPATH。

在含空格及 Unicode 的构建/安装目录制作最终包，再复制到另一目录，在开发运行库不可访问且未设置开发环境变量的环境启动。验证 ready、最短媒体通路和实际加载来源。文本 parser 用例、`otool` 清单通过和真实包启动是三层不同证据。

### 基础 CI 前移 原生验收保留

仓库目前明确采用 manual-only 的检查工作流，安装器只在专用分支/手动触发；这是现有配置策略。建议新建轻量 PR/main 基础门禁，保留昂贵原生与安装任务按需触发，不在本次文档任务中直接改工作流或分支保护。

| 层级 | 触发建议 | 验证范围 | 能关闭的风险 |
|---|---|---|---|
| 基础门禁 | 每个 PR/main | fmt、严格 Clippy、纯状态/队列/协议回归、相关平台编译 | 代码与确定性行为回归 |
| 原生组件 | 对应组件变更或手动 | Windows worker/runtime、macOS 实体 I/O、Linux PipeWire、原生 IPC/生命周期 | 平台调用及进程机制 |
| 质量矩阵 | 发布候选，或有专用节点的定期任务 | 多源、故障注入、恢复、数字输出测量、长测 | 指定工况音频和恢复质量 |
| 最终包 | 每个发布候选 | 全新/升级安装、重定位、启动、最短完整媒体、停止卸载 | 真正交付制品可运行 |

先确认 job 在目标分支确实执行且返回稳定名称，再配置 required checks。Windows/Linux 检查媒体链时须显式采用 `--native-media` 或等效范围，不能让 `tools/check.py` 的平台默认排除把媒体代码漏掉。无声卡的托管 runner 不能标注实体音频通过。

涉及音频效果和性能的运行使用 release。日志记录 commit、工作区差异指纹、依赖/插件/安装包哈希及工具参数。失败首样本和未执行项保留，不靠不断复跑直到一次成功关闭间歇问题。

### D08 checklist

- [x] RPATH 解析覆盖普通路径、空格、Unicode、相对 loader 路径和多个 load commands。P01：文本回归及真实 Mach-O 夹具通过，见 `p01-rpath-after.log`。
- [x] `install_name_tool` 失败导致打包明确失败；独立验证能捕获修改器故意留下的错误。P01：独立二进制读取覆盖薄/通用、32/64 位、两种端序及损坏头；最终包重定位仍待 P10。
- [ ] 最终包在无开发依赖的重定位环境完成启动、媒体和停止。
- [ ] 基础 CI 自动触发、缺失/跳过 job 处理、required check 名称经过实际 PR 验证。
- [ ] Windows 发布同时满足现有专项计划的防火墙开启、非 ASCII 路径、完整包升级及正常退出条件。
- [ ] 每平台单独报告通过范围，macOS 通过不关闭 Ubuntu/Windows 历史失败。

## 十二 开发批次与依赖

### 建议执行顺序

单人开发建议按 `P00 → P01 → P02 → P04 → P03 → P05 → P06 → P07 → P08 → P09 → P10 → P11` 推进。P04 的会话音量与准入取消相对局部，先于复杂时间线改造落地；若退出失效已在当前主用平台复现，P05 应立即提前到 P01 之后，不等待音频重构。

P00 开始就把可执行的回归接入基础门禁，不等 P10。P10 负责最终包和发布约束收敛。每批使用独立、可审查的提交/PR；“批次”不是必须一次合并的大补丁。

以下人日为初始估算：熟悉当前代码的一名开发者，包含实现、相关自动测试和局部验证，不含硬件等待、未定位历史故障和完整认证分发。总计 **46–75 人日**，约 9–15 个五天工作周；这是工作量范围，不是交付日期。P00 结束及 P03/P05 技术验证后重新估算，不能靠取消关键验收压缩数字。

| 批次 | 交付物与问题 | 依赖 | 预计人日 | 合并出口 |
|---|---|---|---|---|
| P00 基线与夹具 | 固定工作区基线、覆盖表、barrier/时钟/存储 fault 夹具、轻量 CI 方案 | 无 | 1–2 | 问题可稳定定位到当前代码，既有改动归属明确 |
| P01 局部修复 | B01 guard；B18 parser/错误处理；B19 Replay 分支 | P00 | 2–3 | 三个独立小变更，回归分别证明旧行为与新结果 |
| P02 意图与 patch | B03–B05；ContextKey、字段调度、Undo、AirPlay patch 和能力识别 | P01 | 4–6 | 慢网、多客户端、跨房间确定性矩阵通过 |
| P03 音频绑定与时间 | B02/B12/B20；LaneBinding、计量分离、时间坐标和分段预取 | P00；接入 P04 会话语义 | 6–10 | 实际 Ingress+Mixer 波形、计量、实时约束通过 |
| P04 AirPlay 状态机 | B06/B07；阶段取消、关联键、会话 gain | P00 | 3–5 | worker 与 Hub 双方 barrier 回归及完整 worker 探针通过 |
| P05 生命周期 | B08/B09/B16；manager、独立停止、Ready、平台回收 | P00；整合现有 lifecycle 改动 | 7–11 | 三端停止/强退/启动夹具，实例隔离通过 |
| P06 存储与绑定 | B10/B13/R02；可恢复日志、写入结果、UUID CAS | P00；与 P05 配置 owner 接口对齐 | 6–10 | 写入全边界中断后可恢复，绑定旧请求不误写 |
| P07 状态与应用进度 | B14/R01；完善 B19 回执及 runtime 边界 | P01/P03/P06 | 5–8 | 副本收敛、队列饱和撤销、Unknown 对账通过 |
| P08 状态产品化 | B11；行内命令状态、诊断 freshness、声音路径 | P02/P03/P05/P07 | 3–5 | 数据语义、真实 UI/IME/辅助功能按平台验收 |
| P09 本地边界 | B15/B17/R03；Linux owner、持续 Sender、CLI 防反馈 | P05/P06 | 3–5 | 双实例、可控 24 h、同机地址/端点矩阵通过 |
| P10 发布包 | B18 完整关闭；版本能力、重定位、升级和 CI 范围 | P01/P05/P06/P07/P09 | 2–4 | 最终候选包通过安装与最短完整媒体路径 |
| P11 联合质量验收 | 四路组合、真实 Apple、跨机、故障恢复和长测 | P03–P10 | 4–6 | 按平台输出发布结论及未闭环项，不跨平台外推 |

### 批次拆分要求

P03 至少拆成三步：先显式绑定，随后计量语义，最后时间段/FIR 边界。每一步保持可运行，不能把时间线变化、缓冲扩容和调度参数修改混在一起。保留原来的队列容量、时钟/迟到策略和恢复包络，确有必要变化时单独给出证据。

P05 先拆 Stop/Status 与长锁，再接 Ready/operation，最后接平台强退保障。已有 `crates/lifecycle` 是集成输入；先确认谁持有 job/管道、谁传入 managed 模式及孙进程继承规则，再扩展调用链。不能只因新增 crate 存在就勾选 B09。

P06 先提供日志恢复和三态存储结果，再改绑定 CAS；去掉 profile 重复输出字段是独立资料迁移，可在安全修复完成后另行发布。P07 先提供新游标与兼容层，再切客户端，最后去掉旧隐式语义。

### 开发总 checklist

- [x] **P00**：冻结本轮源码与未提交差异；为 B01–B20/R01–R03 建立“复现、实现、自动测试、原生验收、证据”记录。见本轮 `baseline.json` 和覆盖表；既有改动为输入，基础回归已接入 `stability.yml`/`review_check.py`，远端实际运行待 P10。
- [x] **P01**：空列表、RPATH、幂等重放分别落地；不把 parser 用例通过写成最终包通过。三项均保留修复前失败与修复后证据；完整包待 P10，runtime 对账待 P07。
- [x] **P02**：所有写入口和 Undo 使用同一调度器；快捷键、快速操作、现场、Mixer、AirPlay 面板均接入。`commands.rs`/`intent.rs` 覆盖 Native control/AirPlay V2；本地生命周期/资料事务保留各自 owner，由 P05/P06 验收。
  - [x] P02 服务端字段 patch：能力显式声明；gain 不覆盖 mute/solo；solo 不持久化；空 patch、非法 gain、旧 session 在保存前拒绝。3 项真实 API 接线回归通过，见 `p02-airplay-patch.log`。
  - [x] P02 Desktop 调度、上下文、确认后 Undo 和能力兼容接线。9 项新增交错回归及原生 IPC 转发回归通过；必要 runtime 能力缺失明确拒绝。Windows/Ubuntu 运行及真实 IME/辅助功能仍按 P08/P10/P11 分项验收。
- [x] **P03**：lane 绑定、计量、时间线三步独立通过；旧头复用及缺口预取真实组件回归闭环。基础门禁、加速两分钟时钟偏差和完整加密 worker 六场景通过；平台/数字输出/长测继续分项。
  - [x] P03 播放模式由控制面设置；旧 timed/native 头、同身份模式切换及中途输出重开回归通过。完整 generation/授权原子及旧代拒绝组件回归通过；Hub 全撤销事务接线仍由 P07 验收。
  - [x] P03 计量分离及精确断流回归：3 项计量、4 项模式/重开、8 项 native Mixer、11 项 timed、2 项音质及实时零分配通过；见 `p03-components-verified.log`。实体输出与长期资源另验。
  - [x] P03 Ingress 源时间坐标、分段预取、FIR 边界及联合波形。PCM v2 提供显式位置，真实 decoder 及 Ingress/Mixer 变分块、拒包、两段波形和零分配回归通过；完整 worker 六场景及本地门禁通过，见 `p03-worker-complete.json` / `p03-gate-verified.log`。
- [x] **P04**：Hub/worker 任一建立阶段可取消；音量初始化与同会话 flush 语义一致。实际状态 barrier、完整 worker 六项、既有七项 SETUP 边界、双来源数字恢复通过。宏观发布/平台矩阵及 Mixer FIFO 授权隔离仍由 P03/P07/P11 完整验收。
- [ ] **P05**：停止入口从 IPC 接入到进程回收全程独立；三端强退和 UI 关闭行为都验证。
  - [x] P05 Ready 独立锁存，不依赖 last_event；Hub/Sender 只在业务 Ready 后确认，超时/即退/迟到 Ready 清理 Child 后报告失败；五场景与 IPC 回归通过。
  - [x] P05 独立 lifecycle endpoint、Stop operation、Start/Stop 代次屏障及桌面不虚构停止成功；服务/Desktop 回归通过。
  - [x] P05 Unix 独立 guardian：私有存活管道、5 秒升级、未 reap 的组 leader 防止 PID/PGID 复用；macOS 后台 SIGKILL/卡死后代/另一实例持续、临时密钥与持久资料隔离通过，见 `p05-parent-kill-verified.json`。日志消费者堵塞约 5.5 秒回收通过；固定源与最终包继续核验。
  - [ ] P05 原生完整媒体链、三端强退/实例隔离、真实 UI 关闭/崩溃及最终包 guardian 接线验收。
- [ ] **P06**：故障注入覆盖每个发布边界；恢复发生在严格配置校验前；UUID 条件贯穿全部管理入口。
  - [x] P06 底层发布阶段结果：NotPublished / PublishedDurabilityUnknown / Durable；七个实际文件边界故障及字节相同但目录同步失败的回归通过，见 `p06-publication-phases.log`（macOS）。
  - [x] P06 双文件日志与恢复入口：Prepared 保旧 / CommitDecided 向前恢复；43 个独立进程中断边界、权限/校验和/schema/身份不变、重试 revision 不增加及实际启动/IPC 接线通过。
  - [x] P06 UUID CAS：Store、CLI、IPC、Desktop、Local Forget 和 Sender 原生命名入口均带捕获 ID/revision；同步回调全程锁存管理对象，旧客户端缺 ID 明确拒绝。
  - [x] P06 Native HTTP/配对耐久未知：保留原candidate/request/预备媒体，确认前不授予，published revoke保持拒绝并独立关闭gate；原请求重试/管理员同epoch恢复与配对同device对账回归通过。
  - [x] P06 AirPlay 发布未知的授予/限制边界：原命令/candidate/owner 冻结；Enable/Allow/Repair/配对信任确认前不授予，撤销在满队列下仍关闭 gate；部分配置重试复用 UUID，worker 写入与 Admin/epoch 恢复接入。Hub76项及严格Clippy通过，见 `p06-airplay-recovery-hub-verified.log`。
  - [ ] P06 三端原生故障/掉电语义、资料迁移及最终候选包联合复验。
- [x] **P07**：新事件副本收敛；配置积压不阻止 gate 关闭；HTTP replay 无外部副作用。配置域/epoch、真实重启、Native/AirPlay软件FIFO矩阵和callback应用进度已验收；三端实机媒体/长测仍由P11验收。
  - [x] P07 pending 健康独立发布与提交合并：Prepared 不持有旧 Authority，commit/abort 不恢复旧健康值；GET/event副本、false/true、会话终止、偏好保留/不恢复旧Solo、历史淘汰和u64末尾提交保留通过。见 `p07-live-health-cursors-verified.log` / `p07-live-health-handler-barrier.log`。
  - [x] P07 Native config_revision/event_sequence：持久配置/运行期条件显式分开，v2省略旧expected_revision且拒绝条件混用；Desktop和Sender使用绑定命令，v1保持旧CAS含义；8项域/版本回归、原有集成回归和严格Clippy通过。
  - [x] P07 Native订阅强绑定epoch/sequence，旧订阅返回升级；真实进程GET→重启→WSS及实际watch重取快照、旧Start拒绝/无幽灵会话通过。
  - [x] P07 AirPlay config_revision/event_sequence：v3条件显式分开，旧v2保留原CAS；持久cfg/profile与身份staging恢复同一版本；业务state隔离遥测/跨域容量，健康变化和未知限制不复用旧游标；确认框冻结原条件。28项API/66项Desktop/32项identity及真实HTTPS兼容/重放/重启通过，见 `p07-airplay-v3-wire/result.json`。
  - [x] P07 Native/AirPlay满队列、旧FIFO与lane复用：实际终态API接线、独立原子lease、旧tail/同ID新binding及另一来源保持通过。
  - [x] P07 desired/applied与Desktop：最新目标满队列仍记录，原目标重试不变号，callback确认；两秒stalled/unknown、同runtime回执对账、通道行内/固定底栏双语反馈与实际截图通过。
- [x] **P08**：诊断未知/过期/零值可区分；控件显示与权威状态、应用进度相符。软件与macOS原生preview范围完成，真实IME/VoiceOver及三端GUI仍保留D07/P11验收。
  - [x] P08 独立诊断/采集时钟、必要字段/近期增量健康、共享Lane Starved及Sender目标身份锁存；Desktop78项、实际后台日志/脱敏与IPC范围通过，合同同步。
  - [x] P08 电平携带binding/stream/output代次；旧会话缓存、未应用binding与output重开失效，release实际Mixer/零分配回归通过。
  - [x] P08 最终600×440中文/1100×760英文截图、完整软件门禁与四份release制品哈希归档；真实IME/读屏及平台GUI仍按单独范围验收。
- [x] **P09**：Linux 多实例冲突可解释；Sender 不再隐式限时；CLI 同机反馈在采集前拒绝。实际Linux双后台/节点与macOS TLS/IPv4-LAN-IPv6通过，真实资源长测按用户要求暂缓。
  - [x] P09 显式UntilStopped/Duration、CLI互斥与范围、跨24小时可控期限及UserStopped/DurationElapsed/Failed；实际TLS Sender限时与私有Stop回执通过。
  - [x] P09 UID owner版本/实例/binding UUID握手、第二state_dir明确拒绝；实际Linux PipeWire owner与双后台IPC场景通过。
  - [x] P09 实际TLS远端地址/本机接口、IPv4/IPv6/zone/ID矩阵；macOS真实loopback/LAN/IPv6同端点在采集前拒绝，不创建会话。
  - [x] P09 共享CLI/后台owner校验、macOS回归/Linux严格类型与ELF、实际双后台及制品绑定；Windows类型/真实资源长测另验。
- [ ] **P10**：最终包在目标环境验证，协议组合有明确兼容结果；基础门禁确实运行。
  - [x] P10 macOS含空格/Unicode候选包与独立Mach-O检查；重定位后的五个payload启动、包内GStreamer与双路DTLS媒体通过。最终源码制品另做固定复验，GUI/安装升级独立保留。
  - [ ] P10 最终版本本地/远端基础门禁及源码、安装包哈希绑定。
- [ ] **P11**：平台质量矩阵、首败、长测和可见 GUI 结果归档，全部发布阻断项有处置结论。
  - [x] P11 macOS软件五组合、20次AP定向断开/worker崩溃恢复与存活来源连续性，通过固定同一Hub制品；首败/阶段/严格计数/临时资料清理归档。
  - [ ] P11 真实Apple、Windows/Ubuntu可见桌面和8/24h长测按用户2026-10-08答复暂缓，平台发布门槛不外推。

## 十三 验证矩阵和执行入口

### 从确定性交错到实机

| 层级 | 工况 | 必需证据 |
|---|---|---|
| 纯状态 | 意图合并/公平性、上下文切换、Undo 冲突、准入阶段取消、游标复制 | 实际状态模块断言和可控 barrier，无靠 sleep 碰时序 |
| 实际音频组件 | 队列旧头、模式复用、预取缺口、拒包、SRC 舍入、连续断流 | 使用真实 Ingress/Mixer/queue 的 PCM 和呈现坐标断言 |
| 事务 | 每个写/rename/sync 边界失败，commit/abort 与健康交错，重放 | 注入点、持久前后状态、回执和副作用次数 |
| 独立进程 | 父强退、子卡死、客户端断开、管道满、迟到 Ready | OS 进程对象、端口/锁、停止方式和耗时 |
| 原生数字音频 | 四路组合、单路退出和恢复、输出重开、跨机发送 | 每 lane 波形/计数、聚合输出、未受影响通道连续性 |
| 真实产品 | Apple 来源、实体设备、权限恢复、可见 GUI、安装升级 | 明确设备/平台、实听确认、截图及运行证据 |

四路组合统一写为“原生数 + AirPlay 数”：4+0、3+1、2+2、1+3、0+4。每组覆盖起播、稳态、单路 disconnect、对应 worker 退出、重新授权后手动接入、输出设备丢失与原设备重开。输出故障是共享故障域，其余单路故障不能中断未受影响来源。

软件媒体和合成加密来源能验证数据链及调度；真实两台/四台 Apple 来源与选择器行为另验。保留项目已有 8 小时健康网络和 24 小时资源增长目标；快速时钟跨 24 小时只关闭 B17 期限逻辑，不能替代真实资源长测。

Windows 还需原生四路恢复及可见交互桌面；Ubuntu 保留既有跨机、多路质量失败，修复后按原负载与明确新制品重验。macOS 历史成功只作为回归基线。2026-10-07 Windows 单设备约 41 秒有效观察窗不能替代上述矩阵。

### 阶段和计数不能混淆

每个场景分为 startup、steady、fault、recovery、stopped，显式记录阶段边界。稳态与恢复窗要求来源保持、无本次问题造成的提前播放/持续无声/旧会话出声；质量计数按既有探针严格条件判定，不通过延后观察窗口藏掉起播异常。

故意断流时欠载应按 D02 定义增加；此时“全程零欠载”反而说明计量有问题。恢复前的注入影响与恢复后的不应发生增量分别报告。被测源真的暂停、产生静音或提前退出时，不把它算成 NeonMix 稳态成功。

### 现有开发命令

以下为开发与验证入口。计划起草时尚未运行；2026-10-07 本轮实际结果按 [实施证据](evidence/review-stability-20261007/README.md) 逐项记录，不把列出的入口当成全部已通过。所有依赖、缓存、临时夹具和构建产物留在项目目录内。

```sh
# 纯状态和组件相关测试；按实际修改选择，不必每次全跑。
tools/dev cargo test -p neonmix-core -p neonmix-control \
  -p neonmix-airplay-adapter -p neonmix-airplay-ipc --locked
tools/dev cargo test -p neonmix-desktop-service \
  -p neonmix-output-binding -p neonmix-identity --locked
tools/dev cargo test -p neonmix-desktop -p neonmix-hub --locked

# 涉及音频质量、时间线或实时约束时执行对应 release 测试。
tools/dev cargo test -p neonmix-core --release --locked --test timed_mixer
tools/dev cargo test -p neonmix-core --release --locked --test timed_quality
tools/dev cargo test -p neonmix-core --release --locked --test mixer_realtime

# 阶段完成后运行项目标准检查，原生运行库按 README 准备。
tools/dev python3 tools/check.py
```

Windows 完整媒体检查使用：

```powershell
./tools/dev.ps1 python tools/check.py --native-media --keep-going
```

Linux 的通用检查如果要覆盖媒体，同样使用 `--native-media` 并准备原生依赖。新增测试名称以实现时实际代码为准；不要把本文拟议接口或用例名当成已经存在的命令。

现有扩展入口包括 [control transactions](../crates/control/tests/transactions.rs)、[authority](../crates/control/tests/authority.rs)、[timed Mixer](../crates/audio-core/tests/timed_mixer.rs)、[timed quality](../crates/audio-core/tests/timed_quality.rs)、[Ingress](../crates/airplay-adapter/tests/ingress.rs)、[生命周期](../crates/desktop-service/tests/lifecycle.rs) 和 [绑定持久化](../crates/output-binding/tests/persistence.rs)。Hub HTTP 回归应放入其现有源码测试模块或实际可编译的集成入口，不能仅测试底层 Authority。

### 证据结构和问题关闭

每轮建议放入 `docs/evidence/review-stability-<实际日期>/`，至少记录以下字段：`issue_id`、`case_id`、平台/设备、commit、工作区差异哈希、制品哈希、参数、阶段、预期、实际、pass/fail/not_run、原始日志路径。使用现有脱敏和 archive policy；资料日志、运行密钥及配对 fixture 不进入证据包。

问题只有在对应自动回归通过、涉及的平台原生验证完成、合同更新且证据可复现时才可标为关闭。对不涉及平台 API 的纯逻辑问题可以按测试范围关闭；对平台专属问题使用“macOS 已验收、Windows 未验收”这样的分项状态，不强行合成全平台通过。

## 十四 协议迁移与回退

### 逐步启用新能力

1. 先添加显式 capability/schema 与双读能力，再启用新客户端写法。Desktop、background、Hub、worker 的本地组合必须检查兼容，缺必要能力时返回可解释错误。
2. AirPlay patch 保留旧全量操作；新客户端对旧服务端使用 D01 的安全兼容流程，不能直接丢掉新字段。
3. 新事件游标通过新协议版本启用；旧 WSS 不伪装可恢复订阅。迁移期间客户端可以完整轮询，明确 freshness 和失败状态。
4. UUID CAS 要贯穿 CLI、IPC、Store 后再启用严格要求。旧管理客户端缺 UUID 时明确拒绝并要求升级，不能补入当前 UUID。
5. 持久事务日志只由了解其 schema 的版本恢复。资料格式升级前保留受私有权限保护的原始版本用于诊断，但不能用旧备份覆盖升级后新增的配对或撤销。

### 回退以完整制品为单位

音频算法变化可在同一资料格式下回退完整已验证二进制组合；不能混用新 Hub 和旧 worker 后宣称兼容。回退版本若不能理解新持久格式，应停止并使用显式的当前数据导出/转换流程，而不是把历史用户目录拷回。

lane/gate/session 永远是运行期状态，不在回退后恢复旧媒体会话。回退本身要验证启动、身份、输出选择、已撤销来源仍被拒绝、显式手动 Start 与正常 Stop。

## 十五 合同更新和推荐决策

### 随实现同步更新的文件

| 文件 | 需要补充的契约 |
|---|---|
| [MEDIA-CONTROL-CONTRACT](MEDIA-CONTROL-CONTRACT.md) | runtime/config/event 版本域、Replay、Unknown、授予/撤销、desired/applied |
| [AUDIO-CONTRACT](AUDIO-CONTRACT.md) | LaneBinding、分段时间坐标、边界 FIR、计量口径、实时约束 |
| [AIRPLAY-IPC-CONTRACT](AIRPLAY-IPC-CONTRACT.md) | 准入阶段取消、关联键、grant 迟到、会话 gain、必要的媒体版本能力 |
| [AIRPLAY-CONTRACT](AIRPLAY-CONTRACT.md) | patch 语义、来源持久偏好与 session Solo、取消后的入口状态 |
| [DESKTOP-IPC-CONTRACT](DESKTOP-IPC-CONTRACT.md) | 生命周期 endpoint、Accepted/Completed、实例代次、Ready、停止预算 |
| [OUTPUT-BINDING-CONTRACT](OUTPUT-BINDING-CONTRACT.md) | UUID CAS、Linux owner 作用域、重建与旧请求 |
| [CREDENTIAL-STORAGE](CREDENTIAL-STORAGE.md) | 日志私有性、恢复顺序、耐久三态、格式迁移和回退 |
| [UX-CONTRACT](../UX-CONTRACT.md) | 字段意图、确认后 Undo、上下文失效、独立 freshness、真实停止结果 |
| [DESIGN](../DESIGN.md) | 行内 pending/error/unknown 表达与声音路径信息，保持现有视觉体系 |
| [STATUS](STATUS.md) 与相关 ADR | 每平台实施/验收状态，以及取代旧版本/持久化描述的决策记录 |

原文档中旧平台 vault 描述、旧 revision 和全量写法应在相应实施批次中核对更新，继续以 ADR-014 等较新决策为准；本提案不直接改写正在使用的合同。

### 推荐默认决策

| 决策 | 本计划采用的默认值 | 代价与重新讨论条件 |
|---|---|---|
| 后台崩溃后的音频 | 本实例媒体退出，不收养 | 崩溃会中断声音；若要长期无人值守连续运行，另行设计受监管服务 |
| Linux 多 state_dir | 首版明确拒绝跨实例共享虚拟输出 | 暂不满足同 UID 多个独立 Sender；真实需求出现后再做共享 owner |
| 桌面 Sender 时限 | 显式停止前持续运行 | 增加真实长期资源验收，不以每日重连掩盖问题 |
| 冲突处理 | 刷新并显示冲突，由用户重试 | 少量额外操作，避免无条件覆盖另一控制端 |
| 多文件配置 | 先事务恢复，后单一权威迁移 | 短期维护日志协议；不把损坏状态自动回滚成旧身份 |
| CI | 新增轻量 PR/main 门禁，原生昂贵任务分层 | 增加基础构建成本；实际启用和分支保护作为开发交付 |
| 小缺口 DSP 策略 | 先保留真实时间，阈值由波形证据确定 | 不承诺所有边界都无可闻变化；不能提前播放或丢有效尾音 |

这些默认值使开发计划可执行，不需要在开始每个小修复前重新讨论全部架构。真正会改变产品范围的事项是持续服务收养、Linux 多实例共享和资料格式回退兼容，实施时应单独形成明确决策。

## 十六 发布验收 checklist

- [ ] B01–B10、B20 中的全部 P1 均有回归和相关运行证据；未满足项不得以普通已知问题带入“稳定性完成”结论。
- [ ] B07 及 B11–B19 各项 P2 达到表中关闭条件；若分批发布，明确剩余问题、平台和触发条件。
- [ ] R01–R03 分别完成故障验证和方案验收，不能在修复前写成已证实事故或已解决问题。
- [ ] 20 项问题、3 项风险与开发批次逐一对应，无遗漏、无重复计作关闭。
- [ ] 核心交错使用真实状态模块与音频组件；模型、stub、合成源、数字输出、实机实听分别标注。
- [ ] callback 零分配及有界性通过，输出重开、旧 generation、撤销后旧 PCM 不破坏隔离。
- [ ] 后台停止/启动结果真实，最终包无本实例遗留进程，UI 关闭仍保持既定行为。
- [ ] 配置中断能恢复，DurabilityUnconfirmed 可诊断，历史绑定请求不能修改新对象。
- [ ] 诊断未知不显示正常，所有统计口径及观察阶段可追溯。
- [ ] Windows/Ubuntu 历史失败按原工况关闭或明确阻断，不借其他平台和短时成功替代。
- [ ] 最终包、升级、协议组合、资料回退边界和每平台可见 GUI 均有独立结果。
- [ ] 更新 STATUS、合同与发布说明，清理本轮无后续用途的临时进程、文件和 fixture，保留失败证据。
