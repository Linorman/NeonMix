# E02 / E03 / E04 逐项证据核对

补充：用户随后授权 Ubuntu ARM64 节点测试。原生基线及 Mac/Ubuntu 短时互通已补验，并修复 1.24 兼容、多网卡媒体地址与编码前队列缺口保护；Ubuntu 的持续测试曾失败，调度与系统周期修复后，五分钟跨机及控制回归已通过（VM 默认 1,024 帧）。旧的一小时结果只归属于原 macOS 制品，详见 [Ubuntu 补验](UBUNTU-ARM64-E02-E04.md)。

核对时间：2026-09-30。范围按开发计划 §4.3–4.5 中 E02、E03、E04 的职责划分；用户明确限定只在 macOS 测试、不使用子 Agent。E05 首次配对/自动发现产品流程和 E07 桌面 UI 不列为这三个工作包的代码交付。用户后续明确：本轮持续一小时即可验收；8/24 小时与跨机模拟端门槛保留在正式长期/发布验收中。

协议/控制修订通过 82 项常规测试后，续轮 E03 审计发现缓冲静音会提前耗尽淡入。现已修正，**当前修订制品的 84 项常规测试、3 项 release DSP 注入、数字控制和 12 组真实故障均通过。当前制品保持原测试不变运行超过一小时，并按用户要求完成本轮验收；正式 8 小时目标没有执行到结束。**


| 要求 | 代码/契约 | 已有权威证据 | 状态 |
|---|---|---|---|
| E02 身份、格式/版本、session/stream/epoch/security context 绑定 | `crates/control/src/lib.rs`，`apps/hub/src/server.rs`，媒体合约 | authority 协商/角色测试，真实 HTTPS/IPv6 探针，native DTLS/SRTP security 测试 | 本机通过 |
| E02 RTP 48k/480 增量及回绕 | `crates/media/src/protocol.rs`、`transport.rs` | 原生加密 seq/timestamp 回绕；错误乱序不推进时钟、初始跨回绕乱序回归 | 本机通过 |
| E02 实际包格式与协商一致 | 认证后 TOC 时长检查；libopus 解码 | 真正经 DTLS/SRTP 认证的 20 ms Opus 进入 10 ms 协商时 PCM=0，invalid 增加；正常 10 ms 原生链路通过 | 本机通过 |
| E02 重复/乱序/迟到/坏包/超长/旧上下文 | 协议包限额、libsrtp replay、授权门 | native security、protocol 与实际 PLC 测试 | 本机通过 |
| E02 有界发包/接收，网络不等待采集 | 独立工作线程与有界 appsrc/appsink/UDP 队列，固定窗口 raw ingress 节流 | 原生坏包洪泛约 49,505 包，节流 1442 次；B 路无欠载增量 | 本机通过 |
| E02 实际 PLC 与拥塞反馈/恢复 | 单 jitterbuffer、Opus PLC，SRTCP RR/APP，滤波水位 | 原生迟到/丢包得到实际 PLC；真实反馈改变 encoder bitrate/DTX 并恢复 | 本机通过 |
| E02 证书、撤销和新密钥隔离 | TLS 控制身份→DTLS fingerprint/key-ready→PCM gate | 错误证书 PCM=0，SRTP/SRTCP replay、撤销和新上下文拒绝旧密文 | 本机通过 |
| E03 时钟/队列图和各层等待职责 | `MEDIA-CONTROL-CONTRACT.md` 中图、容量表、字段 | 对照实际管线属性与运行统计 | 文档与实现已核对 |
| E03 动态 sinc、独立漂移、水位/比率/变化上限与断点重置 | `crates/audio-core/src/mixer.rs` | 当前 70 ms 目标：±100/±500 ppm 各 600 秒、±1000/±2000 ppm 边界与 60 ms delivery stall 通过；不支持范围有界降级 | 本机 DSP 通过 |
| E03 gain/Mute/多 Solo/master/limiter/ramp | Mixer 固定配置和预分配 lanes | 实际 Mixer 波形验证所有低/高增益及总控切换在 240 帧完成；启动与 FIFO 欠载恢复中点/终点验证，实时路径零分配；真实数字增益/Mute/Solo，其他路时间线继续 | 本机通过 |
| E03 回调不阻塞/分配/析构、流增删与原 UID 输出恢复 | Mixer SPSC、RecoverableMixer owner、epoch 重置 | allocation/reallocation/deallocation=0；真实输出改率后同 UID 重开；单流故障隔离 | 本机通过 |
| E03 长期无欠载、无持续资源/延迟增长 | 固定制品的 `hub_soak_probe.py` | 当前修订制品超过一小时的实际连续样本通过；设备与进程已清理 | 用户指定的一小时本轮验收通过，正式 8/24 小时未验收 |
| E04 Hub 单权威、服务端 Principal、权限/Solo/版本/幂等 | Authority/统一 command envelope | 所有角色与真实 HTTPS 操作；并发 200/409；原子落盘失败不提交 | 本机通过 |
| E04 快照→订阅→断线快照→续订/Hub 重启 | `control_client.rs`，快照增量应用 | 6 秒慢控制、快照/WSS 重订阅与重启后事件通过 | 本机通过 |
| E04 撤销/禁止/重新允许/用户停止状态 | 授权门、权威终态与持久许可 | 重新允许不恢复旧会话，撤销与重启保持，成员 Solo 权限拒绝 | 本机通过 |
| E04 revision 耗尽原子性、慢订阅资源回收 | 内部状态变更先检查 revision；所有发送有期限、逐事件再认证 | exhausted revision 状态完全不变；被堵住的错误发送 2 秒后释放槽位 | 回归通过 |
| 跨机矩阵、实际模拟输出延迟、正式 E1/E2 条件 | 完整开发计划 §6；尚需对应实机与参考路径 | 当前没有足以证明的证据；已请求其他 Mac 的可用信息 | 未验证 |
| Beta 前 24 小时与三端门槛 | 开发计划 G3/Beta/E10 | 未执行；本轮只测试 macOS | 未验证 |

