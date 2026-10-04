# Ubuntu ARM64 E02–E04 补验（2026-09-30）

**最新结论：调度与设备周期修复后，21:47 启动的完整五分钟跨机验证及全部控制场景通过。** 欠载、媒体丢包、PLC、PCM 缺口、队列丢弃、回调超预算和采集过期帧均为零。本次 VM 使用系统报告的 1,024 帧周期（21.33 ms），不是强制 256 帧；不据此声明该 VM 的 256 帧或实体模拟端低延迟通过。[成功摘要](evidence/ubuntu-e02-e04/fixed-300s-summary.json)、[完整报告](evidence/ubuntu-e02-e04/fixed-300s-full.json)。以下保留修复前的失败记录；文末说明修复与复现条件。

用户追加授权使用 `192.168.100.112`。节点为 Ubuntu 24.04.3、ARM64、Parallels 虚拟机；本次没有 Windows 或实体声卡验收。原 macOS 一小时验收记录保留，不把旧制品的长期结果转给本次修改后的制品。

## 测试环境与边界

固定 Ubuntu 工作区位于 `/home/parallels/NeonMix/.local/e02-e04-20260930/workspace`。下载的 DEB、Rust 缓存、配置、临时凭证、编译和日志均位于项目内。复用已安装的编译器和 SDK，没有向系统安装开发包。实际音频程序以普通 `parallels` 用户运行。

PipeWire 1.0.5 / WirePlumber 0.4.17 使用独立的项目内 D-Bus、runtime、config 和 state；不修改 GDM 音频会话、默认输出或系统配置。WirePlumber 加载设备与链接策略，禁用实体设备监控；显式实验节点为 `pipewire:neonmix.sink.e02lab`，48 kHz / stereo / 256 帧。该实验节点不代表 E06 正式虚拟输出 owner 已通过完整生命周期验收。

GStreamer 为 Ubuntu ARM64 1.24.2，GLib 2.80.0、Opus 1.4、libsrtp 2.5.0；运行时实际找到全部 11 个必需 element，包括 DTLS/SRTP。仅扫描明确选出的插件。项目内 DEB 的版本和哈希、工作区修改及二进制哈希保存在本轮证据目录。

## 修复

1. **GStreamer 1.24 的崩溃。** `appsrc/appsink.dropped` 在本基线不存在，原实现读取属性会 panic。现在给缓冲加私有序号元数据，在出队处计算真实缺口，保留队列丢弃统计；不改音频 offset、PTS 或格式。原生容量测试验证 appsink 丢 6 块、appsrc 丢 5 块，较新运行时还与其原生属性核对。
2. **多网卡媒体选路。** Mac 同时拥有 `.111` 和 `.208`；HTTPS 到 `.111` 时，通配 UDP bind 曾从 `.208` 回复，Ubuntu 的 connected socket 拒收。现在在 TLS acceptor 保存真实 TCP 本地/对端地址，UDP 绑定同一具体本地地址，并保留 IPv6 scope。客户端不能用请求字段改写这些地址。地址失效时明确失败，不回落到另一网卡。
3. **编码前丢帧的时间线保护。** 原先依赖下游 `DISCONT`，不足以保证 appsrc 丢帧后终止。现在由实际出队序号缺口关闭门，丢弃其后的编码输入，要求显式建立新上下文；真实 appsrc 溢出测试只允许缺口前的第一块通过。

Linux 媒体泵新增线程 CPU 时间，区分等待/调度停顿与执行耗时。没有提高硬实时优先级、增加缓冲或放宽频率/欠载判据。

## 当前结果

