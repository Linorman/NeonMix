# 稳定性开发证据（2026-10-07）

本轮按 `docs/REVIEW-IMPROVEMENT-DESIGN-PLAN-20261007.md` 执行，不使用子 Agent。所有命令经 `tools/dev`；缓存、夹具和构建留在项目内。

## P00 基线

HEAD：`aefb8690320cb114c974be2477b2ebb64dfadf1e`。初始未提交差异 SHA256：`1431033f060ee26823ac5609418c3f7d536da47b73bc15b245a497cf26802155`。

[baseline.json](baseline.json) 记录 2807 个源码文件的哈希、初始 git 状态、平台和 7 个已有构建产物的哈希。既有 UI/i18n、Windows AirPlay 和 lifecycle 修改均为本轮输入，不算本轮实现。完整源码及差异的私有快照在 `.local/review-stability-20261007/baseline/`；`.tmp/` 本地临时资料未归档。公共证据不收录运行身份、配对资料或 PCM。

## 验证方式

复现和修复均使用实际模块。复用 Hub `execute_native` 的 SDK/存储注入点和 `Barrier`，以闭包调用计数验证 Replay；复用 egui `Context::run` 输入事件验证键盘行为；使用 Python 打包用例及项目内真实 Mach-O 夹具验证 RPATH。后续音频采用现有 release Mixer/Ingress 测试的呈现时钟；存储边界和进程夹具在对应批次接入实际代码，未执行不写通过。

基础门禁由新增自动工作流和本地 `tools/review_check.py` 执行，昂贵原生、打包和实机检查仍分层。GitHub job/required checks 的实际运行留待 P10；未修改远端分支保护。

## 问题覆盖

`not_run` 表示尚未运行，`pending` 表示尚未实现；每完成部分同步更新本表与计划 checklist。全平台关闭需要对应原生证据。

| 问题 | 批次 | 复现 | 实现 | 自动测试 | 原生验收 | 证据 |
|---|---|---|---|---|---|---|
| B01 | P01 | fail（空列表方向键 panic） | 共享 guard 完成 | Desktop 45 项 pass | 纯逻辑/实际 egui 输入；真实 VoiceOver 另验 | `p01-mixer-unfixed.log` / `p01-desktop-suite.log` |
| B02 | P03 | fail（旧 timed 头导致 native 持续无声） | 控制面 playback_kind、绑定代次和独立授权原子完成 | 6 项实际 Mixer 回归 pass | 波形组件；平台媒体 not_run | `p03-binding-before.log` / `p03-measurements-after.log` |
| B03 | P02 | 单槽覆盖确认 | 有界字段调度、公平性及终止屏障完成 | 9 项状态交错矩阵 pass | macOS IPC 转发；跨平台 UI 待分项 | `p02-intent-tests-verified.log` |
| B04 | P02 | 全量 triple 机制确认 | Desktop 字段 patch、能力选择及只读全量兼容完成 | Hub API/兼容解析/客户端发出 envelope pass | 纯逻辑及真实 API 接线 | `p02-full-state-tests.log` / `p02-intent-tests-verified.log` |
| B05 | P02 | dispatch/Undo 未绑定上下文确认 | Hub/身份/runtime/context/完整 target 绑定；确认后字段 Undo | 四阶段、身份切换、session 复用、Unknown 冻结对账 pass | macOS 原生 IPC 身份替换与文件清理 pass | `p02-ipc-identity-verified.log` / `p02-intent-tests-verified.log` |
| B06 | P04 | ungranted owner remains occupied（首败） | 全关联键取消、锁存、独立 cleanup、Hub 关闭确认期限 | 五阶段 Worker barrier / Hub 实际资源 barrier pass | macOS 完整 worker、双来源数字恢复 pass；其他平台/Apple 待分项 | `p04-session-before.log` / `p04-final-barriers.log` / `p04-two-source-recovery.json` |
| B07 | P04 | worker 级 gain 保留确认 | SessionState 所有权与新 owner gain=1 初始化 | 白盒会话 gain /旧 decoder seam pass | macOS 完整签名发送端与 encrypted UDP/PCM IPC pass | `p04-session-complete.json` / `p04-final-audio.log` |
| B08 | P05 | 长事务/普通配额和双 IPC 失败机制确认 | 独立 endpoint、Stop operation、UI 未确认状态完成 | 20 秒变更/饱和/冻结代次/未知结果回归 pass | macOS IPC/独立 Child；完整媒体/其他平台待分项 | `p05-bound-matrix-verified.log` |
| B09 | P05 | fail（后台 SIGKILL 后两个卡死媒体进程 8 秒仍运行） | Unix guardian + 现有 Windows Job，Unix 源码完成 | 进程树、密钥 inode/owner 锁及 symlink 回归通过 | macOS 真后台/合成媒体树 pass；Linux 类型检查 pass，三端实际产品媒体另验 | `p05-parent-kill-before.json` / `p05-parent-kill-verified.json` |
| B10 | P06 | 两文件replace半提交机制确认 | 私有事务日志/恢复接线完成 | 43进程中断及实际Hub读取pass | macOS文件范围；其他平台另验 | P06双文件事务节与对应日志 |
| B11 | P08 | 缺字段归零/共用刷新机制确认 | 独立诊断/采集/电平时钟与字段校验完成 | Desktop79及实际Sampler时间回归pass | macOS软件/preview；GUI另验 | P08/P10样本年龄补验 |
| B12 | P03 | fail（断流后再一秒增加 0） | DSP 与计量寿命分离 | 精确 48000、变分块、停止、恢复、静音 PCM 及实时回归 pass | 纯音频组件，非实机 | `p03-measurements-before.log` / `p03-components-verified.log` |
| B13 | P06 | 仅 revision 比较导致重建后旧请求可复用机制确认 | UUID/revision CAS 贯穿 Store、CLI、IPC、UI/确认框/Forget及Sender/native sync | Store/实际dispatch/必填CLI/冻结IPC交错 pass | macOS 软件/IPC；真实平台setter/最终包另验 | `p06-binding-uuid-store.log` / `p06-binding-api-cas.log` / `p06-binding-ipc-contract.log` |
| B14 | P07 | pending健康不发布/版本混用确认 | config/event/runtime分域与提交合并完成 | 真实GET-WSS重启/完整回执/FIFO矩阵pass | macOS TLS/软件；平台媒体另验 | P07最终阶段证据 |
| B15 | P09 | UID owner被不同state_dir复用机制确认 | 协议/实例/binding UUID握手完成 | 共享模块与实际IPC pass | Linux真实节点/双后台/释放重取pass，远端Sender夹具 | p09-linux-background-final.json |
| B16 | P05 | Ready 被 last_event 覆盖/超时成功机制确认 | Child Ready 独立锁存，失败/超时回收后报告失败 | 五种真实 Child/真实 IPC/服务回归 pass | macOS 子进程；其他平台待分项 | `p05-ready-final.log` |
| B17 | P09 | 隐式86400时限确认 | UntilStopped/Duration及终态完成 | 可控跨24h/CLI边界pass | macOS真实TLS限时/停止；资源长测暂缓 | p09-sender-native.json |
| B18 | P01/P10 | fail（旧 parser/忽略删除失败） | parser、错误检查、独立 Mach-O 验证完成 | 7 项 pass | macOS 原生 clang 夹具 pass；最终包 not_run | `p01-rpath-before.log` / `p01-rpath-after.log` |
| B19 | P01/P07 | fail（重放仍 save） | `Preparation::Replay` 和完整 HTTP 回执缓存完成 | Hub 57 / control 15 项 pass | 纯逻辑/HTTP 接线；runtime 待 P07 | `p01-replay-before.log` / `p01-replay-after.log` |
| B20 | P03 | fail（预取丢 A 尾音） | PCM v2 显式位置、固定锚点校验、分段 FIFO/FIR 完成 | 11 项 Ingress 及既有 timed/音质/实时回归 pass | macOS decoder SRC pass；完整 worker/平台另验 | `p03-segments-before.log` / `p03-time-release-verified.log` / `p03-worker-src-verified.log` |
| R01 | P07 | 满配置队列延迟gate风险 | 独立原子lease与FIFO授权完成 | 实际Native/AP终态、旧FIFO/复用pass | 软件/回调；物理尾部另验 | P07 FIFO/application证据 |
| R02 | P06 | 读回相同不证明耐久机制确认 | 三态发布与同请求对账/限制交集完成 | 真实文件注入/实际HTTP/worker范围pass | macOS文件；掉电/其他平台另验 | P06最终耐久阶段 |
| R03 | P09 | loopback判断位于采集之后确认 | 实际peer/接口/后端ID采集前判断完成 | IPv4/mapped IPv6/scope/不同ID矩阵pass | macOS真实loopback/LAN/IPv6 pass | p09-sender-native.json |

