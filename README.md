# NeonMix

跨平台局域网音频中枢。此仓库已建立 **E00 工程基础与 E01 用户态音频实现**；当前是可构建的音频实验框架，不是已通过三端成品验收的 Sender/Hub。准确状态、已知限制和实机证据见 [docs/STATUS.md](docs/STATUS.md)。

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

虚拟设备的可分发驱动、持久房间身份及安装属于 E06/E09。当前不附带 Windows 驱动或 macOS 插件，不会自动安装或改动已有第三方驱动。第三方设备仅可作明确记录的实验对照。

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

CI 在 Windows x64、Ubuntu 24.04 x64、macOS 15 arm64 原生 runner 上执行相同入口并保存证据；未实际运行远端 CI 前不视为已通过。原生音频与设备生命周期须按 [实机验收](docs/ACCEPTANCE.md) 单独验证。

开发宿主为 macOS 时，可执行 `tools/dev python3 tools/cross_linux_check.py`，把固定 Ubuntu 头文件和 Rust target 下载到 `.local/cross-linux` 后检查 Linux 用户态代码；默认只做交叉类型检查。加 `--build-audio --test-binaries` 会实际交叉链接Linux程序与测试；项目内Ubuntu实验环境及数字运行验证见 [LINUX-LAB](docs/LINUX-LAB.md)。它仍不能代替实体声卡验收。

## 目录与边界

| 路径 | 内容 |
|---|---|
| `crates/audio-core` | 音频块、源接口、测试信号、预分配 sinc 转换、有界队列、时间映射、统计 |
| `crates/audio-io` | 原生 CPAL I/O、格式协商、回调、错误归类；公开 48 kHz stereo 输出源接口 |
| `adapters/{windows,linux,macos}` | WASAPI、原生 PipeWire、Core Audio 平台策略与实验 Sink |
| `apps/audio` | 独立音频 CLI 与 JSONL 探针 |
| `apps/desktop` | egui/eframe 设备诊断进程；通过短生命周期子进程读取设备 |
| `drivers/` | E06 驱动工作边界，不含伪驱动或可加载占位模块 |
| `tools/` | 开发环境、离线探针、构建与符号归档 |
| `docs/adr` | 工程、实时、平台能力和语言边界的决策记录 |

当前未实现网络传输、配对、混音、漂移反馈或安装产品；对应后续工作包。GStreamer 的原生库/插件版本和分发验证独立于 Cargo，见 `native-dependencies.toml`；E01 不加载尚未集成的媒体栈。
