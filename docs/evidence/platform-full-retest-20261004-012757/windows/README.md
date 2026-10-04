# Windows x64 完整重测（2026-10-04）

221 项常规测试、正式成员格式、严格 Clippy、release/worker、单路 AirPlay/后台/凭证和四个四路组合通过。四 AirPlay 加四并发读者的 30 秒稳态通过，第二次恢复出现 5 帧定时 PCM 跳帧，质量硬断言拒绝；没有重试覆盖首败。Windows 可见 GUI、托盘和真实键盘交互仍未验收。

## 环境、版本和运行文件

测试节点 `administrator@192.168.100.186`，Windows 11 x64 `10.0.26100`，Rust `1.95.0`，Python `3.13.5`。本轮新源码、target、worker 构建、SDK、临时文件、日志和状态均在 `E:\Desktop\NeonMix-test\r06`；只读复用系统工具及测试根内下载缓存，通过 `run.ps1` → `tools/dev.ps1` 执行。

固定 523 文件快照：canonical SHA256 `0bc68d34179f1f05f05f3fcd856ac01831610661bbe4f95ed838973e50576a64`，archive SHA256 `5a5483459e2f602de77c425de1f976dff26b8ef6eac1887e52b23d11dd27282d`。与上一交付轮的产品代码没有变化；源码开始/结束均逐文件一致：[开始](source-verification-start.json)、[结束及二进制哈希](final-provenance.json)。

已有可运行包继续保留在 `E:\Desktop\NeonMix-test\packages\NeonMix-windows-x64-20261004-source322fda1b`，桌面双击 `Start-NeonMix.cmd`。本机镜像 ZIP 见 [完整交付报告](../../../PLATFORM-DELIVERY-20261004.md)。本轮使用新构建制品作完整 native 重测，没有覆盖旧包或把旧测试算入新轮。

固定输出为 `CABLE Input (VB-Audio Virtual Cable)`，显式设备 ID `wasapi:{0.0.0.00000000}.{b5553555-0ff1-440a-a2a0-c2cc504e62d6}`，48 kHz、f32 stereo、真实 WASAPI 480 帧周期。没有改默认设备或驱动；这是虚拟音频设备上的原生 I/O 和数字计数证据，不是扬声器实听。

## 新执行检查

| 检查 | 结果 |
|---|---|
| workspace 正式成员 fmt / strict Clippy | 通过；全仓 fmt 的既有 vendor 格式首败另存 |
| workspace tests，`--test-threads=1` | 221 passed / 0 failed / 5 ignored，42 targets |
| workspace release、worker、identity/audio/crypto probes | 新编译通过 |
| worker 身份与协议 | 17 类身份格式/错误输入；23 项协议，通过 |
| pair persistence / reconnect / revoke | 5 场景通过 |
| PCM / ALAC / AAC 解码 | 三 codec 通过 |
| 私有 NamedPipe | 六类准入及拒绝通过 |
| 媒体运行、双流、丢包重放、错误指纹 | 通过 |
| 文件凭证 / 独立后台 | 9 / 12 场景通过 |
| PCM / ALAC 暂停 | 每种两次同 codec SETUP 恢复，通过 |
| synchronized | ALAC 同步模式，零新 ingress late / timed skip，通过 |
| Mixer 与管理 | 增益、Mute、多 Solo、偏好重连；权限、撤销与新 PIN 重新配对，通过 |

命令、退出码、运行时刻：[suite](suite.json)、[worker 构建](worker-builds.json)、[runtime](runtime-checks.json)、[文件时间](checks-file-times.json)。ignored 未计通过。worker 第一轮因本轮 SDK 缓存 junction 的真实路径在 r06 外，被 `prepare_airplay.py` 的项目路径检查拒绝；首败保留，复制相同 SDK 到 r06 后重建成功：[首败](worker-build-first-sdk-junction.log)、[环境适配说明](sdk-path-environment-fix.json)。无生产源修改。

配对探针原始 JSON 继承了 `platform='macOS'` 固定标签，Windows 包装遗漏一个字符串替换。实际 Windows 命令、系统元数据、exe 及 worker hash 可相互核对；原始 JSON/日志保留，另记录 [标签审计](harness-metadata-audit.json)，没有为了改标签重新运行测试。

## 五种四路组合

加密合成 AirPlay 源和独立原生 SRTP Sender 接入真实 Hub/worker/WASAPI。稳态、baseline、故障及恢复窗沿用严格质量硬断言；四路原强度的四读者每 5 ms 查询一次，运行时没有编译，零自动重试。

| AirPlay + 原生 | 稳态 | 故障 cycles | 恢复观察 | 本轮结论 | 全场景 CPU 均值 / 峰值 |
|---|---:|---:|---:|---|---:|
| 4 + 0，4 读者 × 5 ms | 30 秒通过 | 计划 3 | 2 / 24，第二个质量拒绝 | **失败** | 6.188% / 11.710% |
| 3 + 1 | 30 秒通过 | 1 | 6 / 6 | 通过 | 5.455% / 13.366% |
| 2 + 2 | 30 秒通过 | 2 | 8 / 8 | 通过 | 5.574% / 17.434% |
| 1 + 3 | 30 秒通过 | 1 | 2 / 2 | 通过 | 5.166% / 19.615% |
| 0 + 4 | 60 秒通过 | 不适用 | 0 | 通过 | 4.938% / 20.314% |

