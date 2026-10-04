# macOS / Windows 完整重测（2026-10-04，01:27:57 快照）

macOS 本轮完整检查通过：236 项常规测试、五种四路组合、40 次恢复、125 个质量窗口，以及正式桌面11个流程。Windows 221 项常规测试、单路及四种组合通过，但 **四 AirPlay 的第二个恢复观察出现5帧跳帧，未通过完整四路质量验收**。Mac→Windows 新执行60秒跨机发送通过。没有修改产品代码，也没有追加为了通过的复验。

这是对上一份 [三平台交付报告](PLATFORM-DELIVERY-20261004.md) 的完整补测。Ubuntu 本次没有再跑，仍保留前轮多路/跨机质量失败结论；不能将本轮 macOS 的通过外推到 Ubuntu 或 Windows 全场景。

## 版本与执行范围

新冻结523文件，包含未提交内容。与上一交付快照522文件相比，仅 `docs/STATUS.md` 和新增交付报告变化，产品源码无差异。

- canonical manifest SHA256：`0bc68d34179f1f05f05f3fcd856ac01831610661bbe4f95ed838973e50576a64`
- source.tar.gz SHA256：`5a5483459e2f602de77c425de1f976dff26b8ef6eac1887e52b23d11dd27282d`
- [输入与差异](../artifacts/platform-full-retest-20261004-012757/snapshot.json)、[逐文件源码清单](../artifacts/platform-full-retest-20261004-012757/source-manifest.json)。

两端重新实际执行构建和测试，依赖/编译缓存可复用，记录新日志、时间与退出码。Windows 新独立目录 `E:\Desktop\NeonMix-test\r06`；macOS 以当前冻结源码执行，经 `tools/dev` 使用项目内缓存/产物。运行阶段没有并行编译，Mac 本机运行/GUI全部结束后才进行跨机发送。

| 项目 | macOS 新执行 | Windows 新执行 |
|---|---|---|
| workspace 常规测试 | **236 passed / 0 failed / 5 ignored** | **221 passed / 0 failed / 5 ignored** |
| 正式成员格式、严格 Clippy、release | 通过 | 通过；全仓 fmt 的既有 vendor 差异另留首败 |
| 当前 worker 与辅助 probes 构建 | 通过 | 通过；SDK旧路径校验失败后仅复制SDK适配本轮目录 |
| 身份/协议/配对、三 codec | 17类身份、协议/配对、PCM/ALAC/AAC通过 | 同类检查通过；NamedPipe六类准入/拒绝通过 |
| 文件凭证 / 独立后台 | 9 / 12 场景通过 | 9 / 12 场景通过 |
| 单路 PCM / ALAC | 各两次暂停/重复SETUP、定时与混音隔离通过 | 同类检查通过 |
| synchronized、Mixer、管理 | 同步、增益/Mute/Solo、重连与来源/权限管理通过 | 同类检查通过 |
| 正式桌面 | 11流程通过 | Session0加载检查；console可见交互未验收 |

忽略项不计通过。命令、原始结果和二进制哈希见 [macOS 平台报告](evidence/platform-full-retest-20261004-012757/macos/README.md)、[Windows 平台报告](evidence/platform-full-retest-20261004-012757/windows/README.md)。本轮旧凭证迁移独立工程/内核驱动没有计入 workspace 成绩。

## 五组合稳态与定向故障恢复

两端使用当前相同探针的质量硬断言和明确输出，四 AirPlay 加四个读者，每次查询后5ms；目标30秒稳态、三轮逐源断开/worker故障，共24次预定恢复。其他组合30秒稳态；四原生为60秒。没有降低强度或放宽断言。

| AirPlay + 原生 | macOS | Windows |
|---|---|---|
| 4 + 0 | **通过**：24次恢复 / 73质量窗 | **失败**：稳态通过；仅2/24次恢复观察，第二窗跳帧+5 / 7已记录窗 |
| 3 + 1 | 通过：6次恢复 / 19窗 | 通过：6次恢复 / 19窗 |
| 2 + 2 | 通过：8次恢复 / 25窗 | 通过：8次恢复 / 25窗 |
| 1 + 3 | 通过：2次恢复 / 7窗 | 通过：2次恢复 / 7窗 |
| 0 + 4 | 通过：60秒 / 1窗 | 通过：60秒 / 1窗 |

主线程从原始steady/baseline/fault/recovery逐对象审计，Mac125窗无聚合 timed late、存活lane欠载、ingress late增量及计数重置。Windows其他四组合52窗、16次恢复全零；四路失败的第2个 `disconnect source2` 恢复窗口聚合跳帧+5，lane1新stream14/epoch4跳5帧，存活source1/3/4欠载和ingress late为零。记录到恢复对象不等于该对象通过断言，故没有将2个已观察对象计作2次成功恢复。[独立逐窗口审计](evidence/platform-full-retest-20261004-012757/matrix-independent-audit.json)。

