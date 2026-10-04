# Windows 更新版原生实验报告（2026-10-03）

Windows 的旧 NamedPipe 地址拒绝、PEM/OpenSSL Applink 与严格 Clippy 阻断已在新快照上通过原生复验。完整构建、身份/协议/配对/解码、真实 Hub 多入口媒体和后台运行均有通过证据。但 **3 AirPlay + 1 原生首轮三个 worker 因 decoder_queue 退出**，同条件复验通过，原因尚未定位；Mac→Windows 首轮 60 秒出现 2 个 overflow PLC 和 480 帧欠载，保留首败，同 profile 重启后 60 秒复验计数全部为零。Windows GUI 仍因 SSH Session 0 无可见窗口，完整桌面交互未验收。

## 版本与环境

测试机器 `192.168.100.186`，用户 `administrator`，Windows 11 `10.0.26100`、AMD64、Rust 1.95.0 MSVC、Python 3.13.5、官方 MSVC GStreamer 1.26.10；C++ worker 使用已有 MinGW GCC。全部源码、工具、缓存、临时资料、SDK、编译产物与日志限于 `E:\Desktop\NeonMix-test`，本轮独立目录 **`E:\Desktop\NeonMix-test\r04`**。旧目录、旧包及用户原有后台保留。

共同冻结快照 516 文件，canonical manifest SHA256 `3ef7d6f5ea47a98b671768030311a494bd95c317c1b032bbc38bd00c038b9c84`；源码归档 SHA256 `8360bc83eca215b0fb81a99c4916f69e26ce62ffef3f1d253bdffc22cc97f352`。初始与最终逐文件均 **0 差异**。没有修改冻结的产品源码，没有用旧包/旧 EXE 替代本轮构建。详见 [源码与制品哈希](final-provenance.json)、[环境](environment.json)。

所有开发命令通过 `run.ps1` → `tools/dev.ps1`，项目内 Cargo cache、AirPlay 下载缓存及 Rust 工具链 junction 复用，`target` 独立重建；已有系统编译器只读复用。GStreamer SDK 原先的 r03 junction 被 `prepare_airplay.py` 的项目内路径守卫拒绝，改为复制到 r04 SDK 后重新构建成功；保留 [首败](worker-build-junction-first.log)、[成功构建](worker-build.log)、[probe 构建](worker-probes-build.log)。该失败属于本轮环境编排，没有放宽产品路径检查。

## 构建与常规检查

| 项目 | 结果 |
|---|---|
| workspace 自动测试 | 42 个 target，**214 passed / 0 failed / 5 ignored** |
| workspace release | MSVC 全 workspace 原生重建通过，138 秒 |
| 严格 Clippy | `--workspace --all-targets --locked --offline -- -D warnings` 通过 |
| 格式 | 显式列出 16 个 workspace package 后 `--check` 通过；`fmt --all` 仍受嵌套 excluded vendor workspace 发现影响，不是源格式错误，未改写格式 |
| C++ worker 与三个 probe | 同快照 MinGW 原生编译通过 |
| GStreamer 数字媒体 | runtime、5 秒双路、5 秒丢包重放、错误身份拒绝通过 |
| 文件凭证 | native locale 下 9 类全部通过、fixture 清理 |
| WASAPI | 本轮 EXE 设备枚举与指定 VB-Audio Virtual Cable 3 秒原生输出通过，48 kHz / 480 帧周期，设备错误与 callback over-budget 均 0 |

证据：[命令/耗时/退出码](suite.json)、[测试](tests.log)、[Clippy](clippy.log)、[成员格式](fmt-members.log)、[release](release.log)、[运行命令](runtime-checks.json)、[文件凭证](credentials-native-locale.log)、[数字媒体](runtime.json)、[原生输出](native-tone.log)。5 项默认忽略没有计入通过；独立 excluded 迁移工程的完整矩阵、驱动编译/加载与发布签名不在本轮范围。

## AirPlay worker 与 Windows 私有 NamedPipe

完整本轮 worker 的 **17 项身份测试**通过：4 种合法 Ed25519 封装保持公钥与 RFC8032 签名且 ready，13 类损坏/缺失/错误封装干净 exit 1、未 ready、未崩溃、文件不变。随后协议 23 项、配对持久性 5 项、PCM/ALAC/AAC 解码均通过。真实 v2 Hub 的所有通过组合同时走 NamedPipe、完整 PEM 读取、认证数字源、定时 Mixer 和真实 WASAPI 输出，旧启动阻断在本机已消失。

