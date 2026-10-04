# 三端同版本实验报告（2026-10-03）

本轮三端使用同一份更新后的源码重新构建与测试。常规测试和 release 构建均通过，但不能判定完整跨平台 AirPlay 验收通过：macOS 四路恢复/重连不可靠，Ubuntu 接收 worker 崩溃，Windows 有两个独立启动阻断。原生 Mac→Windows 60 秒播放通过；Mac→Ubuntu 接通但仍有丢包统计、PCM 丢弃和 Mixer 欠载。

## 实验版本与环境

冻结时间：2026-10-03 00:35:55（北京时间）。512 个文件包含工作区未提交的源码、配置和文档，排除真实凭证、连接资料、旧证据、缓存和构建输出。

```text
canonical source manifest SHA256
b0aa5d6bcf84e1d12dae4ff400cd885cb288f124107332907a9a1cb6cda7e9ef

source.tar.gz SHA256
c9d7f4682764dbad9cc0d22b9ded71aa8e1ea6e8393f6614e608cf636fb08704
```

三端最终逐文件核对均无差异。本轮没有修改生产源码，探针的平台路径、v2 控制和 AX 选择器适配只存在于测试 runner。构建缓存可以复用，应用和 worker 都针对该快照重新构建，未以旧 EXE 或旧运行包代替本轮结果。

| 平台 | 环境 | 隔离工作区 / 留存制品 |
|---|---|---|
| 本地 macOS | macOS 27.0 ARM64、Rust 1.95.0、CoreAudio / BlackHole | `.local/t03m`；不可变制品在 `artifacts/platform-test-20261003/macos/bin` |
| Ubuntu | `192.168.100.112`、Ubuntu 24.04.3 ARM64 / Parallels、PipeWire、GStreamer 1.24.2；应用由已登录用户 `parallels` 运行 | `/home/parallels/NeonMix/.local/t03u`；不可变制品在 `/home/parallels/NeonMix/.local/t03u/artifacts/t03u/bin` |
| Windows | `192.168.100.186`、Windows x64、administrator、MSVC / MinGW、GStreamer 1.26.10 | `E:\Desktop\NeonMix-test\r03`；新 EXE / PDB 在 `target\release` |

源码和制品校验：[共同快照](evidence/platform-test-20261003/snapshot.json)、[文件清单](evidence/platform-test-20261003/source-manifest.json)、[macOS](../artifacts/platform-test-20261003/macos/provenance.json)、[Ubuntu](evidence/platform-test-20261003/ubuntu/source-verification.json)、[Windows](evidence/platform-test-20261003/windows/final-provenance.json)。Mac 跨机客户端是该快照 release 的不可变副本，SHA256 `1f4e87ad5bb187b1b7e2ee2932605986d0e7da31f4e2e912b7cb74c6b33ee7c6`。

## 结果矩阵

| 检查 | macOS | Ubuntu | Windows |
|---|---|---|---|
| 常规自动测试 | **225 passed / 0 failed / 5 ignored** | **215 passed / 0 failed / 5 ignored** | **210 passed / 0 failed / 5 ignored** |
| release 原生构建 | 通过 | 通过 | 完整 workspace + MinGW worker 通过 |
| 严格 Clippy | 通过 | 未通过：Linux unsafe 安全注释缺失 | 未通过：两处 Windows unsafe 安全注释缺失 |
| workspace 成员格式 | 通过 | 通过 | 通过 |
| Worker 解码 | PCM / ALAC / AAC 通过 | PCM / ALAC / AAC 通过 | PCM / ALAC / AAC 通过 |
| 独立 worker 协议/配对 | 23 项协议、5 项配对通过 | 23 项协议、5 项配对通过 | 协议/配对在启动阶段阻断 |
| AirPlay 单路集成 | PCM、ALAC 暂停与重复 SETUP、1+1 混音通过 | worker ready 前崩溃 | worker ready 前失败 |
| 多路集成 | **2 AirPlay + 2 原生通过**；**4 AirPlay 恢复未通过** | 1/2/4 入口配置成功，但 worker 均未接收就绪 | v2 两入口配置成功，但两个 worker 均未就绪 |
| Mixer 实际调音 | 7 个电平场景通过；持久偏好重连 SETUP403 | 未建立 AirPlay 媒体，不能验证其实际调音 | 未建立 AirPlay 媒体，不能验证其实际调音 |
| 文件凭证 | 9 类通过 | 相关 workspace 测试通过 | 9 类通过 |
| 独立后台 | 两次复验各 12 场景通过，首败仍未定位 | 12 场景通过 | NamedPipe status/shutdown 与相关单测通过 |
| 桌面 | 正式启动、导航、中文草稿、隐藏保活、托盘恢复、确认退出通过 | 正式进程、后台/Hub 保活与显式关闭通过；完整交互未验收 | SSH Session 0，五页预览和正式实例无可见窗口验收 |

