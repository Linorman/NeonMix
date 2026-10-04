# NeonMix：设计方案与技术选型

- **修订日期：**2026-09-29
- **文档用途：**定义产品范围、架构、技术选型和行为约定。
- **配套文档：**[NeonMix：完整开发计划](02_NeonMix_完整开发计划.md)，负责排期、任务、测试与发布门槛。

> 实施状态更新（2026-09-29）：E00/E01 工程实现与本机证据见 [实现状态](docs/STATUS.md)。三端成品、虚拟桥接和阶段验收尚未全部完成；本方案中的性能数字仍是初始目标，不是产品承诺。锁定依赖与能力边界见 `native-dependencies.toml` 和 `docs/adr/`。

## 一、项目名称与跨平台范围

**项目与产品名称统一为 NeonMix，定位为跨平台局域网音频中枢。NeonMix v1 必须同时支持 Windows、Linux 和 macOS，三端均提供 Sender、Hub 和桌面管理界面。** Linux 和 macOS 属于首版交付范围，不作为 Windows 版发布后的可选移植。

对外使用 NeonMix、NeonMix Sender、NeonMix Hub；系统虚拟输出建议显示为“NeonMix — 客厅”，房间名可由用户修改，内部身份不依赖显示名称。

平台负责音频接入，共享核心负责媒体协议、时钟适配、混音与控制。当前进入 E00/E01 工程实施阶段；三端完整支持仍是开发和发布要求，已实现与已验证范围以 `docs/STATUS.md` 为准。

### 1.1 NeonMix v1 的产品闭环

**任意两台受支持的 Windows、Linux 或 macOS 电脑选择 NeonMix 虚拟输出，向任一受支持平台上的 Hub 同时发送声音；Hub 混音后从一个有线或 USB 输出播放，支持独立调音与故障恢复。**

| 维度 | 首版范围 |
|---|---|
| 平台与角色 | Windows、Linux、macOS 均实现 Sender、Hub 和桌面管理 |
| 最低架构范围 | Windows 11 x64、Linux x86_64（PipeWire 桌面环境）、macOS arm64（Apple Silicon） |
| 系统版本范围 | 启动时锁定具体 Windows/macOS 支持版本，以及至少一个 Linux 发行版及 PipeWire/会话管理器版本；Beta 前公布完整支持矩阵 |
| 拓扑 | 两台 Sender、一台独立 Hub、一个房间、一个实体输出；允许三端任意搭配 |
| 输入规模 | 正式支持两路；内部做四路压力测试，不据此承诺四路支持 |
| 系统接入 | 各平台提供一个稳定、可由普通应用选择的 NeonMix 虚拟输出；用户通过系统或桌面音频设置选择 |
| 音频 | 内部 48 kHz、立体声、float32 PCM；原生网络编码使用 Opus |
| 控制 | 每路音量、Mute、Solo、断开，房间总音量与总静音 |
| 连接 | 统一局域网发现、配对、认证、加密、撤销；无需互联网账号 |
| 运行 | 用户登录后常驻；关闭管理窗口不停止播放，各端提供明确停止入口 |

以上架构是最低交付基线，不等于支持任意 CPU、Linux 发行版或历史系统版本；额外架构按需求扩展。跨平台验收同时检查各端完整功能和不同平台之间的互通，不能用“共享代码可编译”代替成品支持。

首版不包含 AirPlay、移动端、多房间路由或同步、应用级分轨、Ducking、麦克风回传、公网传输、多声道影院和专业乐器监听。蓝牙输出仅作探索，不进入首版延迟验收。任意视频应用的自动音画同步也不属于承诺。

范围不足以按期完成时，调整资源、时间或非核心功能，不将 Windows、Linux、macOS 中的任一平台从 v1 范围移出。

### 1.2 系统音响、服务发现与回录的区别