独立原生 NamedPipe fixture **6/6** 通过：合法父进程、当前用户 SID、受保护 owner-only DACL 的 pipe 进入 ready，收到 65 字节合成 token；广泛 DACL、未保护 DACL、非父 server PID、缺失 pipe、非法名称均 exit 1、未 ready，已检查的服务端没有收到 token。正向路径验证当前 SID，未声称跨账户 SID 拒绝或 PID 复用模拟；未做单独 pipe 饱和背压时限测试。

证据：[身份](identity.json)、[worker 命令](worker-suite.json)、[协议](worker-protocol.log)、[配对](worker-pairing.log)、[codec](worker-codecs.log)、[NamedPipe 原生安全](pipe-security.json)。`worker-pairing.log` 的 platform 字面及部分首次多路 probe 的 CoreAudio scope 文案继承 macOS harness；实际本轮执行命令、Windows EXE 哈希、Windows 平台字段与 WASAPI ID 均另有记录，不把该文案当作平台证据。后续 harness 已修正这些标签；产品文件未改。

## 多入口音频、故障恢复与管理

| 输入组合 | 结果 | 断开 / 强杀 / 恢复 |
|---|---|---|
| 4 AirPlay + 0 原生 | 20 秒稳态、4 并发状态读者通过 | 两轮，共 16 次恢复 |
| 3 AirPlay + 1 原生首轮 | **未通过**，三个 worker `decoder_queue` 退出 | 尚未进入计划故障阶段 |
| 3 AirPlay + 1 原生针对性复验 | 同条件 20 秒稳态、4 读者通过 | 6 次恢复 |
| 2 AirPlay + 2 原生 | 20 秒稳态、4 读者通过 | 4 次恢复 |
| 1 AirPlay + 3 原生 | 20 秒稳态、4 读者通过 | 2 次恢复 |
| 0 AirPlay + 4 原生 | 60 秒稳态通过 | 本场景不注入 AirPlay 故障 |

通过组合合计 **28 次恢复**；4 读者的通过组合共 44,090 次状态查询、0 失败。所有已保存稳态、故障及恢复窗口的存活 lane 欠载/late packet 增量均 0，存活会话/身份保持；不把离散计数采样称作实体扬声器无缝音频证明。2+2 在稳态前累计 timed late 1 帧，3+1 复验累计 15 帧，稳态期间未增加，不能宣称从启动起绝对零迟到。

首轮 3+1 已成功建立三个独立加密来源并进入播放；之后额外 native Start 的满房间拒绝检查失败时，快照显示三个入口全部 `enabled=false`、`worker_failed`、`failure_stage=decoder_queue`，late packet 分别 **193 / 194 / 196**，Mixer `timed_late_frames=7292`，房间只剩原生 1 路。11,261 次并发状态查询全部成功，不能据此说媒体通过。完整首败 [mix-3plus1.json](mix-3plus1.json) 保留。只做一次针对性复验，6 次恢复完成；逐秒系统 CPU 平均 **5.846%**、最大 **18.711%**。复验没有复现该退出，**未解释首次 decoder_queue 根因**，没有因此再次重复全部矩阵。

额外 Mixer 持久偏好 **9 个观察**通过（gain、mute、单/多 solo、清 solo、断开恢复的保留与清除）；管理 **4 个观察**通过（同源跨入口仅一个 owner、增加/移除空闲入口保持其他来源、撤销不可用旧签名恢复、新 PIN repair 与身份保持）。单路 PCM、ALAC 各两次暂停 + 重复 SETUP 恢复、原生共混及 worker 强杀隔离均通过，两个单路报告 timed late 为 0。

逐组合原始数据及计数汇总：[summary.json](summary.json)、[四 AirPlay](mix-4plus0.json)、[3+1 复验](mix-3plus1-retest.json)、[复验 CPU](mix-3plus1-retest-cpu.jsonl)、[Mixer 偏好](mix-control.json)、[管理](management.json)、[PCM](single-pcm.json)、[ALAC](single-alac.json)。探针自身停止进程并删除各自 fixture。

## Mac → Windows 60 秒协同