## 批次进度

- [x] P00：基线冻结、既有修改归属和逐项记录建立；夹具入口及轻量门禁明确。
- [x] P01：空列表、RPATH、HTTP Replay；对应合同同步更新。最终包与新 runtime 不包含在本批结论中。
- [x] P02：意图调度、字段 patch、上下文和 Undo。
  - [x] 服务端 `PatchMixSource` 能力、字段寿命及回归。
  - [x] Desktop 使用字段意图、能力选择及旧服务端兼容；没有必需 runtime 条件的组合要求升级，不丢条件写入。
- [x] P04：AirPlay 阶段取消和会话 gain；macOS 软件/原生组件验收，跨平台与真实 Apple 保留门槛。
- [x] P03：绑定、计量、时间段的真实组件与 macOS worker 验收；跨平台/最终包/长测独立保留。
- [ ] P05：生命周期独立入口、Ready 和进程回收。
- [ ] P06：事务恢复、耐久结果、UUID CAS。
- [x] P07：游标、gate、回执（软件范围；平台媒体另验）。
- [x] P08：诊断 freshness 和命令状态（软件/macOS preview范围；IME/读屏/其他平台另验）。
- [x] P09：本地 owner、持续发送、防反馈（软件与Linux owner范围；资源长测暂缓）。
- [ ] P10：最终包、兼容和 CI 实际运行。
- [ ] P11：联合质量和实机验收。

## 本轮局部验证与未闭环项

P01 修复前分别复现空列表 panic、RPATH 空格丢失/删除失败继续、HTTP Replay 仍持久化。键盘最终回归见 `p01-keyboard-verified.log`；真实 Mixer 页面末路移除也清理 selection。`p01-backend-verified.log` 包含 Hub 60 项、control 15 项及 desktop-service 测试；不以服务单元测试关闭 P05 三平台进程回收。

P03 模式及计量的 release 核验见 `p03-components-verified.log` 和 `p03-all-components-verified.log`。后续补入明确 callback 提交钟，真实 Ingress 的深输出周期用例 debug 也通过（`p03-ingress-clock-debug.log`），避免虚拟时钟夹具被 debug 计算耗时误判为包年龄。`p03-realtime-clock-verified.log` 验证显式时钟、重开和正常渲染的零分配/析构。此记录对应分段改动前的检查点；后续 P03 时间线见下文独立结果。

工作区在本轮期间另有 Desktop/i18n 源码持续修改，基线记录其初始状态；未将其他来源的新改动算成本轮稳定性实现。曾记录 i18n 缺接口/类型编译错误、无障碍嵌套 Context 读取卡死及新增长名称/缩放、palette 验收失败。Mixer disclosure 现在先读取可访问名称，回调内不再嵌套读 Context；其他共同控件按当前工作区更新。`p01-desktop-suite.log` 是当时 45 项通过的结果；后续扩大检查、编译中间失败和首败保留在各次独立日志中，不能宣称最终全 workspace 或新 UI 矩阵已通过。最新完整 Desktop 构建以 `p01-build-final.log` 为准；更早的 `p01-build-verified.log` 成功不替代之后源文件的验收。

基础门禁的自动 workflow 已定义，本地纯组件检查单独记录；尚未发布/触发远端 CI、配置 required checks、构建或安装最终发布包。未执行任何 Windows/Ubuntu 实机、Apple 来源、实体输出实听或长测。

当前检查点 `checkpoint-gate.log` / `checkpoint-result.json` 四项均 exit 0：格式、7 项打包回归、严格 Clippy、纯组件加 Hub/Desktop 的 231 项 Rust 测试（4 项按原条件 ignored）。`checkpoint-tests.log` 保留完整原始结果；扩大 i18n 的先前失败已随当前工作区修复后纳入此次通过范围，不将这些修复归属为本轮稳定性开发。`p01-build-current.log` 的 Desktop/Hub/background release 构建成功。制品及核心源码哈希见 `implementation-checkpoint.json`；后续源文件/制品变化须重新绑定结果，不以本检查点关闭剩余架构或实机任务。

## P02 验收

`p02-gate-verified.log` / `p02-final-result.json` 四项通过：格式、7 项打包回归、严格 Clippy、243 项 Rust 测试（4 ignored）。`p02-release.log` 与 `p02-screenshot-build.log` 构建通过。`p02-intent-tests-verified.log` 的 9 项新回归使用真实 Desktop 调度、mpsc 输出与 Authority；第一在途回复由测试明确持有，不靠 sleep 碰时序。覆盖多字段/通道、公平性、容量、终止屏障、切换窗口、session 复用、更新草稿、确认后 Undo、兼容读取后的上下文取消、Unknown 重放同一冻结 envelope。`p02-ipc-identity-verified.log` 用真实本地 IPC 和 CLI helper 验证原 ID/revision、凭证路径替换、冻结元数据和私有临时文件清理；它不是真实网络服务或 Windows NamedPipe 验收。

新增 Snapshot/Command/Event runtime 条件是 P02 的绑定/对账前提，不计作 P07 的配置/event 版本分域、pending 健康副本收敛或 gate 饱和撤销已完成。兼容 full MixSource 需存在 runtime 条件；完全缺少必要条件的旧组合明确要求升级。只读解析输出的 command 在实际变更前冻结，重放不再解析。

`p02-ui-preview/` 为项目内 screenshot-feature 程序从既有公开录得状态生成的 600×440 八页预览，Mixer 最小窗口检查未见覆盖后台退出入口/主控的布局问题；预览不连接后台、不播放音频。截图、egui 键盘/AccessKit 树断言不能替代真实 IME/VoiceOver、动态网络行为与实机听测。P08/P10/P11 对应 checklist 保留未完成。AST/schema i18n 检查通过 724 个消息/两种语言。新代码与制品哈希见 `p02-checkpoint.json`；没有新增子 Agent 或项目外开发产物。

## P04 验收

`p04-session-before.log` 保留真实旧 Worker 的 ungranted owner 占用首败。取消命令现携完整关联键；worker 在 state 锁下立即关闭 granted/锁存最多 128 个取消记录，再由单个 cleanup 线程清 decoder/PCM 和连接。Closing 拒绝新 owner。SessionState 持有协议 gain 等媒体资源，新 owner gain=1，同 owner reset 保留合法值。Hub 跟踪关闭确认/五秒期限，取消 helper 用实际 gate/claim/lane 资源接线，producer 失效后立即释放容量，迟到 start/grant 不能恢复旧 owner。