- Ubuntu：62 项选定常规测试、3 项 release 漂移/停顿注入、选定包格式检查、Clippy `-D warnings` 和 ARM64 release 构建通过。覆盖 core、I/O、control、media 与 Hub；不代表整个 E05/E06 平台功能已验收。
- macOS：本次 media/Hub 26 项测试、格式、Clippy 和 release 构建通过；[最终制品纯 macOS 30 秒双路及完整控制回归](evidence/ubuntu-e02-e04/macos-final-control-30s.json)通过，欠载/丢包/PLC/队列丢弃均为零。
- 中间修订中，Mac Hub 与 Ubuntu Sender 的真实 TLS 1.3 / DTLS-SRTP、双路输出、Mute/多 Solo、权限/版本/幂等、撤销、WSS 顺序及重启保持通过。中间修订的 30 秒双路数字读回为零欠载、丢包、PLC、PCM 缺口和队列丢弃；最终制品的 LAN 两方向复验出现单路/Solo 频率偏差，仍为失败，不能把中间修订通过当作最终跨机验收。
- Ubuntu Hub 可建立跨机认证与非零数字输出，多个功能步骤已经实际执行；**持续稳定性未通过**。五分钟试验中 Ubuntu Sender 在约 41 秒遇到约 143 ms 的等待，触发源期限保护；采集回调另有一次超预算、stale 和时间线断点。后续短测也出现频率偏差及媒体丢弃，不能仅凭 `errors=0` 宣称通过。
- 不含编解码器的 60 秒普通 Linux epoll 探针：1 ms 请求等待最大额外延迟约 16.08 ms；最慢 17.08 ms 样本仅消耗约 49 µs 线程 CPU。它证明普通等待也存在抖动，**未复现 143 ms 停顿，不能据此断言全部故障都来自虚拟机或已找出根因**。对应试验期间 CPU 压力接近零，不能把停顿解释为持续 CPU 满载。

本次证明了 Ubuntu 原生基线与实际跨机路径，也修复了由其暴露的代码缺陷。Ubuntu 的持续音频与数字读回仍需继续定位，实体声卡、Linux x86_64、Windows、模拟端延迟和完整发布矩阵仍未验收。

## 复现

从 macOS 使用已认证、位于项目内的 SSH control socket：

```sh
tools/dev python3 tools/e02_e04_probe.py \
  --linux-peer root@192.168.100.112 \
  --peer-workspace /home/parallels/NeonMix/.local/e02-e04-20260930/workspace \
  --hub-platform macos --macos-host 192.168.100.111 \
  --device coreaudio:BlackHole2ch_UID --soak-seconds 30
```

Ubuntu Hub 使用 `--hub-platform linux --device pipewire:neonmix.sink.e02lab`；它要求已建立独立 PipeWire 会话与实验节点。每次探针同时启动 Mac 和 Ubuntu Sender，直接使用 LAN 控制/媒体链路；SSH 只负责进程和文件编排，不转发产品媒体。探针退出时删除两端实验凭证并停止所拥有的进程。

无媒体计时诊断在 Ubuntu 项目中执行：

```sh
.local/audio-env.sh python3 tools/linux_timer_probe.py \
  --seconds 60 --output artifacts/epoll-timer-60s.json
```

完整原始记录位于开发宿主 `artifacts/ubuntu-e02-e04-192.168.100.112/20260930-181511/`，LAN 探针原始目录为 `artifacts/e02-e04/20260930-19*/`。选取的长期证据见 [Ubuntu E02–E04 evidence](evidence/ubuntu-e02-e04/)。早期失败与中间制品保留，不计为最终制品通过。

收尾已核对：两端临时凭证删除，本轮 Hub/Sender/capture 全部退出；Ubuntu 实验 Sink、独立 WirePlumber/PipeWire/D-Bus 均已停止，系统音频会话保持原状。见[清理记录](evidence/ubuntu-e02-e04/cleanup.json)；选取文件哈希见 [SHA256](evidence/ubuntu-e02-e04/SHA256.json)。

## 五分钟复测（2026-09-30 20:56）

按用户要求重跑 Ubuntu Hub + Mac/Ubuntu 双 Sender 的完整 300 秒验证。冻结本轮 Mac 二进制，Ubuntu 使用原固定制品；本轮没有修改实现或并行编译。单/双路前置读回通过，300.003 秒采集完成，两路 Sender 持续运行并在测试收尾停止，但**本轮仍为失败，不标记成功**。

最终欠载 158,400 帧；A 路报告媒体丢包 1,496 包、PLC 718,080 样本、PCM 缺口 51 次、PCM appsink 丢弃 30 块、交接队列丢弃 468,960 帧；B 路无媒体丢包/PLC，但交接队列丢弃 3,360 帧。采集错误与回调超预算为 0，另有 stale 21,248 帧。这些计数证明音频稳定性问题复现，不能用 Sender 未退出或 `errors=0` 判断成功。

[复测摘要](evidence/ubuntu-e02-e04/retest-300s-20260930.json)、[完整诊断](evidence/ubuntu-e02-e04/retest-300s-full-20260930.json)、[制品元数据](evidence/ubuntu-e02-e04/retest-300s-metadata.json)、[清理](evidence/ubuntu-e02-e04/retest-300s-cleanup.json)。原始日志和冻结制品位于 `artifacts/ubuntu-retest-300s/20260930-205626/`；本轮拥有的所有音频/测试进程和两端临时凭证已清理，独立会话已停止。

