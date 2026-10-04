# macOS 完整重测（2026-10-04，01:27:57 快照）

本轮实际新执行 236 项常规测试（5 项忽略）、正式成员格式、严格 Clippy、release 与 worker/probes 构建，全部通过。五种四路数字组合通过，共40次恢复、125个质量窗口；独立审计聚合跳帧、存活 lane 欠载、ingress late 增量均0。正式非 preview GUI 的11个流程也通过。本轮没有修改产品源码，没有追加为了通过的重试。

共同输入523文件，canonical SHA256 `0bc68d34179f1f05f05f3fcd856ac01831610661bbe4f95ed838973e50576a64`，源码与上一交付轮仅报告/STATUS有差异，产品代码相同。开始冻结快照和最终核对无产品变化。[构建命令/时间/退出码](build-checks.json)、[运行命令](runtime-checks.json)、[二进制哈希](binary-sha256.json)、[源码末检](source-verification-final.json)。命令均经过 `tools/dev`，缓存、产物、临时资料在项目内，复用已安装Rust 1.95与原生SDK。运行阶段没有并行编译。

## 功能与矩阵

worker协议、17类身份、配对保持、PCM/ALAC/AAC解码、9类文件凭证和12类独立后台新执行通过。PCM/ALAC各两次暂停和重复SETUP、synchronized、Mixer增益/Mute/Solo、来源管理/撤销等通过。独立身份/配对与数字FairPlay/RTP来源不代表Apple设备互操作。

| AirPlay + 原生 | 稳态 | 恢复次数 | 质量窗口 | 本轮结果 |
|---|---:|---:|---:|---|
| 4 + 0，4读者×5ms | 30s | 24 | 73 | 通过，27,179查询/0错误 |
| 3 + 1 | 30s | 6 | 19 | 通过 |
| 2 + 2 | 30s | 8 | 25 | 通过 |
| 1 + 3 | 30s | 2 | 7 | 通过 |
| 0 + 4 | 60s | 0 | 1 | 通过 |

每个窗口包括真实before/after计数，主线程核对所有baseline、fault、recovery和steady对象，而不是仅检查顶层passed。见 [独立逐窗口审计](../matrix-independent-audit.json)、[本平台摘要](summary.json)及各原始JSON。

四路启动前10个1Hz整机CPU样本平均52.364%、峰值60%，满足起始≤80%门槛。运行阶段只观察CPU，不因测试中的高占用强制中断；四路65样本平均89.783%、峰值98.088%。整机CPU包括用户其他活动，不能将所有占用归因于本测试。[完整CPU采样](cpu.jsonl)。未终止其他用户进程以降低负载。

## 原生数字读回和 GUI

同主机双原生Sender→Hub→明确BlackHole输出→真实采集60秒通过，采集2,880,256帧，silent/drop/errors/callback_over_budget均0；两个Receiver lost/overflow PLC/PCM drop/timing gap均0，Mixer欠载及输出错误0。capture的discontinuities原始计数为1，未改成0；该用例通过不代表一切时钟字段都零。[原始读回报告](native-dual-60s.json)。没有保存PCM文件。

正式GUI使用独立后台/资料/端口，真实AX/键盘执行房间创建、五页导航、中文草稿、Mixer静音/恢复、诊断脱敏、Cmd+W隐藏/托盘恢复、UI崩溃后音频继续及重开、Cmd+Q取消/确认退出。内容区域1100×760与600×440，实际外框1100×792和600×472，标题栏32点；Cmd+K/Escape/Cmd+2和快速操作底部边界通过。[11流程](formal-ui.json)、[GUI命令时间](gui-checks.json)、[窄窗截图](formal-palette-600.png)。图形选择器选择实体输出完成创建后，测试后台在开始音频前明确改用BlackHole；该夹具适配在原始报告中标明。

## 取证与清理

多路测试只增加测试侧诊断钩子，在清理前记录原生Sender退出码和脱敏日志；生产文件/断言/发包负载未改，原探针和钩子SHA随每份 `*-pre-teardown.json` 保存。临时私钥、Bearer、PIN和秘密字段不进入长期诊断。

12份功能/GUI报告的清理标志全部真，记录的10个原生Sender PID全部已退出，专用GUI目录和LAN邀请无残留。[清理](cleanup.json)。没有操作既有用户房间或系统默认音频路由；双原生探针恢复明确选定BlackHole的原采样率。

当前新结果不抹除之前同代码的四路间歇首败，也不能证明它们已根治。真实Apple来源/并发、完整IME/VoiceOver、模拟端听感/音画延迟、8/24小时长测仍在本轮范围之外。已交付运行包保持，未将本轮独立构建替换进去。
