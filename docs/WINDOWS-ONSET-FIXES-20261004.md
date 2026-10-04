# Windows 起播跳帧修复与 macOS 验证（2026-10-04）

针对 [最新完整重测](PLATFORM-FULL-RETEST-20261004-012757.md) 中 Windows 四 AirPlay 恢复跳过 5 帧的问题，已在 macOS 确定性复现并修复共享定时 Mixer 的起播弱点：提前缓存的首包等待播放时，小幅回调呈现时间跳变会让旧实现直接跳过首包样本。修复后同条件跳帧从 5 降为 0，并逐样本核对起播 PCM。

本轮只在 macOS 新执行验证：239 项常规测试通过、5 项忽略；正式 workspace 成员格式、严格 Clippy、release 构建通过。五种四路组合、40 次恢复和 125 个质量窗口通过，PCM/ALAC 暂停及同步模式、双原生 60 秒数字读回通过。**Windows 未原生复测，不能据此关闭 Windows 四路验收，也不能把该弱点写成原 Windows 首败的唯一底层原因。**

## 原始失败与定位

Windows 原始 [4+0 首败](evidence/platform-full-retest-20261004-012757/windows/4plus0.json) 的第二次恢复为 `disconnect source2`，归属 lane 1、stream 14、epoch 4：

| 字段 | 原值 |
|---|---:|
| 首包目标 `last_timed_late_target_ns` | 42,026,092,734 ns |
| 起播观测 `last_timed_late_presentation_ns` | 42,026,208,834 ns |
| 时间差 | 116,100 ns，即 48 kHz 的 5.5728 帧 |
| `last_timed_late_output_frame` | 2,013,696 |
| 聚合 / lane 1 跳帧增量 | 5 / 5 |

四入口均 active/ready，拒绝包与 ingress late 为零；输出没有 epoch 改变、错误或回调超预算。source2 的 Hub 最大循环间隔为 3.1451ms、最大发包间隔为 10.622ms。这些记录没有表明 decoder queue 退出，但也没有首包进入 FIFO 与前后回调锚点的逐事件时序，不能证明原首败一定由回调抖动造成。

旧 `TimedPlayback::next` 在等待未来首帧时一直保持 `position=None`。每次回调重设呈现锚点后，首次起播直接使用最新时间计算 source 偏移并 seek；即使 PCM 已经提前缓存，小幅相位跳变也会跳过采样。`audio-io` 使用 `Instant::now + playback−callback` 映射，Windows `callback` 又来自 WASAPI/QPC 时钟观测；观测到 Rust 回调进入之间的时间差是候选误差来源，本轮未验证它在 Windows 上的实际大小。

macOS 回归使用同一首包目标，在上一回调提前缓存 PCM，下一回调预测时间为目标前 8,900ns，随后增加 125,000ns 相位观测差，得到与 Windows 相同的 116,100ns 起播偏移。旧实现确定跳过 5 帧并失败；[修复前输出摘录](../artifacts/windows-onset-fix-20261004/before-regression.log) 保留，不以成功复验覆盖它。

## 修复与边界

[Mixer](../crates/audio-core/src/mixer.rs) 为已提前进入 FIFO、尚未起播的首包保存等待锚点，按输出帧数推进起播游标；不超过 1ms 的回调相位差继续交由既有 SRC servo 连续纠正。首次取得 PCM 时已经迟到，或等待期间发生超过 1ms 的时间偏差，仍按观测 seek 并记录真实跳帧。负向抖动时，相对最新原始观测最多提前 1ms 起播，已补入 [AirPlay 合约](AIRPLAY-CONTRACT.md)。

stream/epoch、输出换代、lane 重用和断流重新取得时间线会清除等待锚点。没有扩大队列、修改发送方 PTS、增加回调锁或分配，也没有修改探针断言、发包节奏或读者强度。连续播放的游标与相位 servo 保持原逻辑。

[新增回归](../crates/audio-core/tests/timed_mixer.rs) 覆盖初次起播、epoch 重建和断流恢复，分别测试 256/480/512/1056 帧回调及 125µs/1ms 正抖动，共 24 个组合，并与无抖动参考逐样本核对，要求前缀/后续 PCM 非零、无拒绝包。另验证 −116.1µs 不推迟已缓存起播；真正首次晚到 116.1µs 仍跳过 5 帧，2ms 大跳变仍跳过 96 帧。[新回归日志](../artifacts/windows-onset-fix-20261004/onset-regression.log)。

## macOS 新执行结果

所有开发命令经 `tools/dev`，依赖缓存、临时文件、编译及中间产物均在项目内；未下载新的 SDK 或额外库。原 worker 源码未变，运行夹具复制当前 release Hub 与已构建 worker，并记录二者 SHA256。