`p04-final-barriers.log`：五个真实 Worker 接线 barrier（AdmissionPending、AdmittedAwaitGrant、GrantBeforeInstall、GrantInstalled、GrantApplied），重复取消、迟到 admit/grant、旧 connection/request/generation 和三种音量寿命回归通过；无依赖 sleep 的竞态注入。`p04-hub-cancellation-barrier.log` / `p04-hub-verified.log`：实际 Hub claim/gate/lane 返回、另一路 claim 保持、迟到事件隔离、Active 禁止零 context。

`p04-session-complete.json` 六个完整 worker 网络场景通过：签名 pair-verify、加密 classic UDP、实际 decoder 和 PCM IPC；三个取消阶段后 B 都输出 50 个有效 PCM 块；A 的 −144/−18/0 dB、合法重复 stream SETUP、FLUSH/恢复各输出 50 块，B 未发送音量命令仍使用 gain=1。初始 key SETUP 的 455 是既有 Alpha 边界，改验合法 stream SETUP；早期探针的 RTP 重置/取消开关错误及首败日志保留，不算产品原生失败。`p04-setup-regression.json` 既有七项输入边界通过，`p04-final-audio.log` 保留 PCM/ALAC/AAC 真实 decoder seam 回归。

`p04-two-source-recovery.json` 在显式 macOS BlackHole 数字输出完成两来源、四次逐路 disconnect/crash 与恢复，未受影响来源连续性通过，fixture 进程和文件已清理；此结果使用报告内固定制品哈希。`p04-gate.log`/`p04-final-result.json` 基础门禁通过，末次 Hub helper 改动另由测试、严格 Clippy 与 release 构建核验。Source/制品哈希见 `p04-checkpoint.json`。

该批关闭 B06/B07 的实现及上述 macOS 软件/原生组件范围；Windows/Linux Worker、真实 Apple、多源长测、发布包均未执行。Mixer FIFO 缓存的独立授权 generation 仍由 P03/P07 验收，不将本批 gate 关闭解释为声卡立即物理静音。


## P03 时间线与授权组件

`p03-segments-before.log` 使用真实 Ingress/Mixer 复现预取 B 时丢 A 尾音。`p03-time-release-verified.log` / `p03-time-debug.log`：真实队列/Mixer 六项绑定回归、11 项 Ingress（含 1/127/480/1024 输出周期两段波形及 44.1/48 kHz 各 2000 个变长块）、精确断流、timed 与音质回归通过。最后 callback 扫描预算收敛后 release 重验通过；debug 结果对应前一步实现。`mixer_realtime` 扩展八个一帧段/缺口/满配置队列撤销与代次变更，分配、重分配、释放均为零。

保持原 4320-frame FIFO、8-block handoff 与迟到/时钟策略；新增固定段描述符环覆盖最小一帧包，预取不重置活动段。FIR 只读当前段，游标到边界才重建 servo/240-frame 淡入，真实缺口均隔离，不以扩大缓冲或放宽质量阈值掩盖问题。Ingress 显式保留包位置，拒包不参与时间压缩；原源坐标/PTS 矛盾分别计数。

PCM 内部升级为 v2/120-byte，追加 `normalized_sample_position` 并在 startup/ready 明确协商。worker 以固定源标记及实际输出 buffer PTS 计算位置，保留原 source rate/position；不逐块积累舍入。`p03-worker-src-verified.log` 的实际 PCM/ALAC/AAC 24/40/51 个输出 chunk、RTP 回绕和 SRC 连续性通过。早期探针捕获新 header 的 payload offset 错误（`p03-worker-src.log` / `p03-worker-src-detail.log`），修复后重验，不删除首败。完整网络及基础门禁的结果随后补入。

P03 的授权原子能截断已经排队/FIFO/SRC 的旧代 PCM，配置队列满不影响组件撤销；Hub 全部 Stop/Revoke 限制性事务的接线仍由 P07 验收。macOS 组件结果不代表其他平台、真实 Apple、最终包或长测已通过。


`p03-worker-complete.json` 六个实际签名/加密 UDP/PCM IPC 场景通过；取消及同会话音量/flush 的既有条件不受 PCM v2 影响。`p03-phase-release.log` 加速两分钟、±100/±500 ppm 绝对相位/波形回归通过。`p03-gate-verified.log` 的格式、7 项打包用例、严格 Clippy 和 Rust 测试四项通过；首轮 Clippy 的绑定接线嵌套 if 已修复，日志保留。`p03-release.log` Hub/background release 构建通过；本机数字恢复另列，未将其他平台或长测计作本批通过。


P03 数字验证首败 `p03-two-source-recovery.json`：两路稳态分别增加 384 帧欠载、无迟到；Ingress 各拒一个重叠包且形成 383-frame 缺口。根因为用更新后的 NTP mark 重新推算 SRC 输出坐标，偶发一帧重叠。改用实际 SRC output offset，offset 缺失时用固定输出 PTS 锚点；原输入诊断坐标从同一 SRC 输出格点映射，避免用更新后的墙钟标记改变源位置。源 RTP 缺口显式传递 DISCONT，避免 GStreamer 默认容忍约 31 ms 的不连续而压缩小缺口。

`p03-src-gap-final.log`：真实 PCM（加入每包 4000 ns NTP mark 漂移）、ALAC、AAC 连续坐标通过；10 ms/441 源帧缺口保留 481 个内部格点静音位置（SRC 段端点量化 ±1 帧），两个 FIR 段，不跨缺口滤波。`p03-two-source-offset-verified.json` 修复后两路稳态及四次逐路故障/恢复通过；最后固定源坐标版本另在 `p03-two-source-final.json` 验证，不覆盖旧报告。没有扩大队列或放松质量断言。

## P05 Ready（部分实施）

`p05-ready-final.log` 服务检查通过：五种真实 Child startup 情况及实际 IPC Ready→普通 stats 日志回归，Hub/Sender 共享 wait_ready；正常 Ready 返回，失败/超时先回收 Child，迟到 Ready 不复活。就绪状态独立于 last_event 并在 fault/停止/退出时清除。早期夹具用 100 ms 成功启动预算受 macOS 进程冷启动影响，改为 1 s 成功预算（仍明显短于原实现十秒），迟到/未就绪保留 100 ms 的显式失败预算；失败和修正日志均保留。

此检查点时 P05 的独立生命周期 endpoint、operation/代次、过载停止、UI 关闭与三端父强退尚未实现/验收；后续增量见下文，主批次保持未勾选。没有把旧 lifecycle crate 的存在或 Ready 用例通过当作 B08/B09 已关闭。


## P05 独立生命周期与 Unix 回收（跨日继续，2026-10-08）

独立 lifecycle socket/NamedPipe 和内存 Status 已接入；Stop operation 不依赖远端请求锁、普通连接八槽及调用者 socket。Desktop 创建时冻结后台实例/停止代次，Start/Stop 在接入与实际 spawn 两处校验；立即停止启动，旧轮询不覆盖已确认停止，双 IPC 失败显示结果待确认。`p05-bound-matrix-verified.log` Desktop 64 项及真实 IPC/Child 矩阵通过；严格 i18n 726 条双语、premium 静态审计通过。`p05-owner-native-verified.log` 使用项目内明确的 probe 制品完成 50 次取消等待/重开及卡死强退。未设置 probe 的先前失败保留为夹具参数错误，不算产品异常。

`p05-parent-kill-before.json` 在两个真实独立后台中强杀 A，A 的合成 Hub/后代在 8 秒后仍运行，B 不受影响。新增 Unix guardian 继续使用 lifecycle 私有 Stop 管道；只由后台保有写端，EOF 也登记停止。guardian 用独立进程组持有媒体，5 秒协作期限后强杀组；`waitid(WNOWAIT)` 保留 leader 的内核 PID 直到全部后代确认退出，随后才 reap。守护层不按进程名/磁盘 PID 收养或全局杀进程，Windows 继续私有 Job。组件和包须有同目录 guardian，缺失明确失败；macOS package/collect 入口已接入。

