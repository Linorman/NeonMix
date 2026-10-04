# NeonMix

跨平台局域网音频中枢。已建立 **E00/E01 音频基础、E02/E03/E04 的 Sender/Hub、媒体传输、Mixer 与权威控制，E05 自动发现和配对功能，以及 E07 桌面产品与独立后台**；macOS 基础功能验证通过，E02–E04 完整验收仍有缺项。见 [完成度审计](docs/E02-E04-AUDIT.md) 和 [实现状态](docs/STATUS.md)。

macOS 媒体线程停顿修复后，已按用户要求完成一小时本轮验收。新增 Ubuntu ARM64 原生测试及短时跨机验证，修复 GStreamer 1.24 兼容和多网卡媒体地址；Linux 调度和设备周期修复后，Ubuntu 在系统默认 1,024 帧周期下的五分钟跨机与控制回归通过；更长时长及模拟端延迟仍待验收。见[稳定性修复记录](docs/E02-E04-STABILITY-20260930.md)和[Ubuntu 补验](docs/UBUNTU-ARM64-E02-E04.md)。

E02–E04 启动、局域网配置与复现入口见 [使用与验收](docs/E02-E04.md)，字段、权限、时钟和队列见 [媒体控制合约](docs/MEDIA-CONTROL-CONTRACT.md)。完整 macOS workspace 构建前先执行 `tools/dev python3 tools/prepare_gstreamer.py`。

E05 使用 mDNS/DNS-SD、可复制的一次性邀请、证书/Hub UUID 固定校验和相邻文件凭证（明文 JSON、私有权限）。正式入口为 `setup/discover/invite/pair`；已配对 Sender/控制可自动更新地址，撤销关闭活动媒体与 WSS。macOS 本机功能验证通过；操作、证据和其他平台待验证边界见 [E05](docs/E05.md)。


AirPlay Speaker 已实现 macOS 多入口功能 Alpha：1–4 个独立入口与原生 Sender 共享四路容量，每来源独立混音、配对和设备管理。数字源混合与故障恢复已验证；真实 Apple 多设备、选择器隐藏与长测尚未验收。构建见 [AirPlay](docs/AIRPLAY.md)，范围及已知异常见 [实现记录](docs/evidence/airplay/multi-source/implementation-20261002/README.md)。

主规格：[设计与选型](01_NeonMix_设计方案与技术选型.md) · [开发计划](02_NeonMix_完整开发计划.md)。

## 快速开始

逐项要求与未完成条件见 [完成审计](docs/E00-E01-AUDIT.md)。

需要已安装的 Rust 1.95.0、平台 SDK/C 编译工具；Linux 需要 PipeWire 桌面会话及原生开发头文件。Python 3.12+ 仅用于开发编排，产品二进制不依赖 Python。

Ubuntu 24.04 可运行 `tools/dev sh tools/prepare_linux.sh` 将开发库下载并解包到 `.local/native`，构建入口会自动使用它们；不安装到系统目录。

所有开发命令通过 `tools/dev`（Windows：`tools/dev.ps1`），缓存和构建输出留在项目内。工具链缺失时先安装到项目 `.local/rustup`，不要修改系统 Rust 安装：

```sh
# 仅当机器没有 1.95.0 时执行；已有工具链可只读复用。
mkdir -p .local/tmp .local/cargo .local/rustup
RUSTUP_HOME="$PWD/.local/rustup" CARGO_HOME="$PWD/.local/cargo" TMPDIR="$PWD/.local/tmp" \
  rustup toolchain install 1.95.0 --profile minimal --component rustfmt --component clippy
# 使用上面的项目工具链时，后续命令同样设置 RUSTUP_HOME="$PWD/.local/rustup"。

tools/dev cargo build --workspace --locked
tools/dev cargo run -p neonmix-audio -- devices
tools/dev cargo run -p neonmix-audio -- simulate --input-rate 44100 --output-rate 48000
tools/dev cargo run -p neonmix-desktop
```

Windows PowerShell：

```powershell
./tools/dev.ps1 python tools/prepare_windows_gstreamer.py
./tools/dev.ps1 cargo build --workspace --locked
./tools/dev.ps1 cargo run -p neonmix-audio -- devices
./tools/dev.ps1 cargo run -p neonmix-desktop
```

运行 `devices` 后复制所选实体设备的完整 ID。默认测试音 -36 dBFS，播放必须显式指定设备；不改变系统默认路由：

```sh
tools/dev cargo run --release -p neonmix-audio -- play \
  --device 'coreaudio:BuiltInSpeakerDevice' --seconds 3 --rate 44100 --period 127
```