| 检查 | 结果 |
|---|---|
| workspace 测试 | 239 passed / 0 failed / 5 ignored |
| 正式 workspace 成员格式 / 严格 Clippy | 通过 |
| workspace release | 通过 |
| 加速两分钟绝对相位测试 | 通过，±100/±500ppm 与 1ms 回调抖动 |
| PCM / ALAC | 每种两次暂停及重复 SETUP 恢复，通过 |
| synchronized | 通过 |

首轮沙箱执行的后台 7 项测试因 Unix/TCP socket `Operation not permitted` 无法启动；最小 socket 检查确认环境限制后，在获准的沙箱外重跑完整 workspace，通过。原失败日志 [保留](../artifacts/windows-onset-fix-20261004/workspace-tests-sandbox-first.log)。完整命令、退出码、耗时见 [checks](../artifacts/windows-onset-fix-20261004/checks.json) 和 [runtime checks](../artifacts/windows-onset-fix-20261004/runtime-checks.json)。

全部场景使用明确的 `coreaudio:BlackHole2ch_UID`，未改变系统默认路由。构建结束后才运行音频矩阵，运行期没有并行编译或跨机测试。

| AirPlay + 原生 | 稳态目标 | 恢复 | 质量窗口 | 结果 |
|---|---:|---:|---:|---|
| 4 + 0，4 读者 × 5ms | 30s | 24 | 73 | 通过 |
| 3 + 1 | 30s | 6 | 19 | 通过 |
| 2 + 2 | 30s | 8 | 25 | 通过 |
| 1 + 3 | 30s | 2 | 7 | 通过 |
| 0 + 4 | 60s | 0 | 1 | 通过 |

独立逐窗口审计确认聚合及逐 lane 跳帧、存活 lane 欠载和 ingress late 均无新增，没有计数倒退。四 AirPlay 完成 25,145 次状态查询，零错误。原始 [4+0](../artifacts/windows-onset-fix-20261004/4plus0.json)、[3+1](../artifacts/windows-onset-fix-20261004/3plus1.json)、[2+2](../artifacts/windows-onset-fix-20261004/2plus2.json)、[1+3](../artifacts/windows-onset-fix-20261004/1plus3.json)、[0+4](../artifacts/windows-onset-fix-20261004/0plus4.json) 与 [独立审计](../artifacts/windows-onset-fix-20261004/matrix-audit.json) 保留。

**窗口零增量不等于全程零欠载。** 3+1、2+2 的第一路 AirPlay 在稳态开始前分别已有 717、794 帧欠载；旧 Windows 首次 source1 恢复后也已有累计 783 帧欠载。探针恢复窗口检查存活来源，未单独验收恢复来源从首包到稳定的全部欠载。本轮保留这些计数，没有降低或改写为零，也没有定位它们的唯一原因。

启动前 10 个整机 CPU 样本平均 73.347%、峰值 98.917%；四路运行平均 94.885%、峰值 100%。本轮没有声称满足旧轮“连续十样本均≤80%”的启动门槛，也没有根据运行中 CPU 占用中止测试或将其作为 Windows 根因证明。[逐秒采样](../artifacts/windows-onset-fix-20261004/cpu.jsonl)。

双原生 Sender→Hub→BlackHole→实际采集 60 秒通过，采集 2,880,256 帧。采集 silent/drop/errors/callback over budget 为零；两个 receiver 的 lost/overflow PLC/PCM sink drop/timing gap/queue drop/late、Mixer 欠载及输出错误为零。采集 `discontinuities=1` 原值保留，不写成所有时钟字段均零。[数字读回](../artifacts/windows-onset-fix-20261004/native-dual-60s.json) 与 [原始日志目录](../artifacts/e02-e04/20261004-025953/result.json)。

## 制品、清理与尚未验证

本次产品修改只涉及共享 Mixer；另新增三项回归、补充合约及报告/状态文档。已有大量未提交改动保留；相对最新冻结快照的本次精确补丁见 [change.diff](../artifacts/windows-onset-fix-20261004/change.diff)，[源码变化 hash](../artifacts/windows-onset-fix-20261004/source-changes.json)、[测试制品 hash](../artifacts/windows-onset-fix-20261004/hashes.json) 可核对。旧三端运行包和旧首败报告均未覆盖，没有提交 Git 或发布。

全部新运行夹具报告确认进程已停止、秘密目录已删除；数字读回恢复所选 BlackHole 原采样率。运行前后原有四个用户进程 PID 保持，新建 NeonMix 进程无残留；复现/运行的临时脚本删除，保留日志、可重跑脚本和编译产物作为审查与后续复测输入。[清理记录](../artifacts/windows-onset-fix-20261004/cleanup.json)。

Windows 原生四路、Session0 之外的可见 GUI/托盘，以及 Windows 历史 decoder/PCM queue 间歇失败未复验或证明已修复；Ubuntu 原失败保持。真实 Apple 多设备、物理听感与长时验收未计入本轮成绩。
