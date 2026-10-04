# Windows 同版本原生复测（2026-10-03）

本轮只测试主线程冻结的共同快照，在 `192.168.100.186`、用户 `administrator` 的 **`E:\Desktop\NeonMix-test\r03`** 独立构建，没有复用旧EXE，也没有修改产品源码。旧目录、旧包和其他手动会话均保留。

## 版本、构建与检查

共同快照512文件；canonical manifest SHA256：`b0aa5d6bcf84e1d12dae4ff400cd885cb288f124107332907a9a1cb6cda7e9ef`；源码tar SHA256：`c9d7f4682764dbad9cc0d22b9ded71aa8e1ea6e8393f6614e608cf636fb08704`。最终逐文件核对 **0差异**，见 [final-provenance.json](final-provenance.json)。

Rust 1.95.0 MSVC，已有VS2022与MinGW GCC只读复用，Cargo registry/GStreamer 1.26.10 SDK复制至新目录；所有开发命令经 `tools/dev.ps1`，新 `target` 独立。六个EXE及probe哈希均在上述provenance中，含本轮新增的 `neonmix-airplay-profile.exe`。

| 项目 | 本轮结果 |
|---|---|
| workspace fmt | `--all`初次失败：嵌套快照中的excluded vendor crate向上遇到原工程workspace；显式列出16个workspace package后fmt check通过。不是源格式缺陷，未执行格式化改写 |
| 严格Clippy | **未通过**：`crates/airplay-ipc/src/local_windows.rs:38,46` 两个Drop中的unsafe块缺SAFETY注释；`-D clippy::undocumented_unsafe_blocks`报错 |
| 普通自动测试 | `cargo test --workspace --all-targets --locked --offline --no-fail-fast`：42个target、**210通过、0失败、5默认忽略** |
| Rust release | 完整workspace原生构建通过，137秒 |
| C++ worker | MinGW原生worker、crypto DLL、codec probe均构建通过 |
| 文件凭证 | **9类全部通过**；Windows本地code page复验无UTF8 harness解码噪声 |
| 数字媒体 | runtime、五秒双路、五秒丢包/重放、错误身份拒绝均通过 |
| 独立后台 | 本轮私有状态NamedPipe status、shutdown、退出0通过；workspace另含双owner与重开测试 |
| WASAPI | 新EXE设备枚举与3秒默认周期输出通过，48kHz/480帧、未修改默认设备 |

上述测试覆盖workspace及其平台启用target，不包括独立excluded `credential-migrate` 项目的完整迁移矩阵。忽略项保持忽略，不计入通过。

证据：[suite.json](suite.json)、[fmt原始错误](fmt.log)、[members格式复验](fmt-members.log)、[Clippy](clippy.log)、[完整测试](tests.log)、[release](release.log)、[worker构建](worker-build.log)、[文件凭证](credentials-native-locale.log)、[运行探针](runtime.json)、[独立后台](background-smoke.json)、[原生输出](native-output-run.log)。

## 新版本 Mac → Windows 60秒LAN

本轮新Hub在 `https://192.168.100.186:17445`、房间 `NeonMix Windows R03`，使用VB-Audio Virtual Cable的明确WASAPI输出。Mac使用同一共同快照的新制品配对并发送60秒，已结束；测试房间随后删除。

远端保存137个连续诊断样本，其中29个含活动接收器。活动样本全部：输出错误0、callback超预算0、Mixer欠载0、接收丢包0、PCM sink drop0；最高输出peak **0.00403994**，输出帧从4,700,736持续到7,422,816。[汇总](lan-summary.json)、[完整连续JSONL](lan-diagnostics.jsonl)、[Hub启动](lan-hub.log)、[开始诊断](lan-before.json)、[结束诊断](lan-after.json)。Mac侧另有主线程的60秒发送与29点playing采样证据。

这是原生网络→Mixer→WASAPI数字输出的短程验证，不是实体扬声器实听、模拟端延迟或长时稳定性验收。Windows与Mac时钟仍有偏差，未改时钟，邀请生成并拉回后及时配对成功。

## 多AirPlay v2：仍有两个独立启动阻断

本轮实际通过 `airplay-v2` 配置 `receiver_count=2, multi_receiver=true`，配置操作成功，容量limit=4；逐入口Enable请求也进入本轮worker启动路径，但两个receiver最终都 **enabled=false、ready=false、error=worker_failed、failure_stage=decoder**。未建立AirPlay媒体session，之后全局Disable成功。不能把API配置成功算成音频接收成功。见 [完整v2启用结果](airplay-v2-enable.json)。

1. **Hub媒体IPC与worker不匹配。** Rust Windows端提供NamedPipe endpoint，而当前worker只接受IPv4 loopback。使用本轮worker和control v2初始化的独立原生fixture，stdout明确返回 `media IPC must be IPv4 loopback`、exit1；见 [worker-ipc-check.json](worker-ipc-check.json)。这不是旧版本证据，也不是误用v1 guard。
2. **TCP协议路线还会遇到OpenSSL Applink缺失。** 将macOS协议探针的路径、crypto DLL、当前用户私有文件ACL等适配到Windows后，真实worker完成TCP媒体IPC连接，但读取PEM身份/进入ready前exit1；stderr明确为 `OPENSSL_Uplink(...,08): no OPENSSL_Applink`。原生MinGW worker与官方MSVC GStreamer/OpenSSL的这一启动路径失败独立于NamedPipe拒绝。见 [协议最终stderr](worker-protocol-final.log)及 [配对启动失败](worker-pairing-final.log)。协议/配对用例未完成，不能计为通过。

独立codec probe通过PCM、ALAC、AAC三类解码，以及probe内置的control v2 generation/connection/request/session/epoch约束检查；见 [worker-codecs.log](worker-codecs.log)。它不经过完整PEM身份读取，因此其通过不能证明完整worker可启动。没有两路真实Apple来源或两路加密数字源经此Windows Hub播放成功的结论。

适配均在测试harness中完成，产品512文件保持原样。初次harness缺OpenSSL CLI路径、缺TMPDIR与icacls输出编码的问题保留初始日志，已与上述真实产品启动失败区分。

## 桌面与环境边界

本轮新desktop EXE分别启动Hub、Sender、Mixer、Devices、Diagnostics五个preview页面及正式模式。六次均为SSH **Session 0**，等待8秒后仅枚举到隐藏NeonMix/WGL/NVIDIA OpenGL窗口，未出现可见窗口，未主动退出；测试随后只结束自己启动的UI进程。[gui.json](gui.json)保留每页实际窗口与状态。

因此桌面可构建、单测通过，但本轮可见窗口、托盘、中文输入及关闭窗口保持音频等正式交互 **未验收**。`post-lan.json`中GUI harness退出0仅表示采集过程完成，不代表UI通过。未调整用户权限、计划任务或显示配置来绕过Session 0。

所有测试源/依赖/缓存/临时/制品均在指定目录范围内；未安装驱动、修改防火墙、系统默认音频路由、系统服务或时钟。PowerShell调用中的Bypass仅限启动进程，未写入持久ExecutionPolicy配置。

## 清理与留存

本轮r03下全部NeonMix测试进程已退出，删除LAN profile/文件凭证/invite、GUI与后台fixture、dummy key；本地中转invite也已删除。旧目录的用户手动会话没有终止。新源码、EXE/PDB、SDK/cache、worker构建产物和全部日志留在r03供复现。[cleanup.json](cleanup.json)确认无剩余r03测试进程。