忽略项不计作常规通过。macOS 另外显式运行了 3 项 release 漂移模拟并通过，不能解释为实体长测。平台条件编译导致用例数量不同，不把三端相同用例重复相加作为独立覆盖率。普通 workspace 检查不覆盖被排除的独立 credential-migrate 项目全部迁移矩阵。

三端 `cargo fmt --all` 初次因嵌套工作区的 excluded vendor 向上匹配原工程而失败；显式 workspace 成员格式复验通过。macOS 对 vendor 单独执行 rustfmt：mdns/tympan 通过，CPAL 有真实格式差异。这些 CPAL 文件与上一轮相同，属既有问题，不能报告为所有源码格式通过，也不能归为本次更新引入。

## 同版本跨机音频

Mac 使用新快照客户端，分别向两端新 Hub 发送 437 Hz、−36 dBFS 合成音源 60 秒。配对和控制使用正常 TLS 身份验证；本轮没有采集麦克风或调整系统默认输出。跨机区间暂停本轮编译、本地多路压力和远端 AirPlay 调试。每两秒采集公开快照和诊断。

| 指标 | Mac → Ubuntu | Mac → Windows |
|---|---:|---:|
| 配对 / 发送退出码 | 0 / 0 | 0 / 0 |
| 成功查询样本 | 28（27 个活动接收样本） | 29 个活动样本 |
| 会话状态 | `network_degraded` | `playing` |
| 输出设备可用 / 非零电平 | 是 / 是 | 是 / 是 |
| 输出错误 / 回调超预算 | 0 / 0 | 0 / 0 |
| 活动样本最大 `lost_packets` | **445** | **0** |
| 活动样本最大 PCM sink dropped | **6** | **0** |
| 采样中的累计 Mixer underrun | **53,760 帧** | **0** |
| 本轮短程质量判定 | **未通过** | **通过** |

Ubuntu 最后一次查询位于 60.283 秒，接收器已移除，丢包数组为空；这不代表丢包为零。活动阶段最后一次查询在 58.149 秒，接收丢包统计为 445、PCM 丢弃为 6；欠载累计在末次查询达到 53,760 帧。这里记录接收器统计，不据此单独断言物理网络发生同等数量的 UDP 丢失。

Windows 远端也留存完整连续诊断，活动区间与 Mac 采样一致：输出、丢包、PCM 丢弃、欠载和超预算指标均为零。两端均是数字网络→Mixer→原生输出的短测，未测实体扬声器实听或模拟端延迟。

证据：[Mac→Ubuntu 摘要](evidence/platform-test-20261003/cross-platform/mac-to-ubuntu.json)、[逐次采样](evidence/platform-test-20261003/cross-platform/mac-to-ubuntu-samples.jsonl)、[Mac→Windows 摘要](evidence/platform-test-20261003/cross-platform/mac-to-windows.json)、[逐次采样](evidence/platform-test-20261003/cross-platform/mac-to-windows-samples.jsonl)、[Windows 远端连续采样](evidence/platform-test-20261003/windows/lan-diagnostics.jsonl)。

## 失败与定位

### macOS：四路恢复和重连不可靠

两次四 AirPlay 实验的初始稳态都四路非零，约 11 秒稳态的欠载/迟到为零，但故障恢复矩阵都没有完成：

- 首次第二路定向断开后的 PCM 恢复出现 `worker_failed/media_transport`。此前采样的存活三路欠载/迟到增量为零、session 保持；失败后的全局 timed late=620 是聚合计数，不能归因于存活三路。
- 同版本复验完成三次恢复后，第四路初始 SETUP 返回 403。当时容量 active=3、reserved=0、available=1，不能只按房间已经满员解释。
- 单独 Mixer 实验的 −6 dB（RMS≈0.499 倍）、Mute、单 Solo、多 Solo 和清除 Solo 等 7 项观察通过，但 A 断开、allow_source、重连后的 SETUP403，持久偏好重连未通过。

与之对应，2 AirPlay + 2 原生完成 4 次定向断开/SIGKILL 与恢复，所采样稳态/故障/恢复区间的存活路欠载与迟到增量均为零。因此本轮有混合四路的正向证据，但不能声称四个 AirPlay 来源的完整恢复验收通过。3+1、1+3 等其他组合没有在本轮重新跑完，未借用上一轮结果计作本轮通过。

证据：[四路初败](../artifacts/platform-test-20261003/macos/multi-4.json)、[四路复验](../artifacts/platform-test-20261003/macos/multi-4-retry.json)、[2+2](../artifacts/platform-test-20261003/macos/multi-2plus2.json)、[Mixer](../artifacts/platform-test-20261003/macos/mix-control.json)。

### Ubuntu：真实 worker 崩溃

新版 v2 configure/enable_receiver/pair_receiver 进入实际启动路径，1/2/4 入口配置成功、Hub 输出持续，但 worker 在 ready 前退出，收到/释放音频块为零，数字来源尚未开始配对。这不是沿用旧 v1 探针的 503 或权限误判。

