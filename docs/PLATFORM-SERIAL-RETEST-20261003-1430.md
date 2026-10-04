# Ubuntu 与 macOS 严格串行重测（2026-10-03，14:30 快照）

本轮 Ubuntu 和 macOS 均重新执行了构建、常规测试和运行实验。两端五种四路组合的稳态断言通过；macOS 的故障恢复观察窗也全部通过。**Ubuntu 原生跨机音频质量仍失败，AirPlay 故障恢复窗口仍有聚合跳帧；同源码上一轮四路失败尚未解释。** 因此本轮通过不能解释为所有间歇问题已经修复。

按用户最新要求，本轮仅重新测试 Ubuntu 和 macOS。Windows 保留前一轮同源码结果，单独列为历史附录，没有计入本轮重新执行的测试。

## 版本、执行顺序与环境

冻结源码共 519 文件，包含当时未提交开发内容；开始与结束逐文件校验均无差异。本轮没有修改产品源码或放宽质量断言。规范 manifest SHA256：

```text
53ee66ce3ed30a46bb08dafa9a47dfe955aac41313e4d3b15e345f8dbe3d8e5e
```

本轮与 12:41 轮次源码完全相同。新归档 SHA256 为 `f0cdbce09265121b1257d99c996585618fa75271b1d1adf5d15b0d5be3152c18`，归档字节差异不代表源码变化。[快照](evidence/platform-serial-retest-20261003-1430/snapshot.json)、[逐文件清单](evidence/platform-serial-retest-20261003-1430/source-manifest.json)可核对。

| 平台 | 环境 / 输出 | 新执行时间与工作区 |
|---|---|---|
| Ubuntu `192.168.100.112` | Ubuntu 24.04.3 ARM64 / Parallels，GStreamer 1.24.2，OpenSSL 3.0.13，PipeWire 1.0.5；显式 HDA 输出，48 kHz / 1024 帧 | 14:34:37 开始，14:48:00 清理确认；`/home/parallels/NeonMix/.local/tl3` |
| 本地 macOS | ARM64，项目内 GStreamer 1.28.7；显式 BlackHole 数字输出 | 首条新检查 14:48:23；`/Volumes/projects-mac/NeonMix/.local/tm3` |

时间为 Asia/Shanghai。Ubuntu 的构建、单路、多路、GUI、专用 LAN 完成并清理后，才开始 macOS 的构建和本机测试，间隔约 22.62 秒。[实际命令和进程生命周期审计](evidence/platform-serial-retest-20261003-1430/serial-order-audit.json)证明新轮没有 Ubuntu/macOS 本机测试重叠。Mac 仅在 Ubuntu 专用跨机窗口充当发送端，期间没有其他本机实验。

缓存与已编译依赖允许复用，但每条检查和运行实验均实际重新执行，使用新时间戳及新日志；没有沿用旧通过记录。CPU 只采样，不设置强制起步/中止门槛。Mac 原有四个用户实例保留，测试不等于整机没有其他后台负载。

## 新执行的构建、单路和后台检查

| 检查 | Ubuntu 新轮 | macOS 新轮 |
|---|---|---|
| workspace 常规测试 | **222 passed / 0 failed / 5 ignored** | **232 passed / 0 failed / 5 ignored** |
| 格式 / 严格 Clippy / release | 正式成员格式、workspace/all-targets `-D warnings`、release 通过 | 正式成员格式、workspace/all-targets `-D warnings`、release 通过 |
| 当前 worker / CMake probes | 构建通过 | 构建通过 |
| 协议 / 身份 / 配对 | 23 / 17 / 5 项通过 | 23 / 17 / 5 项通过 |
| PCM / ALAC / AAC 解码 | 通过 | 通过 |
| 文件凭证 / 独立后台 | 9 / 12 场景通过 | 9 / 12 场景通过 |
| PCM / ALAC 单路 | 各两次暂停、重复 SETUP 通过，timed late 为 0 | 各两次暂停、重复 SETUP 通过，timed late 为 0 |
| synchronized 模式 | 通过 | 通过 |
| Mixer / 管理 | 增益、Mute、单/多 Solo、重连偏好、撤销和重新配对通过 | 同类检查通过 |

