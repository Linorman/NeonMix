# macOS 同快照复测证据（2026-10-03）

本轮 workspace 225 项通过、5 项忽略；严格 Clippy 与 release 构建通过。额外 release 漂移模拟 3 项通过，不计作实体长测。完整索引见 [summary.json](../../../../artifacts/platform-test-20261003/macos/summary.json)；[provenance.json](../../../../artifacts/platform-test-20261003/macos/provenance.json) 记录 macOS 27.0 arm64、Rust 1.95.0、512 文件逐项校验无差异与二进制哈希。

## 结果

| 项目 | 结果与证据 |
|---|---|
| Workspace | [checks/result.json](../../../../artifacts/platform-test-20261003/macos/checks/result.json)：tests、clippy、release、simulate、媒体双路/丢包重放/坏指纹拒绝/漂移均成功 |
| 格式 | workspace 成员 `cargo fmt -- --check` 成功；`--all` 因嵌套隔离目录中被排除的 vendor 向上匹配原 workspace 而失败。直接 rustfmt 验证 mdns/tympan 成功、CPAL 有真实格式差异，保留 [CPAL diff](../../../../artifacts/platform-test-20261003/macos/fmt-cpal-direct.log)。CPAL 相应源文件与上一轮相同，非此次更新引入 |
| Worker | [协议](../../../../artifacts/platform-test-20261003/macos/worker-protocol-retry.log) 23 项、[解码](../../../../artifacts/platform-test-20261003/macos/worker-codecs.log) PCM/ALAC/AAC-LC、[配对](../../../../artifacts/platform-test-20261003/macos/worker-pairing.json) 5 项通过 |
| 单路 | [PCM 1+1](../../../../artifacts/platform-test-20261003/macos/single-pcm.json)、[ALAC 1+1 和两次暂停/repeated SETUP](../../../../artifacts/platform-test-20261003/macos/single-alac.json) 通过；真实 worker、Hub、Mixer 和 BlackHole 输出；timed late=0；worker 崩溃保留原生来源 |
| 2+2 | [两 AirPlay + 两原生](../../../../artifacts/platform-test-20261003/macos/multi-2plus2.json) 通过。4 次定向断开/SIGKILL 与恢复完成；每个所采样稳态/故障/恢复区间，存活路欠载/late 增量=0，全局 timed late 增量=0。Session 上下文保持，最终全局关闭后 worker/预留均=0 |
| 4 AirPlay | [初败](../../../../artifacts/platform-test-20261003/macos/multi-4.json)、[同版复验失败](../../../../artifacts/platform-test-20261003/macos/multi-4-retry.json)。两次稳态约 11 秒都四路非零，稳态欠载/late=0。初败在第二路断开恢复 PCM，worker_failed/media_transport；复验在第四路恢复初始 SETUP，403。未完成所有故障矩阵，不能计作通过 |
| Mixer | [调音](../../../../artifacts/platform-test-20261003/macos/mix-control.json) 7 项电平观察通过，−6dB RMS≈0.499 倍、Mute、单/多 Solo 与清除正确；A 断开后 allow_source + 重连 SETUP403，持久偏好重连未通过 |
| 文件凭证 | [探针](../../../../artifacts/platform-test-20261003/macos/credential-store.log) 9 场景通过，fixture 清理；不读取真实用户凭证、不访问原平台秘密库 |
| 后台 | [初败](../../../../artifacts/platform-test-20261003/macos/background.log) 在诊断不应包含 fixture 私有值/标识的宽断言失败；记录没指明具体字段，不能断言秘密泄露。仅给测试增加定位且不改产品后，[复验](../../../../artifacts/platform-test-20261003/macos/background.json) 和 [再次复验](../../../../artifacts/platform-test-20261003/macos/background-second-retry.json) 各 12 场景通过；初败原因未定位 |
| 桌面 | [两尺寸五页面](../../../../artifacts/platform-test-20261003/macos/ui-retry/result.json) 共 10 张基础页面新原生截图，另有键盘导航和搜索无结果两张，中文输入/草稿保留、Space 导航、搜索/清除通过。原探针 button 选择器已过时，当前 AX navigation 为 checkbox；只适配测试 runner。600 窄窗内容通过滚动访问；截图是合成公开状态，非实际同时播放的 UI 证据 |
| 正式 GUI | [正式实例](../../../../artifacts/platform-test-20261003/macos/formal-ui-retry.json) 启动、后台、AX 导航、中文草稿、Cmd2、隐藏窗口后台保活与托盘恢复通过；该轮退出超时因未处理新版确认弹窗。[确认流程](../../../../artifacts/platform-test-20261003/macos/formal-ui-confirm.json) 正确操作后 CmdQ、Escape 取消、确认退出 UI/后台均通过 |
| BlackHole | [数字播放/采集](../../../../artifacts/platform-test-20261003/macos/blackhole/result.json) 437Hz、−36dBFS、回调预算与原生时钟通过；不改默认设备、不记录 PCM |

初次 4 路失败时，前两个定向故障所采样的存活三路欠载与 late 增量都为零，session 保持；第一路恢复区间也为零。第二路恢复失败后的最终全局 timed late=620 是 Mixer 聚合计数，不能归因于存活三路，亦不能据故障前的零增量声称完整恢复区间连续性通过。复验失败前完成三次恢复，第四次 admission403 时容量 active=3/reserved=0/available=1。

## 测试适配与限制

源码由主线程冻结的 `source.tar.gz` 解到 `.local/t03m`，无生产源修改。APFS clone 复用编译缓存但独立 target 重编译，独立 CMake worker 重建；原 CMakeCache 指向主工程的初次准备失败以及缺 EXCLUDE_FROM_ALL 探针 target 的初败日志保留。配对探针首次报告落点不存在，创建证据目录后重跑成功。所有产物、缓存、临时资料都留项目内。

主线程同时使用本轮 Hub 制品进行 Mac→Ubuntu/Windows 协同；跨机期间暂停本地多路/高负载测试。当前真实用户手动 Hub/worker 与默认 background 保持原 PID，不重启、不改默认输出。后台生命周期测试使用仅测试用 wrapper 给 serve 增加空闲 loopback listen，避开用户 7443 端口；不改变服务二进制。AirPlay fixture 音量从原脚本 0.08/0.12 改为 −36dBFS，仅在 runner 内替换合成波形，不改产品与断言。

数字来源经过真实协议/解码/输出链路，但 FairPlay envelope 合成，不代表本轮真实 iPhone/iPad/Mac 互操作。未测物理端到端、音画同步、每组合正式长测、8/24 小时或发布分发。本轮保留上述失败与复验，不用旧通过结果替换新失败。
