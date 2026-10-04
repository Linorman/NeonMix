# NeonMix 实现状态

## 安装包后台启动与退出修复（2026-10-04）

Windows 以内置 Administrator 或提升权限运行时，启动器用默认安全属性创建的 `%LOCALAPPDATA%\NeonMix` 属主为 `BUILTIN\Administrators`，后台按“目录必须属于当前用户 SID”拒绝启动，界面因此列不出设备。启动器现以当前用户 SID 为属主、私有 DACL 建目录，并把此前提升运行留下的 Administrators 属主目录改回当前用户；其他属主仍拒绝。Windows x64 节点用安装副本复验：属主修复后后台启动，`devices` 返回 9 个端点，`shutdown` 后后台退出。后台不可达时“退出后台”现直接关闭界面，不再留下隐藏窗口进程。macOS“开始共享”报“后台操作失败”系旧手测 Hub 占用 7443；端口占用现明确提示，`HubStart` 等待 Hub 启动或退出后再返回结果。

更新：2026-09-29。按用户最新指示，本轮聚焦macOS构建与E00/E01验收；Windows/Linux保留已有实现和证据，暂不作为本轮阻断。**这不等于三端G0、G1或v1完成。**

## 已实现

| 范围 | 实现 |
|---|---|
| E00 单仓基础 | Cargo workspace、固定Rust 1.95.0/Cargo.lock、独立平台crate、Git仓库、项目内缓存/临时目录/产物入口 |
| E00 应用与自动化 | 独立音频CLI、egui诊断程序、三端CI定义、Python独立锁环境、构建/依赖/源码SHA256清单和符号归档 |
| E01 统一音频 | 固定容量音频块、源位置与epoch/断点、sine/impulse/silence、Mute推进、44.1/48/96kHz转换及Mono映射 |
| E01 原生I/O | WASAPI render endpoint loopback / Core Audio虚拟设备input side / native PipeWire sink monitor，三端实体输出 |
| E01 设备与时钟 | 稳定ID、格式和周期查询、原生回调、设备变化/故障、原始设备帧位置、48k换算、重开/回退重置 |
| E01 实时与诊断 | 预分配SPSC、溢出代际清理/过期丢弃、暂停恢复首块保留、静音和无数据分开、错误码与回调统计 |

CPAL 0.18.2采用仓库内扩展（native I/O revision 3），公开WASAPI IAudioClock、HAL sampleTime、PipeWire ticks，并修复PipeWire节点移除、空设备列表和SPA输入chunk处理。补丁、原版SHA256和Apache-2.0许可证在 `patches/`、`vendor/`；不是声称原版CPAL已经提供所有能力。原始坐标、归零/换算后的设备位置与播放延迟推算值分开记录。

## 已完成验证

- `cargo fmt --all -- --check`、workspace Clippy `-D warnings`、**30项测试**、workspace release build、离线探针全部通过；[检查记录](evidence/checks.json)。
- Windows x64 GNU目标和Linux x64目标的**完整workspace交叉检查**通过，包含音频程序与Rust UI；[Windows记录](evidence/windows-workspace-check.log)、[Linux记录](evidence/linux-workspace-check.log)。这是条件编译/类型检查，不是MSVC原生链接或Linux运行证据。
- Linux新增原生运行证据：Ubuntu 24.04 x86_64／PipeWire 1.0.5／WirePlumber 0.4.17，在项目内QEMU TCG环境通过23项核心/I/O测试和9组数字运行场景，包括44.1/48kHz、Mono下混、系统Mute、暂停恢复、设备移除和健康空列表；[运行证据](evidence/linux-runtime.json)。实际ELF已链接运行；不作为实体声卡验收。
- macOS arm64／本机SDK 27.0：实体输出44.1kHz、127帧与48kHz、511帧真实回调通过，原始sampleTime有效、递增，错误数为0；[44.1k记录](evidence/macos-output-44100.jsonl)、[48k记录](evidence/macos-output-48000.jsonl)。短测不能代替长期稳定性或模拟端延迟验收。
- 新接入External Headphones：44.1/48/96kHz、127/256/257/511/1024帧的输出回调与时钟换算已有实测，静音/恢复的样本计数正常；用户确认左右声道正确且无杂音。实体拔插时原流报设备不可用并退出，同ID重连输出正常；运行中外部改率会使原流明确失败。首次44.1k启动失败已保留，修复后的5次切换均通过。另在同进程连续关闭/重开3次，验证epoch递增、位置归零及旧快照隔离。详见 [耳机测试](HEADPHONE-TEST.md)。
- macOS已有Itour虚拟设备：静音采集、暂停、恢复、无数据统计与epoch切换通过，恢复首块位置为0且带RESUMED标记；[采集记录](evidence/macos-capture.jsonl)。
- macOS egui界面成功启动并查看截图；中文字体、真实设备列表可见。4种状态有headless绘制测试，设计/静态审计无错误；完整辅助功能和另外两端GUI实测待补。
- steady-state采集、队列与sinc输出分配计数为0，未发生重分配或释放；此结论仅覆盖项目测试路径，不扩展为第三方内部或系统硬实时保证。
- Core Audio服务重启恢复通过：原采集/输出均以设备不可用报错退出；同UID重开后非零数字桥接恢复，无错误或回调超时。[恢复记录](evidence/macos-blackhole/service-recovery/result.json)。
- BlackHole新增300秒数字通路基线：48k/256帧、14400256输入帧，错误/回调超时/drop/stale均为0，时钟连续；期间终止诊断UI未中断音频进程。[稳定性记录](evidence/macos-blackhole/soak-300s/result.json)。这是5分钟基线，不是长期稳定性承诺。

