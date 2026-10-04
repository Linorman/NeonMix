# Ubuntu 重新测试与三平台运行包（2026-10-04）

## macOS / Windows 完整重测补充（01:27:57 快照）

已按追加要求对两端完整重新执行测试。macOS 236项通过、5项忽略，五组合40次恢复/125个质量窗口、双原生60秒读回和正式GUI11流程通过。Windows 221项通过、5项忽略，其余四组合52窗/16次恢复通过；四AirPlay稳态通过，但第2个恢复观察跳帧5帧，仅观察2/24，整体四路质量仍未通过。新Mac→Windows60秒通过，29个活动样本质量计数全零。Windows可见GUI仍受Session0限制，未计交互验收。完整新日志、首败、独立逐窗口审计及清理见 [两端完整重测报告](PLATFORM-FULL-RETEST-20261004-012757.md)。

以下为先前Ubuntu交付轮的记录；其中“macOS未重跑整套”“Windows未重跑完整矩阵”只描述该轮范围，现已由上方新补测更新。Ubuntu本次未再重跑，其多路和跨机失败结论保持。已交付包保留，最新快照仅文档变化，产品源码一致。

Ubuntu 本轮构建、226 项常规测试、单路 AirPlay 功能和正式桌面/后台生命周期通过，但多路与跨机音频质量仍未通过。三个平台的运行包已留存，包含运行库、插件、启动入口及版本记录；压缩包逐文件校验和重定位启动均已完成。能够启动与音频质量通过分别判定，失败记录没有被复验覆盖。

## 下载、留存与启动

| 平台 | 本地压缩包 | 机器上的可运行目录 | 手动启动 |
|---|---|---|---|
| Ubuntu 24.04 ARM64 | [Ubuntu tar.gz](../artifacts/packages/NeonMix-Ubuntu-ARM64-20261004.tar.gz) | `/home/parallels/NeonMix/artifacts/packages/u26-20261004` | 在已登录桌面用普通用户 `parallels` 运行 `./start-desktop.sh` |
| Windows x64 | [Windows ZIP](../artifacts/packages/NeonMix-windows-x64-20261004-source322fda1b.zip) | `E:\Desktop\NeonMix-test\packages\NeonMix-windows-x64-20261004-source322fda1b` | 在实际登录桌面双击 `Start-NeonMix.cmd` |
| macOS Apple Silicon | [macOS ZIP](../artifacts/packages/NeonMix-macos-arm64-20261004.zip) | 本项目 `artifacts/packages/NeonMix-macos-arm64-20261004` | 双击 `NeonMix.app` 或 `Start-NeonMix.command` |

请保留完整目录。包运行不需要 Rust、Python、编译器或 GStreamer 开发环境；仍使用各平台的系统音频及图形会话。Ubuntu 使用匹配的 Ubuntu 24.04 ARM64/glibc，包内带中文字体；Windows/macOS 使用系统中文字体。Unix 平台使用较短的可写真实路径，避免本轮已实际遇到的 Unix socket 路径长度限制。

首次启动由产品生成私有状态与凭证，没有预置配对或秘密。关闭窗口会保留独立后台，结束本包实例请使用“退出后台”。Hub 页面显式选择输出、开始共享后再开启 AirPlay；源码/许可、具体 CLI 用法及已知问题均随包提供。旧手测包没有覆盖。

| 压缩包 | 字节数 | SHA256 |
|---|---:|---|
| Ubuntu | 126,364,060 | `cba300f0886f033068ca90c9f8e9264352aa1566bf71f0aa81afb31b9d3ddd66` |
| Windows | 65,450,836 | `46823171e47bf939c70cad966b522fbb61989587ed41cad20786480dd5dfdfd7` |
| macOS | 49,745,906 | `96b379e1e7e42d175fc00a221a2e302280f7ed3c9df41c098072bb7bc1d5f2f1` |

## 固定版本和实际执行范围

三端使用同一 522 文件快照，包括当时未提交内容。开始/结束核对均无产品源码差异。本轮没有修改生产 Rust/C++ 逻辑，仅适配测试入口和打包；macOS 新增原生 bundle 启动器，其源码单独随包提供。

- canonical manifest SHA256：`322fda1b52872be7e880f750a02ef01fe0034895ecba6aa10741c504078de860`
- 输入 source.tar.gz SHA256：`3cc83620794eceb5fabf635853d9ff56e194330a89e51ac69f4db76bffb08faa`
- [快照清单](../artifacts/platform-delivery-20261004/source-manifest.json)、[快照元数据](../artifacts/platform-delivery-20261004/snapshot.json)。包内亦保存测试源码和版本/制品哈希。

