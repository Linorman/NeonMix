# Windows 更新快照原生实验（2026-10-03，12:41 快照）

**四 AirPlay 未通过。** 最新原生二进制在四路定向复验的第三来源接入期间，第一个 worker 出现 `pcm_output_queue`，已有三路中的第一路退出；其余两路继续播放。没有到达第四路、30 秒稳态或计划故障恢复。三 AirPlay + 一原生、二 AirPlay + 二原生、一 AirPlay + 三原生、四原生的稳态和定向恢复均通过。Windows 可见 GUI 仍受 SSH Session 0 限制，没有验收。

## 版本、机器与构建

机器 `192.168.100.186`，`administrator`，Windows 11 `10.0.26100` x64，Rust 1.95.0 MSVC，Python 3.13.5；官方 MSVC GStreamer 1.26.10，已有 MinGW GCC 编译 C++ worker。独立根目录 `E:\Desktop\NeonMix-test\runs\v20261003-1241`，缓存、临时文件、SDK、源码、target 和证据均在用户指定根目录内。GStreamer SDK 复制到新根，项目内缓存和只读已安装 Rust 工具链复用。全部开发命令通过 `run.ps1` → `tools/dev.ps1`，音频场景与编译串行。

共同 519 文件，canonical manifest SHA256 `53ee66ce3ed30a46bb08dafa9a47dfe955aac41313e4d3b15e345f8dbe3d8e5e`；初始与本机实验后的逐文件均 0 差异。本轮产品源码未修改，Hub 与 worker 是新目录内本轮编译的制品；见 [环境](environment.json)、[初始校验](source-verification.json)、[最终源码与二进制哈希](final-provenance.json)。

| 检查 | 结果 |
|---|---|
| workspace all-targets 测试 | 42 targets，**217 passed / 0 failed / 5 ignored**，74.73 秒 |
| 严格 Clippy | 全 workspace/all-targets、`-D warnings`、locked/offline，通过 |
| release | 全 workspace 原生 MSVC，通过，135.56 秒 |
| 格式 | `fmt --all --check` 遇 excluded vendor 嵌套 workspace 发现错误；显式 16 个 workspace package 的只读格式检查通过 |
| worker/probes | 本轮 MinGW 编译通过；协议23项、身份17项、配对持久性5项、PCM/ALAC/AAC解码通过 |
| NamedPipe | 6项原生正反安全场景通过；包括当前SID、owner-only受保护DACL、父server PID，以及广泛/未保护DACL/错PID/缺失/非法pipe拒绝 |
| 媒体与文件凭证 | GStreamer runtime、5秒双路、丢包重放、错误身份拒绝通过；文件凭证9类native locale通过 |
| 原生输出 | 明确指定 WASAPI VB-Audio Virtual Cable，3秒输出完成，实际480帧周期，输出errors/callback-over-budget均0 |

[完整构建命令/退出码](suite.json)、[自动测试日志](tests.log)、[worker命令](worker-suite.json)、[协议](worker-protocol.log)、[解码](worker-codecs.log)、[NamedPipe](pipe-security.json)、[运行命令](runtime-checks.json)、[凭证native locale](credentials-native-locale.log)保存。5个忽略项不算通过；驱动安装、发布签名及 excluded 迁移工程完整矩阵不在本轮范围。

## 多入口、稳态质量与故障恢复

明确选择已有 `wasapi:{0.0.0.00000000}.{b5553555-0ff1-440a-a2a0-c2cc504e62d6}`，没有修改系统默认输出。

| 组合 | 结果 | 完成恢复 | 系统CPU均值 / 峰值 |
|---|---|---:|---:|
| 4 AirPlay首轮 | 夹具清理WinError5、原质量report未写出，质量证据不足 | 未知 | 4.83% / 10.36% |
| 4 AirPlay一次定向复验 | **失败**，第三来源接入期间第一worker `pcm_output_queue`，未到稳态 | 0 | 3.87% / 8.61% |
| 3 AirPlay + 1原生 | 20秒稳态与4读者通过 | 6 | 5.74% / 12.35% |
| 2 AirPlay + 2原生 | 20秒稳态与4读者通过 | 4 | 5.95% / 15.42% |
| 1 AirPlay + 3原生 | 20秒稳态与4读者通过 | 2 | 7.15% / 19.36% |
| 0 AirPlay + 4原生 | 60秒稳态通过 | 0 | 3.68% / 19.97% |

四个通过组合共12次恢复、40个稳态/故障/恢复采样窗口。所有这些窗口聚合timed late、存活lane underrun、存活AirPlay ingress late增量均0；状态读者共27,529次成功、0错误。CPU为1Hz全系统观察，不设强制门槛；不是模拟端实听或连续波形录音证明。[逐组合质量/CPU汇总](summary.json)保留。

