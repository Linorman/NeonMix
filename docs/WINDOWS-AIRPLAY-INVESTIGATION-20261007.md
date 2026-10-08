# Windows AirPlay 问题调查（2026-10-07）

本轮已复现原配对 iPhone（iOS 27 / Spotify）“只响一声后停止”的直接故障链：Windows worker 的 NamedPipe 写端吞吐不足，音频过期并最终 PCM 队列超限退出。配对与ALAC格式正确，声卡回调错误为0。已修改媒体写端并通过原生吞吐/超时/停止回归；用户已确认同一配对身份的修复版“现在正常了”，本次短时实机播放通过。另三项协议参数/UDP边界缺陷已修复。

## 原配对播放故障：Windows NamedPipe 吞吐不足（P1，已修复并实机复验）

原资料的私有副本保留 receiver ID、密钥与已知来源。用户确认“只响了一声就停止”，对应日志中 pair-verify、FairPlay、SETUP/RECORD 均成功，格式为ALAC / 44.1kHz / spf352。Hub收到208块，释放24块，拒绝184块且全部为late_packets；没有identity_rejections、malformed_packets、clock_resets或输出设备errors。最终 `worker_failed / pcm_output_queue`。这与此前用户安装导出的208/184计数一致。见 [原配对失败证据](evidence/windows-airplay-20261007/iphone-paired-before.json)。

失败前逐次采样：0.546秒增加35块、0.543秒增加34块、0.549秒增加35块，约64块/秒。源端每352帧产生一包，44,100/352≈125.3包/秒；worker有解码输出，但送往Hub的速度不足，短暂声音后队列不断增长并触发200块上限。

原因在 [Windows MediaPipe::write](../apps/airplay-worker/platform.h)：原 `PIPE_NOWAIT` 写入在单包大小的管道背压下反复sleep_for(5ms)，该MinGW运行时在测试机表现为约15.6ms调度等待，每秒约64包。使用生产适配器、真实owner-only NamedPipe、独立父子进程和相同3176字节包做对照，600包原来耗时9,349,045μs；修复后耗时21,381μs。此为批量吞吐测试，不是声学延迟指标。[修复前](evidence/windows-airplay-20261007/pipe-before.json)、[修复后](evidence/windows-airplay-20261007/pipe-fixed.json)。

