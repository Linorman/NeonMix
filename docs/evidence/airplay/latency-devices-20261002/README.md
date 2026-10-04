# AirPlay 延迟与设备显示：macOS 验证

默认低延迟音乐模式将来源的协议预缓冲转换为短待播余量；按每个 epoch 固定平移 PTS，保留连续采样和现有 sinc SRC。可选音画同步保留来源时间。房间设备页合并当前/最近 AirPlay 来源，活动来源在首屏，按名称、摘要 ID、AirPlay 类型搜索和计数，管理员控制保持原权限及 revision。

## 数字链路结果

来源为合成签名、PIN、NTP 和 CBC 加密 ALAC，解码后经过真实 worker、Hub、定时 Mixer 和 CoreAudio BlackHole 输出。原生 Sender 在播放后加入，检验 1+1 混音；最后 SIGKILL worker 检验原生音频继续。每份 JSON 自带二进制 SHA256 与 fixture 清理结果。

| 运行 | 最近包协议等待 | 固定播放提前量 | 接收后待播估计 | 迟到包 / 定时跳帧 |
|---|---:|---:|---:|---:|
| [音画同步](synchronized-alac.json) | 1,938.1ms | 0ms | 1,938.1ms | 0 / 0 |
| [低延迟，30 秒稳定段](low-latency-alac.json) | 1,937.6ms | 1,819.9ms | 117.7ms | 0 / 0 |
| [低延迟，96 包启动预缓冲](low-latency-prefetch-alac.json) | 1,940.9ms | 1,804.4ms | 136.5ms | 0 / 0 |

三组接入拒包均为零，1+1 混音和 worker 故障隔离通过。选择音画同步后立即关闭再开启的持久化回归通过；该测试曾暴露停止标志抢先跳过保存，现先处理已确认的控制动作。30 秒报告的 Hub 制品早于这一停止顺序修复；其后的同步和预缓冲报告使用最终 Hub 制品，音频调度相同。

以上数值是最近接收包的协议目标与接收时间差，不是手机/App/网络到模拟扬声器的端到端测量，也没有与原生 AirPlay 做物理对照。低延迟会使声音早于来源的视频。没有执行 Windows/Linux 测试。

## 代码与界面检查

相关 49 项常规测试、核心 39 项测试通过；核心 4 项 release-only 测试未运行。相关 crate 的 Clippy `-D warnings` 含 screenshot feature 通过，release Hub/desktop/background 与 worker 构建通过。worker 的协议准入/拒绝、已有配对重连、加密 PCM 回归，以及 PCM/ALAC/AAC 解码接缝通过。

新增 egui AccessKit 回归检查来源显示、大小写名称/类型/ID 搜索、无匹配、管理员控制与成员只读。预览 fixture 是从已有公开状态派生的测试数据，不是本轮 Apple 来源证据；不含配对码。`devices-active-600.png`、`devices-active-1100.png` 验证活动来源在首屏；Hub 图验证模式选择及活动时不可切换；断开/撤销 fixture 验证相应状态。所有界面复用现有 token、控件与确认流程。

macOS Hub 当前采用项目内私有 Unix PCM socket，worker 已补齐连接并核对当前 UID/Hub PID。独立数字协议探针继续使用 worker 的 loopback TCP 入口。原生源凭证和 PIN 没有放宽。

汇总、当前源码和制品哈希见 [summary.json](summary.json)。每次 Mixer 探针均停止自己的进程，删除 fixture 文件与测试 Keychain 条目；依赖、缓存、构建和证据均留在项目目录。

## 复现及更新现有测试实例

```sh
tools/dev cargo test -p neonmix-airplay-adapter -p neonmix-hub -p neonmix-desktop -p neonmix-desktop-service
tools/dev cargo clippy -p neonmix-airplay-adapter -p neonmix-hub -p neonmix-desktop -p neonmix-desktop-service --all-targets --features neonmix-desktop/screenshot -- -D warnings
tools/dev python3 tools/airplay_mixer_probe.py --profile release --codec alac --native-start-order hot --steady-seconds 30 --report .local/tmp/airplay-low-latency.json
tools/dev python3 tools/airplay_mixer_probe.py --profile release --codec alac --playback-mode synchronized --native-start-order hot --report .local/tmp/airplay-synchronized.json
tools/dev python3 tools/airplay_mixer_probe.py --profile release --codec alac --prefetch-blocks 96 --native-start-order hot --report .local/tmp/airplay-prefetch.json
```

首次验证时保留了另一个配对测试 chat 使用的运行副本。随后用户明确要求替换：已更新当前实例的 Hub/worker/UI/后台，核对副本与 release 一致、原房间身份与 1 台来源配对保留、低延迟接收就绪、GUI/Hub 运行及输出帧持续推进；见 [replacement.json](replacement.json)。来源需要重新选择原房间。后续运行中的实例可通过下列入口更新 Hub/worker/UI。

```sh
tools/dev python3 tools/airplay_manual_session.py restart --refresh-hub --refresh-worker --refresh-desktop
```