`p05-parent-kill-verified.json`：约 5.07 秒回收 A 树，A 的一个模拟运行密钥被清理，B 树与 B 密钥保持；两个 supervisor 均退出。持久 profile 保留，全部临时进程/目录移除。首轮两次强杀组遇全僵尸组 `EPERM`，错误阶段见 `p05-parent-kill-receipt.json`；现在不重复已成功的强杀，对尚未发送强杀时的 EPERM 仍核验消失，不能由错误码直接宣称退出。组 reaping 的 WouldBlock/Interrupted 在原两秒期限内继续，不提前丢失 owner。

运行密钥只捕获 live owner 的 canonical UUID 临时命名，使用已打开私有目录及文件 inode/dev 身份；组退出后还需取得 profile/state owner 锁。锁被新 owner 持有、对象被替换、receiver 目录 symlink 都不会删除新对象，分别返回固定 cleanup 原因。三个真实文件回归见 `p05-guardian-cleanup-tests.log`。字节内容没有进入证据；这些是假密钥 fixture，不是真实配对资料。

`p05-guardian-pipe-full.json`：合成后代持续写日志、父完全不读取日志时，私有 Stop 与组回收仍约 5.52 秒完成；stdout/stderr 转发有 250 ms 写预算，不阻断生命周期。guard 层显式忽略 SIGPIPE 并继续处理 owner 丢失。首个普通父退出结果缺目录快照/夹具 state 文件的失败与进程组首败日志均保留，未持续复跑直到一次成功后删除失败。

`p05-guardian-linux-cross.log` 是 Linux x86_64 分支/接口的交叉类型检查，非原生运行。最后字段/文件元数据/组回收调整需在最终门禁重新绑定源码与制品；release/真实产品媒体、Windows/Linux/GUI 和签名最终包不由上述合成树结论关闭。P05 主 checkbox 保留未完成，后续批次也未计为完成。


最后的 Unix actor 调整禁用了 guardian 外层 `kill_on_drop`：失败的 Rust runtime 丢弃 Child 时只关闭存活写端，让 supervisor 完成树回收，而不是先杀掉 supervisor。`p05-runtime-drop-native.log` 三项 native owner 回归通过；独立运行的新测试一度将已退出但无人 reap 的 guardian zombie 算作“仍运行”（`p05-runtime-drop-isolated.log`），现由该测试作为真实直接 parent 明确 waitpid，不依赖其他并发测试提供 orphan reaper；修正结果见 `p05-runtime-drop-isolated-verified.log`。这些是进程夹具证据，未包装成真实声音/实体设备通过。

最终本地门禁 `p05-guardian-gate.log` 四项均 exit 0，含新增 desktop-service/lifecycle；strict Clippy 见 `p05-guardian-clippy-final.log`。Unix release 制品构建见 `p05-final-release.log`，Linux 最后接口交叉检查见 `p05-guardian-linux-cross-final.log`。只有发生后续修改才补相应检查，不反复跑音质或跨平台长测凑通过。

`p05-final-gate.log` / `p05-final-result.json` 四项最终门禁通过；release 后台/guardian 的实际父强杀及日志消费者堵塞分别由 `p05-release-parent-kill.json`（约5.06秒、密钥/另一实例保持）和 `p05-release-blocked-pipe.json`（约5.57秒）确认。最后源码、tracked差异及制品哈希见 `p05-unix-checkpoint.json`；untracked新文件单独进入源码哈希，未用tracked diff冒充完整源码。Linux交叉/真实三端/GUI/最终包边界继续保留。


## P06 发布阶段原语（进行中）

`identity::files::replace_reported` 明确保留 NotPublished / PublishedDurabilityUnknown / Durable 和原始 io::Error；替换后的目录同步失败不被字节一致升级为 Durable。实际 temp create/write/file sync/temp directory sync/publish/postpublish/final directory sync 七个边界注入错误（原 OS code 28 保留）、临时文件清理及同字节 fsync 失败通过：`p06-publication-phases.log`。此为 macOS 文件组件；Win32 仍保留现有 write-through 原语，失败且来源消失/无法确认时保守保留发布未知，Windows 原生和掉电耐久没有在此宣称通过。

旧 replace 入口暂作为原错误兼容适配；Hub 的读回相同即成功分支尚待迁移到阶段结果，双文件恢复日志及 UUID CAS 也未完成。因此此处仅勾选底层原语子项，不关闭 R02、B10、B13 或 P06。


## P06 双文件事务与启动恢复

`identity::hub_settings` 新增私有且有界的同目录日志，持 profile→state 的固定锁顺序。写入 Prepared 后不改目标文件；CommitDecided 先耐久，再逐个发布 state/profile。恢复先校验 schema、四份原/目标内容校验和、版本递增、目标路径及“只改房间名/输出ID”；Hub UUID、身份引用、certificate、权限与偏好不能借日志改写。当前文件必须仍是记录中的旧或新内容，否则拒绝覆盖较新配置。发布决定后不做两文件回滚。

`p06-journal-faults-scoped.log` 的三项组件矩阵覆盖 36 个实际 I/O 注入点、未知 schema、损坏校验和、篡改 identity、新 owner 内容以及发布后同步错误，原 OS error code 保留。`p06-process-interruption.json` 真写入进程在 43 个 flush 后受控 barrier 被 SIGKILL，再用新进程恢复；每点均得到全旧或全新 pair，身份/ACL/preferences不变、再次恢复不增 revision。非模拟函数执行，不以 sleep 碰杀进程时机；不覆盖断电文件系统耐久。

第一次日志发布前死亡也可能留下 staging，因此事务写入使用仅该 profile 的哈希/UUID命名空间；在相同 profile/state locks 下只清理 canonical、私有正常文件，不 glob 删除其他模块的 `.neonmix` 临时文件。嵌套 state 路径仍通过原 canonical/symlink 拒绝边界。秘密 journal 和 scoped staging 已加入 archive_policy 排除，测试只导出阶段/计数，所有私有 fixture 都清理。

Hub product `identity::config` 和后台启动在严格输出/profile一致校验前恢复，HubSettings IPC 改为此 coordinator，不再用不可信回滚。实际产品配置读取回归生成完整身份/证书，在 state 新/profile旧的中断 pair 下仍恢复启动且私钥/admin token不变；完整日志随后在 `p06-wiring-tests-final.log` 固定。`p06-ipc-recovery-final.log` 验证真实后台重启恢复、IPC保存和同意图重试（revision仅增加一次）；先前夹具 hub directory 权限错误保留，校正为实际产品的0700后，并明确断言已到达StatePublished才算中断。

界面使用现有共享 fault/双语资源显示“设置提交结果待确认 / 设置需要恢复”，不把已决定但未完成的提交画成失败回滚。完整 R02（HTTP限制性/授予性边界）、B13 UUID CAS、资料迁移与三端原生发布仍未完成，P06主项保持未勾选。

最后 `p06-final-component-tests.log`：Desktop64、服务13+6+15+5、Hub63、identity31均通过（另3个需明确native probe的owner测试保持原条件ignored）；`p06-journal-gate-final.log` / `p06-journal-final-result.json` 格式、8项打包/工件排除、严格Clippy、全部选择范围测试通过，728条双语检查通过，release构建见 `p06-journal-release.log`。源码/制品哈希见 `p06-journal-checkpoint.json`。