修复使用 `FILE_FLAG_OVERLAPPED + PIPE_WAIT`，等待独立完成事件；管道一旦可写即可唤醒，无需在每包之间等待调度tick。仅允许一个待完成写，保留250ms整包预算、原管道大小及所有队列上限。停止/超时调用CancelIoEx并等到完成后再释放缓冲和OVERLAPPED，防止异步I/O继续引用已释放内存。首次对照中阻塞写250,319μs超时，50ms后请求停止时58,440μs返回。[Microsoft的异步管道说明](https://learn.microsoft.com/en-us/windows/win32/ipc/synchronous-and-overlapped-input-and-output)与[取消完成要求](https://learn.microsoft.com/en-us/windows/win32/fileio/canceling-pending-i-o-operations)支持该实现方式。

原生回归入口为 [C++生产适配器探针](../apps/airplay-worker/pipe_probe.cpp)和[Windows父端驱动](../tools/windows_airplay_pipe_probe.py)，不依赖音频设备，不改变全局计时器分辨率。修复版在测试根的 `patched-airplay` 中运行、使用同一个 `paired-state` 完成实机复验；随后已停止本轮进程并清理测试资料副本，原D盘安装不覆盖。

所有本轮 Windows 文件位于 `E:\Desktop\NeonMix-test\investigation-20261007`；读取原 `D:\NeonMix` 后复制为隔离运行副本，使用全新测试身份、独立 17448 端口。没有替换原安装、输出原私钥内容、关闭防火墙或更改系统音频格式。后续免 PIN 复验在测试根内完整复制原 Hub 资料并设置当前用户私有权限；原资料不修改。既有 `fix-20261007`、r06 等目录只读复用/检查，不清理他人文件。仓库原有未提交 UI / Hub API / 后台改动保留。

## 1. 新身份 PIN 尝试的范围纠正（历史诊断，非原配对根因）

隔离目标为 `NeonMix — Win Diagnostic 1007`。接收器发布正常，WASAPI 输出已打开为 48 kHz / 480 帧。实际连接顺序：

| 相对 worker 时间 | 事件 | 结果 |
|---:|---|---|
| 52,338 ms | TCP open | 成功 |
| 52,339 ms | GET /info | 200，440 字节完整发送 |
| 52,343 ms | POST /pair-pin-start | 200，97 字节完整发送 |
| 52,348 ms | TCP close | `peer_closed`，由来源端关闭 |

没有 `/pair-setup-pin`、`/pair-verify`、SETUP、RECORD；`received_blocks=0`，没有丢失诊断事件。用户所说约 20 秒是手机侧表现；协议连接在响应后约 5 ms 即已关闭，不能写成服务端播放 20 秒后掉线。接收机当前三个 Windows Firewall profile 均 disabled，本轮没有改变其设置，因此不能把这次连接失败归因于 Windows Firewall。

[实机事件证据](evidence/windows-airplay-20261007/iphone-attempt.json)。同样的“PIN-start 后主动关闭、没有后续配对”此前也在 macOS 记录过，见 [历史配对调查](AIRPLAY-PAIRING-INVESTIGATION.md)，故目前没有证据表明它是 Windows 独有问题。

**解决路径**：先确认 iPhone 是否实际显示并提交 PIN。如果没有，使用 iOS 控制中心的系统 AirPlay 选择器做对照，区分 Spotify 内入口与系统提示流程；随后在同一接收身份、同一有效 PIN 窗口下重试，必要时做手机重启前后对照。只有观察到 SRP、pair-verify、音频 SETUP 和连续 PCM 才算推进到播放测试。源端主动关闭可能属于 PIN 输入前的正常换连接过程，单凭关闭不能证明是哪个字段或哪端的错误。[上游配对流程说明](https://github.com/FDH2/UxPlay/wiki/crypto)也将 PIN-start 与后续 PIN 配对分开。不能据此改成无认证，也不能凭猜测扩大视频或 AirPlay 2 能力声明。

## 2. 缺少 spf 导致 ALAC 无声并退出（P1，已修复）

`raop_handlers.h` 连续读取 `controlPort`、`ct`、`spf` 时复用同一个 `uint_val`。libplist 在节点不存在时保持输出值不变，ALAC 的 `ct=2` 因而被错误当成每帧 2 个采样。worker 原有 352 帧默认值只在 spf=0 时生效，构造出错误的 ALAC decoder cookie。

安装版原生复现：相同合法加密 ALAC 包，显式 spf=352 输出 95,024 字节 PCM IPC；省略 spf 则显示 `source_frame_count=2`、PCM 为 0，持续输入后 `decoder metadata limit exceeded` 并退出。修复在读取可选 spf 前清零，缺失时进入既有 codec 默认值。修复后相同 1,100 包输出 3,494,456 字节 PCM IPC，无 fatal，进程存活。

这是可以独立导致“无声、之后断连”的缺陷；原配对实机提供了正确的spf352，因此它不是该次实机故障的直接原因。

## 3. 参数缩窄绕过范围检查（P2，已修复）

spf 在进入 worker 的 8192 帧上限检查之前先转成 `unsigned short`。`65538` 因而变成 2，被 SETUP 200 接受，重复第 2 项无声行为。`controlPort` 也有相同的 16 位缩窄风险。

现于 SETUP preflight、停止旧媒体之前校验可选字段的整数类型和范围；超限 spf、字符串 spf、超限 controlPort 均返回 400。有效参数、缺省 spf 和已验证的所有编码保持原行为。

## 4. Windows UDP 接收失败被当成同步包（P2，已修复）

`raop_rtp.c` 将 `recvfrom` 的返回值存为 unsigned。Windows 对超长 datagram 返回 -1 / WSAEMSGSIZE，转成很大的正数后越过最小长度检查，已部分写入的缓冲被当成完整同步包解析。安装版注入一个超长控制包后，出现 `timestamp_jump`，PCM 只输出到重置前的 47,376 字节。

现保留有符号返回值并在读取报头前丢弃失败、过短或超长包；接收缓冲多留 1 字节，使 POSIX 的截断行为也能识别超出协议上限的包。Windows/macOS 同一测试均完整输出 95,024 字节、无 reset，正常音频继续。没有放大媒体队列或隐藏时间线异常。

第 2–4 项维护在 [上游补丁](../patches/airplay-audio-only.patch)，不是只修改 `.local` 中的生成源码。[回归探针](../tools/airplay_setup_probe.py)支持 Windows/macOS 显式选择匹配的 runtime、worker 和 crypto helper；[Crypto helper](../apps/airplay-worker/auth_probe.py)仅增加可选库路径，不改变默认调用。

## 其他确定的潜在问题与解决方案

| 优先级 | 问题与证据 | 解决方案 / 验证要求 |
|---|---|---|
| P1 | [安装器](../packaging/windows/neonmix.nsi) 按进程名全局 `taskkill /F /T /IM`。安装/卸载一个副本也会强制中断其他目录的 NeonMix 会话。随后递归删除 `$INSTDIR\bin/plugins/airplay`，没有验证目标是不是专属安装目录。 | 按安装路径与用户限定进程，先通过后台 IPC 正常停止；使用安装清单或可信安装标记校验目录，避免覆盖共享目录。需双副本、正在播放和非专属安装路径的隔离验收。本轮没有运行安装/卸载器。 |
| P1 | [Windows installer CI](../.github/workflows/windows-installer.yml) 的 worker smoke 仅把空输入送给 worker，看到任意 fatal 就通过；该错误发生在 `gst_init` 和插件检查之前，不能证明插件、配对、音频可运行。 | CI 构建 crypto probe，针对最终包运行带正确私有配置的 ready + 加密音频探针（可接入本轮新增探针），并检查至少一个真实 PCM 包；保留跨机/Apple 设备验收，不能用 localhost 代替。 |
| P2 | [worker](../apps/airplay-worker/main.cpp) 在 callback 发出 fatal 后退出主循环仍 `return 0`，而异常分支才返回 1。本轮旧版 decoder metadata fatal 的实际退出码为 0，且继续处理队列时重复发出 fatal。 | 记录首个 terminal failure，关闭媒体入口，只发一次 fatal，正常 stop 与失败使用不同退出码；增加 fatal/正常停止的进程级回归。Hub 当前也读 fatal 事件，不能夸大成 Hub 必然忽略故障。 |
| P2 | UxPlay 的 NTP/RTP socket 初始化失败由 void start 函数内部直接返回；SETUP handler 仍可能应答 200 和 0 端口，上游日志又被 worker 关闭。 | 将 start 改成可检查结果，失败时释放已建资源并返回明确 SETUP 错误、记录脱敏阶段。用占用端口/资源失败注入验证；本轮未复现该失败，不作为 iPhone 根因。 |

以上四项是本轮审查发现并给出的修复方案，尚未修改对应产品代码或宣布验收完成。既有多路长测、真实 Apple 并发和 Windows 可见 GUI 的历史验收边界仍以 `docs/STATUS.md` 为准。

## 本轮验证与限制

- Windows 安装版问题前置复现：[原始结果](evidence/windows-airplay-20261007/windows-setup-before.json)。
- Windows 新 worker 7/7 边界回归：[结果](evidence/windows-airplay-20261007/windows-setup-fixed.json)。
- macOS 新 worker 7/7 同一边界回归：[结果](evidence/windows-airplay-20261007/mac-setup-fixed.json)。
- Windows/macOS 既有协议 probe 均通过，分别记录 63 条传输事件，包含正确/错误 PIN、配对签名、重连、撤销、旧代际隔离、re-SETUP、加密 RTP→PCM：[Windows](evidence/windows-airplay-20261007/windows-protocol.json)、[macOS](evidence/windows-airplay-20261007/mac-protocol.json)。
- macOS PCM/ALAC/AAC-LC 解码与控制 v2 probe 通过；配对持久化 5/5 通过：[解码](evidence/windows-airplay-20261007/mac-codecs.log)、[持久化](evidence/windows-airplay-20261007/mac-pairing.json)。
- 维护补丁可应用到锁定上游，并准确生成已测试的两个修改文件：[补丁核对](evidence/windows-airplay-20261007/patch-verification.json)。

构建/探针配置中曾遇到 PowerShell `-D` 参数拆分、测试配置缺少 pairing_attempts、Windows MSVC/MinGW DLL 与插件混用、macOS 开发包装器覆盖插件路径，以及 macOS 测试发送缓冲默认不足。均修正测试环境后执行上述最终结果；没有把这些夹具失败归为用户安装的根因。旧安装使用它自己的匹配 runtime；新 Windows worker 使用既有项目内 MSVC SDK 与 MinGW 编译器，运行需按报告中的 SDK/DLL 匹配，**不能将该 EXE 单独覆盖到原 MinGW 安装包**。

未运行全部 Rust workspace 测试（没有修改 Rust 产品逻辑），未构建或安装新的正式安装包。本次iPhone实际Windows播放由用户确认正常，但没有进行物理音质、音画延迟或长时稳定性验收。

复现新增探针（macOS）：

```sh
tools/dev python3 tools/airplay_setup_probe.py \
  --worker .local/airplay/build/neonmix-airplay-worker \
  --crypto .local/airplay/build/libneonmix-airplay-probe-crypto.dylib \
  --openssl /opt/homebrew/opt/openssl@3/bin/openssl \
  --plugins .local/airplay/plugins \
  --report artifacts/airplay-setup-result.json
```

Windows 使用 `tools/dev.ps1 python tools/airplay_setup_probe.py`，显式指定 `.exe/.dll` 路径、匹配的 `--plugins`，并按查找顺序传入 `--dll-dir`（本机成功顺序为现有 MinGW bin、项目内匹配的 GStreamer bin）。报告路径必须在当前项目内；无音频设备、无 mDNS 发布、仅 loopback 合成来源。

收尾已停止本轮自有 Hub/worker，清理重复源码压缩包、临时音频夹具、过期测试脚本和失败的 CMake 构建目录；原 D 盘安装与起初复制的运行树逐文件哈希一致。未删除测试根外文件。隔离运行副本、稳定测试身份和构建/证据保留用于继续确认 iPhone 配对；临时 PIN 已过期，重试前需重新启用测试接收器。见 [清理核对](evidence/windows-airplay-20261007/cleanup.json)。

## 用户补充后的免 PIN 复验

用户明确表示此前已配对、不会出现 PIN 输入框。重新检查发现原资料为 receiver v2、两个入口（仅「NeonMix — 客厅」启用）、一个来源和一条绑定。已将完整原 Hub 资料复制为测试根内 `paired-state`，逐文件 SHA256 核对一致并设置当前用户私有 ACL。隔离 Hub 使用原接收 ID/密钥/配对记录与原输出设备，独立监听17448；未再生成新接收身份或强制重新配对。用户选择「NeonMix — 客厅」后重现“只响一声后停止”，已取得成功免PIN重连及PCM输出队列失败的记录，见本文开头。此前新身份目标的 PIN-start 日志不能作为原身份缺少配对的证据。

追加管道生命周期回归：吞吐、250ms背压超时、停止取消、对端关闭后后续写失败四项均通过，见 [完整结果](evidence/windows-airplay-20261007/pipe-lifecycle-fixed.json)。对端关闭的第一版夹具错误地要求“当时已提交的写一定返回失败”；Windows可能完成该写，随后下一次写才报告关闭。夹具已增加一次后续写，验证关闭后不会继续成功或挂起；未修改产品实现来绕过失败，保留 [首轮断言结果](evidence/windows-airplay-20261007/pipe-lifecycle-assertion-before.json)。

## 修复后 iPhone 实听与最终清理

用户明确反馈“现在正常了”。日志覆盖76个active样本、约41.101秒活动会话，共接收4,712块、释放4,627块；late_packets、identity_rejections、clock_resets、overflow_packets、worker错误、输出errors与callback_over_budget均为0。结束时收到两条TEARDOWN 200，随后peer_closed，未出现原来的PCM队列超限。见 [最终状态](evidence/windows-airplay-20261007/iphone-paired-fixed.json)、[连续计数摘要](evidence/windows-airplay-20261007/iphone-paired-fixed-summary.json)及 [实测构建哈希](evidence/windows-airplay-20261007/paired-tested-build.json)。

质量边界保留：起播阶段85块计入malformed_packets（该字段同时包括无效包和时钟不确定度超限，现有日志不能区分）；从开始出声后到停播前不再增长。最后音频停止、TEARDOWN完成前Mixer累计underrun_frames为75,943；稳定出声区间曾保持0。不能把本次功能修复写成全程零丢弃、零欠载或长期质量通过。

测试接收进程已停止。原Hub资料12个文件逐文件哈希与测试前一致；删除仅涉及本轮创建的paired-state/manual-state私有副本和临时运行脚本，源码、已测构建与脱敏证据保留，未删除测试根外文件。[最终清理](evidence/windows-airplay-20261007/paired-cleanup.json)。上述初轮清理记录保留作历史证据，最终状态以本节为准。原D盘安装尚未替换，单独复制该混合SDK构建EXE到原安装并不是经过验证的安装方案。
