<div align="center">

# N E O N M I X

### 多端声音，一个混音空间。<br>Many sources. One mix.

把局域网里的声音汇聚到一起。<br>
Bring the sound of your local network together.

![Rust](https://img.shields.io/badge/Built_with-Rust-DEA584?style=flat-square&logo=rust&logoColor=white)
![Desktop](https://img.shields.io/badge/Desktop-macOS_%C2%B7_Windows_%C2%B7_Linux-343B48?style=flat-square)
![Audio](https://img.shields.io/badge/Audio-LAN_%2B_AirPlay-6C5CE7?style=flat-square)
![Languages](https://img.shields.io/badge/Language-中文_%2F_English-00A896?style=flat-square)

[简体中文](#简体中文) · [English](#english) · [Build / 构建](#build--构建)

</div>

---

## 简体中文

**NeonMix 是用 Rust 构建的跨平台局域网音频中枢。** 它将电脑发送端与 AirPlay 音频接入同一个房间，让你在桌面混音台上管理每路声音，再通过选定的音频设备播放。

把工作电脑的声音、另一台设备上的音乐和多个音频来源放进同一套工作流：发现房间、完成配对、开始发送，然后按你的节奏调音。

<table>
<tr>
<td width="50%">

### ◉ 汇聚声音

通过原生 Sender 发送电脑音频，或通过 AirPlay 接收音频。房间将不同来源组织成独立通道。

</td>
<td width="50%">

### ≋ 掌控混音

逐路调节增益、静音与 Solo，查看通道电平，再用总控调整整个房间的输出。

</td>
</tr>
<tr>
<td>

### ⌁ 自然连接

局域网自动发现、一次性邀请与设备配对，让房间连接和成员管理集中在同一处。

</td>
<td>

### ◇ 专注桌面

深浅主题、中英文切换、键盘快捷操作与独立音频后台。关闭窗口后，声音仍可继续。

</td>
</tr>
</table>

### 声音如何流动

```mermaid
flowchart LR
    S["电脑 · Native Sender"] --> H["NeonMix Hub · 房间"]
    A["AirPlay · 音频来源"] --> H
    H --> M["Mixer · 增益 / Mute / Solo"]
    M --> O["音频设备 · Speakers / Headphones"]
```

### 开始使用

1. 在接收电脑上打开 NeonMix，选择输出设备并开启房间共享。
2. 在发送电脑上发现房间、完成配对并开始发送；AirPlay 来源选择房间对应的接收入口。
3. 打开 Mixer 调整各通道，按需静音、独听或调节总音量。

桌面界面负责控制，独立后台负责音频运行。房间、设备和混音状态围绕同一个音频工作流组织。

## English

**NeonMix is a cross-platform local-network audio hub built in Rust.** It brings native computer senders and AirPlay audio into a shared room, gives each source a channel on a desktop mixer, and plays the mix through your chosen audio device.

Bring your work computer, music from another device, and multiple audio sources into one workflow: discover a room, pair, start sending, and shape the mix.

| | What you can do |
|---|---|
| **◉ Bring sources together** | Send computer audio with a native Sender or receive AirPlay audio, with separate channels for each source. |
| **≋ Shape the mix** | Adjust per-channel gain, mute and solo, watch channel levels, and control the room’s master output. |
| **⌁ Connect locally** | Discover rooms on your network, pair through one-time invitations, and manage connected devices. |
| **◇ Make it your desktop** | Choose light or dark appearance, switch between English and Chinese, and work with keyboard shortcuts. An independent background service keeps audio running when the window closes. |

### Your first mix

1. Open NeonMix on the receiving computer, choose an output device, and enable room sharing.
2. Discover and pair with the room from a sending computer, then start sending. For AirPlay, select the room’s receiver entry.
3. Open Mixer to balance channels, mute or solo individual sources, and set the master volume.

**Signal path:** Native Sender / AirPlay → Room Hub → Channel Mixer → Audio Output.

## Build / 构建

需要 Rust **1.95.0**、平台 SDK 与 C/C++ 构建工具；开发脚本使用 Python **3.12+**。Linux 使用 PipeWire 桌面会话。所有开发命令经项目包装器执行，依赖缓存与构建产物保存在项目内。

Use Rust **1.95.0**, your platform SDK and C/C++ build tools, and Python **3.12+** for development scripts. Linux uses a PipeWire desktop session. The project wrappers keep dependency caches and build output inside the repository.

### macOS

```sh
tools/dev python3 tools/prepare_gstreamer.py
tools/dev cargo build --workspace --release --locked
tools/dev target/release/neonmix-desktop
```

### Windows · PowerShell

```powershell
./tools/dev.ps1 python tools/prepare_windows_gstreamer.py
./tools/dev.ps1 cargo build --workspace --release --locked
./tools/dev.ps1 target/release/neonmix-desktop.exe
```

### Linux · Ubuntu 24.04

```sh
tools/dev sh tools/prepare_linux.sh
tools/dev cargo build --workspace --release --locked
tools/dev target/release/neonmix-desktop
```

AirPlay 接收器使用独立的 CMake worker。macOS 可运行 `tools/dev python3 tools/prepare_airplay.py` 构建；原生依赖与安装包组装步骤见 [macOS workflow](.github/workflows/macos-installer.yml) 和 [Windows workflow](.github/workflows/windows-installer.yml)。

The AirPlay receiver is a separate CMake worker. On macOS, build it with `tools/dev python3 tools/prepare_airplay.py`. See the [macOS workflow](.github/workflows/macos-installer.yml) and [Windows workflow](.github/workflows/windows-installer.yml) for native dependencies and installer assembly.

### 工程导航 / Inside the project

| Path | 职责 / Purpose |
|---|---|
| `apps/desktop` | 桌面房间与混音界面 / Desktop room and mixer interface |
| `apps/hub` | Hub 与 Sender / Hub and Sender CLI |
| `apps/audio` | 音频设备与探针 / Audio devices and probes |
| `apps/airplay-worker` | 独立 AirPlay 接收进程 / Dedicated AirPlay receiver worker |
| `crates/audio-core` · `crates/audio-io` | 音频处理与原生 I/O / Audio processing and native I/O |
| `crates/media` · `crates/control` | 媒体传输与房间控制 / Media transport and room control |
| `crates/identity` | 发现、配对与身份 / Discovery, pairing and identity |
| `crates/desktop-service` | 独立后台与生命周期 / Background service and lifecycle |
| `adapters` · `drivers` | 平台音频集成 / Platform audio integration |
| `tools` | 开发、构建与验证 / Development, builds and verification |

```sh
# 环境检查 / Inspect the development environment
tools/dev python3 tools/doctor.py

# 工程检查 / Run project checks
tools/dev python3 tools/check.py
```

---

<div align="center">

**你的设备，你的声音，你的混音。**<br>
Your devices. Your sound. Your mix.

</div>