忽略项未计通过；格式范围不扩大到 excluded vendor。独立迁移工程和驱动安装未纳入本轮 workspace 结果。Ubuntu 的实时 ALAC 固定编码输入夹具可复用，但 worker 解码、暂停和恢复实际重跑；来源和哈希随平台证据留存。

全部命令及退出码：[Ubuntu](evidence/platform-serial-retest-20261003-1430/ubuntu/workflow-checks.json)、[macOS](evidence/platform-serial-retest-20261003-1430/macos/all-checks.json)。

## 五种四路组合

下表的稳态通过要求：聚合 `timed_late_frames`、各 lane 欠载、AirPlay ingress late 均无增量。定向恢复覆盖断开及杀死 worker，再重新接入；存活来源的 session 保持，受控来源的 lane 清除。

| AirPlay + 原生 | Ubuntu：稳态 / 恢复次数 / 恢复窗口聚合跳帧 | macOS：稳态 / 恢复次数 / 恢复窗口聚合跳帧 |
|---|---|---|
| 4 + 0，四并发读者 × 5 ms | 30 s 通过 / 16 / **123** | 30 s 通过 / 24 / 0 |
| 3 + 1 | 30 s 通过 / 6 / **226** | 20 s 通过 / 6 / 0 |
| 2 + 2 | 30 s 通过 / 4 / **51** | 15 s 通过 / 8 / 0 |
| 1 + 3 | 30 s 通过 / 2 / **788** | 15 s 通过 / 2 / 0 |
| 0 + 4 | 60 s 通过 / 0 / 0 | 60 s 通过 / 0 / 0 |

Ubuntu 完成 28 次恢复、84 个恢复相关窗口，加 5 个稳态窗口。存活 lane 欠载及 ingress late 增量均零，但表中的恢复聚合跳帧合计 **1,188 帧**，不能由存活通道计数零推导为全局无跳帧，也不能唯一归因于被恢复来源。四路读者成功 29,997 次、零查询错误。

macOS 完成 **40 次恢复、125 个观察窗口**。主线程从每个原始稳态、baseline、fault、recovery 对象独立核对：聚合跳帧、存活 lane 欠载、ingress late 增量均为零；四路成功查询 **32,495 次、零错误**。四路整体 CPU 均值 71.485%、峰值 91.152%，仅为 1 Hz 系统采样；其他四组合 CPU 均值/峰值为 55.686/68.360%、46.653/55.621%、38.632/51.003%、31.644/52.759%。没有为了通过而降低四路读者强度或增加重试。

两端新轮矩阵均首轮通过，没有触发针对失败的复验。[独立原始计数审计](evidence/platform-serial-retest-20261003-1430/matrix-independent-audit.json)、[macOS 分阶段质量汇总](evidence/platform-serial-retest-20261003-1430/macos/summary.json)可核对。

**此前同源失败保留：** 12:41 轮次 Ubuntu 四路首轮跳帧增加 3,441，唯一复验四 lane 欠载增加 133/234/176/42；Mac 旧轮四路跳帧增加 1,827、一个 lane 欠载增加 3,359、ingress late 增加 4。[Ubuntu 前轮报告](evidence/platform-validation-20261003-1241/ubuntu/README.md)、[Mac 前轮原始失败](../artifacts/platform-validation-20261003-1241/macos/four-airplay.json)均保留。本轮没有代码修正，不能用新的通过记录抹除或解释这些失败，也不能声称已经证明其原因是 CPU 竞争。

## Mac → Ubuntu 原生跨机 60 秒

客户端为同快照 macOS release CLI，SHA256 `21114bb1229f895eeec41ccd7db8ff8d51198cc0a669855b9490f75030175b51`。合成 437 Hz / −36 dBFS 音源经真实 TLS / DTLS-SRTP / Opus / PipeWire 路径发送；这是原生 NeonMix 发送，不是 Apple AirPlay 来源。

配对与发送退出码均为 0，28 个活动采样无查询失败，有非零输出。最后活动样本 59.858 秒：

| 指标 | 结果 |
|---|---:|
| 会话 / 输出 | `network_degraded` / available |
| `lost_packets` / overflow PLC | 400 / 400 |
| 两者差值 | 0 |
| Mixer underrun | **12,960 帧** |
| PCM sink drop / timing gap / queue drop | 0 / 0 / 0 |
| ingress late / timed late / 输出错误 / 回调超预算 | 0 / 0 / 0 / 0 |

