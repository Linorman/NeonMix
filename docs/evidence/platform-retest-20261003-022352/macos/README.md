# macOS 冻结版原生复测（2026-10-03）

macOS 功能基线通过：workspace **229 passed / 0 failed / 5 ignored**，release、严格 Clippy、工作区成员格式、worker 的协议/配对/身份与 PCM/ALAC/AAC 解码，以及正式桌面 GUI/后台生命周期均已验证。用户撤销固定 CPU 上限后，按完全原始 4 AirPlay + 4 状态读者（5ms）重跑，五种四路数字组合的最终完整场景合计 **40 次断开/SIGKILL 恢复**通过；所采样稳态/故障/恢复窗的 Mixer aggregate timed_late、存活 lane underrun 和 per-ingress late 增量均为 0。

**四 AirPlay 并发状态读者原强度最终完整通过。** 用户最后明确“不要强行设置CPU上限”，取消人为百分比起步/中止门槛，暂停本轮编译和其他压力后，原 4 读者 × 5ms、30 秒稳态、3cycles 的 24 次恢复全部通过，32,171 次状态查询、0 错误；CPU max 89.522%、p95 77.037%。此前 5ms 两轮和独立 20ms 补充因人为 CPU 门槛中止的历史仍保留，不计完整通过；低压 4+0 的 8 次恢复是补充基线，不重复计入最终 40 次。没有定位历史 CPU 高占用的唯一原因。协议探针首轮媒体连接等待超时，原断言的诊断复验通过，首轮原因仍未知。

## 版本与环境

共同 tag：`platform-retest-20261003-022352`；516 文件共同快照 canonical SHA256：`3ef7d6f5ea47a98b671768030311a494bd95c317c1b032bbc38bd00c038b9c84`。在项目内 `.local/t04m` 独立工作区构建、测试，前后全部 516 文件无差异；没有更改产品、测试断言或 vendor 源码。[源码校验](source-after.json)确认冻结制品也未改变。

macOS 27.0 / Build 26A5416b / arm64 / 12 logical cores；Rust/Cargo 1.95.0，Python 3.14.6。Cargo/CMake 构建并行度 2；工具、SDK 只读复用或复制到项目内，全部开发命令经 `tools/dev`，缓存/临时文件/产物均在项目。实际音频显式选择 `coreaudio:BlackHole2ch_UID`，保持系统默认路由；没有重启 CoreAudio 或修改权限等关键系统设置。[环境](environment.json)

这是合成加密来源经过真实 Hub/worker/GStreamer/Mixer/CoreAudio 的数字路径；不是 Apple 实机互操作、模拟端实听/音画延迟、8/24 小时长测或发布签名验收。

## 结果

| 检查 | 本轮结果 |
|---|---|
| workspace 自动测试 | 229 passed / 0 failed / 5 ignored，串行测试线程；忽略项未计通过 |
| workspace release / 严格 Clippy | 全部成功；Clippy `--workspace --all-targets --locked -- -D warnings` |
| 工作区成员格式 | 全部成功；显式选择 cargo metadata 的 workspace_members；不扩大到 excluded vendor |
| worker 协议 | 最终 23 项检查通过；首轮 media.accept 等待超时保留 |
| worker 配对 | 5/5，包括签名绑定、重启免 PIN、缺失记录修复、撤销优先、身份替换拒绝 |
| 身份 | 完整 worker 17 fixtures：4 合法编码身份/签名保持、13 错误编码干净拒绝，文件不变 |
| decoder / control v2 | PCM 24、ALAC 40、AAC-LC 51 个有界 PCM 块；44100→48000、timestamp/RTP wrap、代际/连接/请求/session/epoch 隔离通过 |
| Mixer 持久偏好 | 9 个观察检查；−6dB/Mute/单 Solo/多 Solo/清 Solo、重连保留 gain/mute 清 Solo；同时 4 个 5ms 状态读者，该短场景 CPU 条件满足 |
| 来源管理 | 跨接收入口限制、撤销、正确新 PIN 修复、来源/接收身份保持通过 |
| 单路 PCM / ALAC | 各两次暂停+重复 SETUP 恢复，late_packets 和 timed_late_frames 均 0，原生来源混音及 worker 故障隔离通过 |
| 独立后台 | 12/12：私有 IPC/凭证、实际输出、客户端崩溃隔离、取消邀请/自连接拒绝、脱敏导出、Hub SIGKILL 报告、显式重启/退出 |
| 正式 GUI | 10 个场景通过：AX/中文草稿/五页导航、原生输出下拉与创建、共享、Mixer、诊断导出、⌘W/托盘、GUI 崩溃隔离、重开、⌘Q/Escape/确认退出 |