四 AirPlay 共 10,800 次查询、0 查询错误；这不替代音频质量通过。具体首败为 `cycle 1 disconnect source 2 recovery digital timing quality`，`recovery_interval Mixer skipped timed PCM`：Mixer aggregate timed late 从 0 到 5；逐 lane 归属为 source 2 新恢复 lane 1、stream 14、epoch 4 的 5 帧。其余存活 source 1/3/4 的欠载和 ingress late 增量均 0。未完成剩余恢复，所以不能宣称 24 恢复通过，也不指定唯一根因。

其余四组合共 52 个质量窗口、16 次故障恢复均无 aggregate timed late、存活 lane 欠载或 ingress late 新增量。计数标准为对应观察窗增量，启动阶段既有计数不会被改写为零。所有原生 Sender 在清理前均存活；退出状态和原始 telemetry 提前保存在各组合 `native_sender_diagnostics`，随后显式停止夹具。

[矩阵汇总](summary.json)、[4+0 首败](4plus0.json)、[Hub 事件](4plus0-hub-events.json)、[3+1](3plus1.json)、[2+2](2plus2.json)、[1+3](1plus3.json)、[0+4](0plus4.json)、[逐秒 CPU](cpu-samples.json) 保留完整观察数据。全核 CPU 较低不能证明单线程、调度、锁竞争或源节奏没有影响，不把它当作根因结论。

## macOS 跨机协同

独立 LAN 房间使用同 snapshot 本轮 Hub 与固定 WASAPI 输出，端点 `https://192.168.100.186:17446`。接收夹具预算 3,600 秒，等待 Mac 编译与本机媒体矩阵全部结束才开始同快照 native Sender 60 秒测试。邀请通过产品 CLI 生成真实 300 秒 TTL，私有文件传递，不改 payload 或系统时钟。

本轮 60 秒 Sender 已完成，配对和发送退出码均 0；Mac 连续 29 个活动样本无查询错误，最后一例在 59.323 秒。Windows 接收端另有 29 个非零音量活动样本，采样跨度 56.724 秒、输出推进 2,723,040 帧。

lost、overflow PLC、PCM sinkDropped、queueDrop、late、PCM timingGap、PLC samples、Mixer underrun、timed late、输出错误和 callback over budget 的活动样本最大值全部为 0，输出 RMS 最低 0.002796；活动段整机 CPU 均值 4.047%、峰值 8.271%。因此本轮 native LAN 质量通过；这不代替 Apple AirPlay 或模拟输出实听。

原始 Sender 以 `sender_stopped` 正常结束，发出 5,999 包、queueDrops=0。独立 snapshot 观察记录 `playing` → `user_stopped`，receiver/media worker 正常移除，`errors=[]`；Hub 持续可查询，直到显式停止前仍存活，接收预算并未耗尽（3,600 秒预算、实际 1,353.886 秒）。清理前同时保存最后 snapshot、diagnostics 和进程/错误信息，避免把正常 End、预算到期和异常提前退出混为一谈。

[Mac 连续结果](../cross-platform/mac-to-windows-native-60s-summary.json)、[Sender 原始 telemetry](../cross-platform/mac-to-windows-native-60s-sender.jsonl)、[Windows 接收汇总](lan-summary.json)、[668 个接收采样](lan-diagnostics.jsonl)、[独立生命周期原始记录](lan-lifecycle.jsonl)、[生命周期转换](lan-lifecycle-summary.json)、[清理前终态](lan-terminal-state-before-cleanup.json)、[明确停止记录](lan-exit-before-cleanup.json) 均已留存。

邀请经产品生成和私有传递；本轮刷新时远端 Unix 1791050572.53、本地取回结束 1791050573.25，相差不足 1 秒。没有继承旧轮时钟偏移结论，也没有改系统时间。

## GUI 与范围界限

Hub/Sender/Mixer/Devices/Diagnostics 五预览及正式窗口均重新启动，进程在采样时存活；六个 `NeonMix` 窗口都在 SSH Session 0，`visible_window=false`。[GUI 证据](gui.json)只支持加载/进程存活，不支持真实可见桌面交互。已有 administrator console Session 1，但现有权限的 `WTSQueryUserToken` 返回 1314；[环境](environment.json)记录此界限，没有提升权限、切换会话或改策略。

本轮没有真实 Apple 设备、Apple 并发 picker、物理音画指标、扬声器实听、8/24 小时长测或发布验收。

## 清理与证据

所有自有媒体矩阵/GUI 后台和秘密夹具在 LAN 前已清理：[LAN 前清理](pre-lan-cleanup.json)。协同后明确停止 Hub 43208 和观察器，二者退出码均 0；移除本轮 `.local/windows-lan`，其中凭证/邀请一并清理，本地邀请已消费删除。最终自有 NeonMix 进程数 0，523 文件源码差异 0：[最终清理](cleanup.json)、[最终源码和制品](final-provenance.json)、[包装脚本哈希](harness-provenance.json)。源码、新制品、SDK、缓存与旧包保留。没有删除 `E:\Desktop\NeonMix-test` 外文件，没有改防火墙、默认设备、驱动、权限策略、计划任务或系统时间。

远端原始日志保留；本地证据只经显式白名单和秘密字段/PEM/bearer 脱敏后回传：[导出审计](export-audit.json)、[本地字节复核](local-export-verification.json)。82 个导出文件实际 bytes SHA256 全部匹配；继承导出器此前记录的是 LF 内存字符串哈希，已保留[首轮审计](export-audit-first-newline.json)并改为保存后取实际 CRLF 文件字节，[说明](export-newline-audit.json)保留，原始测试数据没有变化。没有回传状态目录、凭证库或邀请内容。跨机时序与本机观察使用各侧 monotonic elapsed，文件和检查 UTC 原样保留。
