# ADR-009：E06 虚拟输出的设备、桥接与生命周期

状态：macOS 自研 HAL 与指定 Ubuntu 桌面功能已实测；三端 E06 尚未全部验收。

## 身份与边界

虚拟输出是本机音频系统对象，身份不绑定房间名、Hub IP 或会话 UUID。Sender 明确选择这一对象的可读一侧；媒体加密、网络发送和 Hub 增益均在用户态。虚拟设备时钟由本机音频引擎推进，网络离线不使系统应用的 render 调用等待。Sender 停止后不得自动恢复广播；系统端点的存在不意味着正在发送。

| 平台 | 设备身份 | 写入与读取 | 生命周期 |
|---|---|---|---|
| macOS | Core Audio UID `com.neonmix.audio.virtual-output` | `WriteMix` 后的双声道 PCM 写入插件内有界、逐帧时间戳标记的缓冲；`ReadInput` 按 sample time 读取，缺口填零 | `.driver` 由 Core Audio 管理；Sender 只按 UID 打开输入，不控制设备加载 |
| Linux | PipeWire `node.name=neonmix.sink.default` | 桌面应用选择 Sink；原生 PipeWire sink capture 由现有 I/O 适配接入 Sender | 独立本地 owner 持有 proxy，Sender 停止/断网不删除节点；节点移除或服务器重启后有界退避重建 |
| Windows | `ROOT\NeonMixAudio` / 固定 KS reference strings | WaveRT render 驱动源码及 Rust WASAPI shared-mode loopback；Windows 运行未验证 | OS 管理已安装端点，独立于 Sender；安装/签名仍待实现 |

macOS 固定 48 kHz、stereo、float32。Core Audio 对其他应用格式协商/转换，Sender 的现有采集转换最终输出 48 kHz Opus。插件本身不执行重采样、网络、文件 I/O 或同步等待。`GetZeroTimeStamp` 使用固定 `DeviceSpec` 采样率，避免实时回调中的锁；这个对上游依赖的本地修订见 `patches/tympan-aspl-0.1.0-realtime-clock.md`。

插件环形缓冲为 48000 帧，按 sample time 取模。每帧有独立的原时间标记：写前标记失效、写入左右样本、写后发布标记；读端前后检查标记，未写、过期、并发覆盖或新 StartIO 都返回静音。音频数据回调不分配内存、不加锁、不执行系统调用；时钟回调使用预计算的 mach timebase 并读取单调时钟。`tympan-aspl` 自带环形缓冲仍被接口层使用，但读端输出以 NeonMix 的带标记缓冲为准。

## 平台选择

Windows 已收敛微软简化 SysVAD 的单 render 驱动源码及项目内构建入口，明确软件端点音量并去掉采集/文件写入。源码许可、稳定硬件 ID、编译证据和运行门槛见 [ADR-010](ADR-010-windows-render-driver.md)。

Linux 由 `neonmix-audio virtual-output --state-directory <private-directory>` 独立持有固定节点，和网络 Sender 生命周期分开。每个 UID 的独占 owner 标记与文件锁防止重复进程，退出/崩溃释放所有权；原生 core/proxy 错误、节点移除及已观察到的 WirePlumber 主 client 移除会通知 owner，在 250ms 到 5 秒退避中重建。节点声明自身驱动时钟，Sender 只读取明确 ID。Mac 上的锁测试和 Linux 交叉类型检查通过；指定 Ubuntu ARM64 桌面已通过音量、绑定、服务重启/节点移除及数字重开，注销与其他发行版仍待验收。

macOS 明确选择的 BlackHole 开发提供者已验证系统选路、普通应用、音量/Mute、无源及 Sender 生命周期；不打包第三方驱动。自研 Rust CFPlugIn 已补齐标准主音量/Mute、多客户端生命周期和 SDK ABI，项目内 bundle 完整 ad-hoc 签名通过，按用户授权安装后实际系统加载、数字桥接、名称与服务恢复通过。本机开发加载不能替代正式签名、公证与 E09 分发。详细证据和缺口见 [E06](../E06.md)。

2026-09-30 补充：HAL 的本地依赖 revision 4 删除可变回调别名，改为共享 Sync 回调、实时有界原子入口及控制线程排空；时钟读取得到同一版本的 anchor/seed。实际 `.driver` 的 Apple CFPlugIn 宿主加载通过，仍待 coreaudiod 系统加载。

Linux 本地 owner 改用本地 `pw_impl_node` adapter，经 `pw_core_export` 导出到用户会话；`pw_impl_node_update_properties` 原地修改 description/nick，改名保持 `node.name` 与对象。服务重建保持逻辑身份，允许运行时 global ID/object.serial 变化。FFI 控制对象只在所属 main loop 创建、修改和销毁，先移除 listener、销毁 export，再销毁 impl-node；context/core 引用最后释放。源依据是锁定 PipeWire 1.0.5 的 [pw-cli export-node](https://github.com/PipeWire/pipewire/blob/1.0.5/src/tools/pw-cli.c)、[native node export](https://github.com/PipeWire/pipewire/blob/1.0.5/src/modules/module-client-node/remote-node.c) 与 [adapter factory](https://github.com/PipeWire/pipewire/blob/1.0.5/src/modules/module-adapter.c)。同用户名称 IPC、绑定恢复与退避不进入音频数据回调。export readiness 同时等待 bound/bound_props 与 Core roundtrip，不依赖事件顺序。macOS 交叉检查和指定 Ubuntu 桌面原生运行均通过。

Windows 原生选择新增不可变包标记；名称更新用公开 property store，显式管理操作需要管理员权限，普通 Sender 不自动提升。不会静默修改系统默认输出。安装/运行证据依旧单独验收。
