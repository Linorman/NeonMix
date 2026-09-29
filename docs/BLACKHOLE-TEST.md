# macOS BlackHole E01 验收

2026-09-29，BlackHole 2ch 0.7.1，设备UID为`coreaudio:BlackHole2ch_UID`。用户安装驱动并授权重启音频服务后，系统成功枚举设备。测试显式指定虚拟输入/输出，不改变默认输出，不采集实体麦克风，不保存PCM。

## 最终结果

| 场景 | 证据与结果 |
|---|---|
| 48kHz桥接 | 437Hz源读回436.9944Hz，峰值0.01584893，与−36dBFS测试源一致 |
| 44.1kHz桥接 | 437Hz源读回436.9959Hz，保留44.1k输入格式；输出完成48k到44.1k转换 |
| 96kHz桥接 | 437Hz源读回436.9976Hz，保留96k输入格式；输出完成48k到96k转换 |
| 无声源 | 仍收到静音回调，所有帧均为零；no_data_intervals为0，正确区分静音与无回调 |
| 暂停/恢复 | 暂停期间记录无数据；恢复首块带RESUMED标志，新epoch，源相对位置从0开始 |
| 系统设备Mute | 通过Core Audio设置BlackHole输出端Mute；输入实际PCM变为零，解除后恢复非零，设备原始Mute值最终恢复 |
| 声源停止/重启 | 437Hz源停止后连续读到静音；659Hz新源启动后恢复非零，正确识别新频率，没有drop/stale帧 |

7组场景全部通过，接受的采集与输出均未报告原生错误或回调超时。三档桥接的原始设备时钟递增，换算到48kHz的帧位置符合整数比率。最终设备采样率恢复为48kHz。详细证据：[桥接结果](evidence/macos-blackhole/bridge-summary.json)、[生命周期结果](evidence/macos-blackhole/lifecycle/result.json)、[最终状态](evidence/macos-blackhole/final-state.json)。

## 本次发现与修复

最初三档采集全零，且原生流没有报告错误。设备音量为1、Mute为0；AVFoundation权限状态为Denied，TCC日志将NeonMix归属于启动它的宿主应用。用户开启麦克风权限后，同一二进制无需修改音频通路就读回了正确测试音。虚拟输入同样受Microphone权限限制，旧Itour全零记录缺少此前提，不能作为其驱动不兼容的结论。

macOS适配器现通过AVAudioApplication.recordPermission在打开采集前检查权限；未获准时返回明确的PermissionDenied提示。API仅在控制线程查询，不请求或修改系统授权。该接口在macOS 14可用，低于14.6部署目标。底层原生调用被限制在macOS适配器内，共享核心不依赖平台类型。

停源重启测试还发现频率诊断将静音间隔连在两段音调之间，曾给出347.38Hz。修复后连续50ms低于门限会结束当前信号分段，累计帧数和RMS仍包含真实静音，最长有效段的频率恢复正确。加入437Hz→3秒静音→659Hz回归测试。旧失败原始日志保留在`artifacts/blackhole/lifecycle/20260929-205633/`，没有放宽断言。

最终格式、Clippy、30项自动测试、release构建与离线模拟全部通过；硬件测试默认忽略的同进程重开用例不混入30项计数。[检查记录](evidence/checks.json)

## 复现

确认启动程序的宿主应用或终端已获“系统设置 → 隐私与安全性 → 麦克风”权限，然后运行：

```sh
tools/dev python3 tools/probe.py --release --device coreaudio:BlackHole2ch_UID --rate 48000
tools/dev python3 tools/probe.py --release --device coreaudio:BlackHole2ch_UID --rate 44100
tools/dev python3 tools/probe.py --release --device coreaudio:BlackHole2ch_UID --rate 96000
tools/dev python3 tools/macos_runtime_probe.py --device coreaudio:BlackHole2ch_UID
```

生命周期脚本会短暂调整所选虚拟设备的输出Mute，并在正常结束或异常时恢复原值；输入和输出固定为48kHz、256帧。它不操作其他设备或默认路由，日志位于项目`artifacts/blackhole/`。

本记录证明本机数字桥接和所列生命周期行为；不提供模拟端延迟、长期稳定性或其他macOS版本的保证。BlackHole是测试依赖，NeonMix自研插件及签名分发仍分别属于E06/E09。

## 5分钟稳定性基线与UI进程隔离

48kHz、256帧周期下连续采集300秒，共14400256帧；437Hz测试源的最终频率估计为436.999975Hz。采集/输出的原生错误、回调超时、丢帧、过期帧及无数据间隔均为0。输入只出现最初START断点，epoch始终为1；输出原始位置与时间戳逐次递增，epoch未变化。

排除启动观察窗口后，每秒观察区间中的最大精确零样本比例约0.0021%，符合正弦波过零点的少量零值；探针门槛为0.1%，没有出现整段非预期静音。期间启动诊断UI并对其发送SIGTERM，音频进程仍存活且数据连续。这验证UI进程终止隔离，不等于正常关窗、键盘或辅助功能验收。

该结果是300秒数字通路基线，不扩展为长时间稳定性、跨设备漂移或模拟端延迟保证。[完整结果](evidence/macos-blackhole/soak-300s/result.json)、[UI隔离记录](evidence/macos-blackhole/soak-300s/ui-isolation.json)。复现：

```sh
tools/dev python3 tools/soak_probe.py --device coreaudio:BlackHole2ch_UID --seconds 300
```

## 音频服务重启恢复：已通过

已增加受控入口：先确认输入/输出都在静音运行，再通过系统管理员窗口请求重启coreaudiod；成功后要求旧流明确报错退出、同一UID重新出现、重开的数字桥接通过。若授权晚于测试流退出，命令会先检查原进程是否存活，避免在测试结束后再执行重启。

首次授权等待30秒超时，测试流已清理，coreaudiod仍为原进程；该次没有记录为通过，也不据此判定音频后端有缺陷。[未完成记录](evidence/macos-blackhole/service-recovery-awaiting-authorization/result.json)。再次执行应在操作人员准备好处理系统窗口时进行；当前入口默认给予60秒授权时间：

```sh
tools/dev python3 tools/macos_service_recovery_probe.py --device coreaudio:BlackHole2ch_UID --restart-coreaudio
```

用户准备好系统管理员验证后复测成功：采集和输出均在服务重启后报告DeviceNotAvailable（错误码1），各以退出码1结束；同一个BlackHole UID重新枚举成功，新进程读回436.996833Hz测试音，峰值0.01584893，采集与输出错误和回调超时均为0。这验证服务失效可见、显式重开后桥接恢复，不代表CLI会自动重连。[成功记录](evidence/macos-blackhole/service-recovery/result.json)。
