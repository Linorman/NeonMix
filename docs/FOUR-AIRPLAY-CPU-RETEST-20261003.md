# 四路 AirPlay 起始负载门槛复测（2026-10-03）

本轮 **通过**：四路认证加密数字来源、四个并发状态读者（每次查询后5ms）、30秒稳态、3轮逐源 disconnect / worker_crash，完整完成 **24次故障恢复、73个质量窗口**，状态查询 **29,331成功 / 0错误**。全部稳态 / baseline / fault / recovery窗口独立核对：聚合 timed late、存活 lane underrun、ingress late增量均0，无计数重置。

时间：2026-10-03 21:37:18—21:38:30（Asia/Shanghai），实际实验约72.722秒，之前另有10秒起始CPU采样。仅测试macOS显式CoreAudio BlackHole输出，沿用上一轮同一Hub / worker二进制，未修改产品源码、未编译、未降低测试强度；哈希见[执行记录](evidence/four-airplay-cpu-retest-20261003/execution.json)。

用户先指定超过80%时结束并待资源释放重测，随后明确“测试中占用超过80%是正常的”。据最新说明，将80%作为**启动前整机CPU门槛**，运行中测试自身高占用只观察，不因超过80%单独中断。使用与旧轮相同的Mach host CPU tick差值、1Hz采样；10个连续启动前样本均≤80%，无需中止或额外等待。没有终止其他用户进程来降低负载。

| CPU观察范围 | 样本数 | 平均 | 峰值 | 超过80%的样本 |
|---|---:|---:|---:|---:|
| 启动前 | 10 | 27.266% | 36.318% | 0 |
| 整个测试（包含接入、故障、恢复及清理） | 72 | 64.753% | 77.75% | 0 |

[原始CPU采样](evidence/four-airplay-cpu-retest-20261003/cpu.jsonl)、[全部分阶段计数与来源节奏](evidence/four-airplay-cpu-retest-20261003/result.json)、[独立汇总及清理](evidence/four-airplay-cpu-retest-20261003/summary.json)。首次尝试便完整通过，没有进行以通过为目的的追加重试。自有夹具进程和临时凭证资料均已清理，原四个用户进程全部存活；不改默认音频路由或用户房间。可重放脚本和运行日志在项目内 `artifacts/four-airplay-cpu-retest-20261003/`。

此前的四路稳态等待超时与恢复 `media_frame_timeout` 留在[上一轮报告](PLATFORM-SERIAL-FIXES-20261003.md)。本轮同二进制通过只能说明这次运行成功，不能唯一证明旧失败由CPU竞争造成，也不能宣称间歇问题已修复。合成认证加密AirPlay及CoreAudio数字计数不代替真实四台Apple设备、模拟端听感 / 音画延迟或8 / 24小时长测。
