# Ubuntu 冻结快照原生复测（2026-10-03）

**构建、严格 Clippy、219 项常规测试、PKCS#8 v2 身份启动及多路功能/故障隔离通过；原生 LAN 音频质量与 AirPlay 定时播放质量未通过。** 多路 probe 的 `passed=true` 只代表它现有的功能断言，不能扩大为无跳帧或声音质量验收。

## 版本、环境与制品

共同冻结快照 `platform-retest-20261003-022352`，516 文件，canonical manifest SHA256 `3ef7d6f5ea47a98b671768030311a494bd95c317c1b032bbc38bd00c038b9c84`；测试前后逐文件校验均无差异。

机器 `192.168.100.112`，Ubuntu 24.04.3 LTS / aarch64 / Parallels，Linux 6.14，Rust 1.95.0，OpenSSL 3.0.13，GStreamer 1.24.2，PipeWire 1.0.5。独立工作区 `/home/parallels/NeonMix/.local/t04u`。应用通过既有 `parallels` 的 systemd user / GNOME 会话运行，显式选择 `pipewire:alsa_output.pci-0000_00_01.0.analog-stereo`；真实输出周期 1024 帧，采样率 48 kHz。默认输出与系统服务设置未改。

最终 release Hub SHA256 `5211f8b7a6dcba8093029c0460ad440720952afee6e4e7387f968870c67f9124`；worker SHA256 `62f1fc16bbb43dafeb69c854fd7b2c7e1fc6df7660c4c8b9fe8465420242aeb0`。独立制品副本留存在 `/home/parallels/NeonMix/.local/t04u/artifacts/t04u/bin/`，涵盖 desktop/background/hub/audio/worker。这是依赖本工作区 SDK 的可运行制品，未声明为新的便携分发包，旧手测包没有覆盖。其余哈希及 ELF 架构见 `binary-sha256.json`、`binary-architecture.txt`。

## 检查结果

| 检查 | 结果与边界 |
|---|---|
| workspace tests | 219 passed / 0 failed / 5 ignored，串行执行，忽略项不计通过 |
| 格式 / 严格 Clippy / release | 显式 workspace 成员格式、workspace all-targets `-D warnings`、release 通过；excluded vendor 不扩大为格式验收 |
| Worker 协议 / 配对 | 23 个协议检查、5 项配对持久性场景通过 |
| 实际音频解码 | 独立原生 worker PCM/ALAC/AAC decode probe 通过 |
| 身份与启动 | 17 项 loader + 完整 worker：4 种合法封装保持身份及 RFC8032 签名，13 类错误干净 exit1、不 ready、不崩溃、不改写文件；当前 Ubuntu OpenSSL 3.0.13 下的 ring PKCS#8 v2 通过；真实 Hub 生成的身份也已启动/接收 |
| 2 AirPlay | 2 轮 8 次断开/SIGKILL恢复，功能断言通过 |
| 4 AirPlay + 4 状态读者 | 30 秒稳态，2 轮 16 次恢复，26,546 次成功查询 / 0 错误，功能断言通过 |
| 2 AirPlay + 2 原生 | 4 次故障恢复，功能断言通过 |
| 3 AirPlay + 1 原生 | 6 次故障恢复，功能断言通过 |
| 1 AirPlay + 3 原生 | 2 次故障恢复，功能断言通过 |
| 0 AirPlay + 4 原生 | 60 秒数字稳态功能通过；不能从电平/会话断言推导全部原生接收质量计数为零 |
| 调音与管理 | 9 个 Mixer 观察检查通过，含增益/Mute/单多Solo及重连；4 个管理检查通过，含跨入口限制、撤销、新PIN配对及身份保持 |
| 独立后台 | 12/12 场景通过，含私有IPC/文件凭证、测试音、快照、崩溃隔离、邀请取消、脱敏导出、显式shutdown |

多路组合合计 36 次已完成的定向恢复。所采样存活 AirPlay lane 的 ingress late_packets 与 Mixer underrun 增量为零，但 **Mixer aggregate timed_late_frames 在稳态明显增长**，因此不能声明多路播放无跳帧。

## 定时播放质量失败

| 稳态组合 | 时长 | Mixer aggregate timed_late_frames 增量 |
|---|---:|---:|
| 2 AirPlay | 11 秒 | 186,065 |
| 4 AirPlay | 30 秒 | 999,658（23,743 → 1,023,401） |
| 2 AirPlay + 2 原生 | 11 秒 | 218,262 |
| 3 AirPlay + 1 原生 | 11 秒 | 343,779 |
| 1 AirPlay + 3 原生 | 11 秒 | 108,109 |
| 0 AirPlay + 4 原生 | 60 秒 | 0 |

上述为 Mixer 聚合计数，不能精确归因于某条存活来源。完整 per-lane/aggregate before/after 在各组合 JSON 的 `steady_continuity` 和每次 `isolation_checks` 中保留。多路合成来源按 PCM s16 / 44.1 kHz / 352 帧配置，真实 Hub `live` 检查 ingress source_rate=44100、有效块数、身份及电平；多路原报告未另保存每个接收器的 codec/mode 快照，不伪称其为额外观测。

单路 PCM 和 ALAC 均实际配对、签名验证、FairPlay/加密接收成功，`low_latency` 下各两次暂停 + 重复 SETUP 均恢复非零电平，worker ingress late_packets=0、timeline_rejections=0。但完整回归在 `pause and resume with changed protocol lead` 阶段报 `retiming skipped output samples`：PCM 最后 Mixer timed_late_frames=26,236，ALAC=29,285。单路不能计作完整通过。暂停两次的详细计数、解码格式和失败点见 `single-pcm.json`、`single-alac.json`；实际格式分别为 `pcm_s16` / `alac`，source_rate=44100、source_frame_count=352。