真实音频使用 `--release`：debug 下的 sinc 运算不能作为实时性能基线。`--period` 是请求值，实际值以 JSON 的 `actual_period_frames` 和回调统计为准；不保证每个设备接受任意周期。

## 采集与设备事件

```sh
# macOS：使用已安装虚拟设备的可读 UID。不是麦克风/系统进程 tap 替代虚拟设备。
tools/dev cargo run --release -p neonmix-audio -- capture \
  --device 'coreaudio:YOUR_VIRTUAL_DEVICE_UID' --seconds 10 --pause-at 3 --resume-at 5

# Windows：所选虚拟 render endpoint 的 WASAPI loopback；实体端点仅用于前期验证。
tools/dev cargo run --release -p neonmix-audio -- capture \
  --device 'wasapi:YOUR_ENDPOINT_ID' --mode loopback --seconds 10

# Linux：在一个终端建立临时实验 Sink；退出即移除，不改默认输出。
tools/dev cargo run --release -p neonmix-audio -- sink --room lab --seconds 60
# 另一个终端读取它；普通应用可手动选择 NeonMix — lab。
tools/dev cargo run --release -p neonmix-audio -- capture \
  --device 'pipewire:neonmix.sink.lab' --mode loopback --seconds 30

tools/dev cargo run -p neonmix-audio -- watch --seconds 20
```

macOS虚拟输入需要启动程序的宿主应用/终端具备“隐私与安全性 → 麦克风”权限；BlackHole也适用。权限未获准时程序会明确报错。[BlackHole实测与复现](docs/BLACKHOLE-TEST.md)。

`capture` 输出 JSONL 头信息及统计，不保存 PCM。`silent_frames` 表示收到零样本；`no_data_intervals` 表示监视间隔没有音频帧，二者不能混称。暂停后恢复换 epoch，格式重建与时间戳倒退重置时间线。具体契约见 [音频合约](docs/AUDIO-CONTRACT.md)。

E06 的显式 BlackHole 提供者和 macOS Rust HAL 见 [E06 开发与验收](docs/E06.md)。BlackHole 和自研 `.driver` 的实际系统加载、数字通路、名称绑定及服务恢复已验证。Linux 由独立 `neonmix-audio virtual-output` owner 持有节点，指定 Ubuntu ARM64 桌面的功能和服务恢复已通过；Windows WaveRT render 原型已交叉编译/链接，运行验收待执行。

```sh
# macOS：仅构建并检查项目内 bundle，不安装到系统。
tools/build_macos_hal.sh
# 本机已安装 BlackHole，且 Sender 宿主获得麦克风权限后：
tools/dev cargo run --release -p neonmix-hub -- send \
  --credential .local/hub-lab/sender-a.json --virtual-output \
  --virtual-output-provider blackhole --seconds 30
```

单声道验证可使用 `sink --channels 1` 创建实验端点，测试音支持 `play --channel left|right|anti-phase`。左右反相下混到单声道应抵消；`--channel left` 的峰值应减半。

## 构建与验证

```sh
tools/dev python3 tools/check.py                 # fmt、clippy、测试、release、模拟探针
tools/dev python3 tools/doctor.py                 # 工具链、SDK、原生组件清单
tools/dev python3 tools/collect_build.py          # 二进制、符号、锁文件、源文件哈希、许可清单
tools/dev uv sync --project tools --locked       # 独立 Python 工具环境；无第三方依赖
# 数字虚拟桥接测试，必须指定设备；不是模拟端延迟测量：
tools/dev python3 tools/probe.py --release --device '<virtual-device-id>'
```

`artifacts/checks/` 保存检查日志；构建包和 dSYM/PDB/debug 文件保存在 `artifacts/`。`target/` 保留带调试信息的构建产物。源码、依赖、编译器和产物哈希由 `collect_build.py` 关联；仓库尚无提交时记录 Git 失败状态和文件哈希，不伪造 commit。

CI 定义包括 macOS 15 arm64 与 Windows x64 原生构建。Windows workflow 使用 `windows-2025` 检查完整 Rust workspace、Credential Manager、NamedPipe、媒体链路及桌面窗口；驱动使用 `windows-2022` / VS 2022 与项目内锁定 WDK，归档 `.sys`、`.inf`、`.cat`、PDB 和构建日志。Windows GStreamer 1.26.10 的官方 MSVC MSI 经 SHA256 校验后仅解包到项目内。Ubuntu 24.04 x64 暂无启用的 CI job。原生音频与设备生命周期须按 [实机验收](docs/ACCEPTANCE.md) 单独验证。