## 当前证据

- [常规测试与 DSP 检查](evidence/e02-e04/contract-result.json)、[原 82 项测试日志](evidence/e02-e04/contract-tests.log)、[当前目标水位漂移日志](evidence/e02-e04/contract-drift.log)。
- [12 组真实故障](evidence/e02-e04/contract-completion-20260930.json)，包括新增 raw datagram flood 隔离。首轮只按 socket 连续非空计数而未触发节流的失败保留为[首次失败](evidence/e02-e04/raw-ingress-first-failure-20260930.json)。
- [当前 IPv6 功能](evidence/e02-e04/contract-control-ipv6-20260930.json)。
- [前轮 30 分钟与调度修复](E02-E04-STABILITY-20260930.md)，其制品哈希与当前新增协议/控制改动分开；不能用旧制品结果覆盖新改动。

## E03 淡入修订证据

- [当前 84 项测试与 DSP 检查](evidence/e02-e04/ramp-result.json)、[测试日志](evidence/e02-e04/ramp-tests.log)、[当前漂移与 delivery-stall 日志](evidence/e02-e04/ramp-drift.log)。
- [修复前两项波形失败](evidence/e02-e04/ramp-before-fix.log)。修订后真实 Mixer 在恢复中点达到一半目标、240 帧达到完整目标；同样检查 −20 dB、Mute、+12 dB 与总控切换。buffering、FIFO reset 与输出 owner 清积压都重新规划 envelope，缓冲静音不推进它。
- [修订后的原生数字控制](evidence/e02-e04/ramp-control-20260930.json)、[12 组真实故障](evidence/e02-e04/ramp-completion-20260930.json)。

## 长期验证进度

北京时间 2026-09-30 14:58 开始的运行 `artifacts/hub-soak/20260930-145803/` 已受控停止。该运行句柄 `57068` 返回终态，结果为 `KeyboardInterrupt()`；结果文件额外记录受控停止的原因，设备与临时凭证清理完成。它提供约 15 分钟连续性的部分证据，不能记为 8 小时通过。

中止原因是功能审计找到 E03 的淡入问题，并由实际 Mixer 波形回归证明：缓冲 100 ms 后的恢复中点直接到达完整 0.02 幅度；低增益 Mute 与 +12 dB 切换的时长也不一致。原失败日志保留在 `artifacts/e03-ramp-before-fix.log`。已改为预分配的 240 帧增益 envelope，缓冲静音不消耗恢复 fade，启动、FIFO 欠载恢复和输出 epoch 重置保持同一规则。

修订制品于北京时间 2026-09-30 15:37 启动原参数 8 小时运行，固定二进制与测试参数在运行中均未修改。用户随后明确一小时验收即可。观察至样本 3720.16 秒时，实际数字捕获累计 **179,424,000 帧**，静音帧、欠载、丢包/PLC、PCM 缺口、队列丢弃、原生错误与回调超预算均为 0，原始周期快照足以证明超过一小时的连续性。最大 Sender 等待额外延迟分别约 4.626/4.645 ms。

[一小时验收报告](evidence/e02-e04/ramp-soak-3600s-20260930.json)明确采用最后的实时 `capture_stats`，没有把它冒称为 `capture_complete`。验收完成后受控停止；原 8 小时报告仍保留 `KeyboardInterrupt` 与未跑满的事实。原始目录 `artifacts/hub-soak/20260930-153710/`，句柄 81289 已终态，进程退出、设备采样率恢复、临时凭证清理完成。

三进程线程数保持 32/21/21，文件描述符区间为 23–28 / 17–19 / 17–19，physical footprint 波动均不到 1 MiB，未见持续增长。本次只证明用户要求的本机一小时开发回归；不推导为正式 E1/E2、8/24 小时或模拟端测量通过。

运行入口：

```sh
tools/dev python3 tools/hub_soak_probe.py --device coreaudio:BlackHole2ch_UID --seconds 3600
```

这是同机两路 CLI 测试音经真实加密 UDP、原生编码/输出和 BlackHole 数字读回，不能替代跨机实体输出或混合 OS 条件。全过程检查欠载、丢包/PLC、PCM 缺口/丢弃、原生错误及回调预算，采样 Hub/两路 Sender 的 CPU、physical footprint、线程与文件描述符。本轮结果与清理已核对。后续正式长期验收另按相应时长重新执行。
