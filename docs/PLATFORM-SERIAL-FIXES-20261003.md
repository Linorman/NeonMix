# 串行复测问题修复与 macOS 验证（2026-10-03）

针对 [14:30 最新测试](PLATFORM-SERIAL-RETEST-20261003-1430.md) 修复原生接收队列容量和期限混用，以及恢复阶段验收漏断言。AirPlay 历史间歇断流的上游原因仍未确定；新增逐路跳帧、Hub 调度及数字发送夹具节奏诊断。此次只在 macOS 验证，不把本机通过写成 Ubuntu / Windows 已通过。

## 已定位与修改

**原生 overflow PLC：** `rtpjitterbuffer` 的 `drop-on-latency=true` 按已有队列 RTP 时间跨度达到 40 ms 驱逐最旧包，不判断实际 deadline，也可能不产生原生 loss event。输出线程短暂延迟就能使完整认证序列出现本地缺口，继而被现有 fallback 计为 overflow PLC。macOS 原生 DTLS/SRTP/Opus 受控实验保持正常 10 ms 发包、40 ms jitter，首次 jitter 输出交接暂停 35 ms：旧版认证收到 30 包、PCM 14,400 帧、late=0，但 lost=overflow=2；修复后相同实验 lost=overflow=late=0，PCM 连续且无丢弃。[前后证据](evidence/platform-serial-fixes-20261003/native-jitter-findings.json)。

`crates/media/src/transport.rs` 现保持 40 ms 缺包期限并关闭按 RTP 跨度驱逐；独立跟踪最多 32 个已认证、尚未由 jitter 输出或 loss 事件退役的包。包含扩展序号、乱序、回绕、迟到退役处理。容量耗尽明确撤销当前媒体上下文；认证前 wire 队列和解码 PCM 队列仍分别有界。新增当前 / 峰值在途包数与耗尽诊断，保留 overflow fallback 观察异常缺口。受控 650 ms 输出停顿验证真实容量耗尽、32 包上限与撤销；已有真实丢包、迟到、SRTP 重放和到期回归保留通过。

这是原生 overflow 的可复现产品缺陷。Ubuntu 历史 400 次 overflow 与完整序号符合该机制，但旧数据不能还原确切调度触发；本轮没有在那里复测，不能宣称已确定唯一诱因或通过跨机验收。

**恢复验收缺口：** `tools/airplay_multi_source_probe.py` 原先只对 steady 做质量硬断言，baseline / fault / recovery 仅记录数据，仍可能 `passed=true`。现每个恢复窗口要求聚合 timed late、存活 lane 欠载及 ingress late 零增量，也拒绝 ingress 计数重置。用原始历史数据重放，准确拒绝 Ubuntu 四个恢复窗口、共 1,188 帧，接受 Mac 原 125 个零增量窗口；旧证据未修改。[验收重放](evidence/platform-serial-fixes-20261003/recovery-oracle-replay.json)。

**AirPlay 尚未定位的部分：** Ubuntu 1,188 帧全部在 recovery 增加，1+3 组合只有恢复 AirPlay 使用 timed lane，故其中 788 帧可以归因于该来源；其他组合旧聚合计数无法唯一归因。旧 Mac 四路 steady 的一个 lane 欠载 3,359、ingress late 4，随后 Mixer 跳过 1,827 帧，符合供给断流后重新获取过期 PCM，但不能判断断流发生在夹具、worker、Hub 调度还是设备时钟。

`crates/audio-core` 增加物理 lane 生命周期累计 `timed_late_frames_by_lane`，以及最近跳帧对应 stream / epoch / 输出帧 / PCM 目标时间 / 输出呈现时间。跨 lane 重用不清零，独立回归核对跳帧来源及存活 lane 零增量；无分配和波形连续性回归通过。Hub 暴露这些字段，并在每个 AirPlay worker 记录 loop gap、control 和 PCM 处理最大耗时；夹具记录逐源最大发包调度落后及相邻包间隔。未通过改变时间线或放宽质量条件掩盖旧失败。字段语义更新于 [媒体合约](MEDIA-CONTROL-CONTRACT.md) 和 [AirPlay 合约](AIRPLAY-CONTRACT.md)。

## macOS 新执行验证

正式成员格式、严格 workspace / all-targets Clippy、workspace release 均通过；常规测试 **236 passed / 0 failed / 5 ignored**，忽略项不计通过。[构建命令和退出码](evidence/platform-serial-fixes-20261003/build-checks.json)、[最终哈希核对](evidence/platform-serial-fixes-20261003/verification-final.json)。构建完成后才开始音频实验，期间没有自有编译任务与音频矩阵并行。未改 worker 源码，核对与 14:30 manifest 相同，复用项目内 worker；本轮 Hub / 核心 / 媒体实际重新编译。