**跨机质量未通过。** `lost_packets` 包含 overflow PLC，不能直接当作物理网络丢包。首末活动样本 received 与最高序号都增加 5,783，支持这段认证序号未观察到缺口；该检查不是抓包，尚不足以确定本地 overflow 的唯一原因。

Ubuntu 独立采样最后点较早，计数为 397，与 Mac 较晚点 400 分别按原始时序保存，没有拼成同一份样本。guest 活动 CPU 均值 4.233%、峰值 5.263%。[完整采样与摘要](evidence/platform-serial-retest-20261003-1430/cross-platform/ubuntu-native-lan-summary.json)、[原始查询](evidence/platform-serial-retest-20261003-1430/cross-platform/ubuntu-native-lan-samples.jsonl)、[计数核对](evidence/platform-serial-retest-20261003-1430/cross-platform/ubuntu-native-counter-forensics.json)。

## 正式桌面、留存和限制

Ubuntu release Xwayland 正式 GUI 通过 Ctrl+1…5 导航、Ctrl+K、关闭请求后 Hub 保持、结束 UI 后后台保持、同一后台重开以及确认退出。全新私有资料的窄窗实测 **600×440**，主线程查看截图确认 Ctrl 提示与底部操作完整；宽窗实际 1100×699，受 GNOME 可用高度限制，未称为请求的 760 高度。Alt+F4 后仍 IsViewable，不据此宣称托盘隐藏形态正确；Wayland 原生鼠标、IME 和完整托盘流程未覆盖。

macOS 正式非 preview GUI 11 个流程通过，包括房间创建与认证、五页导航、中文草稿、Mixer 静音、脱敏导出、Cmd+W 隐藏/托盘恢复、UI 崩溃后音频保持与重开、取消/确认退出。真实内容区域 **1100×760 / 600×440**，原生外框分别 1100×792 / 600×472，标题栏为 32；⌘K / Escape / ⌘2 与面板底部边界通过。最初把外框/内容尺寸混用的夹具记录保留，校正后重新执行，不把初轮不准确尺寸计作通过。

截图：[Ubuntu 窄窗](evidence/platform-serial-retest-20261003-1430/ubuntu/gui-narrow-palette.png)、[Mac 窄窗](evidence/platform-serial-retest-20261003-1430/macos/formal-palette-600.png)。

完整逐平台报告：[Ubuntu](evidence/platform-serial-retest-20261003-1430/ubuntu/README.md)、[macOS](evidence/platform-serial-retest-20261003-1430/macos/README.md)。Ubuntu 当前制品留在 `/home/parallels/NeonMix/.local/tl3/artifacts/s1430/bin/`；Mac 留在 `.local/tm3/target/release` 及本轮隔离实验目录，源码/二进制 SHA256 均随平台证据留存。它们依赖项目内运行库，不宣称是新便携包；此前手测包未覆盖。

本轮自有进程、邀请与秘密夹具已清理，Ubuntu 三个 unit 为 inactive / MainPID 0；Mac 自有进程和临时目录为空，原四个用户进程保持存活。[Ubuntu 清理](evidence/platform-serial-retest-20261003-1430/ubuntu/cleanup.json)、[Mac 清理](evidence/platform-serial-retest-20261003-1430/macos/cleanup.json)。未改系统默认音频路由、驱动、持久服务或用户权限策略。真实 Apple 多设备互操作、模拟端听感/音画延迟、8/24 小时长测不在本轮通过结论内。

## Windows：旧轮次附录

Windows 没有在此次仅 Ubuntu/macOS 的重跑中再次测试。此前 12:41 同源码轮次：217 项常规测试通过、5 项忽略，构建及 NamedPipe 原生检查通过；Mac→Windows 60 秒的 29 个活动采样指标为零。但四 AirPlay 定向复验在第三来源接入期间，第一个 worker 报 `pcm_output_queue`，未到第四来源或稳态；Session 0 可见 GUI 仍未验收。错误端口和清理脚本异常与产品失败分别留存，旧轮自有进程已清理。[Windows 旧轮完整报告](evidence/platform-validation-20261003-1241/windows/README.md)。
