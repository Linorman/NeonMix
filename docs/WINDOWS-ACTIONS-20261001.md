# Windows GitHub Actions 验证（2026-10-01）

Windows x64 的完整 Rust workspace 已通过格式检查、Clippy `-D warnings`、原生自动测试与 release 构建。Credential Manager、NamedPipe、WASAPI 健康空设备列表、GStreamer 加密媒体和采样率模拟通过。WaveRT 驱动的原生 MSVC/WDK 构建、INF 验证及 Inf2Cat 通过。**桌面 GUI 启动未通过**：托管 runner 的 OpenGL 能力不满足 egui 所需的 2.0。没有实体音频设备，不能据此判断虚拟机持续采集 Xrun 的根因或宣称声卡长测通过。

## 测试版本与环境

- release、运行探针和驱动：源码快照 `6b2690d435a0003aea490a5e06b7a3cdcfcc8ec2`，[运行记录](https://github.com/Linorman/NeonMix/actions/runs/36841991251)。该轮完整测试在 Hub 计时断言处停止，其他运行结果分别归档。
- 完整 workspace 自动测试复验：`acff3c306f15816d9cf6bcd662adef39e1a48ab2`，[复验记录](https://github.com/Linorman/NeonMix/actions/runs/36846853256)。该轮更改回滚测试的计时范围，并使用 `--no-fail-fast` 继续全部测试；没有更改生产媒体逻辑。
- 用户态：Windows Server 2025 x64，`windows-2025-vs2026` 镜像，Rust 1.95.0 / MSVC。它是 Windows 原生执行，仍属于 GitHub 托管虚拟机。
- 驱动：Windows Server 2022 x64，VS 2022 x64 MSBuild，MSVC 14.44.35207，项目内 SDK/WDK 10.0.26100.6584 / KMDF 1.33。
- GStreamer：官方 MSVC x86_64 1.26.10 runtime/devel MSI，固定 SHA256 后行政解包到 `.local/gstreamer-windows`；没有运行产品安装。

本轮在独立 `codex/windows-native-20261001` 分支测试当前工作区快照，保留本地 `main`、索引和既有未提交改动。独立 AirPlay C++ worker 不属于 Cargo workspace，本轮没有验证其 Windows 运行。

## 结果

| 检查 | 结果与范围 |
|---|---|
| 格式、Clippy | workspace 格式及所有 target 的 `-D warnings` 通过；vendor 依赖的既有警告记录在日志中 |
| 自动测试 | 50 个测试 target 完成，131 项通过、0 项失败、5 项默认忽略；其中平台凭证用例已在独立显式探针中通过。其余为三项长时漂移/停顿注入和一项实体设备重开，不能计为通过 |
| release | 完整 workspace MSVC 构建通过，四个应用及 PDB、源文件/制品 SHA256 已归档 |
| Credential Manager | 随机临时凭证写入、读回、删除及二次删除通过 |
| NamedPipe | 当前用户启动、双 owner 拒绝、关闭及再次打开通过 |
| WASAPI 枚举 | AudioEndpointBuilder / Audiosrv 运行，枚举返回 `[]`；健康空列表通过 |
| 媒体 runtime | app、Opus、RTP、jitterbuffer、DTLS/SRTP 等必需原生组件通过 |
| 五秒双路 | 两路均认证并解码，输出 240000 帧，Mixer 欠载 0，peak 0.008050，RMS 0.003929 |
| 五秒丢包与重放 | `--drop-every 17 --replay` 通过，PLC 实际执行，输出 240000 帧，Mixer 欠载 0 |
| 错误身份拒绝 | 两秒错误证书指纹探针通过，1191 次拒绝、PCM/peak 为 0 |
| 采样率模拟 | 44.1/48/96 kHz → 48 kHz 三档通过；属于离线软件模拟 |
| GUI | 首个 Hub 页面启动退出 1，`egui_glow requires opengl 2.0+`；其余四页未执行，不能标记 GUI 通过 |
| WaveRT | 十个翻译单元原生编译/链接、INF/Inf2Cat 通过；下载后七份文件哈希与 x64/native PE、NX/ASLR/CFG 核对通过；未签名、未系统加载 |

## 本轮修复

托盘的 `with_icon_templated` 是 macOS 专用 API。现在 macOS 保留 template 图标，Windows/Linux 使用通用 `with_icon`；修复 Windows 桌面编译阻断。本机七项桌面回归通过，静态设计审计无问题。

WDK 构建显式定位 VS 2022 的 x64 MSBuild，选择项目内 x64 StampInf 工具；删除没有采集端点时不可达的安装路径，标记 PortCls 委托参数，并把生成的 SYS 纳入打包列表。保留警告和 INF/目录校验，归档 SYS/INF/CAT/PDB 与解析后的工具链属性。

首轮回滚断言把原生媒体初始化和回滚一起计时，该 Windows 测试组总耗时约九秒，整体调用超过五秒。复验从失败提交处开始计时，只测 prepared worker 清理；保留五秒阈值、producer 所有权和 Mixer 未提交状态断言。生产 Hub 在建立资源前已经探测原生 runtime。原始失败日志仍保留，准备耗时与回滚计时的范围明确区分。

检查入口支持失败后继续和聚焦复验；单个 Cargo 测试 target 失败也不再跳过其他 target。每项结果及时落盘，缺失二进制产生明确失败记录。release 成功后才归档应用。编译缓存留在项目内并由 Actions 复用。

## 复现与证据

```powershell
./tools/dev.ps1 python tools/prepare_windows_gstreamer.py
./tools/dev.ps1 python tools/check.py --native-media --keep-going
./tools/dev.ps1 python tools/windows_runtime_probe.py
./tools/dev.ps1 python tools/collect_build.py --native-media
./tools/dev.ps1 python tools/prepare_windows_driver.py
./tools/dev.ps1 powershell -File tools/build_windows_driver.ps1
```

[选取的原始日志与 JSON](evidence/windows-actions-20261001/)保留初次失败、完整构建、运行探针和驱动验证。应用及 PDB/GStreamer 制品在[运行制品](https://github.com/Linorman/NeonMix/actions/runs/36841991251/artifacts/11152988481)，驱动及符号在[驱动制品](https://github.com/Linorman/NeonMix/actions/runs/36841991251/artifacts/11152005965)。这些是测试制品，安装、签名分发、Driver Verifier/HLK/HVCI、真实 GUI 交互、实体声卡/虚拟端点、热插拔/休眠和持续音频稳定性仍按项目验收边界执行。