## 验收补充与剩余事项

| 项目 | 当前结果与下一步 |
|---|---|
| macOS数字虚拟桥接闭环 | BlackHole 2ch 0.7.1已读回非零测试音，44.1/48/96kHz桥接通过；本轮细项见[BlackHole验收](BLACKHOLE-TEST.md)。此前Itour全零记录缺少授权前提检查，不能据此认定驱动不兼容 |
| Windows / Linux实体音频 | Windows目前只有交叉检查；Linux已通过虚拟机数字路径，但缺少实体声卡/USB和实际输出测量。仍需对应专用机器，虚拟机不替代实体设备验收 |
| macOS USB/服务重启/休眠 | BlackHole已补44.1k输入；耳机接口实体拔插、运行中服务重启故障报告及同UID重开桥接均通过。休眠唤醒仍需受控测试；USB属于额外设备覆盖 |
| 原生CI | 三端workflow已写好，仓库未配置远程地址，尚未运行GitHub Actions；macOS arm64硬门槛和原生报告归档均在配置中 |
| 依赖/驱动成品 | GStreamer属E02集成；Windows驱动/macOS插件属E06，签名安装属E09；未交付伪驱动或关闭系统安全的安装流程 |

首次debug小周期探针曾收到一次原生错误；本轮在关闭虚拟机后立即进行的release短周期测试也报告过一次Xrun。没有足够证据确定根因，失败记录保留。其后空闲条件下的6次静音短测（44.1/48kHz、127/128/257/511/960帧）均通过，不能据此宣称长期稳定。故障事件现带完整统计和时钟快照，便于后续定位。

新增一致的输出时钟快照接口：原始帧位置与同次观测时间戳、采样率、实际回调帧数一起发布，独立于预测播放时间；并发读取不会组合两次回调的数据。零分配测试也覆盖这一路径。

## 本轮修复与实验边界

- BlackHole首次全零的明确阻断是宿主应用的麦克风权限被拒绝；授权后同一二进制三档桥接通过。macOS后端现加入原生权限预检查，权限不足时明确失败，不用全零回调表示有效采集。
- 频率诊断将连续50ms低于门限的样本作为信号分段，避免把停源静音计入两次测试音的频率跨度；新增437Hz→静音→659Hz回归测试。

- Core Audio采样率通知现在读取实际值，只有与流配置不匹配或查询失败时才报告失效；监听注册后再次核对，覆盖配置到监听之间的变化。首次44.1k失败的精确时序无法从旧日志恢复，因此不把后续短测成功当作根因已经完全证实。Windows/Linux已有运行证据属于revision 2；revision 3只修改macOS监听与CLI版本标识。

- 32块采集队列可装下完整8192帧回调，仍独立执行100ms过期清理；不会因单个合法大回调的尾块使整批数据失效。
- 流主动析构不再计作设备丢失；实际移除所选PipeWire节点仍会失败，且不会切换到其他设备。
- PipeWire枚举排除应用流；健康空列表与服务失败分开。SPA chunk检查范围/对齐、支持环绕及EMPTY/CORRUPTED标记，流失效后不再调用数据回调。
- epoch统一分配，重开不会复用暂停/断点重置用过的编号；耗尽时明确停止，不能回绕。
- 桥接探针检查测试音频率、RMS、原始设备时钟和回调预算，不能仅凭非零数据判通过；Mono测试分别检查左声道的1/2幅度和反相抵消。
- 并发重型编译时，TCG曾发生超过周期的回调，造成一轮44100Hz探针失败。失败日志保留在artifacts/linux-vm，空闲条件下重测通过。新增callback_over_budget；它与原生Xrun分开，不能把errors=0等同于无欠载。

## 使用与维护

启动与测试命令见 [README](../README.md)，完整验收步骤见 [ACCEPTANCE](ACCEPTANCE.md)，音频字段/容量/错误码见 [AUDIO-CONTRACT](AUDIO-CONTRACT.md)。

`.local/`保存可复用下载与工具缓存，`target/`保存编译及中间产物，`artifacts/`保存运行日志、UI截图、二进制与符号包，全部位于项目内并被Git忽略；已选取的长期证据进入`docs/evidence/`。逐项完成审计见 [E00-E01-AUDIT](E00-E01-AUDIT.md)。当前没有创建提交或远程仓库。Linux实验环境与复现入口见 [LINUX-LAB](LINUX-LAB.md)。