| AirPlay + 原生 | 本轮结果 | 完整恢复 / 质量窗口 |
|---|---|---|
| 4 + 0、4 并发读者 × 5 ms | **两次均未完成验收**；首测稳态等待超时，定向复验 30 s 稳态零增量，随后第三来源 worker 恢复超时 | 首测 0；复验 6/24，记录 21 窗 |
| 3 + 1 | 20 s 稳态及全部恢复通过；聚合跳帧 / 存活欠载 / ingress late 无增量 | 6 / 19 |
| 2 + 2 | 15 s 稳态及全部恢复通过；同类增量零 | 8 / 25 |
| 1 + 3 | 15 s 稳态及全部恢复通过；同类增量零 | 2 / 7 |
| 0 + 4 | 60 s 稳态通过；同类增量零 | 0 / 1 |

四路首测已完成四来源接入并产生 17,951 次成功状态查询，后来等待输出超时；原夹具没有更新 phase，报告误标为独立配对阶段，也没保留安全的 wait label 及失败时发包节奏。原记录完整保留。修正这三个诊断问题后，仅做一次相同负载、相同产品二进制的定向复验，不降低读者强度或稳态时长：[首败](evidence/platform-serial-fixes-20261003/four-airplay.json)、[定向复验](evidence/platform-serial-fixes-20261003/four-airplay-diagnostic-retest.json)、[复验命令及理由](evidence/platform-serial-fixes-20261003/diagnostic-retest-check.json)。

复验 20,339 次查询零错误；完成 6 次恢复后，cycle 1 / worker_crash / source 3 在 `paired source restored alongside survivors` 等待超时。来源 3 `failure_stage=media_frame_timeout`、收到 / 释放 PCM 均 0；另外三个来源保持会话和非零电平。CoreAudio clock epoch=1、discontinuities=0、errors=0、callback_over_budget=0、聚合 timed late=0，排除该次因输出时钟重置或设备错误导致全房间失效。全部已记录质量窗口仍无跳帧 / 存活欠载 / ingress late 增量，但恢复未完成，不能计四路通过。首次失败缺少同类输出状态，不能据复验解释其唯一原因。

四个数字来源的最大发包间隔约 49.3 / 61.1 / 65.1 / 84.0 ms；其最大调度落后约 43.0 / 54.5 / 64.0 / 77.3 ms。最大值不是故障时点关联证据，不能据此直接认定是 Python 夹具或 CPU 竞争造成媒体帧超时。Hub / worker 的精确阻塞环节尚未还原，保留首败和复验，未盲目延长 250 ms IPC 帧读取期限。

双原生 Sender 60 秒 BlackHole 数字读回 **2,880,000 帧**，silent / drop / stale / callback 超预算 / 输出错误均 0；两路 lost / overflow PLC / late / PCM sink drop / timing gap / queue drop 均 0，Mixer underrun=0，jitter 认证在途峰值各 6 包、无容量耗尽。[数字读回](evidence/platform-serial-fixes-20261003/native-dual-60s.json)。PCM / ALAC 单路各两次暂停、重复 SETUP 与同步模式通过；native 媒体回归包含真实丢包 PLC、迟到、重放和容量耗尽。

[完整汇总](evidence/platform-serial-fixes-20261003/summary.json)与[运行命令及退出码](evidence/platform-serial-fixes-20261003/runtime-checks.json)可核对。全部日志及可重放 harness 留在 `artifacts/platform-serial-fixes-20261003/`，长期报告 JSON 和前后失败证据进入 `docs/evidence/platform-serial-fixes-20261003/`。所有命令经过 `tools/dev`，缓存、临时资料和构建输出均在项目内。自有夹具和秘密资料均已删除，原四个用户进程继续存活；见[清理](evidence/platform-serial-fixes-20261003/cleanup.json)。

## 范围

本轮不运行 Ubuntu / Windows，不改用户房间、默认音频路由、系统驱动或持久后台设置。合成认证加密 AirPlay、真实 CoreAudio / BlackHole 数字路径和短期计数 / 数字读回，不能代替真实 Apple 多设备互操作、模拟端听感 / 音画延迟或 8 / 24 小时长测。历史 Windows decoder / PCM queue 间歇失败仍保留，暂无唯一根因；未对其盲目扩大缓冲或改变策略。
