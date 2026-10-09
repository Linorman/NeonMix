<div align="center">

# N E O N M I X

### 多端声音，一个混音空间。

把局域网里的声音汇聚到一起。

![Rust](https://img.shields.io/badge/Built_with-Rust-DEA584?style=flat-square&logo=rust&logoColor=white)
![Desktop](https://img.shields.io/badge/Desktop-macOS_%C2%B7_Windows_%C2%B7_Linux-343B48?style=flat-square)
![Audio](https://img.shields.io/badge/Audio-LAN_%2B_AirPlay-6C5CE7?style=flat-square)
![Languages](https://img.shields.io/badge/Language-中文_%2F_English-00A896?style=flat-square)

[English](README.md) · **简体中文** · [构建](#构建)

</div>

---

## 认识 NeonMix

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

## 构建

需要 Rust **1.95.0**、平台 SDK 与 C/C++ 构建工具；开发脚本使用 Python **3.12+**。Linux 使用 PipeWire 桌面会话。所有开发命令经项目包装器执行，依赖缓存与构建产物保存在项目内。

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

### 工程导航

| 路径 | 职责 |
|---|---|
| `apps/desktop` | 桌面房间与混音界面 |
| `apps/hub` | Hub 与 Sender |
| `apps/audio` | 音频设备与探针 |
| `apps/airplay-worker` | 独立 AirPlay 接收进程 |
| `crates/audio-core` · `crates/audio-io` | 音频处理与原生 I/O |
| `crates/media` · `crates/control` | 媒体传输与房间控制 |
| `crates/identity` | 发现、配对与身份 |
| `crates/desktop-service` | 独立后台与生命周期 |
| `adapters` · `drivers` | 平台音频集成 |
| `tools` | 开发、构建与验证 |

```sh
# 环境检查
tools/dev python3 tools/doctor.py

# 工程检查
tools/dev python3 tools/check.py
```

---

<div align="center">

**你的设备，你的声音，你的混音。**

</div>