本轮 GDB 取得 SIGSEGV 调用链：`ed25519_key_get_raw` → `pairing_get_public_key` → `raop_init2`。上游密钥加载失败返回 NULL 后仍被用于读取公钥。诊断 wrapper 先停止再 exec 原冻结 worker，PID 保持，未破坏 Hub 对 worker PID 的验证；媒体写线程也表明 Unix IPC 已建立。

独立 OpenSSL 生成密钥的协议探针通过，而 Hub 生成的身份路径触发崩溃，提示需要检查身份读取/格式与 NULL 处理。**本轮没有确定 PEM 读取失败的具体原因**，不能仅凭堆栈宣布是哪一种编码或库版本不兼容。

1/2 入口启动失败时 Hub/native 输出仍运行；四入口首轮观察到 native 提前退出（参数 600 秒、夹具仅约 24 秒，非正常时长结束），原始子日志未留存，原因未知。额外 24 秒留日志复验未复现，原生电平保持、12 次 poll 均未退出，最后由清理明确停止；不能据复验消除首观察。由于没有 AirPlay 活跃媒体，以上只验证启动失败的部分隔离行为，不是四路媒体故障隔离验收。详见 [Ubuntu 平台报告](evidence/platform-test-20261003/ubuntu/README.md)、[GDB](evidence/platform-test-20261003/ubuntu/worker-gdb.log)、[入口生命周期](evidence/platform-test-20261003/ubuntu/entry-lifecycle-final.json)、[针对性取证](evidence/platform-test-20261003/ubuntu/four-native-retake.json)。

### Windows：两项独立 worker 启动阻断

v2 配置两入口和启用调用进入新版 worker 路径，但最终两个入口均 enabled=false、ready=false、worker_failed，未建立 AirPlay 媒体：

1. Hub 提供 NamedPipe 地址，当前 worker 只接受 IPv4 loopback。使用本轮 worker、control v2 和私有文件 fixture 明确返回 `media IPC must be IPv4 loopback`，exit1。
2. 独立 TCP 协议 fixture 成功连接媒体 IPC 后，在读取 PEM 身份/ready 之前退出，stderr 为 `OPENSSL_Uplink(...,08): no OPENSSL_Applink`。这一路 MinGW worker 与官方 MSVC GStreamer/OpenSSL 的启动失败独立于 NamedPipe 拒绝。

PCM/ALAC/AAC 解码 probe 不经过完整 PEM 身份读取，其通过不能替代完整 worker 启动。协议/配对没有完成，未计通过。见 [v2 启用结果](evidence/platform-test-20261003/windows/airplay-v2-enable.json)、[IPC 拒绝](evidence/platform-test-20261003/windows/worker-ipc-check.json)、[协议 stderr](evidence/platform-test-20261003/windows/worker-protocol-final.log)。

### 检查与未定位现象

- Linux Clippy：`crates/airplay-ipc/src/local_unix.rs:154` 的 zeroed unsafe 块缺安全说明；Windows Clippy：`local_windows.rs:38,46` 的两个 Drop unsafe 块缺安全说明。它们是 lint 失败，原生编译及普通测试仍通过，不能混称编译阻断。
- macOS 后台首轮在“诊断不含 fixture 私有值/标识”的宽断言失败。首份日志不能指明具体字段；只增强测试定位后同产品两次各 12 场景通过。首败未定位，也没有足够证据认定秘密泄露。
- macOS 旧 AX runner 把导航当 button，当前角色为 checkbox；选择器适配后预览与正式交互通过。退出 runner 起初漏处理新版确认框，按产品流程复验后取消、确认退出通过。这些是测试适配问题，没有改产品去迁就旧脚本。
- Windows 六次新 GUI 启动均在 Session 0，只出现隐藏 WGL 窗口。构建和单测通过不能替代真实登录桌面的可见窗口、托盘与输入验收。

## 完整记录与清理

平台详细记录：[macOS](evidence/platform-test-20261003/macos/README.md)、[Ubuntu](evidence/platform-test-20261003/ubuntu/README.md)、[Windows](evidence/platform-test-20261003/windows/README.md)。本地界面证据为合成公开状态的 [五页面、两尺寸截图与交互索引](../artifacts/platform-test-20261003/macos/ui-retry/result.json)，不是四台真实 Apple 来源同时播放的截图。

本轮没有修改系统默认音频路由、驱动、持久服务/权限策略、防火墙或时钟。所有 Windows 文件操作限定在 `E:\Desktop\NeonMix-test`。测试进程、临时房间、邀请和生成凭证清除；原有用户手动会话和旧运行包保留。本轮新制品与日志保留供复现，不能把旧手测包当作本轮新版本。

未完成真实 Apple 多设备互操作、视频伴音、模拟端/物理音画指标、每组合正式长时矩阵、8/24 小时长测或签名发布验收。本报告的结论只覆盖上述冻结版本、命令与实际采样区间。