四路首轮的原始 [traceback](mix-4plus0.log)、[CPU](mix-4plus0-cpu.jsonl)、[脱敏Hub事件](mix-4plus0-hub-events.json)均保留。原探针在finally中删除仍被Windows短暂占用的worker EXE，然后才写report，因此没有首轮质量结果；不能将其当作已确认的decoder/质量失败，也不能用复验推断首轮原因。仅修改本轮Windows测试harness，改为先保存report，再等待自有EXE解锁并清理；没有修改519文件产品快照。首轮夹具最终清理完成。

一次定向复验同输出、同二进制、同四路/30秒稳态/4读者/2轮恢复配置。完整 [失败report](mix-4plus0-retest.json)显示第1来源 `pcm_output_queue`，received481、accepted294、late187、Mixer `timed_late_frames=3213`；第2/3来源active，第四来源尚未开始，读者尚未启动（successful0）。因此异常发生在独立配对/接入阶段，不能归因并发读者；本轮细分诊断缩小到PCM发送队列，造成队列故障的原因仍未唯一定位。report保存后等待10次×0.1秒，夹具清理完成。没有再次重复整个矩阵或以通过条件替代原强度。

## 单路、后台与GUI

单路PCM/ALAC各两次1秒暂停+重复SETUP恢复、原生共混、worker故障隔离通过，两个报告timed late均0；synchronized模式保持协议PTS并通过。[PCM](single-pcm.json)、[ALAC](single-alac.json)、[同步](single-sync.json)。Mixer 9项偏好观察及管理4项观察通过，[Mixer](mix-control.json)、[管理](management.json)。

独立后台12/12场景通过，真实NamedPipe、Get-Acl私有目录/导出断言、文件秘密引用、低电平实际输出、客户崩溃后Hub保持、取消邀请/自连接拒绝、诊断脱敏、故障不无限重启、显式停止/关闭均验证。[后台结果](background-full.json)。日志中既有icacls本机代码页与PYTHONUTF8 reader噪声保留；实际原生ACL断言通过，文件凭证另以native locale干净验证，没有将日志噪声认作产品权限失败。

本轮desktop EXE实际启动5个preview页面和正式模式，各8秒，均Session0、仅隐藏窗口、无可见窗口，进程未自行退出后终止自有UI。[GUI结果](gui.json)。Ctrl快捷键提示/导航、600×440快速操作截图、可见桌面/托盘/中文输入/关闭窗口保留音频等交互**未验收**；没有修改会话策略、切换console、建立计划任务或改系统设置。

## 协同与清理

Ubuntu整轮及其测试进程/采样停止后，由主线程使用同快照macOS CLI正常邀请配对，实际向`https://192.168.100.186:17446`发送60秒，fresh本轮Hub使用明确选择的VB-Audio WASAPI输出，Windows没有并行编译或压力场景。pair/send均exit0，主线程29个活动样本playing/非零输出且查询无错误，见[Mac端协同结果](../cross-platform/windows-native-lan.json)。

Windows保存128个2秒诊断点，其中29个receiver存在且输出RMS非零的活动点，活动点跨度56.706秒（不把采样跨度写成60秒连续录音），output frames推进2,721,600；lost、overflow PLC、PCM sink drop、queue drop、late、timing gap、PLC samples、Mixer underrun、timed late、输出errors和callback over-budget的活动样本最大值均0。系统CPU活动均值7.98%、峰10.78%；不是端到端模拟音质/延迟或Apple来源互通验证。[完整诊断](lan-diagnostics.jsonl)、[活动窗口汇总](lan-summary.json)、[前](lan-before.json)/[后](lan-after.json)已保存。启动/空闲累计silent_frames没有被统称为零。

主线程首次等待器误填18443，收到邀请后identity连接失败，未请求实际17446服务端；已独立保留[首次地址编排错误](../cross-platform/windows-native-lan-endpoint-harness-first.json)。随后按实际17446正常完成完整60秒，未重启本轮Hub、未放宽邀请过期校验。邀请仅在测试窗口生成120秒、本地0600保存，协同完成后远端profile/邀请及本地中转邀请清理。遵守用户“Ubuntu和Mac不能同时测试”约束，Mac协同期间Ubuntu整轮已停止。

本机独立场景及LAN完成后519文件0差异，自有NeonMix进程0残留；GUI状态、首轮残留夹具及LAN秘密fixture已清理。当前没有原有NeonMix进程；不把历史PID存在视为当前事实。源码、release EXE/PDB、SDK和本轮所有证据保留独立runs目录，旧根/旧手测包保持。未删除指定根外文件、未安装驱动、未修改防火墙/持久ExecutionPolicy/服务/默认音频路由/时钟。[最终清理](cleanup.json)、[LAN前夹具清理](pre-lan-cleanup.json)、[最终哈希](final-provenance.json)。

本报告对应已终结的 `platform-validation-20261003-1241` Windows轮次。用户后续仅重跑Ubuntu/macOS，此处结果作为既有不同轮次证据，不代表Windows再次测试。最终只读[终结核对](closure-check.json)再次确认519文件0差异、自有进程/采样0残留、GUI/LAN/多路秘密夹具不存在；没有启动新的Windows测试。