整体验证发现的 P05 Ready→立即exit竞态保留在 `p06-wiring-tests-final.log`；guardian现从真实子进程观察Ready并保持50ms启动观察后发出managed_child_ready，父端仅接收此确认。ready_version=1为明确必要能力，旧guardian组合在spawn元数据处拒绝，不等到超时伪装就绪；业务日志仍独立保存。正常/永不就绪/迟到/普通日志/立即退出夹具及完整服务回归通过；该调整随本轮release一起绑定，不引用旧P05二进制作新范围通过。


## P06 输出绑定 UUID CAS

Store rename/enable/disable/remove 与 native effect 使用 `(expected_output_id, revision)`；A 删除/B 重建且两者 revision=1 时拒绝 A。`p06-binding-uuid-store.log` / `p06-binding-full-tests.log` 包含旧管理写全部 ObjectReplaced、native回调调用数0、nil ID拒绝、binding/sender授权身份保持与并发冲突。`with_expected` 的真实文件锁保持到回调返回；lock 回归确认原生 setter 期间竞争管理者无法获取锁。记录 schema未迁移。

CLI五个管理动作要求必填 `--expected-output-id`/`--expected-revision`（`p06-binding-cli-contract.log`），实际 execute 对五动作的旧 A全部在native路径前拒绝（`p06-binding-api-cas.log`）。IPC和Desktop从所读绑定取得条件并冻结；缺字段旧IPC拒绝。确认框保持原Request，Local Forget在其读取绑定时捕获UUID，Sender startup native name 使用启动 snapshot 的UUID/revision，不能仅用旧name改变后来绑定。

`p06-binding-ipc-contract.log` 使用真实IPC与受控CLI helper：保存回执为A，另一客户端在同步前创建B；后台 sync-name仍发送A的UUID和保存后的revision，不从B加载补条件，保存回执身份保持A，native_name_synced=false。旧rename报固定output_object_replaced，缺UUID老请求拒绝。此为命令/软件状态接线；原生名称实际API/权限及各平台载体仍独立验收，不把helper当真实设备setter。

既有e06 macOS/Linux运行探针已显式携每次snapshot UUID/revision，未自动补当前UUID到旧revision。界面沿用现有错误文字和双语资源；缺合法UUID的只读记录显示升级提示而不发管理写。最终检查随后按实际源码/制品记录；P06主项与HTTP耐久未知/R02仍保持未关闭。

UUID CAS 最终门禁 `p06-binding-gate.log` / `p06-binding-final-result.json` 四项exit0；729条双语严格检查通过（`p06-binding-i18n.log`）。Hub65、绑定7项、Desktop64及服务/IPC16项等选择范围测试通过（`p06-binding-full-tests.log`）；release构建见 `p06-binding-release.log`。Hash快照 `p06-binding-checkpoint.json` 绑定最终代码与三份制品；更新的e06/e07脚本经过AST解析，未当作已执行的设备验收。


## P06 Native HTTP 与配对耐久未知

删除 `persist_saved` 的“replace错误但字节相同→成功”分支，按真实 publication 阶段返回 busy（未发布）或 `profile_durability_unconfirmed`。`p06-native-unknown-tests.log` 的真实文件同字节/目录同步失败用例确认后者不能成为saved成功；保存函数不丢失P06底层阶段判据。

Native未知候选保持 Authority pending、原Command/Principal/PreparedCommand及原SDK worker/socket/lane预备资源。实际会话未加入公开active列表、预备媒体未activate；不同持久写不越过候选。重试相同request/body仅重做同一持久同步，不再分配session/端口/SDK。同步重试即使失败为NotPublished也不会abort掉先前已发布候选；完成回执仍在原commit串行步骤中缓存。已完成的另一个exact replay仍先于pending返回，无save/prepare副作用。

Published撤销立刻拒绝受限身份的认证并撤销实际lane授权/worker；限制独立于DSP配置八槽，queue满也可关闭。当前不可回滚的原始候选仍留待确认，后续sync失败不重新授权。正常durable commit也先关闭终态媒体再尝试DSP收敛；这补齐R01一部分Native接线，AirPlay FIFO及desired/applied进度完整矩阵仍属于P07。

配对registration在未知结果下保留同一prepared/device候选、不开放token。邀请open/cancel在候选期间拒绝，原请求重新redemption恢复后只出现同一device UUID；完成Book replay仍不save第二次。Blocking transaction actor继续不随HTTP客户端断开取消。

`POST /v1/persistence/recover` 只接受原request_id与当前runtime_epoch，并在操作锁下重查现有Admin授权；可确认原Native或registration候选，不形成新业务命令/新媒体。管理员恢复回执缓存32条，回复丢失后重查不再次执行。当前认证拒绝列表不在候选被取出同步时消失；健康及读接口继续可达。数值诊断新增pending/durable而不导出ID或候选内容。客户端P02的Unknown分类已保持原envelope对账，不制造新request_id。

Native/配对、角色/epoch恢复门禁、配置满撤销、同candidate session/device、SDK副作用与旧exact replay回归由 `p06-durability-final-tests.log` 固定（Hub71项）；组件集成结果另列。最初多处编译/用例字段失配保留，未写成平台媒体失败。AirPlay persist_with 在发布未知时仍可能开放新入口/重新允许来源，该路径尚待修复；不能将上述Native结论写成R02或P06完整关闭。


## P06 AirPlay 配置与 Worker 信任耐久未知（2026-10-08）

AirPlay profile 写入保留底层 PublicationError 及原 IO error；persist_with 不把读回相同字节升级为成功。NotPublished 明确失败；已发布未知保留原完整命令、候选 profile、配置准备阶段及原目标 owner。相同 ID/body 只重做原候选同步，ID 换 payload 拒绝；其他配置/worker信任写入不能越过待确认候选。完成回执仍优先重放，不重复保存/副作用。

有效权限投影取旧 profile 与候选的交集，已发布撤销/禁用保持限制，gate/lane 独立于 DSP 队列撤销；同步再失败不重新允许。Enable/Allow/Repair、配对窗口与新注册信任在确认前不应用。部分接收器身份准备继续使用既有私有日志，原请求对账保留 staged UUID/key；所有身份已创建后的同步失败重试不再运行 add_receiver。原会话结束/换 owner 的媒体动作不重定向。停止中的 AirPlay owner 不因管理员恢复重新启动。

Worker 接收 registered 或保存默认偏好时也使用 profile transaction 状态；发布未知保留原候选、停掉当前 worker，不把新信任加入共享有效 profile。管理员读取 pending_command_id（普通成员不获 ID），使用 `/v1/persistence/recover` 携同 epoch 和当前授权确认；后台配对信任恢复结果为 saved_worker_stopped，不自动启动 worker。响应同样有界缓存供丢失回复查询。诊断只输出 pending/durable 布尔值，不导出候选、公钥或秘密。

`p06-airplay-recovery-hub-verified.log` 包含 Hub76项通过；22项AirPlay API回归涵盖真实文件 SyncPublishedDirectory 注入、读回相等但未知、Publish 前权限失败（候选字节也相等）、满 Mixer 队列的 published revoke、重复失败不重复 epoch/action、Allow/Repair 不授权/开窗、部分配置继续同请求、同步重试不再创建身份、worker registration grant保留、实际管理员恢复handler/epoch/auth及回执重放。`p06-airplay-recovery-clippy-verified.log` 严格 Clippy 通过。早期编译和旧回执断言失败日志保留，校正后的结果为准；这些是 macOS 本地软件/文件回归，不是 Windows/Linux 原生持久或实体 Apple/断电证明。完整门禁与release结果另在本节追加。

