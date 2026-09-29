# macOS 耳机实测记录

日期：2026-09-29。设备：External Headphones，稳定标识 `coreaudio:BuiltInHeadphoneOutputDevice`。通过Core Audio显式打开所选端点，没有修改系统默认输出路由。所有原始日志、缓存与构建产物均在项目目录内。

## 结果

| 项目 | 结果与证据 |
|---|---|
| 设备能力 | 双声道f32，枚举支持44.1/48/96kHz，周期范围15～4096帧；[设备清单](evidence/macos-headphones/devices-before.json) |
| 左右声道听感 | 48kHz、256帧、437Hz、−36dBFS；先左3秒、间隔1秒、再右3秒。用户确认“先左后右，清晰且没有杂音”；[确认记录](evidence/macos-headphones/user-confirmation.json) |
| 应用Mute/恢复 | 48kHz、511帧、−42dBFS，1秒Mute、3秒恢复；静音期间输出帧和时钟继续推进，恢复后出现非零样本；[原始日志](evidence/macos-headphones/mute-resume.jsonl)、[检查](evidence/macos-headphones/summary.json) |
| 采样率/周期切换 | 修复后48k/511、44.1k/127、96k/1024、44.1k/256、48k/257五组均通过，错误和callback_over_budget均为0，原始时钟递增且48k帧位置换算正确；[检查](evidence/macos-headphones/fixed-summary.json) |
| 运行中被改率 | 一个48k静音流运行时，在同设备打开96k静音流；原流报告StreamInvalidated（错误码7）、退出码1，新流正常；[故障日志](evidence/macos-headphones/external-rate-original.jsonl) |
| 实体拔插 | 用户拔下耳机、间隔后重插；watch记录removed后added，原流报告DeviceNotAvailable（错误码1）、退出码1；[设备事件](evidence/macos-headphones/hotplug/watch.jsonl)、[结果](evidence/macos-headphones/hotplug/result.json) |
| 同ID重开 | 重插后新进程显式打开同一UID，48k/256输出145664帧，569次回调，错误和回调超时均为0；[重开日志](evidence/macos-headphones/hotplug/reopened.jsonl) |
| 同进程重开 | 同一进程关闭并重开3次，epoch依次为1/2/3，每次首次native_stream_frames=0、submitted_frames=256，旧读端保持关闭时的快照，均无错误或回调超时；[原生测试记录](evidence/macos-headphones/same-process-reopen/result.json) |
| 最终构建 | 格式、Clippy、29项workspace测试、release构建、离线模拟全部通过；最终二进制48k静音输出通过，之后左右听感和拔插使用同一最终二进制；[检查](evidence/checks.json)、[二进制指纹](evidence/macos-headphones/final-build.json) |

## 发现与修改

第一轮44.1k/127帧在首次音频回调前发生StreamInvalidated；[失败日志](evidence/macos-headphones/44100.jsonl)保留。随后未改代码的三次重试成功，说明它不是必现故障。旧日志没有原生错误文本或属性通知时序，无法据此完全确认根因。

检查发现CPAL的Core Audio监听原先只要收到采样率通知就判定流失效。现改为读取实际nominal rate：与流配置相符时继续，不符或查询失败时明确报错；监听注册后再次核对以覆盖配置期间的变化。该防护处理可能迟到的自身改率通知，同时保留真实外部改率的故障报告，native I/O revision更新为3。补丁及SHA256保存在`patches/`和`vendor/cpal-provenance.json`。

第一次临时检查脚本将退出前最后一次回调的peak当作全程峰值，误判三个音调场景。peak实际是最近回调值，正常停流淡出后为0；修正为检查运行中快照及累计非静音帧数后，三组均通过。原始JSONL未修改，最初检查结果保存在`artifacts/headphones/summary-initial-checker.json`。

## 复现与边界

```sh
tools/dev target/release/neonmix-audio play --device coreaudio:BuiltInHeadphoneOutputDevice --rate 44100 --period 127 --seconds 3 --signal silence
tools/dev python3 tools/hotplug_probe.py --device coreaudio:BuiltInHeadphoneOutputDevice --seconds 180
```

拔插脚本等待READY后才操作，拔下至少3秒再接回。它只播放静音，设备失效后不会回退到其他端点；保存原流故障并在相同ID重新出现后开启新流。

上述记录证明本机耳机端点所测范围内的输出、主观听感和生命周期。拔插脚本重开使用独立进程，两个进程都从epoch=1开始；另以显式硬件测试在同一进程内验证了3次正常关闭/重开的epoch与位置重置。这是两个不同场景，尚未把拔插和进程内重开合为同一用例。首次失败与成功记录同时保留；短测没有给出模拟端延迟、长时间稳定性、USB设备、系统休眠或其他系统的保证。应用Mute不等于系统Mute，本次没有修改系统音量。结束时耳机保持连接，nominal rate恢复为最初的48kHz。

Windows原生运行、Linux实体音频、macOS非零虚拟桥接和远端CI等未完成项继续见[总状态](STATUS.md)。已有Linux运行结果和开发包来自revision 2，本次修改仅影响macOS监听及CLI版本标识。

## 同进程重开测试入口

```sh
tools/dev env NEONMIX_TEST_OUTPUT_DEVICE=coreaudio:BuiltInHeadphoneOutputDevice cargo test --release -p neonmix-audio --test output_reopen --locked -- --ignored --nocapture --test-threads=1
```

该测试只输出静音，必须显式提供稳定设备ID；常规CI中默认ignore，不会打开机器的默认音频设备。本机原生测试通过1项，原有29项自动检查的计数保持不变。新增测试同时通过Clippy与格式检查；Windows路径仅交叉类型检查，未声称运行。
