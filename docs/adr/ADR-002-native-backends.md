# ADR-002：三端原生后端复用与虚拟设备边界

状态：用户态实现已接通；原始设备帧计数已实现；macOS已用BlackHole验证数字桥接，三端完整实机验收尚未完成。

使用锁定CPAL 0.18.2，内部复用Windows windows-rs/WASAPI、Linux pipewire-rs、macOS Core Audio绑定，核心无平台类型。Linux明确选择PipeWire，不降级至ALSA/PulseAudio。选择依据是该版本新增native PipeWire、稳定ID、实际period查询与设备timestamp；不把“打开声卡”当作创建虚拟驱动。

- Windows：对明确选择的render endpoint建立事件驱动loopback，虚拟端点与实验实体端点走同一采集桥接。驱动创建、签名和分发在E06/E09。
- Linux：CPAL对sink设置 `stream.capture.sink=true`，固定 `target.object` 并使用DONT_RECONNECT。独立 `sink` 命令通过pipewire-rs创建support.null-audio-sink，保持proxy生命周期，room slug形成node.name，退出移除；这是可选择的临时实验Sink，不是E06持久配置。
- macOS：通过稳定UID打开已安装虚拟设备的input side；不使用process tap/AirPlay作为系统虚拟设备替代。HAL插件/受控桥接仍在E06。

CPAL原版不公开原始帧计数，因此保留 `vendor/cpal` 和可审查的 `patches/cpal-0.18.2-native-position.patch`：WASAPI复用同一次IAudioClock读取的position、Core Audio读取已存在回调的有效sampleTime、PipeWire复用pw_time.ticks/rate。原始位置读取没有增加音频数据回调内的原生调用、分配、锁或平行后端。不能把未修改的CPAL 0.18.2当作具备该接口。原始平台坐标和播放时刻估计分别报告；无效坐标是None。上游升级时需重新核对或移除此补丁。

设备活跃状态变化由原生错误回调报告，闲置枚举列表由控制线程1Hz检测。

本次macOS既有Itour虚拟设备能够提供静音采集及暂停恢复，但写入相同UID后没有读回非零信号，因此不视为虚拟桥接通过，也不作为本产品分发依赖。

参考：[CPAL 0.18.2 source](https://docs.rs/crate/cpal/0.18.2/source/)、[PipeWire Rust bindings](https://pipewire.pages.freedesktop.org/pipewire-rs/pipewire/)。精确依赖以Cargo.lock为准。

Linux运行实验补充：23项核心/I/O测试与9组PipeWire数字场景在Ubuntu x86_64虚拟机通过，见docs/evidence/linux-runtime.json。运行实验推动了主动析构/设备移除的区分、健康空列表、应用流过滤、SPA缓冲范围和静音标志修复。原始接口扩展不再是唯一补丁；vendor/README.md和补丁文件是当前完整变更清单。虚拟机没有作为实体输出验证替代。

2026-09-29耳机实测后，native I/O revision 3加强Core Audio采样率通知处理：通知回调查询实际nominal rate，与流配置一致时继续工作；不一致或读取失败则停止流。注册监听后也核对一次，避免漏掉配置期间的变化。查询发生于属性通知/控制路径，不在音频数据回调中。设备运行中被另一静音流改率的实测仍触发StreamInvalidated；首次启动失败与后续通过记录见[耳机测试](../HEADPHONE-TEST.md)。Apple接口约定参考[AudioObjectAddPropertyListener](https://developer.apple.com/documentation/coreaudio/audioobjectaddpropertylistener(_:_:_:_:))。

现有Itour两个UID的成对复验：Output端写入/Input端读取，以及Input端写入/Output端读取，两组采集和输出退出码均为0，但各240128帧全部静音，峰值和RMS均为0。两组都未通过桥接；不根据设备名称推断它们已经组成可用环回。原始记录见docs/evidence/macos-paired-virtual-probe/。

BlackHole 2ch 0.7.1补验：安装后重启音频服务才出现设备；最初48/44.1/96k采集均全零。通过AVFoundation状态及TCC归属日志确认宿主应用麦克风权限被拒绝，授权后同一二进制三档桥接均读到437Hz测试音。因此此前Itour全零记录缺少权限前提，不作为其驱动不兼容的结论。macOS适配器新增AVAudioApplication.recordPermission预检查（macOS 14起可用，低于14.6部署目标），仅在控制线程查询，不改变权限，不打开默认麦克风。见[BlackHole验收](../BLACKHOLE-TEST.md)。