最终 `p06-airplay-durability-gate.log` / `p06-airplay-durability-final-result.json` 的 format、packaging、clippy、tests均exit0；identity31项单独通过（`p06-airplay-durability-identity.log`），release构建通过（`p06-airplay-durability-release.log`）。源码与Hub/Desktop/Background/Guardian四份release制品绑定在 `p06-airplay-durability-checkpoint.json`。本轮所有编译/缓存/夹具/日志位于项目内，未使用subagent；P06主项和三端验收仍未勾选。


## P07 pending 健康独立发布与原对象合并（2026-10-08）

移除 PreparedCommand 中的完整 Authority 克隆与 PendingTransaction 的 deferred health 列表。PreparedChanges 保留不可变候选、原对象基线/命令/credential/preferences；提交把配置域和该命令的对象变化合并入最新 runtime，不复制过去的事件/回执。输出 unavailable/available、现有session状态在 fsync/pending期间即时publish；abort只清候选，不回滚健康，不重复这些已发布事件。

会话在保存 StreamMix期间终止：偏好gain/mute完成保存，stream不重新创建，新会话不继承旧Solo。注册设备等待期间另一现有session连续Playing/NetworkDegraded/NetworkInterrupted的变化保留；Start在提交时采用最新output availability。回执指向实际提交游标，原请求exact replay仍命中该结果。u64 sequence末尾保留一个提交位置，拒绝无法编号的健康变更且不部分修改公开状态；等值健康更新仍是无副作用成功。

`p07-live-health-cursors-verified.log` 的10项事务回归包括真实Snapshot::apply_event的false→GET→true→commit/abort、副本一致、会话终止和偏好保留、健康期间缓存淘汰/缺号SnapshotRequired、未版本化修改拒绝，以及既有指纹/角色/runtime重放。`p07-live-health-handler-barrier.log` 在真实execute_native persistence seam挂两段Barrier；实际SDK预备线程仍占原reservation，另一线程即时更新健康并调用产品GET handler，副本逐事件收敛；成功/失败都得到当前状态，未靠sleep碰时序。此为真实handler/SDK的本地接线验证，不称作网络WSS重启验收。

`p07-live-health-verified2.log` 中control11项authority+8项transactions及Hub77项通过，新增cursor最后两项在上述10项记录中通过；严格Clippy见 `p07-live-health-clippy-verified.log`。最初旧deferred断言和u64等值更新回归、以及Prepared大小/字段迁移的编译错误日志保留，最终验证结果为准。此阶段明确保留legacy revision/expected_revision，未宣称已完成显式config_revision/event_sequence、GET→WSS跨重启、Native/AirPlay FIFO联合撤销或desired/applied UI；P07主项继续未勾选。最终组件门禁及制品记录另行追加。

最终 `p07-live-health-gate.log` / `p07-live-health-final-result.json` 的格式、打包规则、严格Clippy、全部选择范围测试均exit0：Rust 314项通过、7项维持其原生探针条件ignored，包含control21项与Hub77项。release构建通过（`p07-live-health-release.log`）；最终源码及Hub/Desktop/Background/Guardian制品哈希见 `p07-live-health-checkpoint.json`。只完成本地软件/handler范围，不关闭P07主项或发布平台验收。


## P07 Native 版本域与真实 GET→WSS 重启（2026-10-08）

Snapshot/Event/Receipt提供Native control_version=2、config_revision、event_sequence；revision继续是旧事件序号。持久配置实际差异才前进config，Start/Stop/Solo/健康不改变它；explicit StreamMix gain/mute保存偏好，不让Start制造默认持久偏好。Prepared候选的config固定，commit receipt使用实际最新event_sequence。持久资料保存config字段，旧缺字段资料用保存revision作导入基线，不换Hub身份/ACL。HubSettings既有config字段随日志的旧/新资料一起递增和验证，不能绕过配置版本。

v2写入必须有runtime/credential、对应config/runtime条件并省略旧expected_revision；v1继续原数字CAS语义，新旧条件混用拒绝。Desktop Native意图/Undo用Command::bound生成完整条件，保留原intent UUID；Sender读取已认证me绑定身份，明确冲突刷新不跨原runtime。旧实验v1写入没有v2跨重启保证，该限制保持明确。WSS upgrade前和每batch验证epoch，after-only旧订阅返回426；control_client每次重连先GET并带该epoch/sequence订阅。

`p07-version-domains-tests.log` 8项新域测试通过，范围包括配置CAS不被健康扰动、运行期CAS仍必要、StreamMix双条件/Solo单条件、新旧条件/metadata混用拒绝、pending config固定/回执event最新、恢复旧epoch拒绝、原子副本错误、旧资料导入及config耗尽仍允许runtime。`p07-version-domains-integration.log`：control29项、Desktop64、Hub78、identity31及服务/IPC范围通过；`p07-version-domains-clippy-verified.log`严格Clippy通过。扩大的完整IPC正文使Work触发large_enum_variant，保留inline有明确限定豁免：实际UI队列sync_channel(1)，不削掉冻结条件，不扩大未界定队列；其他警告仍按-D warnings验证。

`tools/control_epoch_probe.py` 与 `artifacts/review-checks/p07-control-epoch-final/result.json` 固定release SHA，实际init私有lab，Hub TLS1.3/BlackHole输出、bearer成员Start与Admin配置事务、WSS事件。GET拿到旧活动session后原Hub真实退出，新Hub从同资料/cert启动；原Start正文重放和旧epoch订阅都snapshot_required，新快照无session/stream且config保持。真实release watch客户端保持跨重启运行，snapshots=2/subscriptions=2，新epoch已清旧session；不是函数层restore代替产品进程，也不是手工GET代替客户端恢复。旧after-only HTTP426，新epoch101。无Sender音频采集、无默认路由变更，不把此控制测试称作媒体质量验收；私有fixture已删除，进程只按本次Popen句柄回收。

首次probe错误把Native snapshot_required预期为409，实际既有契约400；错误结果/清理记录在`p07-control-epoch/result.json`保留，后续正确断言与实际客户端结果为准。制品构建见`p07-version-domains-release.log`。AirPlay版本域及Native/AirPlay旧FIFO/desired-applied仍未完成，P07主项保持未勾选；最终完整门禁与哈希记录随后追加。

最终 `p07-native-version-gate.log` / `p07-native-version-final-result.json` 格式、打包规则、Clippy、测试四项exit0；Rust323项通过，7项维持原条件ignored。identity31项另在集成日志通过。最终release成功，`p07-native-version-wire-result.json` 使用相同Hub制品哈希再次验证同runtime Start完整重放、旧epoch Start拒绝、真实客户端重取；源码和四份release制品绑定在 `p07-native-version-checkpoint.json`。所有fixture/缓存/产物在项目内，私有epoch lab全部清理。P07主项仍未勾选。


## P07 AirPlay 配置/事件版本与确认条件（2026-10-08）

AirplayCommandV2载体新增显式command_version=3与config/event条件，legacy expected_revision为可选且v3不允许存在；旧v2仍保留原字段/数值语义，新字段不混用。角色、基本版本/身份/runtime、已完成回执先于profile初始化和外部写；未知原候选重试先于新CAS。Desktop普通写绑定当前两域，确认Repair/Revoke冻结原完整命令和UUID；未来快照不能替换确认时所读版本。

ReceiverProfile保存config_revision（旧缺字段以1导入）；add_receiver的私有staging每次变化赋新版本，最终配置保存再按实际差异赋号，同一已赋号candidate的同步重试不增版本。新pending identity journal版本2验证这个字段，旧版本1仍按原语义恢复；秘密/引用不迁移或重新生成。Worker配对注册/默认偏好也通过相同cfg赋号，失败保存原候选。配置信息有确认/有效限制两个投影，未知期间configuration保持原已确认配置，立即限制属于业务状态并有新序号。