| 完整四路组合 | 稳态要求 | 完整恢复次数 | CPU mean / p95 / max | 最小余量 |
|---|---|---:|---|---|
| 4 AirPlay + 0 原生，4 状态读者 × 5ms，观察 CPU 不强制上限 | 30s | 24 | 66.166 / 77.037 / 89.522% | 10.478%（瞬时） |
| 4 AirPlay + 0 原生，0 状态读者（另列补充基线） | 20s | 8 | 61.602 / 76.544 / 78.619% | 21.381% |
| 3 AirPlay + 1 原生 | ≥11s | 6 | 54.295 / 64.090 / 65.919% | 34.081% |
| 2 AirPlay + 2 原生 | ≥11s | 8 | 47.882 / 60.033 / 66.639% | 33.361% |
| 1 AirPlay + 3 原生 | ≥11s | 2 | 40.272 / 51.372 / 51.372% | 48.628% |
| 0 AirPlay + 4 原生 | 60s | 0 | 32.588 / 42.311 / 70.441% | 29.559% |

最终五种完整组合（含原强度 4 读者，排除另列低压基线）共 125 个稳态/故障/恢复采样窗；低压补充另有 25 窗，global timed_late 和存活 lane 欠载/late 增量为 0。Mixer 重连另一个采样窗同样为 0。保留 before/after 原始计数：[逐窗口审计](timing-audit.json)。采样不代表连续模拟端无缝播放测量。

## CPU 观察与历史保护中止

1Hz Mach `HOST_CPU_LOAD_INFO` 整机采样，不将单核 100% 等同整机满载。历史阶段曾等连续 10 个样本 <80%，两次连续 >80% 或任一样本 ≥95% 中止自己的进程组。**用户最后撤销固定上限后，完全取消百分比起步/中止门槛，不设置 CPU 亲和或硬 cap**，仅记录 CPU；最新原强度重测与编译/其他压力串行。历史门槛已不再用于最终运行。未终止用户原有进程；跨机协作期间暂停本轮编译/运行探针。

| 独立压力场景 | 实际运行 | 查询与恢复 | CPU mean / p95 / max | 结论 |
|---|---|---|---|---|
| 4 路 + 4 读者，5ms 首轮 | 32.906s | 13,545 成功查询；恢复循环未开始 | 66.465 / 81.910 / 86.717% | CPU 条件不满足，保护中止 |
| 同配置一次复验，Ubuntu VM heavy 已停止 | 53.219s | 25,388 成功查询；14 次恢复已完成 | 70.470 / 81.643 / 92.821% | CPU 条件不满足，整体不通过 |
| 4 路 + 4 读者，20ms 补充 | 10.525s | 未完成稳态/恢复 | 66.484 / 94.653 / 94.653% | 历史 CPU 条件不满足，整体不通过 |
| **4 路 + 4 读者，原始 5ms，撤销固定上限后最终重测** | **66.402s** | **32,171 次成功查询、0 错误；完整 24 次恢复** | **66.166 / 77.037 / 89.522%** | **完整通过，CPU 仅观察** |

中止的 5ms 复验已完成部分恢复的采样窗计数为 0，但不得把历史中止轮累加到最终完整 40 次矩阵，或把中止轮本身计作完整压力通过；最新取消固定上限的完整原场景另有独立结果。历史两轮 5ms 各记录 2 个查询错误，但没有错误时间戳；保护 SIGINT 会同时结束自有 Hub/worker，不能据此唯一归因于正常运行查询失败或关停。原始值保留，最终完整无固定门槛运行的 0 错误另有独立证据。[保护中止分类](cpu-interruptions.json)