Windows 完整检查使用 `./tools/dev.ps1 python tools/check.py --native-media --keep-going`，运行探针使用 `./tools/dev.ps1 python tools/windows_runtime_probe.py`。GitHub 托管 runner 不提供本项目指定的实体音频设备；报告会明确记录音频长测和驱动加载的未验证范围。

开发宿主为 macOS 时，可执行 `tools/dev python3 tools/cross_linux_check.py`，把固定 Ubuntu 头文件和 Rust target 下载到 `.local/cross-linux` 后检查 Linux 用户态代码；默认只做交叉类型检查。加 `--build-audio --test-binaries` 会实际交叉链接Linux程序与测试；项目内Ubuntu实验环境及数字运行验证见 [LINUX-LAB](docs/LINUX-LAB.md)。它仍不能代替实体声卡验收。

## 目录与边界

| 路径 | 内容 |
|---|---|
| `crates/audio-core` | 音频块、源接口、测试信号、预分配 sinc 转换、有界队列、时间映射、统计 |
| `crates/audio-io` | 原生 CPAL I/O、格式协商、回调、错误归类；公开 48 kHz stereo 输出源接口 |
| `crates/identity` | mDNS/DNS-SD、多地址候选、一次性邀请、TLS 身份固定与三端平台凭证库 |
| `crates/media` / `crates/control` | Opus/RTP/DTLS-SRTP、反馈、权威状态、权限、版本、幂等与事件 |
| `apps/hub` | 独立 Sender/Hub CLI、TLS 1.3 API、原生输出 owner 与恢复 |
| `adapters/{windows,linux,macos}` | WASAPI、原生 PipeWire、Core Audio 平台策略；Linux Sender 生命周期 Sink |
| `apps/audio` | 独立音频 CLI 与 JSONL 探针 |
| `apps/desktop` | egui/eframe 桌面 Hub/Sender/Mixer、设备管理与诊断；独立线程连接本地后台 |
| `crates/desktop-service` | 受限本地 IPC、独立 neonmix-background、Hub/Sender 生命周期与脱敏导出 |
| `drivers/` | macOS Rust HAL 插件源码与打包；Windows 驱动边界 |
| `tools/` | 开发环境、离线探针、构建与符号归档 |
| `docs/adr` | 工程、实时、平台能力和语言边界的决策记录 |

自动发现和首次配对已接入桌面 Mixer 与独立后台（E07）；安装产品与签名分发继续对应 E09。GStreamer 仅进入媒体程序，E01 音频 CLI 保持独立；原生库/插件锁定与分发边界见 `native-dependencies.toml`。

E06 的持久输出入口为 `neonmix-hub output add/show/rename/enable/disable/remove` 与 `send --output-binding <directory>`，字段、精确设备选择及名称边界见 [输出绑定合约](docs/OUTPUT-BINDING-CONTRACT.md)。

E06 已补齐 HAL 并发与异常客户端生命周期、Linux 原生名称/绑定恢复及 Windows 自有端点识别/显式名称同步。项目内实际 HAL bundle 可通过 `tools/dev python3 tools/macos_hal_bundle_probe.py` 验证；安装入口、实际系统证据及未完成门槛见 [E06](docs/E06.md)，Ubuntu 复现范围见 [Ubuntu E06](docs/UBUNTU-ARM64-E06.md)。

## 桌面产品（E07）

通过 `tools/dev cargo build --workspace --release --locked` 构建后，运行 `tools/dev target/release/neonmix-desktop --state-dir .local/desktop`。同目录须有 `neonmix-background`、`neonmix-hub`、`neonmix-audio`。关闭窗口保持音频，托盘可恢复界面；停止发送与退出后台分别有明确入口。Hub 设置、发现配对、输出绑定、Mixer、权限管理及脱敏诊断见 [E07 使用与验收](docs/E07.md)，进程与权限边界见 [IPC 合约](docs/DESKTOP-IPC-CONTRACT.md)。macOS 已有运行交互验证；Windows x64 构建、测试及软件探针见 [Actions 报告](docs/WINDOWS-ACTIONS-20261001.md)，托管 runner 的 GUI 启动未通过。三端实体与发布门槛另行验收。

文件凭证的格式、备份、目录搬迁、显式迁移与回退见 [文件凭证使用与升级](docs/CREDENTIAL-STORAGE.md)；存储决策以 [ADR-014](docs/adr/ADR-014-file-credentials.md) 为准。系统凭证测试记录保留为历史证据，不能计作新文件方案的验收。