Ubuntu 是此次完整重测对象。Windows 为更新后的同快照构建和运行包验收，未重新执行完整多路长矩阵。macOS 重新执行 release/worker 构建、运行包 GUI/后台/Hub/AirPlay 就绪及 Finder 启动；没有将历史 macOS 常规测试或多路成绩计入本轮。

## 构建与功能检查

| 检查 | Ubuntu 新执行 | Windows 新执行 | macOS 新执行 |
|---|---|---|---|
| 常规测试 | **226 passed / 0 failed / 5 ignored**，串行 | **221 passed / 0 failed / 5 ignored** | 未重跑整套 |
| 正式成员格式、严格 Clippy | 通过 | 通过；全仓 fmt 的既有 vendor 首败单独保留 | 本轮范围为构建/包验收 |
| workspace release、worker | 通过 | 通过 | 通过 |
| worker 身份/协议/配对、三 codec | 通过；17 类身份、23 项协议，PCM/ALAC/AAC | 通过；私有 NamedPipe 六类准入/拒绝也通过 | 包集成 worker ready；本轮未重跑整套协议 |
| 文件凭证、独立后台 | 9 / 12 场景通过 | 9 / 12 场景通过 | 包内私有 IPC、UI 退出后后台保持和明确关闭通过 |
| PCM/ALAC 暂停、重复 SETUP | 各两次通过 | 各两次通过 | 未扩大到本轮全部媒体矩阵 |
| synchronized / Mixer / 管理 | 同步、增益、Mute/Solo、权限/来源管理通过 | 媒体运行及单路集成通过 | 认证输出及 AirPlay 启动通过 |

`ignored` 不计通过。Ubuntu 可选旧凭证迁移工具另外 18 项测试和 Clippy 通过，release 链接因磁盘不足失败，未放入主包；它不属于核心运行所需程序，也没有把其测试数并入 226。

完整命令和退出码：[Ubuntu 检查](evidence/platform-delivery-20261004/ubuntu/checks.json)、[Windows 检查](evidence/platform-delivery-20261004/windows/suite.json)、[macOS 构建](evidence/platform-delivery-20261004/macos/release.json)。

## Ubuntu 五种四路组合

使用真实 PipeWire 输出和合成认证加密来源。最新探针对稳态、故障及恢复期均执行质量硬断言；没有降低读者强度或放宽断言。

| AirPlay + 原生 | 本轮结果 | 证据中的主要指标 |
|---|---|---|
| 4 + 0，四并发读者 × 5 ms | **失败**，30 秒稳态失败，未进入恢复 | timed late 增 **4,086**，逐 lane 826/1,170/917/1,173；欠载及 ingress late 增量 0 |
| 3 + 1 | **失败**，稳态通过，恢复未完成 | 第四个故障/恢复观察中 timed late 增 **20** |
| 2 + 2 | **失败**，稳态失败 | AirPlay lane 欠载增 **62**，timed late 增量 0 |
| 1 + 3 | **通过** | 30 秒稳态及两次恢复、七个质量窗口均无质量计数增量 |
| 0 + 4 | **失败**；唯一诊断复验也失败 | 首轮原生 Sender 提前退出且夹具未保留退出原因；诊断轮四来源存活 60 秒，但逐 lane 欠载 960/1,440/960/960，共 **4,320** |

四路状态查询 16,155 次、零查询错误不能替代媒体质量通过。VM 整机 CPU 的均值/峰值为 23.99%/29.55%，不能排除单线程、锁竞争、VM 调度或夹具发包节奏，不指定唯一根因。

首轮与诊断轮均留存：[原始矩阵汇总](evidence/platform-delivery-20261004/ubuntu/native-matrix-summary.json)、[主线程逐窗口独立审计](evidence/platform-delivery-20261004/ubuntu-matrix-independent-audit.json)。诊断仅补退出码和原生 telemetry，未改变二进制、负载或质量断言。

## 最终 Ubuntu 包的音频与 Mac 协同

最终包在含空格路径、清洁开发环境中可启动，Hub/worker/GUI/后台的模块来自包内或系统。但包内五秒 AirPlay 质量检查仍有 **48 帧 Mixer 欠载**；timed late 增量及输出错误为零。将 [重定位功能通过](evidence/platform-delivery-20261004/ubuntu/relocation.json) 与 [质量不通过](evidence/platform-delivery-20261004/ubuntu/relocation-verdict.json) 分列。

macOS 使用同快照原生 Sender，对此最终重定位包计划发送 60 秒、437 Hz/−36 dBFS 音频；双方均无编译、压缩或其他自有音源。配对成功、25 个活动查询样本无错误，但 Sender 在 **55.369 秒**报告 `Connection refused`，未完成 60 秒。