## 根因与修复后的验收

本轮定位到两组具体问题：

1. Linux 的媒体优先级实现原先没有执行任何配置，却返回 `configured=true`；CPAL 的 realtime 功能也未启用，普通 `client.conf` 不加载 PipeWire 的实时模块。SSH `runuser` 又不能获得本机桌面会话的 RTKit 授权，实际 PipeWire server、客户端数据线程、回调和媒体工作线程都曾处于普通调度。现在媒体线程请求 nice −10，并读回实际 policy/nice、报告授权失败，退出时恢复；CPAL 启用 realtime-dbus，优先使用已有 RLIMIT_RTPRIO 授权，再回退到 RTKit；PipeWire 上下文使用发行版 `client-rt.conf`，保留显式环境覆盖。
2. 旧隔离测试脚本将系统 `vm.overrides.default.clock.min-quantum` 从 1,024 改成了 256；Hub 固定请求 256，Sender 采集固定请求 480。启用调度后的两轮 256 帧试验已经没有媒体丢包/PLC/欠载，但仍有输出超预算与交接丢弃。现在恢复系统的 VM 周期策略，Linux Hub、重开和 Sender 采集采用系统默认周期；探针也采用实际默认周期。设备明确报告 1,024–2,048 帧，最终回调为 1,024。RTP 仍是 480 帧/10 ms，jitterbuffer 仍是 40 ms，Mixer 目标水位仍为 70 ms。

无音频及带音频负载的 UDP 对照各收齐 6,000 包。最大单向延迟变化分别约 21.58/14.44 ms；原报告的 `lost_packets` 多数来自接收 jitterbuffer 容量淘汰及相应 PLC，不能直接当成物理网络丢包。该对照与调度读回、逐秒诊断共同用于定位，没有通过放宽音频断言取得成功。

没有桌面登录会话的实验以 root SSH 编排，但音频进程降为 UID 1000、有效 capabilities 为零，仅给这些子进程 `rtprio=20`、`nice=31`、`rttime=200000µs` 资源上限。使用 `setpriv` 保留这些上限，因为当前 `runuser` 会重置 nice/rtprio 限制。PipeWire 数据线程实际优先级 20，CPAL 回调 RR 10，媒体工作线程维持 SCHED_OTHER/nice −10。没有修改系统 polkit、用户组、全局调度或桌面音频配置；常规桌面部署可使用 RTKit 授权。

最终实际捕获 14,399,488 帧。采集首个周期 1,024 帧为启动静音，后续不增长；clock epoch 保持 1，只有初始 START 断点，stale 为 0。输出原始位置与提交位置保持一个回调周期的固定差值，没有继续漏周期。完整脚本还通过单/双路、Mute、多 Solo、总控、权限/版本/幂等、撤销、禁止/重新允许、WSS 顺序和重启保持。

本次针对性检查为 Ubuntu 24 项与 macOS 28 项测试，以及各自的 Clippy、格式与 release 构建。最终 Ubuntu 源文件/制品哈希见 [manifest](evidence/ubuntu-e02-e04/linux-stability-final-manifest.json)，测试日志见 [Linux](evidence/ubuntu-e02-e04/final-scheduling-tests.log) 和 [macOS](evidence/ubuntu-e02-e04/final-macos-tests.log)。CPAL 额外补丁与来源已登记在 `vendor/cpal-provenance.json` 和 `patches/cpal-0.18.2-linux-scheduling.patch`。

修复后测试入口（先启动归档配置中的私有 PipeWire 会话与 Sink）：

```sh
tools/dev python3 tools/e02_e04_probe.py   --linux-peer root@192.168.100.112   --peer-workspace /home/parallels/NeonMix/.local/e02-e04-20260930/workspace   --hub-platform linux --peer-audio-rlimits   --device pipewire:neonmix.sink.e02lab --soak-seconds 300
```

私有会话和 Sink 使用相同的 `prlimit --rtprio=20:20 --nice=31:31 --rttime=200000:200000 -- setpriv --reuid=parallels --regid=parallels --init-groups --` 前缀启动。完整配置、原始诊断、对照与重现 helper 保存在 `artifacts/ubuntu-diagnosis/`；成功原始日志在 `artifacts/e02-e04/20260930-214742/`。所有本轮进程、私有会话和临时凭证已清理，[清理记录](evidence/ubuntu-e02-e04/linux-stability-cleanup.json)。本次标记五分钟开发验证成功，保留旧失败，不扩展为 Windows、实体声卡或新制品的一小时验证。