v3 state有独立业务投影，PCM计数、PIN/动态剩余时间、跨Native域capacity是观察数据；它们不冒充AirPlay配置/业务变化。GET/新命令比较当前投影与上次发布，discovery/ready等未手工递增的业务变化不能沿用旧event_sequence；现有owner显式推进的序号不会重复推进。Native版本另列，未宣称跨域原子读取。u64序号耗尽不能wrap或让变更沿用有效游标，event_sequence=null/snapshot_required且拒绝新写；停止/限制继续安全收尾。

`p07-airplay-v3-integration-verified.log`：Hub84（AirPlay API28）、Desktop66、identity32均通过，包含新增域条件、遥测/业务区分、runtime CAS、未知候选cfg冻结、旧epoch/混用/缺字段、持久身份和cfg重读、序号耗尽、确认框旧版本/ID及gain-mute/Solo条件。identity新测试覆盖旧profile字段导入、staging+最终配置、重复prepare不增加以及cfg耗尽；既有真实SIGKILL/恢复仍通过。`p07-airplay-v3-clippy-verified.log`严格Clippy通过，release构建见`p07-airplay-v3-release-verified.log`。

固定release `artifacts/review-checks/p07-airplay-v3-wire/result.json` 实际TLS HTTPS验证：v2初始化配置但不启用worker，v3定向rename一次cfg增加、完整响应重放、混入expected_revision返回426；原Hub退出/同私有资料重启后receiver/configuration/版本保留，旧v3epoch命令409 snapshot_required。相同制品继续验证Native重放/旧Start拒绝、真实watch两次GET/订阅及旧会话消失，未把控制测试作Apple媒体或波形验收。所有lab/密钥已删除，缓存/日志/制品只在项目内。

最初Option字段调用点、旧直接改status夹具（未读发布游标）及advance_event错误插入EndpointState的编译失败日志保留；修正后的真实快照/状态Owner结果为准。Native/AirPlay队列饱和与旧FIFO/lane复用、desired/applied及产品反馈仍未完成，P07主项保持未勾选。完整门禁/最终哈希另追加。

首次完整门禁在生命周期的hung ordinary query夹具失败：约一秒内未见启动PID标记，未进入Status/Stop断言，非AirPlay/停止功能失败证明。单用例0.62秒通过（`p07-airplay-gate-lifecycle-investigation.log`）；改为五秒启动marker barrier，查询早退和超时都先Shutdown清理后报告，未改Status/Stop断言。实际16项并行组24.08秒全通过（`p07-airplay-lifecycle-parallel-verified.log`）；首败日志保留，最终门禁另记。

最终 `p07-airplay-v3-gate-final.log` / `p07-airplay-v3-final-result.json` 的四项门禁均exit0：Rust331项通过、7项维持原条件ignored；identity32项另外在集成日志通过。固定release实际wire结果与当前Hub哈希一致，完整记录见 `p07-airplay-v3-wire-result.json`；最终源码、文档与四份release制品绑定在 `p07-airplay-v3-checkpoint.json`。临时epoch/profile夹具均清理，本轮无subagent。P07剩余FIFO/desired-applied和各平台完整媒体验收仍未关闭。


## P07 终态 FIFO 授权与 callback 应用进度（2026-10-08）

普通AirPlay apply_action 的Revoke/Disconnect/Disable现在先撤销对应lane授权，不能仅等worker关闭ingress gate；Native终态沿既有revoke_terminals独立失效。Mixer正常退场tail保留原binding，生成inactive配置后也不能绕过撤销；新binding与旧stream/epoch相同也不带旧PCM。实际软件验收边界是观察到失效后的渲染不消费旧lease，未承诺物理声卡立即静音。

MixerConfig命令携目标序号，Control记录不同最新目标desired（包括队列满未入队），同目标retry不再赋号；callback有界逻辑块应用后Release发布applied。discard_backlog在控制线程可能更新内部DSP但不发回调确认。读取applied再desired保持合法配对。原八槽/48k块扫描上限和Realtime零分配没有扩大。Hub纯配置不在生成目标前做capacity preflight，Start外部SDK仍按原预备规则。回执/诊断media_application有epoch、desired/applied/pending、时长、两秒stalled和拒绝次数；mixer_dirty只表示enqueue重试，不作已应用。

Desktop为已保存字段维护有界context/target观察。只有新读数同epoch、applied达到目标且当前字段仍相等才显示DSP已应用；诊断缺失→进度未确认，scope切换清除，其他人更新→该值已更新，绝不重发已保存写。原undo是保存后的独立逆操作。沿用command_note行内16px、底栏36px、警告文字/完整tooltip-AX及共享Message，不改变token/布局。增加四条双语消息，精确技术ID例外移至实际行428（未扩大白名单）。

`p07-native-fifo-gate-final.log` 真实execute_native Native Stop/未知Revoke→实际Mixer、另一Timed来源连续；`p07-api-fifo-gate.log` 初始AP实际case，最终集成覆盖Revoke/Disconnect/Disable三种→Timed FIFO撤销、另一Native持续及同ID新binding。PCM直接注入实际owned handoff，API/authorizations/DSP都是真实接线，不把它称作DTLS或Apple端媒体测试。起初Native构造AudioBlock每包保留START导致不能priming，修正为真实连续块NONE，早期波形失败保留。

`p07-config-progress-core.log`/最终core范围：队列满时desired2/applied1不同，retry仍2，旧积压不确认新目标；实际callback确认后才2/2，discard控制线程不冒充，非法配置不改目标，退休尾部lease与实时零分配通过。`p07-application-desktop-verified.log` Desktop68项（新增pending/stalled/unknown、不重发、foreign runtime/房间变化）；`p07-application-integration-verified.log` 含Hub87、core及服务范围通过。`p07-complete-replay-matrix.log`补齐128回执淘汰/不同身份/已结束Start及身份优先于协议形状；Core版本域10项通过。

`p07-application-i18n-verified.log` 733条双语AST/呈现检查通过；`p07-application-premium-verified.json`静态UX审计零违例/未解决。实际macOS截图：`p07-application-preview/stalled-600-zh.png` 最小窗固定底栏/滚动未移位，`pending-1100-en.png` 总控旁待应用提示和完整底栏。预览仅读公开夹具和注入application状态，不连后台或采集音频；e07历史读数仅验证呈现，不冒充当前声学测量。native截图与egui键盘/AX回归支持改动范围，真实IME/VoiceOver仍属于P08/P11。

`p07-application-wire/result.json`固定release再执行实际TLS Native/AP版本/完整重放/重启+watch；BlackHole实际output callback确认desired=applied=3，同epoch成立。应用回执pending等于desired>applied，随后新diagnostics确认，私有lab清理。release构建见`p07-application-release.log`；最终完整门禁另列。未把Wave fixture或回调确认作为三端实机、模拟端延迟或长期验收。

最终 `p07-application-final-gate.log` / `p07-application-final-result.json` 四项exit0，Rust342通过/7保持原条件ignored；源码/四份release与实际wire绑定在 `p07-application-checkpoint.json`。P07软件阶段checklist已完成，P08及三端媒体/实体/长测/最终包仍未完成；总goal保持active。


## P08 诊断新鲜度与Sender身份（2026-10-08，进行中）

沿用工作区已有Measurement/ObservationClock，核对实际接线并补齐顶栏/总控读取、每来源年龄、Sender目标和电平绑定。输入源码/差异哈希见p08-input.json。Desktop已增加到78项，Available全零/缺字段/不可用/过期、控制GET继续成功但诊断持续失败、有效静音/无新采样/近期错误、目标房间与身份切换及旧session电平拒绝通过；p08-desktop-scoped-verified.log。后台真实record_process_line在SenderStart前锁存认证目标，普通stats不能改写；observed_snapshot给出真实stats年龄，export_whitelist不保留目标身份、名称/地址/路径/未知字段，p08-semantic-tests-final.log。