| 最后活动区间指标 | 结果 |
|---|---:|
| lost / overflow PLC / late / queueDrop | 0 / 0 / 0 / 0 |
| PCM sinkDropped / timingGap | **93 / 8** |
| Mixer underrun | **15,360 帧** |
| 输出设备错误 | 0 |
| 输出电平 | 非零 |

这轮不再观察到旧的 overflow PLC，但整体质量仍失败。`lost_packets` 包含 overflow PLC，不能直接等同于物理网络丢包。

远端媒体 receiver/media_worker 先消失，Hub 仍可查询、输出时钟继续推进约 210 秒，直到显式收尾才结束。unit 无 RuntimeMaxSec 限制，夹具上限未到期，因此不是接收夹具超时或 Hub 整体提前退出。会话终止的唯一原因尚未定位，也不能只凭 `Connection refused` 判定发送器本身有缺陷。

[Mac 连续样本与结果](evidence/platform-delivery-20261004/cross-platform/mac-to-ubuntu-final-package-summary.json)、[Sender 日志](evidence/platform-delivery-20261004/cross-platform/mac-to-ubuntu-final-package-sender.jsonl)、[Ubuntu 生命周期/变化区间](evidence/platform-delivery-20261004/ubuntu/lan-lifecycle.json)已保存，未再次发送以覆盖首败。

## 包独立性、桌面与留存边界

Ubuntu 仅对新包副本执行 `strip --strip-debug`，原 release 保留；所有 ELF 分配 section（含 `.text/.rodata`）的内容、地址、flags、大小均相同。最终包经过真实音频、GUI、后台和含空格路径重定位测试。主线程另验证全部 **1,113** 个校验项，零差异，启动脚本仍可执行、无运行资料/凭证/registry：[独立包审计](evidence/platform-delivery-20261004/ubuntu-package-independent-audit.json)。

Windows 六个程序及 DLL/插件/scanner 随包提供。含空格路径、清洁 PATH 下 CLI、480 帧 WASAPI 周期、后台、Hub 推进及 AirPlay ready 通过；PE 依赖无缺失，实际模块只有包/系统路径。全部 **984** 个校验项一致。桌面在 SSH Session 0 加载检查通过，但可见 GUI、托盘和交互仍未验收，完整多路 decoder 间歇失败保留为历史已知问题。

macOS 提供六个产品程序及原生 `.app` 启动器。GStreamer/编解码依赖改为相对 Frameworks 路径，深层签名校验通过；清洁环境下正式窗口、Cmd+K/Cmd+2、后台保留、认证 CoreAudio 输出和 AirPlay 就绪通过，实际模块无开发路径。Finder/LaunchServices 最终可见窗口启动通过；初轮外部磁盘访问授权及过早 AX 查询记录保留，打包入口改为保持已注册 `.app` 进程及其权限归属。macOS 包为 ad-hoc 内部测试签名，没有 Developer ID 公证或正式发布验收。全部 **1,126** 个校验项一致。

Ubuntu 正式 GUI 覆盖 Ctrl+1…5/Ctrl+K、关闭、重开、相同后台及确认退出；600×440 窄窗实测通过，普通窗实际 1100×699，受桌面可用高度限制，未宣称达到请求的 760 高度。macOS 包内 600×440 正式窗口和快速操作截图已目检。这些检查不代替完整 IME、屏幕阅读器或全部托盘体验。

本轮没有真实 Apple 来源/多 Apple 并发、扬声器实听、端到端音画延迟、8/24 小时长测、驱动安装或发布结论。

所有自有进程、邀请、秘密夹具和解包验证副本已清理；旧包、新运行包、源码和必要缓存保留。Windows 所有操作位于 `E:\Desktop\NeonMix-test`，未删除其外文件，未修改防火墙、设备路由、驱动或持久权限策略。Ubuntu 路由/默认设备及系统配置保持；macOS 没有操作既有用户房间，Finder 外部磁盘访问走正常系统授权。

完整平台报告：[Ubuntu](evidence/platform-delivery-20261004/ubuntu/README.md)、[Windows](evidence/platform-delivery-20261004/windows/README.md)、[macOS](evidence/platform-delivery-20261004/macos/README.md)。清理记录：[Ubuntu](evidence/platform-delivery-20261004/ubuntu/cleanup.json)、[Windows](evidence/platform-delivery-20261004/windows/cleanup.json)、[Mac 启动/夹具](evidence/platform-delivery-20261004/macos/finder-launch.json)。