Mac四路27,179次状态查询、零错误；Windows四路10,800次、零错误。查询成功不能替代声音质量通过。Windows四路没有复跑，没有用另外四组合通过覆盖首败，其原因仍未定位。

Mac四路启动前10个连续1Hz总CPU样本平均52.364%、峰值60%，满足起始≤80%条件。测试运行中只记录，不因高占用单独中断；四路运行均值89.783%、峰值98.088%。Windows四路平均6.188%、峰值11.710%。整机统计不提供锁/单线程/VM调度的唯一因果解释，也不能证明先前间歇失败已经根治。

## Mac 原生双路数字读回

同主机双Sender→Hub→明确BlackHole输出→真实采集，60秒通过，采集2,880,256帧。silent/drop/errors/callback_over_budget均0；两个Receiver lost/overflow PLC/PCM drop/timing gap均0，Mixer欠载和输出错误0。capture原始discontinuities为1，未将其改写为0，也不宣称所有时钟字段都零。[新数字读回报告](evidence/platform-full-retest-20261004-012757/macos/native-dual-60s.json)。测试结束恢复明确选定BlackHole的原采样率，不改系统默认路由。

## Mac → Windows 新执行60秒

双方本机矩阵结束、Mac正式GUI及所有自有音源/编译清理后，独立Windows Hub保持就绪，Mac同快照release CLI以437Hz/−36dBFS合成音源发送。接收器3600秒预算，邀请使用产品支持的真实300秒TTL；本轮即时两机时钟偏差小于1秒，没有调整系统时间。

配对、发送均退出0，29个活动样本无查询错误，最后活动样本59.323秒 `playing`、输出可用且非零电平。所有活动样本的lost/overflow PLC/PCM sinkDrop/timingGap/queueDrop/late、Mixer欠载、输出错误和回调超预算均0。[连续采样与摘要](evidence/platform-full-retest-20261004-012757/cross-platform/mac-to-windows-native-60s-summary.json)、[发送日志](evidence/platform-full-retest-20261004-012757/cross-platform/mac-to-windows-native-60s-sender.jsonl)。

Windows独立观察确认 `playing → user_stopped`，receiver/media worker正常清除；Hub仍存活到明确收尾，Hub/observer退出0，预算没有到期。LAN段总CPU均值4.047%、峰值8.271%。本轮通过不代替此前间歇失败的解释、长时稳定性或物理听感验收。

## GUI、包和取证边界

Mac正式GUI新执行房间创建、五页导航与中文草稿、静音/恢复、脱敏导出、Cmd+W/托盘恢复、GUI崩溃后音频保持/重开、取消/确认退出共11流程。1100×760及600×440内容区域实测；原生外框1100×792、600×472，标题栏32点。Cmd+K/Escape/Cmd+2及快速操作底部边界通过，窄窗截图已目检。[GUI原始结果](evidence/platform-full-retest-20261004-012757/macos/formal-ui.json)、[窄窗截图](evidence/platform-full-retest-20261004-012757/macos/formal-palette-600.png)。

Windows检测到console Session1已登录，但现有WTS权限返回1314；没有提升token、创建系统任务或修改权限策略。六页面加载检查发生于SSH Session0，未计作console可见窗口/快捷键/托盘体验通过。

诊断只在测试侧增加清理前原生Sender退出码与脱敏日志采集，原产品/断言/发包节奏未改。Windows配对raw报告继承错误 `platform=macOS` 文案，实际执行和EXE哈希证明是Windows，单独审计保留；证据导出首校验发现LF内存hash与CRLF文件bytes不符，修正为实际文件bytes后82文件全部匹配，没有重跑测试来掩盖格式问题。私钥、Bearer、PIN和秘密字段不进入长期导出。

三平台已交付可运行包保持，未覆盖或重新打包；产品源码无变化，文档快照和独立构建路径变化不代表运行包字节相同。下载和启动继续见 [交付报告](PLATFORM-DELIVERY-20261004.md)。旧Mac四路间歇失败、Windows四路新首败，以及Ubuntu前轮质量失败均保留。本轮不覆盖真实Apple设备/多Apple并发、完整IME/屏幕阅读器、物理音画延迟、8/24小时长测或签名发布。

所有自有进程、邀请和私有夹具已清理；Windows所有文件/产物位于指定根，旧包保留，未改关键系统设置。清理与版本：[Mac](evidence/platform-full-retest-20261004-012757/macos/cleanup.json)、[Windows](evidence/platform-full-retest-20261004-012757/windows/cleanup.json)、[Mac源文件末检](evidence/platform-full-retest-20261004-012757/macos/source-verification-final.json)。