主线程使用同一 516 文件源码新构建的不可变 macOS client，经正常邀请配对、原生网络→Mixer→WASAPI，房间 `NeonMix Windows R04`、endpoint `https://192.168.100.186:17446`，明确选择 VB-Audio Virtual Cable 输出。两次均在 Windows 无本轮编译或额外压力探针时运行，并保存 2 秒间隔的诊断及 Windows 系统 CPU。

首轮正常 pair/send 通过，Windows 有 30 个活动样本；发送约 14 秒新增 `lost_packets=2` 且全等于 `overflow_plc_packets=2`、Mixer 欠载 **480 帧**，随后没有增长；PCM sink/queue drop、late、输出错误、callback over-budget 为 0，持续 playing/非零电平。活动 CPU 平均 **2.179%**、最大 **5.292%**。此轮存在运行瞬态，未称作全程零欠载。

只重启本轮自有 Hub、保持同 profile/身份并清零累计后，第二次正常 pair/send 60 秒通过；Windows 29 个活动样本的 lost、overflow PLC、queue/PCM drop、late、timing gap、Mixer 欠载、输出错误及 callback over-budget 全部 0，非零输出。活动 CPU 平均 **2.525%**、最大 **6.02%**。首败未被复验覆盖，2 个 overflow PLC 的原因尚未唯一定位。

首轮完整 [诊断](lan-first/lan-diagnostics.jsonl)、[重启前 profile digest](lan-first/profile-digests.json)、复验完整 [诊断](lan-diagnostics.jsonl)、[前](lan-before.json)/[后](lan-after.json) 与 [汇总](summary.json) 均保存；Mac 端 CPU/发送/playing 证据由主线程保留。不是物理扬声器实听、Apple 来源互操作或端到端模拟延迟证明。

邀请编排首败另保留：TTL 600 超出产品 1..300 范围，随后试图覆盖旧邀请被 CLI 拒绝，我未及时检查刷新日志导致转发旧邀请、Mac 配对 expired。之后使用全新文件生成 300 秒并核验实际到期时间，正常配对完成；远端与 Mac 当时 epoch 约差 1 秒，**不是系统时钟阻断**。见 [刷新首败](invite-refresh-first-failure.log)、[只读时钟/新邀请核对](clock-refresh-check.json)。没有放宽产品过期校验或修改系统时间。

## 后台、GUI 与清理

完整独立后台 **12/12** 场景通过：真实 NamedPipe、私有状态目录和导出 ACL、文件凭证引用、真实低电平输出、管理员身份快照、客户进程崩溃后 Hub 保持、邀请取消与自连接拒绝、诊断/导出脱敏、Hub 故障被报告且不无限重启、显式重启/停止/退出后台清理。首轮 POSIX fixture 只用 `mkdir(0700)`、未应用 Windows ACL，在 status 前 timeout；补齐自有目录私有 ACL 后复验通过。首败没有保留后台 stderr，不能仅凭 timeout 唯一归因。复验日志中的 icacls UTF8 解码警告是 harness 控制台编码噪声；实际状态目录及导出权限另经 `Get-Acl` 原生断言通过，纯文件凭证 9 类另有 native locale 干净复验。详见 [后台结果](background-full.json)、[首败](background-full-first-failure.json)、[后台日志](background-full.log)。

本轮新 desktop EXE 实际启动五个 preview 页面及正式模式，各 8 秒，均仍为 SSH Session 0，只能枚举隐藏窗口、无可见窗口且未自行退出。测试只终止自己启动的 UI。因此可见 GUI、托盘、中文输入、关闭窗口保留音频等交互 **未验收**；[gui.json](gui.json) 的 runner exit 0 仅表示采集完成。

最终 r04 自有 NeonMix 进程 **0 残留**；本轮 LAN profile/邀请、GUI fixture 与探针私有文件已删除，本地中转邀请也已删除。原有 `E:\Desktop\NeonMix-test\target\release\neonmix-background.exe` PID **33236** 保持运行。源码、release EXE/PDB、SDK/cache/native build 和全部实验记录保留 r04；旧手测包保持。没有删除指定目录外文件、安装驱动、改防火墙、持久 ExecutionPolicy、服务、默认音频路由、权限策略或时钟。[清理](cleanup.json) 与 [最终哈希](final-provenance.json) 可核对。