截图首轮发现顶栏缓存电平仍在过期时出声视觉，保留p08-preview初始证据；随后顶栏/行式总控改为diagnostics_current。MeterStats沿既有有界原子快照扩展binding_generation/stream_epoch/output_epoch/observed；callback按绑定或输出重开清电平窗口，Hub对当前控制绑定校验，无样本/旧代返回available=false与null，不制造有效零。Desktop另外核对同session/stream_epoch。p08-realtime-final.log实际release的Mixer8项、lane_binding4项与callback零分配通过；首个新增用例按一次next_frame判定错误，修正为真实480帧逻辑块边界，产品应用策略未变。

初轮服务Ready夹具在全包并行构建负载下超过一秒，但单用例1.08秒完成全部五场景；保留p08-initial-tests.log/p08-ready-isolated.log。允许就绪场景的夹具启动预算改为5秒，与此前marker测试一致，未改失败场景100ms期限或产品MEDIA_START_TIMEOUT。p08-semantic-tests-final.log服务14单元/6intent/16lifecycle/5fault及Hub87通过。最终完整门禁、重截屏和哈希正在补齐，P08主项暂未勾选；未把纯软件状态/preview当作Apple音频、真实IME或VoiceOver验收。

最终 `p08-final-result.json` 四项门禁exit0，Rust353项通过/7项保持原生条件ignored；743条双语AST/呈现检查通过。release构建成功，四份制品和源码哈希见 `p08-checkpoint.json`。最终窄窗截图 `p08-diagnostics-600-zh-CN.png` 已确认过期诊断为未取得、顶栏空电平，独立有效Sender采集仍保留；`p08-sender-600-zh-CN.png` 控制Studio B/发送Studio A文字同时正确，1100×760英文图保存在artifacts/review-checks/p08-preview-final。只在原定位更新技术文字例外（未扩大字面白名单）；premium静态审计零违例/未解决。所有preview临时fixture已删除，本轮无subagent。P08软件阶段完成，D07真实IME/VoiceOver、三端GUI和长期媒体验收继续未勾选。


## P09 owner、运行期限与防反馈（2026-10-08，最终接线验证中）

UntilStopped/Duration采用同一RunLimit，桌面显式持续，手工CLI仍默认10秒，--seconds限制1..86400且与--until-stopped互斥。可控elapsed跨86400边界以及u64范围回归通过；实际TLS/BlackHole数字输出CLI在限时结束记录duration_elapsed、持续模式私有stop记录user_stopped。原远端Stop对账失败不再吞掉原media失败，失败为failed。实际loopback/LAN/IPv6同端点在open_capture/play前返回local_feedback_loop，不生成session；接口失败/元数据不足不当作安全。固定证据p09-sender-native.json，完整短测日志在artifacts/review-checks/p09-sender-native。

Linux沿UID内核socket排他机制增加NUL+有界JSON握手；版本/当前UID/后台代次/binding UUID/ready都核对，显示名不是owner。后台和生产binding CLI共用output-binding的同一平台client；后台还把原代次传给Sender。不同UUID或实例明确resource_owned_by_other_instance，缺句柄不收养。p09-linux-owner-native.json实际PipeWire节点/两个state目录/释放后重取通过；p09-linux-background-final.json实际两个Linux背景进程、guardian、私有IPC和真实节点，B启动被拒绝、B退出不破坏A、A停止后B重开取得owner及最终kernel socket释放通过。该后台场景的远端Sender CLI为夹具，明确没有测真实Linux媒体质量。

交叉编译入口补齐目前CPAL所需DBus开发/运行包，全在.local/cross-linux；Linuxshared owner/后台严格Clippy、release ELF链接以及Mac相关软件范围通过。首次交叉检查缺CC/直接依赖，首次后台探针误用换行而非长度IPC帧；失败日志保留，纠正夹具和依赖后结果为准。无全局kill/默认路由修改，三轮所有owned子进程与私有profile夹具均清理。最后IPC核对又发现诊断redact会把meter session_id剔除，已将仅meters/lanes里合法运行期UUID/数值/null保留；普通身份和导出仍剔除，真实redactor/export回归接入。

用户在本轮明确暂缓真实Apple、Windows/Ubuntu可见桌面和8/24小时长测。这些验收项目继续未勾选；本轮后续收尾限于代码、软件/数字路径与项目内候选包，不据此宣称发布放行。项目标准check.py已有--only字典语法错误已修复，完整workspace/原生包组件与短媒体检查最终结果另列。

P10前最终补验发现GET可以读取冻结音频callback的旧电平。现沿已有callback钟发布sampled_at_ns，Hub转换真实sample_age_ms，Desktop单独判断电平年龄。实际Mixer固定100/500ms发布钟和GET反复成功但样本4000ms的回归、release实时零分配通过；79项Desktop。P09 IPC运行期session关联也已修正且不会进入白名单导出。最终制品/完整检查和候选包以下节为准。


## P10 项目检查与macOS候选包（2026-10-08）

最终标准 `tools/check.py --keep-going` 全部18项exit0，workspace 433项Rust测试通过；包含格式、严格Clippy、双语、文件凭证、release、模拟、HAL ABI/bundle/host/vendor、媒体runtime/双路/丢包重放/错误指纹与release漂移。见p10-standard-result.json；各子日志在artifacts/checks。电平独立sampled_at_ns/age的最后变更已包含在本次检查，实际Mapper时间、冻结callback/新GET、IPC保留合法meter session并剔除导出的定向回归另附。

`artifacts/installers/P10 稳定性 最终`保存ad-hoc签名app与DMG；DMG SHA256、包内29份库与制品见p10-macos-package-final.json。在第二个中文/空格目录复制完整app，独立Mach-O读取无开发引用，移除全部开发运行路径后五个payload启动、包内GStreamer和双路DTLS媒体完成；dyld加载路径未指向开发GStreamer。p10-relocation-final.json记录每payload哈希与临时app清理。只设置包内插件/扫描器和项目内registry/cache，与实际launcher一致；没有执行会把资料写入默认~/Library的launcher，也不把payload验证当作GUI或全新/升级安装验收。

本地origin为私有GitHub仓库，基础stability工作流仍在待提交修改中，实际PR门禁正在整理。未合并发布，用户暂缓的实机/桌面/长测仍未关闭。


## P11 软件组合（2026-10-08，实机/长测暂缓）

同一最终Hub制品五组合4+0/3+1/2+2/1+3/0+4通过；每组合按startup/steady/fault/recovery/stopped记录，AirPlay20次定向disconnect/worker crash恢复、未受影响Native/AP会话连续性及既有严格质量计数条件通过。配对身份保留、容量拒绝不泄露reservation、全禁用关闭worker/运行key通过。p11-matrix.json及各组合原始result记录制品与场景；不是Apple硬件、独立模拟端延迟或资源长测。首轮3+1在disconnect阶段TypeError：新契约inactive meter为null，旧探针直接做数值比较；保留artifacts/review-checks/p11-digital首败，改为核对available=false/stream_id=0和实际callback render_state=Inactive/Stopped，不伪造零值，活动来源等待有效值。没有降低连续性或计数阈值。

最终p10-final-wire.json再次验证真实Native/AirPlay版本组合、旧条件拒绝、完整响应重放、Hub重启GET→WSS重取与callback desired/applied；和上述五组合使用相同Hub制品。所有数字probe私有资料和owned进程已回收。