ALAC 实时源使用 macOS 只读 FFmpeg 生成的固定 183 字节合成音频包，再由 Ubuntu 实际 worker 解码，未在 Ubuntu 安装 FFmpeg。生成命令、波形及包哈希见 `alac-fixture-provenance.json`。独立 decoder 的 PCM/ALAC/AAC 原生结果不依赖这个替代编码入口。

## Mac → Ubuntu 60 秒协同

主线程使用同快照 Mac release native Sender，认证和发送 exit0，28 个活动采样；Ubuntu 全段没有编译或并行本机探针。主线程活动样本末次 `network_degraded`，lost_packets=398 且全部等于 overflow_plc_packets，队列丢弃=0，PCM sink drop/timing gap 各1，Mixer underrun=16,320；timed_late/output_errors/callback_over_budget 为0。与旧版队列放大现象相比可见进展，但不构成质量通过。不能将 Sender 结束后 receiver 移除、空数组解释成丢包0。

Ubuntu 每2秒连续后台 diagnostics 在原始 artifacts 的 `lan-samples.jsonl` 保留；主线程的原始 Mac 发送、完整Hub诊断和活动采样由跨机证据保存。

## CPU 条件

Ubuntu `/proc/stat` 聚合数据按1Hz持续记录，4逻辑核；每个 runtime 开始前要求连续10个样本 utilization<80%，同时不进行编译。编译后续 jobs=2，构建 runner 仅占自有2/4核；没有改 VM 核数或原有用户进程。runtime 有持续 >80% / 接近100% 的中断保护，本轮 Ubuntu runtime 没有触发。

| 场景 | 平均 | P95 | 最大 | 最小 idle |
|---|---:|---:|---:|---:|
| 原生 LAN 活动窗口 | 8.212% | 12.437% | 12.879% | 87.121% |
| 单路 PCM（失败） | 9.524% | 19.403% | 19.403% | 80.597% |
| 单路 ALAC（失败） | 8.524% | 12.782% | 12.782% | 87.218% |
| 4 AirPlay / 4读者 | 25.066% | 27.543% | 27.750% | 72.250% |

其余每场景窗口、平均/P95/最大和样本数见 `cpu-summary.json`；原始1Hz记录在 `artifacts/platform-retest-20261003-022352/ubuntu/remote/cpu.jsonl`。Ubuntu是当前Mac宿主的VM；Mac CPU需要同时看主线程宿主记录，不能把guest CPU当作宿主余量证明。

## 夹具首败与修正

首轮 fmt runner 错把成员 glob 当字面 manifest 路径，随后按 cargo metadata 枚举正式成员通过。worker 首次准备被 SDK symlink 解析逃出当前嵌套root、分离runtime插件布局及 multiarch OpenSSL include 阻断；复用 SDK 的项目内只读 hardlink、按原 Linux CMake/pkg-config 路线显式 include 后四个新 target 编译通过。旧制品没有用于当前通过结论。

首次真实Hub探针尚未生成 worker：新TMPDIR继承 parallels 的 umask002成为0775，被既有 private media root guard拒绝。仅把本轮独立 `t04u/.local/tmp` 设0700、在本轮环境入口 umask077 后重跑，真实接收与多路功能通过。ALAC首尝试因Ubuntu没有FFmpeg而未进入接收，随后使用上述Mac生成固定fixture。上述都是明确的测试夹具/构建适配阻断，首败JSON/log保留在原始 artifacts；产品源码未修改。

## GUI、留存与验收边界

正式 Wayland desktop 进程启动，实际文件后台与Hub可运行；终止 GUI 后 Hub 保持、显式shutdown成功。GNOME Screenshot DBus拒绝调用，原始拒绝在 `gui-lifecycle.json`，不能伪称已截图或已完成鼠标/键盘交互。随后同一正式 release GUI 在自有 Xwayland 会话补验真实 Ctrl+1..5，Hub/Sender/Mixer/设备/诊断五页导航成功，截图逐张人工检查，后台继续在线，终止GUI后后台保持，shutdown/fixture清理成功。证据 `gui-x11.json` 与 `gui-hub.png`、`gui-sender.png`、`gui-mixer.png`、`gui-devices.png`、`gui-diagnostics.png`，均为真实未连接房间的空态，不是 preview。Xwayland仅通过该进程环境选择，未改系统显示设置；不能扩大为Wayland鼠标、托盘/IME或完整配置流程验收。该GUI窗口CPU avg5.656% / P95与max15.539%，最低idle84.461%。Linux截图中的快捷键提示仍显示macOS的⌘符号，而实际测试使用Ctrl，这是一项已观察到的提示一致性问题。

本轮不涉及真实 Apple 设备互操作、物理音画/P95、8/24小时长测、驱动安装或签名分发。独立测试进程、秘密fixture与邀请的最终清理由 `cleanup.json` 记录；完整原始结果和可复现Linux适配脚本在 `artifacts/platform-retest-20261003-022352/ubuntu/remote/`。

以 `parallels` 登录桌面后，可从当前制品副本启动新的手测资料（不复用本轮已清理的测试凭证）：

```sh
cd /home/parallels/NeonMix/.local/t04u
.local/env.sh systemd-run --user --collect \
  "$PWD/.local/env.sh" "$PWD/artifacts/t04u/bin/neonmix-desktop" \
  --state-dir "$PWD/.local/manual04"
```
