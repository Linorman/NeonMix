# 平台测试修复与 macOS 复验（2026-10-03）

本轮针对 [最新三端测试报告](PLATFORM-TEST-20261003.md) 修复准入竞争、身份读取、Windows 媒体 IPC 和原生 PCM 交接。仅在 macOS 执行运行测试，Windows/Linux 做本机静态或库兼容性验证；没有更新远端运行包。

## 已定位并修复

### SETUP 403：把短暂锁竞争当作授权拒绝

旧 Hub 对 `profile.try_lock()` 的 `WouldBlock` 立即回复拒绝，状态查询与持久设置事务都可能触发。无额外查询的旧版四路试验完成 24 次恢复；增加 4 个并发状态读者后，第三路断开恢复在有空余容量时 SETUP403，期间 7,782 次查询全部成功。保留 [原版失败](../artifacts/platform-fixes-20261003/baseline-contention.json)，不把偶然通过当作已无问题。

现在 profile 和 Engine 锁均无阻塞重试，沿用读取事件时记录的 400ms 截止时间，不在超时后占用名额；worker 原有 500ms 等待不变。授权仍检查当前权限、代际、容量和来源。已失效的 start/grant 在等锁前丢弃；拒绝原因局部累计，在 Engine 可用时发布，诊断不阻塞媒体。新增回归覆盖锁竞争、截止点和断开后的旧 grant。

第一轮修复版四路带 4 个状态读者完成 40 次断开/SIGKILL及恢复，31,593 次状态查询无失败，各采样稳态/故障/恢复区间存活路欠载和 late 增量为零，见 [压力复验](../artifacts/platform-fixes-20261003/fixed-contention.json)。最终制品的复验结果另列于下表。

### Linux 身份崩溃、Windows OpenSSL Applink

Hub 的 ring/rcgen 生成带公钥字段的 PKCS#8 v2，而旧 OpenSSL 3.0 解析路径拒绝此格式。上游 `pairing_init_generate` 没检查 Ed25519 loader 返回的 NULL，导致读公钥时崩溃。macOS 用项目已有 OpenSSL 3.0.9 源码构建对照：同一个 seed 的 v1 能读取，ring v2/legacy v2 均失败；本机较新库能读取，解释了平台差异。此实验验证 3.0 系列机制，不能当作 Ubuntu 原环境同库版本复验。

loader 改为自身 CRT 读取文件，再用内存 BIO 解 PEM，避免 MinGW `FILE*` 跨入 MSVC OpenSSL 的 Applink 路径。严格接受所用 Ed25519 v1/v2 封装，检查 v2 公钥与 seed 一致；损坏/错误算法/缺失身份干净失败，不改写或重新生成身份。pairing 正确传播 NULL。

完整 macOS worker 的 17 项身份测试通过：4 种合法编码保持同一公钥及 RFC8032 签名，13 类错误 exit1、未 ready、未崩溃，文件不变；修复 loader 链接 OpenSSL 3.0.9 同样 17 项通过。证据：[旧库对照](evidence/platform-fixes-20261003/openssl-3.0-compat.json)、[旧库修复验证](evidence/platform-fixes-20261003/identity-openssl-3.0.json)、[完整 worker](evidence/platform-fixes-20261003/identity.json)。

### Windows NamedPipe 与平台 Clippy

worker 新增与 Hub 匹配的私有 NamedPipe 客户端，发送 token 前验证父 PID/进程创建时间、同用户 SID、受保护的 owner-only DACL，禁止服务器 impersonation。非阻塞发送整包预算 250ms，停止标志在写入间隙检查。Hub 保留客户端 PID/SID 验证；未退回 TCP。

补齐 Linux/Windows 三处 unsafe 安全注释，以及 IPC crate 单独 Windows 编译缺失的 `Win32_System_IO` feature。macOS-hosted Windows C++ 语法检查、Windows/Linux IPC 严格 Clippy 和 macOS IPC 13 项通过。证据 [windows-static.json](evidence/platform-fixes-20261003/windows-static.json) 明确区分静态检查和原生运行；Windows 实际启动、ACL 拒绝和背压仍需当地复验。

### 原生 PCM 队列溢出放大

一个合法 120ms PLC 样本有 5760 帧，会拆为 12 块，超过 Mixer SPSC 的 8 槽。旧实现连续 push 不仅丢当前块，还让已排队整代音频失效。现在最多暂存一个解码样本，按空槽交接；保留到达时间、80ms 新鲜度限制与原有播放水位。新增实际 GStreamer appsink 回归验证前8块/后4块分段交接且内容、位置连续，无 GAP/dropped/stale。媒体 crate 的14项测试与严格Clippy通过。