mDNS 发现 Hub，只说明客户端知道其地址；要在 Windows 系统输出列表出现音响，仍需音频端点。WASAPI loopback 捕获已有渲染端点的混合声音，不能自行创建输出设备，也不会自动阻止本地播放。Loopback 仅支持共享模式，并受受保护内容等限制。[Microsoft：Loopback Recording](https://learn.microsoft.com/en-us/windows/win32/coreaudio/loopback-recording)

因此分别验收“声音可以传过去”和“普通应用可以选择虚拟 Speaker”。Windows 路线支持正常使用所选共享模式输出端点的应用，Linux/macOS 按各自音频系统的标准输出路径验证；显式选择其他输出、特殊独占路径或受保护内容的应用另建兼容性记录。

Hub 只需向主机已经支持的实体设备播放，不因接收网络声音而需要虚拟声卡。被动音箱仍需功放，外部 DAC 也必须具备正常的系统支持。

## 二、总体架构与模块边界

```text
Windows 应用 → 虚拟 Render Endpoint / WASAPI ─┐
Linux 应用   → PipeWire 虚拟 Sink ────────────┼→ NeonMix Sender
macOS 应用   → Core Audio 虚拟设备 ──────────┘       ↓
                                        统一 Opus + SRTP/SRTCP
                                                   ↓ 局域网
NeonMix Hub：每路认证接收 → 重排/抖动缓冲 → 解码/丢包处理
    ↓
每路时钟适配 → 有界 PCM 队列 → Mixer → 总增益/限幅
    ↓
输出适配：WASAPI（Windows）/ PipeWire（Linux）/ Core Audio（macOS）
    ↓
实体输出

独立控制链路：统一发现 → 配对 → 会话 → 调音/状态 → 撤销
```

最终播放由实体输出的消费节奏驱动。UI、数据库、控制请求及网络到包事件都不决定音频输出时机。

### 2.1 模块职责

| 模块 | 职责 |
|---|---|
| 平台音频适配 | Windows/WASAPI、Linux/PipeWire、macOS/Core Audio 的采集、输出、设备事件和虚拟输出生命周期 |
| Discovery / Identity | 发现候选 Hub、配对、身份校验、凭证撤销 |
| Session Manager | 创建与关闭会话，版本协商、并发配额及恢复状态 |
| Media Adapter | RTP/RTCP、加解密、重排、解码；后续兼容协议独立接入 |
| Stream Processor | 时间线转换、漂移估计、重采样、短 PCM 队列 |
| Mixer / Output | 增益、Mute/Solo、混音、限幅、输出时钟及设备故障 |
| Control / Diagnostics | 权威状态、控制事件、统计、错误与诊断导出 |

共享核心通过平台接口访问采集、输出、设备枚举、凭证保护和后台生命周期，不直接依赖 Win32、PipeWire 或 Core Audio 类型。平台专用代码分别放入 Rust 适配模块，FFI 收敛在边界；Windows 必要的最小 C++ 驱动独立构建，macOS 优先验证 Rust 插件 ABI。选择 Rust 本身不能证明跨平台完成，具体语言边界见第三节。

每路输入必须交付带身份、时间位置和断点标记的音频块。单独传 PCM 字节不足以处理重连、长期漂移或后续同步。

### 2.2 最小数据模型

| 对象 | 含义 |
|---|---|
| Device | 已配对的设备身份，与 IP 无关 |
| Session | 一次连接与授权上下文，以 `session_id` 标识 |
| Stream | 一路声音；`stream_id` 与设备身份分开 |
| Bus | 一组输入的混音结果 |
| Output | 一个实际播放输出，绑定稳定设备标识 |
| Route | Stream 到 Bus、Bus 到 Output 的连接 |

v1 只实现固定的单 Bus、单 Output 路由，不建设通用音频图编辑器或插件平台。保留对象边界，不提前实现全部多房间功能。

内部音频块至少包含：

```text
stream_id / stream_epoch
source_sample_position / sample_rate / channel_layout
frame_count / discontinuity_flags / PCM
```

`source_sample_position` 表示本路源采样时间线的位置，不能以网络发送时刻替代；不同 Sender 的位置也不能直接相减。重连采用新 `session_id`；时间线重置或格式重建时更新 `stream_epoch`。会话管理器将这些字段绑定到媒体上下文，不要求全部塞进每个 RTP 包。

## 三、技术选型与语言决策

### 3.1 结论：Rust 主导，Python 辅助，C++ 限于必要例外

**NeonMix 的用户态产品代码以 Rust 为主：Sender、Hub、实时混音、时钟控制、网络会话、权限、配置和桌面 UI 均优先使用 Rust。Python 用于离线分析、测试编排及开发工具，不作为正式音频进程的运行时。C++ 不再作为应用主语言，只在已证明必要的平台模块中保留。**

项目难点是输出期限、跨设备时钟、系统虚拟设备和可分发性，不能仅按代码量或开发速度选择语言。Rust 能承担用户态实时音频，不需要先选 C++ 才能获得原生性能；但没有语言能未经实测就保证“完美完成”，Rust 的安全检查也不等于硬实时保证。

Windows、Linux、macOS 的完整首版支持优先于“全部代码必须纯 Rust”。尽量不自行编写 C++，不等于禁止调用现有 C ABI 或复用成熟原生库；否则会迫使项目重写媒体和密码组件，反而增加风险。

### 3.2 三种语言的项目适配

| 维度 | Rust | Python | C++ |
|---|---|---|---|
| 连续音频、混音、漂移控制 | 原生编译，无追踪式 GC；可预分配并控制实时路径，推荐 | 可调用原生库完成音频，但不宜让解释器执行期限敏感的逐块处理 | 能胜任，实时音频生态成熟，但本项目无须因此把整个核心写成 C++ |
| 并发、协议、设备状态 | 所有权和类型系统有助约束生命周期与数据竞争；仍需设计线程边界 | 原型和编排方便；实时工作仍需下沉到原生代码 | 能胜任，需更严格人工维护内存与并发约束 |
| 三端原生 API | Windows、PipeWire、Core Audio 可通过绑定/FFI 接入 | 通常依赖绑定或扩展；不能替代虚拟驱动/插件本身 | 平台样例丰富，适合确有必要的薄适配 |
| Windows 虚拟音频驱动 | 有开发工具，但不能据此认定生产级音频驱动路线已成熟 | 不作为内核音频驱动实现语言 | SysVAD 路线的现实交付基线，保留最小范围 |
| 管理界面与常驻程序 | egui/eframe 可作为三端 Rust GUI 基线 | 能做 UI，但增加解释器与绑定打包；若底层 Rust 已具备则收益有限 | Qt 可做，但不作为默认技术栈 |
| 测量、日志、离线分析 | 可做，但研究与分析迭代通常不如脚本方便 | 最适合承担工具角色 | 可做，没有优先采用的理由 |
| 工程复杂度 | Cargo 统一大部分产品代码，FFI 和原生依赖仍需单独管理 | 工具开发快，但不额外引入到用户安装路径 | 仅为驱动或不可替代桥接保留独立构建 |

这是一项基于工作负载与可用生态的工程判断，不是三种语言的通用排名，也不包含未经测量的性能倍数。若团队缺少 Rust 经验，应在 G0 计入学习和原型成本，不默认改语言就会缩短工期。

### 3.3 Rust 能覆盖什么，以及不能省掉什么

GStreamer 提供 Rust 绑定，现有 RTP/RTCP、Opus、抖动缓冲和 DTLS-SRTP 方案可保留，通过 `gstreamer-rs` 驱动，无需新增 C++ 媒体层。绑定仍依赖 GStreamer 和对应原生插件，不意味着得到无外部运行库的单个 Rust 可执行文件。[GStreamer Rust bindings](https://github.com/GStreamer/gstreamer-rs)

三端采集/输出适配保持独立：Windows 评估微软 `windows` crate 调用 WASAPI，Linux 使用 `pipewire-rs`，macOS 评估 Core Audio Rust 绑定及必要的 SDK FFI。共享核心不泄漏各端句柄或平台类型。[Rust for Windows](https://github.com/microsoft/windows-rs)、[PipeWire Rust bindings](https://pipewire.pages.freedesktop.org/pipewire-rs/pipewire/)、[Core Audio bindings](https://docs.rs/objc2-core-audio/latest/objc2_core_audio/)

CPAL 可用于音频 I/O 原型或满足要求的后端复用，但不能把“能打开声卡”当作“能创建系统虚拟声卡”。是否采用它取决于锁定版本能否提供所需设备位置、时间戳、热插拔及缓冲控制；不足时直接用平台接口，不叠加两套输出后端。[CPAL](https://github.com/RustAudio/cpal)

Rust 实时路径仍须禁止动态分配、阻塞锁、磁盘/网络等待和昂贵析构；特别避免最后一个 `Arc` 在回调中释放大型对象。设备回调或专用音频线程负责按期输出，Tokio 只承担非实时控制和后台任务。异步任务调度不能替代硬件消费节奏。

FFI 限定在平台和媒体适配模块，记录缓冲所有权、长度、线程约束与释放方式。Rust panic 不得越过原生 ABI，回调不能依赖 `unwrap` 或以 panic 作为错误处理；适用的恢复/终止策略按进程风险确定。安全 Rust 不能消除外部库或错误 `unsafe` 带来的风险。[Rust FFI](https://doc.rust-lang.org/nomicon/ffi.html)

### 3.4 Python 的边界

Python 适合音频测量结果分析、相关法延迟计算、日志汇总、故障测试编排、安装回归及实验脚本。这些任务可以通过 CLI、文件或测试 API 与 Rust 进程交互，避免为了工具脚本给音频核心增加 PyO3/嵌入式解释器。

Python 并非不能播放或处理音频；问题是本项目需要持续按期处理，并同时承受 UI、网络和设备事件。Python 音频接口同样要求回调不能分配、阻塞或执行耗时不确定的操作。即使计算下沉到 NumPy/原生库，也不能据此证明 Python 回调链路具有可控尾延迟。[python-sounddevice 回调约束](https://python-sounddevice.readthedocs.io/en/latest/api/streams.html)

不能简单用“Python 有 GIL”作为全部理由：CPython 已有可选 free-threaded 构建，但扩展兼容、对象管理、回调和调度问题仍需验证。对 NeonMix，选择 Rust 实现持续音频路径、Python 做非实时工具，工程边界更清楚。[Python free threading](https://docs.python.org/3/howto/free-threading-python.html)

用户安装 NeonMix 无需另装 Python，Python 工具停止或崩溃不得影响已运行的 Sender/Hub。

### 3.5 C++ 的保留范围与退出条件

**Windows 虚拟声卡是当前最有依据的 C++ 例外。** 微软 `windows-drivers-rs` 当前 README 仍明确注明处于早期、不建议生产使用；用户态 `windows` crate 可调用 WASAPI，并不代表同一方案已解决 PortCls/WaveRT 虚拟驱动。SysVAD 有现成 C++ 驱动结构，不能把它替换为 Rust 视作简单语法迁移。[windows-drivers-rs](https://github.com/microsoft/windows-drivers-rs)、[SysVAD](https://github.com/microsoft/Windows-driver-samples/blob/main/audio/sysvad/README.md)

Windows 路线按以下顺序评估：

1. G0 检查是否有满足音频、许可、再分发、稳定性及安全设置要求的现成驱动；满足时可由 Rust Sender 接入，减少自行维护的驱动代码。目前未选定此类产品，不能把它当作已解决依赖。
2. 若无合适方案，保留基于 WDK/SysVAD 的最小 C++ 驱动模块，独立构建与测试，只负责端点、时钟、缓冲及必要接口，业务、网络和编码全部留在 Rust。
3. 全 Rust Windows 驱动作为独立研究方向；只有完整音频功能、长期稳定性、签名分发和维护成本均有证据优于现有路线时才替换，不让首版交付依赖该研究成功。

macOS 不预设需要 Objective-C++。AudioServerPlugIn 是可通过 ABI 实现的插件接口，Apple 的最小示例使用 C；优先验证 Rust 导出所需 ABI、接口表、设备时钟和用户态桥接。现有 Core Audio 绑定是否覆盖完整插件接口必须逐项检查，必要时补 SDK 绑定或薄 C 桥接，不能把“有 Core Audio crate”当作完整虚拟驱动已存在。[Apple：Audio Server Driver Plug-in](https://developer.apple.com/documentation/coreaudio/creating-an-audio-server-driver-plug-in)

Linux 的 PipeWire 用户态虚拟 Sink 与输出优先 Rust 实现，不需要为此新写内核驱动。所有新增 C/C++ 模块必须在 ADR 写明缺失能力、替代方案、保留边界和测试方式；不能因为示例是 C++ 就把整个应用改回 C++。复用依赖中的 C/C++ 代码与自行开发产品代码分别统计，安全和许可要求同样适用。

### 3.6 修订后的技术基线

| 层次 | 首选 | 使用边界 |
|---|---|---|
| 主语言与公共核心 | Rust stable、Cargo workspace | Sender、Hub、会话、权限、混音和时钟；固定工具链与锁文件 |
| 媒体组件 | `gstreamer-rs` + GStreamer 原生插件 | 继续复用 RTP/Opus/DTLS-SRTP；三端打包插件和动态库 |
| 实时 DSP | Rust 自有轻量 Mixer/漂移控制器 | 预分配、有界队列、可控回调；不重写编解码器和加密算法 |
| 重采样 | `rubato` 异步 sinc 路线，G0 验证 | 动态微调比率、预分配处理接口；测群延迟、CPU、音质与漂移收敛 |
| Windows 用户态 | `windows` crate + WASAPI | 与虚拟驱动分开；不让原生句柄泄漏到公共核心 |
| Linux 接入 | `pipewire-rs` | 虚拟 Sink、原生输出、设备事件与会话恢复 |
| macOS 接入 | Rust Core Audio 绑定/FFI；Rust AudioServerPlugIn 原型 | 绑定缺口、ABI、插件桥接、权限及生命周期逐项验证 |
| Windows 虚拟设备 | 合适现成驱动优先评估；否则最小 SysVAD C++ 模块 | 唯一明确预留的自研 C++ 范围，不向上扩展业务 |
| 控制通道 | Tokio + Axum + rustls 集成 | HTTPS/WSS、TLS 1.3 和认证；不用于音频回调，不以 TLS 库替代 DTLS-SRTP |
| 桌面 UI | Rust `egui/eframe`，G0 通过后冻结 | 独立 UI 进程；验证中文输入、缩放、键盘/无障碍、托盘与三端打包 |
| 配置与凭证 | 原子 JSON + profile 相邻文件凭证 | .credentials 明文 JSON，按当前用户权限隔离；旧资料仅由独立工具离线迁移 |
| 测试与分析工具 | Python | 测量、离线分析、编排；不进入正式音频进程 |
| 构建与发布 | Cargo + 三端 CI + 平台安装工程 | WDK 与插件/原生依赖单独处理；CMake 仅在依赖或局部桥接确需时使用 |

`rubato` 支持可变比率异步重采样及预分配实时处理，适合本项目漂移补偿的候选需求；它不是漂移估计器。“异步重采样”指采样时钟不锁定，不是 Tokio async。选定版本的接口与延迟必须实测；固定比率 FFT 重采样不能直接替代持续微调。SpeexDSP 保留为验证不达标时的备选，而非同时维护两套默认路径。[rubato](https://github.com/HEnquist/rubato)

`egui/eframe` 支持原生 Windows、Linux、macOS 应用，可作为调音、设备列表、状态和诊断的 Rust UI 基线。选择它可减少自研语言与绑定层，但不保证原生系统外观或所有辅助功能自动达标，平台托盘和后台生命周期仍需适配；G0 用真实界面流程验证。[eframe](https://docs.rs/eframe/latest/eframe/)

控制服务使用 Axum/Tokio 配合 TLS 集成，TLS 证书绑定与首次信任仍由项目实现。DTLS-SRTP 继续由媒体栈承担，不因换用 rustls 而取消或重复实现。[Axum](https://github.com/tokio-rs/axum)、[rustls](https://github.com/rustls/rustls)

### 3.7 保留的组件取舍

- **WebRTC：**有成熟团队经验时可作为替代媒体栈；首版只选一套，不能仅因语言偏好重写协议组件。
- **Qt/QML、PySide：**不再是默认 UI。Python 包装 Qt 仍引入 Qt 原生依赖和额外打包层，不能据此宣称移除了 C++ 依赖。只有 Rust UI 的明确功能缺口经原型证实后才重新评估。
- **BlackHole：**可作 macOS 接入对照；许可与再分发先确认，不作为未经说明的外部手工依赖。[BlackHole](https://github.com/ExistentialAudio/BlackHole)
- **Snapcast：**仍是后续多房间复用候选，不能代替首版系统虚拟输出。[Snapcast](https://raw.githubusercontent.com/snapcast/snapcast/develop/README.md)

GStreamer、Rust crates、原生插件和平台组件分别锁定版本并审核许可。Rust 主导不等于纯 Rust 依赖树，也不免除原生运行库的分发义务。[GStreamer Licensing](https://gstreamer.freedesktop.org/documentation/frequently-asked-questions/licensing.html)

## 四、三端音频接入与分发

### 4.1 Windows：分两步验证

**回录技术验证：**应用 → 真实输出端点 → WASAPI loopback → Sender。用于验证双路传输、混音和时钟，不视为系统 Speaker 交付。本机仍可能播放，不能假设“系统静音后远端一定仍有声”。

**虚拟 Speaker 验证：**应用 → 虚拟 Render Endpoint → 用户态 Sender。优先验证对该虚拟端点进行 WASAPI loopback；利用音频引擎回录能力，可能避免自定义内核数据接口，但虚拟端点仍须由驱动提供并正确推进时钟。

```text
应用 → Windows 音频引擎 → 虚拟 Render Endpoint
                              ↓
                       WASAPI loopback → Sender
```

桥接方式属于待验证方案，不是微软对任意自定义驱动的兼容保证。测试连续播放、静音、无活动应用、格式切换、Sender 不在线、休眠恢复和系统音量。只有该路径不能满足需求时，才增加受访问控制保护的专用 PCM 接口。

### 4.2 Windows 驱动职责与设备身份

按第三节保留的 Windows C++ 驱动仅负责端点、格式、时钟、缓冲和生命周期；网络、Opus、配对、TLS、重连与控制全部放在 Rust 用户态。Hub 离线或发送队列满时，虚拟端点仍按正常时钟运行，不能等待网络而阻塞系统音频引擎。

首版一个 Sender 只添加一个房间对应的虚拟输出，绑定 `hub_id / output_id`。IP 或房间名称变化不创建新端点，断网也不删除端点。未来多房间端点由用户主动添加，不能由广播自动安装。

默认输出由用户在 Windows 设置中选择，不依赖未公开接口静默切换。应用级捕获、多个虚拟端点和单机同时承担 Sender/Hub 留待后续，避免首版引入音频回路及复杂系统路由。

### 4.3 Windows 分发条件

驱动样例能加载、测试签名可用和正式可分发是不同里程碑。账户、证书、提交路线及目标系统测试从第一阶段推进。微软当前文档要求相关提交账户关联有效 EV 证书；具体提交签名要求按发布时文档执行。[Microsoft：Driver code signing requirements](https://learn.microsoft.com/en-us/windows-hardware/drivers/dashboard/code-signing-reqs)

Attestation 被微软列为测试场景签名路线，不等于 Windows 认证，也不能用于面向零售用户的 Windows Update 发布。正式版按适用的 HLK/WHCP 路线规划；不同分发渠道的条件分别确认，不把 Windows Update 的限制扩大为全部渠道的统一结论。[Microsoft：Driver Signing Options](https://learn.microsoft.com/en-us/windows-hardware/drivers/dashboard/driver-signing-offerings)、[Attestation Signing](https://learn.microsoft.com/en-us/windows-hardware/drivers/dashboard/code-signing-attestation)

Windows 普通用户 Beta 和正式版必须在目标系统正常启用 Secure Boot、内存完整性的环境下通过安装验证，不以关闭安全功能作为使用条件。

### 4.4 Linux：PipeWire 虚拟 Sink 与输出后端

NeonMix Rust Sender 通过 PipeWire 绑定创建可被桌面音频设置和普通应用选中的虚拟 Sink，将其音频送入原生 Sender；Hub 通过 PipeWire 输出到用户指定的实体设备。PipeWire loopback 可用于虚拟 Sink/Source 原型，但路由与用户态数据读取仍需实际实现和验证。[PipeWire：Loopback](https://docs.pipewire.org/page_module_loopback.html)

首版锁定至少一个 PipeWire 桌面发行版及会话管理器组合，验证 Sink 身份、应用选路、格式协商、音量、无应用活动、PipeWire/会话管理器重启和设备拔插。重新登录后恢复用户允许的虚拟输出，正常运行不要求 root。仅有 PulseAudio 或 ALSA 的环境不自动纳入首版支持。

交付至少一种适用于受支持发行版的可安装包，明确系统依赖、用户级启动方式、升级及卸载清理；不能把一组手工命令当作最终安装体验。

### 4.5 macOS：Core Audio 虚拟设备与输出后端

NeonMix Sender 通过 Core Audio 虚拟音频设备提供系统可选输出，优先验证 Rust 实现 AudioServerPlugIn ABI 的路线和到用户态 Rust Sender 的有界数据桥接；Hub 使用 Core Audio 输出到指定实体设备。Apple 提供创建虚拟音频设备的 Audio Server Driver Plug-in 示例，但它不包含完整网络 Sender，桥接、时钟及生命周期仍需实现。[Apple：Creating an Audio Server Driver Plug-in](https://developer.apple.com/documentation/coreaudio/creating-an-audio-server-driver-plug-in)

macOS 首版不能以 AirPlay 或只捕获声音的 API 替代系统可选 NeonMix 输出。BlackHole 可作原型对照，若用于正式分发须先确认许可，不把外部手工安装作为未说明的依赖。

虚拟设备插件只承担设备、时钟及有界数据交换，网络与编解码留在 Sender。验证 Apple Silicon、支持的 macOS 版本、音量/Mute、音频服务重启、休眠、设备拔插及安装后重启要求。完成应用、插件和安装包适用的 Developer ID 签名、公证及 Gatekeeper 验证，不要求用户关闭 SIP 或 Gatekeeper。[Apple：Notarizing macOS software](https://developer.apple.com/documentation/security/notarizing-macos-software-before-distribution)

三端统一绑定 `hub_id / output_id`，均由用户明确添加房间并选择系统输出；网络与房间名称变化不重建身份。具体系统权限提示可以不同，但角色、混音、停止发送和恢复行为保持一致。

## 五、实时音频：时间、缓冲与调度

### 5.1 内部格式与输出节奏

内部统一为 48 kHz、立体声、float32。逻辑块可取 480 帧，即 10 ms，但设备一次请求的帧数可能不同；输出适配层处理实际请求，不能用 `sleep(10ms)` 驱动播放。

各平台实际输出格式可能不是 48 kHz 或立体声（Windows 需检查共享模式混音格式）。输出后端查询并记录格式，在支持范围内转换；输出帧计数按实际采样率换算到内部时间线，不能直接当作 48 kHz 帧位置使用。首版选择明确的受测设备，无法适配时给出错误。

### 5.2 每路独立处理

```text
SRTP 验证与解密 → RTP 重排/抖动缓冲 → Opus 解码/丢包掩蔽
    → 格式统一/长期速率补偿 → 有界短 PCM 队列 → Mixer 按输出需要取数
```

每路独立维护包序号、扩展时间戳、解码器、缓冲目标、漂移估计和统计。A 的网络异常只能使 A 降级，Mixer 不等待 A 后再播放 B。

MVP 从固定目标缓冲起步，例如 40～60 ms，具体档位经测量确定。先实现容量上限、迟到丢弃、欠载掩蔽和恢复，再考虑自适应缓冲。延迟突增时不能靠无限积压维持连接。

GStreamer 的 `rtpbin` 内部已使用 `rtpjitterbuffer`，不要无意再串联第二个抖动缓冲。组件默认值也不是产品的低延迟设置，应显式配置等待期限、丢包事件及溢出策略。[GStreamer：rtpjitterbuffer](https://gstreamer.freedesktop.org/documentation/rtpmanager/rtpjitterbuffer.html)

### 5.3 GStreamer 与自有 Mixer 的边界

媒体工作线程从 GStreamer 取出解码结果，保留 PTS、持续时长和断点信息，再送入预分配队列。最终输出线程不能阻塞等待 `appsink`。

原型列出每层队列容量、单位、满/空策略及实际停留时间，包括 `appsink`。其数量、字节、时间限制和丢弃策略按锁定版本配置，不能假设在线最新版的所有属性都存在。[GStreamer：appsink](https://gstreamer.freedesktop.org/documentation/app/appsink.html)

记录管线时钟、base-time、PTS 到输出时间线的映射及 sink 同步设置。是否启用某层时钟等待由完整链路决定，不能叠加“抖动缓冲 + sink 等待 + Mixer 长队列”，也不能为降低延迟把所有等待盲目关闭。验收应能解释每段延迟。

### 5.4 长期时钟漂移

两台声称 48 kHz 的设备仍可能有实际速率差。相对误差 100 ppm 时，每秒差 4.8 帧，一分钟累积约 6 ms。只做短时间播放会漏掉缓冲耗尽或积压。

单房间以 Hub 实际输出消费速率为基准，对每路进行小幅连续的异步重采样。漂移估计综合源采样位置、输出消费位置和长期队列水位，并对短期网络抖动滤波。

控制器需要调整幅度与变化速率上限、启动收敛、断点重置和异常退回策略；允许范围通过注入正负时钟偏差验证。抖动缓冲处理网络到达变化，重采样处理长期采样速率差，不能让两套控制器反复修正同一误差。

### 5.5 实时线程约束

输出线程仅做有界数据读取、必要的固定成本处理、增益、混音、限幅与输出提交。网络读取、数据库、磁盘日志、UI、对象创建销毁和长时间等待的锁放在非实时上下文。

采用预分配缓冲与有界队列；单生产者/单消费者场景可用 SPSC 环形队列。控制状态在音频块边界应用，移除流后的对象回收在非实时线程完成，避免最后一次引用释放触发昂贵析构。

## 六、混音与控制语义

首版输出可表示为：

```text
y[n] = Limiter(master_gain × Σ(active_i × gain_i × x_i[n]))
```

`x_i` 已完成格式和时钟适配；`active_i` 由 Mute/Solo 规则决定，实际增益切换使用短渐变。

| 操作 | 约定 |
|---|---|
| 输入音量 | 仅调整该路，内部采用明确的 dB/线性映射 |
| Mute | 不输出该路，但继续消费和推进时间线；取消后播放当前声音 |
| Solo | 无 Solo 时播放所有未静音流；存在 Solo 时只播放被 Solo 且未静音的流，允许多个 Solo |
| Disconnect | 终止会话并释放资源；区别用户停止、管理员断开与网络故障 |
| 房间总控 | 仅影响本产品 Mixer 输出，不代表控制 Hub 上其他应用的声音 |
| 新流加入 | 不因连接数增加而将全部音量除以 N |
| 恢复播放 | 清掉过期音频，在允许恢复的状态下安全淡入 |

Solo 会有意改变其他路的可听状态，验收应要求“符合 Solo 规则，其他路时间线持续推进”，而不是其他路音量完全不变。多路相加需保留混音余量，最终输出前限幅并显示过载；数字限幅不构成对任意功放或音箱的物理安全保证。

源设备/应用音量、Hub 输入音量、房间总音量分别定义。系统端点音量或协议音量已应用到 PCM 时，不再重复乘同一增益。测试静音、音量零点、恢复和端点音量回调的实际行为。

首版显示“设备名 / 系统音频”。Hub 无法从已混合 PCM 可靠拆回 Spotify、浏览器和游戏；应用标签需要可信元数据，独立调音需要独立捕获流。Ducking 及优先级分类推迟，不加入首版控制模型。

## 七、媒体传输、发现与安全

### 7.1 原生媒体参数

| 参数 | 初始设置 |
|---|---|
| 编码 | Opus，48 kHz，立体声 |
| 码率 | 初始目标 192 kbit/s，按质量、负载与网络测试调整 |
| 包长 | 默认 10 ms；后续可评估 20 ms 稳定模式 |
| 传输 | RTP/RTCP over UDP 单播，SRTP/SRTCP 保护 |
| 发包 | 按音频节奏发送，队列有界；载荷适应目标路径 MTU |
| 接收 | 包大小、并发流、解码资源和缓冲均有限额 |

Opus RTP 时间戳使用 48 kHz 时钟，10 ms 增量为 480，双声道不乘二。UDP 不提供拥塞控制，应用必须根据反馈调整发送策略。[RFC 7587](https://www.rfc-editor.org/rfc/rfc7587.html)

可保留 PCM 调试模式，但不建设第二套对外协议。48 kHz、双声道、16 bit PCM 的纯音频速率为 1.536 Mbit/s；10 ms 数据为 1,920 字节，不能沿用单个小 UDP 包的封装而依赖 IP 分片。

首版不做音频重传，以按期限丢弃、Opus 丢包掩蔽及必要的平滑静音为基础。需连通“丢包事件 → depayloader → 解码器”的处理链并检查统计，安装 `opusdec` 不等于掩蔽已生效。带内 FEC 取决于编码模式、发送配置和实际码流，也可能增加等待，不作为首版音乐丢包恢复保证。[GStreamer：opusdec](https://gstreamer.freedesktop.org/documentation/opus/opusdec.html)

反馈至少包含丢包、迟到、队列水位和接收状态。持续拥塞时减小码率或暂停问题流并提示，不无限增加缓存。RTP 序号/时间戳回绕、旧会话包及格式断点必须测试。

### 7.2 自动发现

采用 mDNS + DNS-SD，原型服务类型可为 `_neonmix._tcp.local.`；正式使用时确认命名及注册要求。发布稳定 Hub ID、名称、协议版本、控制端口和必要能力，不放凭证。

发现结果只是候选设备，名称、TXT 记录、IP 和 SSRC 都不是认证依据。mDNS 主要在本地链路工作；访客网络、客户端隔离、多子网或防火墙可能导致“同一 Wi-Fi 但不可发现”。支持已配对地址更新和高级手工地址入口，正常流程以发现为主。[RFC 6762](https://www.rfc-editor.org/rfc/rfc6762.html)、[RFC 6763](https://www.rfc-editor.org/rfc/rfc6763.html)

### 7.3 配对、媒体身份与撤销

首次配对须建立用户可验证的身份绑定。优先验证高熵一次性信息或携带公钥指纹的二维码；桌面设备不能假设都有摄像头，需提供等价流程。短数字码应使用成熟安全配对机制，不能直接作为长期令牌。

已配对连接验证保存的公钥身份；未知密钥变化要求重新确认。HTTPS 不替代首次信任建立，不能通过长期关闭证书校验处理自签名证书。

媒体默认评估 DTLS-SRTP，控制通道提供经过认证的会话参数及对端证书指纹，媒体连接校验一致后才允许进入解码/播放链。GStreamer 提供 DTLS/SRTP 能力及对端证书信息，家庭成员身份和授权仍由项目检查。[GStreamer：dtlsdec](https://gstreamer.freedesktop.org/documentation/dtls/dtlsdec.html)

每次重新建立媒体安全上下文使用新协商密钥，明确会话与 SSRC 绑定及密钥失效时间；不能只改 `stream_epoch` 就认为旧媒体包失效，也不能复用旧密钥却随意重置 SRTP 包计数。使用成熟库的认证与重放保护。[RFC 3711](https://www.rfc-editor.org/rfc/rfc3711.html)

权限以设备为首版主体：成员调整自己的输入音量、Mute 和停止发送；Solo 会影响其他输入的可听状态，须具备房间控制权限。房间控制者可调整房间总控及其他输入，管理员管理配对与撤销。权限由服务端认证上下文推导，不接受客户端自报角色。撤销时立即终止该设备活动会话并拒绝重连；管理员断开后明确重新允许条件，防止 Sender 自动抢回会话。

默认不录音、不持久化 PCM、不主动上传诊断、不做公网端口映射。日志排除密钥和令牌，导出前告知内容；认证前也限制握手、连接与包处理资源。

## 八、延迟、状态与使用体验

### 8.1 延迟定位

端到端延迟包含采集/桥接、成帧、编码前瞻、网络、抖动缓冲、解码/重采样、输出缓冲及外部设备处理。网络 Ping 很低不代表声音延迟很低。

首版先保证一个可解释的平衡配置，初始目标为受控有线环境 P95 ≤120 ms、受控 Wi-Fi 环境 P95 ≤180 ms，测量口径和确认时点见开发计划。暂不保留缺少原型依据的 40～80 ms 交互模式目标，更多模式由实测和用户需求决定。

只掌握音频的 Hub 不能保证任意浏览器或播放器同步延迟视频。虚拟设备报告延迟也不等于所有应用补偿动态网络延迟。竞技游戏、节奏游戏、乐器监听需单独验证。

蓝牙及电视/功放内部处理增加的延迟单独测量，不纳入有线/USB 基线。界面中的缓冲估计、网络往返时间和实测端到端延迟分别命名，不把 RTT/2 显示为已测声音延迟。

### 8.2 首次使用与状态

Hub 流程：命名房间 → 选择实体输出 → 低音量测试音 → 配对设置 → 开始共享。输出绑定稳定设备标识，不随系统默认输出变化而转到耳机或未知设备。

Sender 流程：发现房间 → 配对 → 添加虚拟输出 → 用户在系统设置选择 → 后续认证连接。持续显示发送范围、目标房间、输入电平和停止发送入口。

Mixer 每路显示设备/流名称、电平、音量、Mute/Solo 及连接/网络状态；房间显示输出设备、总音量、电平与限幅。状态至少区分无输入、缓冲、播放、网络降级、静音、重连、管理员断开和输出丢失。

诊断先区分“没有收到音频”“收到静音样本”“输出不可用”。这些状态不能直接推断音源应用、内容类型或授权限制，只显示可证实原因及排查建议。

### 8.3 控制接口草案

E04 当前原型采用版本化统一命令与增量事件接口，实验字段、错误、权限、信任记录/撤销和幂等规则见 [媒体与控制合约](docs/MEDIA-CONTROL-CONTRACT.md)；工作包完成度见 [审计](docs/E02-E04-AUDIT.md)，下表保留为原始草案。

```text
GET    /v1/hub
GET    /v1/outputs
GET    /v1/devices
GET    /v1/streams
POST   /v1/sessions
DELETE /v1/sessions/{id}
PATCH  /v1/streams/{id}/mix
PATCH  /v1/outputs/{id}/mix
GET    /v1/events                 # WebSocket 升级
```

示例修改体：

```json
{
  "gain_db": -6.0,
  "muted": false,
  "solo": false,
  "expected_revision": 42
}
```

Hub 是状态权威。修改请求执行权限校验与版本冲突检查，事件带状态版本；重连先取快照，避免漏事件后显示过期状态。配对、撤销和管理员重新允许播放的接口需在协议设计时补齐，上表不是完整 API 合约。

设备别名、配对关系和音量偏好可持久化；RTP 状态、缓冲、瞬时电平及临时连接不写成配置。数据模型稳定后再冻结接口，两份文档不重复维护端点表。

### 8.4 进程与恢复

Rust 音频核心不依赖 egui/eframe 等 UI 对象。早期 CLI 原型可合并非 UI 功能；三端在 Beta 前均采用登录用户下的独立音频进程与管理 UI，通过有权限检查的本地 IPC 连接。这样才能兑现 UI 崩溃不停止音频，仅隐藏窗口到托盘不足以提供崩溃隔离。

“关闭窗口”保持后台播放；“停止发送”停止该 Sender 音频；“退出后台”结束该实例的音频与连接。用户停止或管理员禁止的状态不能被重连覆盖。首版不要求系统服务，也不承诺注销后继续播放。

断网恢复清掉旧积压并建立当前时间线。实体输出丢失时暂停该输出并提示，不切到未知设备；输入队列继续有界，重开后更新输出时钟映射并淡入。恢复矩阵及测试见开发计划。

## 九、后续扩展的进入条件

### 9.1 首版之外的扩展

Windows、Linux、macOS 的原生 Sender/Hub 属于 v1 必选范围，按第四节执行。后续扩展仅指移动端、浏览器控制台、额外 CPU 架构/系统版本以及多房间能力。

| 扩展 | 候选路线 | 开发前需要证明 |
|---|---|---|
| iPhone / iPad | 独立 AirPlay 接收适配器 | 接收平台、并发、时间信息交接与商业发布条件 |
| Android | AudioPlaybackCapture + 用户授权 | 应用边界、权限撤销、后台运行与设备兼容 |
| 浏览器 | 远程控制台 | 认证、角色与移动交互；不当作系统音频捕获方案 |

Android 播放捕获受用户授权、用户配置文件、音频用途和源应用捕获策略限制，不能承诺任意应用的全局虚拟输出。[Android：Capture video and audio playback](https://developer.android.com/media/platform/av-capture)

### 9.2 AirPlay 独立验证

Shairport Sync 可作为原型候选，先在其主要支持的 Linux/BSD 环境验证。它可输出到 pipe/stdout，但后续 Mixer 和设备的实际播放时刻不再完全由接收器控制；裸 PCM 管道不能自动保留完整同步能力。[Shairport Sync](https://github.com/mikebrady/shairport-sync)

初始扩展目标限于“一路 AirPlay 与原生输入共同混音”。不承诺多个 iPhone 选择同一目标后并发混音，也不将 AirPlay 延迟套用原生指标。验证时间戳交接、额外缓冲、音量是否重复应用、后到发送者的打断行为。

Apple 将 AirPlay audio 列入 MFi 技术范围，具体产品形态的授权、认证及商标条件需独立确认；开源实现能运行不等于官方认证。[Apple：MFi Program](https://mfi.apple.com/en/home.html)

### 9.3 多房间先路由，后同步

多房间路由允许不同房间播放不同内容；同步播放需要统一组时钟、样本到播放时刻的映射、输出延迟补偿与持续漂移修正，不能只多发一份 UDP 包。

扩展结构为 Source → Routing/Mixing → Program Bus → 定时分发 → 各房间 Renderer。同一节目尽量混音、编码一次，Coordinator 所在房间也遵守组时间，不能提前本地播放。

先在有线及受控设备上测量房间间偏差，再扩展 Wi-Fi。慢房间可退出同步组，不能无限拉高全屋延迟；任意蓝牙、电视或功放组合不承诺固定亚毫秒同步。复用 Snapcast 与自建分发的取舍留到该阶段，以同一测量方案比较。