正式 GUI 有一个 89.983% 瞬时样本，workspace 编译/测试有一个 81.438% 样本，均未持续；因此不声称这些阶段全程 ≥20% 余量。1Hz 监视分为两个区间：02:29:58–03:12:02（2,514 样本）及 03:20:45–03:22:50（125 样本，覆盖最后原强度重测）；总共 2,639 样本、均值 39.156%、p95 68.254%、最大 94.653%，没有 ≥95% 样本。中间无测试运行的空档没有采样，不伪称连续监视；逐场景原始 timestamp/epoch/util/logical_cores 在 [CPU JSONL](cpu-monitor.jsonl)，汇总在 [CPU 汇总](cpu-summary.json) 和各 `*-checks.json`。

最初默认并行 Rust LTO 在收到新增 CPU 约束时观察到整机 0% idle；已立即暂停/停止自己的编译，以 2 jobs 接续完成。该被中断的构建不计成功，日志单独保留（`release-first-unbounded-interrupted.log` / `build-checks-first-interrupted.json`）；不把它混入受保护运行探针结果。

## 原始失败与 harness 适配

`cargo fmt --all` 在这个项目内嵌套工作区扫描 excluded `vendor/mdns-sd-scoped` 时把它判为父仓 workspace 成员而报错；原日志保留，改用明确 workspace_members 的成员格式检查通过。没有改 manifest 或 vendor 文件。

pairing 首轮内部检查已完成，但默认证据目录不存在；identity 的 17 个内部检查完成，但传到冻结 ROOT 外的报告路径被探针拒绝。创建冻结工作区自己的证据目录、改为项目内报告路径后复验成功。协议首轮等待媒体连接超时，诊断 harness 只补失败出口信息，成功路径/断言不变；该首轮未知原因保留，不宣布产品已修复。原 `quality-checks.json` 以及三个原失败日志均保存，最终结果另在 `worker-retry-checks.json`。

GUI 首轮按 button 找折叠控件，实际 AX role 是 checkbox；第二轮发现产品“实体输出”下拉明确过滤 BlackHole 等虚拟设备；第三轮 System Events 瞬时进程索引报错。原截图/AX/JSON/退出码均保留。最终适配只根据真实 AX role 调用、对瞬态 AX 查询重试；先通过 GUI 选物理输出并创建配置（没有启动物理音频），再通过这个 fixture 的后台 API 改为 BlackHole，重开正式 GUI 后开始共享。没有修改产品 GUI，所有实际音频仍只到 BlackHole。[正式 GUI 结果与限制](formal-ui.json)

## 制品、清理与复现

不可变五制品保留在项目 `artifacts/platform-retest-20261003-022352/macos/bin`，包括 `neonmix-hub`、`neonmix-desktop`、`neonmix-background`、`neonmix-audio`、`neonmix-airplay-worker`；[SHA256](binary-provenance.json)。为保持 Rust 的 `@loader_path/../../.local/gstreamer/prefix/lib`，本轮 tag 目录下 `.local/gstreamer` 链到独立 `.local/t04m` SDK。没有生成新的分发包或覆盖旧手测包。

所有本轮 Hub/worker/Sender/UI/后台已停止，所有临时凭证/邀请/Unix socket 与 fixture 已删。`.local/t04m/.local/tmp`、`.local/airplay-multi` 均为空；原有用户 Hub 7443 PID **98625**、后台 **81271/98624**、worker **98648** 保持存活。源码与制品前后完全一致，没有修改共享 `STATUS.md` 或主报告。[汇总与清理校验](summary.json)

可复现编排保留在 [harness](harness/)，命令、退出码、耗时与 CPU 样本统计保存在 `build-checks.json`、`quality-checks.json`、`member-checks.json`、`worker-retry-checks.json`、`runtime-checks.json`、`runtime-cpu-retry-checks.json`、`runtime-bounded-checks.json`、`runtime-low-load-checks.json`、`runtime-uncapped-checks.json` 和 `gui-checks.json`。跨机 LAN 由主线程用相同 Hub `765b8761…` 客户端副本验证，另在主报告汇总；本地结果不替代远端结果。