Ubuntu 原始末活动样本的 `lost_packets=445` 全等于 `overflow_plc_packets`，jitter `num-lost/late=0`；首末活动样本 received 与 highest_sequence 都增加5614，timestamp增加 `5614×480`，这段认证RTP无序号缺口，见[计数核对](evidence/platform-fixes-20261003/ubuntu-native-forensics.json)。原始 `queue_drops=154560` 帧表明交接丢弃被放大。本次修复确定的队列缺陷，尚不能宣布 Ubuntu jitter 驱逐及整条音频质量已修复。

## 仍未完全定位

原报告四路恢复的 `media_transport` 没有 errno、写入偏移或 reader 终止原因，无法唯一归因。此次发现 Hub 的明确背压/分帧超时可能被随后 worker 的通用写失败覆盖，已调整两条终止路径的分类优先级并回归；decoder/control 故障不被覆盖。没有为未知原因放大超时。后续复验通过不等于解释了历史每一次失败。

独立后台首轮脱敏宽断言仍缺具体字段证据，不能断言泄露或宣布已找到产品根因。Windows Session 0 GUI、真实 Apple 多设备、物理音画延迟、8/24小时长测及发布签名继续不在本轮通过范围。

## 运行与证据

所有开发命令通过 `tools/dev`，下载/缓存/临时资料/编译产物均位于项目。macOS 数字输出明确选择 `coreaudio:BlackHole2ch_UID`，没有改变系统默认路由或操作原有用户会话。合成加密来源验证真实 Hub/worker/解码/Mixer/CoreAudio 路径，不等于 Apple 设备互操作或模拟端实听。

最终源码与制品哈希：[provenance.json](../artifacts/platform-fixes-20261003/provenance.json)。复验命令、耗时、退出码由 [runtime-checks.json](../artifacts/platform-fixes-20261003/runtime-checks.json) 保存；可复现编排在 [run-runtime.py](../artifacts/platform-fixes-20261003/run-runtime.py)。

## 最终 macOS 验证结果

| 检查 | 结果 |
|---|---|
| Rust workspace | 228 passed / 0 failed / 5 ignored；随后最后诊断改动的 Hub 全部54项通过，含新增分类回归，不把重复用例相加 |
| 格式 / Clippy / release | workspace格式、严格Clippy、release通过；最终Hub再检查通过。格式结论不扩大到 excluded vendor |
| Worker 协议 / 配对 / 解码 | 23项协议、5项配对与PCM/ALAC/AAC通过 |
| 身份 | 完整worker 17项；OpenSSL3.0.9 loader 17项通过 |
| 4 AirPlay + 0 原生 | 3轮、24次断开/SIGKILL恢复；30秒稳态；4并发读者、31,819次查询无失败 |
| 3 AirPlay + 1 原生 | 6次故障恢复通过 |
| 2 AirPlay + 2 原生 | 2轮、8次故障恢复通过 |
| 1 AirPlay + 3 原生 | 2次故障恢复通过 |
| 0 AirPlay + 4 原生 | 60秒稳态通过 |
| Mixer 持久偏好 | 9个观察检查通过，−6dB/Mute/单Solo/多Solo/清Solo、断开重连保留gain/mute而清Solo；4个状态读者同时运行 |
| 管理 | 同源跨入口限制、撤销、新PIN重新配对及身份保持通过 |
| 单路回归 | PCM、ALAC各两次暂停+重复SETUP恢复通过；timed late=0 |
| 独立后台 | 12/12场景通过，诊断/导出脱敏通过，fixture清理；原有7443用户Hub保持原PID |

五种组合合计40次最终制品故障恢复。所采样稳态/故障/恢复窗口的存活来源欠载与late增量均为0，session保持；不把离散计数采样称作模拟端无缝播放证明。[逐组合摘要](evidence/platform-fixes-20261003/macos-runtime-summary.json) 与原始JSON均保留。各探针确认自有进程停止、fixture文件删除。

验证日志：[workspace](../artifacts/platform-fixes-20261003/workspace-tests.log)、[最后Hub测试](../artifacts/platform-fixes-20261003/hub-tests-final.log)、[Clippy](../artifacts/platform-fixes-20261003/clippy.log)、[最后Hub Clippy](../artifacts/platform-fixes-20261003/hub-clippy-final.log)、[release](../artifacts/platform-fixes-20261003/release-final.log)、[格式](../artifacts/platform-fixes-20261003/format-final.log)。

后台复验：[12场景结果](../artifacts/platform-fixes-20261003/background.json)、[清理核对](../artifacts/platform-fixes-20261003/background-cleanup.json)。探针现使用自有空闲 loopback 端口及项目内副本，不占用用户7443。首次 runner 复制位置破坏 `@loader_path` 相对库路径，尚未进入音频即失败；保留[首败](../artifacts/platform-fixes-20261003/background-runner-rpath-first-failure.json)，恢复相对目录布局后通过，未改产品。这不解释原报告的脱敏宽断言首败。
