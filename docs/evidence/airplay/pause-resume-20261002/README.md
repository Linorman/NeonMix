# 暂停恢复兼容调查（2026-10-02，macOS）

存在可复现、可修正的接收端问题；目前没有证据证明 iOS 27 与本项目整体协议不兼容，也尚未证明这些修订消除了用户 iPhone 上的实际断开。候选改动保持现有引擎与配对，优先修正媒体生命周期。

## 当前方案与上游依据

项目固定 UxPlay v1.73.7 / df67c212a433cf6dda3676dd40c097900d24e645，加纯音频和准入补丁，由独立 worker 解密/解码；Hub 负责媒体 epoch、低延迟映射、Mixer 和 CoreAudio。当前使用 44.1kHz ALAC 等编码、NTP/UDP 的 type 96 音频，不声明 buffered audio / PTP / 镜像。

- [UxPlay #539](https://github.com/FDH2/UxPlay/issues/539) 报告了 iOS 27 恢复时同 codec 重新 SETUP、旧时间锚点导致无声，并给出重新建立管线的修复建议。它涉及上游的镜像/音频 renderer；本项目不编译该 renderer、appsink 不按 PTS 等待，不能直接套用其补丁或宣称为相同根因。
- [v1.73.7 发行说明](https://github.com/FDH2/UxPlay/releases/tag/v1.73.7) 已包含 iOS 27 TEARDOWN 行为修正。当前源码保留该修订，泛泛升级版本不足以证明暂停恢复问题已解决。
- [Shairport Sync 5.5.2](https://github.com/mikebrady/shairport-sync/releases/tag/5.5.2) 的 OS 27 修正针对 buffered 音频中的未知数据块；本项目当前 profile 不走该数据流，不能视作可直接移植的修复。它可作为日后 buffered/PTP 引擎评估候选，需要新的 profile、认证、IPC 时钟适配与实机验收。

## 本地证据与问题边界

`observed-before.json` 记录调查开始时的接收器状态：287 个迟到拒包、接收器未崩溃、最后有成功应答的 TEARDOWN 与对端关闭。当前来源名称不能确认为 iPhone；历史环形记录也没有完整的暂停起点，故只能证明接收端出现过迟到，不能把这份历史快照归因于用户描述的某次手机操作。POST `other` 的 64-byte body 长度本身也不能识别路由；新增固定 audio_mode/rate_anchor/set_rate/set_property 路由类别，仍不记录原始 body 或任意 URL。

数字源复现得到两个独立缺陷：

1. [baseline-idle-resume.json](baseline-idle-resume.json)：先以 2 秒协议提前量播放，暂停 2 秒，再以 250ms 提前量恢复且不发送 FLUSH。Hub 保留了约 1.82 秒旧提前量，634 个恢复包被拒为迟到，控制连接仍在，音频未恢复。
2. [baseline-resume-setup.json](baseline-resume-setup.json)：同连接、同 codec 恢复 SETUP 得到 200，但 dataPort/controlPort 为零。上游 `raop_rtp_start_audio` 在 reader 已运行时直接返回，没有填充新的端口输出。

数字源是合成签名/PIN/FairPlay 接缝、NTP 与加密 ALAC，使用真实 worker、Hub、Mixer/CoreAudio。它用于检验可疑生命周期，不冒充 Apple 客户端或证明特定 iOS 行为。

最终 53 项相关测试与严格 Clippy 通过。五次同连接重新 SETUP 的 [final-five-resume-setups.json](final-five-resume-setups.json) 每次均恢复非零电平和音频释放、媒体重建计数依次 1–5，未重新配对，迟到/时间倒退/定时跳帧为零。短暂停、FLUSH、同步模式与预缓冲均有对应最终报告；汇总见 [summary.json](summary.json)。

当前实例更新时，Hub 进程停在 macOS `SecKeychainFindGenericPassword` 读取已有密钥，尚未启动 AirPlay worker；Hub/管理员/接收器三个条目均存在。未重建房间身份或清除已有配对，需先完成系统钥匙串访问，再做真实手机复测。诊断见 [startup-observation.json](startup-observation.json)。

## 候选修正

- 按 RTP 与 PTS 的相对位移检测协议时间断点，偏差超过 50ms 时暂停 PCM 并申请媒体重建；Hub 授予新 epoch 后计算新提前量。最终实现不按“无音频 250ms”触发，避免误伤正常启动预缓冲或把暂停当断网。
- 重复音频 SETUP 先 join 旧 RTP reader，然后重建 decoder/UDP 端口；清除旧 RTP/NTP 同步点，保持 NTP owner、配对及 RTSP 控制连接。
- FLUSH 增加有界 enum reason；旧无 reason 消息兼容，伪造 context 与任意 reason 拒绝。诊断增加媒体重建次数和类别，认证/第二来源/撤销规则不放宽。

`fixed-*` 报告含迭代时制品哈希；`final-*` 为最终媒体断点方案，`fixed-prefetch.json` 验证最终实现未误重建正常 96 包预缓冲。报告里的制品 SHA256、清理结果及实际场景分别可查。当前数字回归通过后仍须 iPhone 的原 App 做连续暂停/恢复，捕获受控媒体重建、迟到包和协议关闭记录；若仍失败，才依据真实消息选择是否补充特定恢复方法或扩大引擎支持。

## macOS 复现

```sh
tools/dev python3 tools/airplay_mixer_probe.py --profile release --codec alac --native-start-order hot --pause-seconds 2 --resume-reset none --report .local/tmp/pause-no-flush.json
tools/dev python3 tools/airplay_mixer_probe.py --profile release --codec alac --native-start-order hot --pause-seconds 2 --resume-reset setup --report .local/tmp/pause-setup.json
tools/dev python3 tools/airplay_mixer_probe.py --profile release --codec alac --native-start-order hot --pause-seconds 0.1 --resume-reset none --report .local/tmp/pause-brief.json
```

探针检查暂停时控制连接仍授权、恢复保持 source id、释放音频恢复、电平非零、无迟到/定时跳帧、1+1 原生混音以及 worker 故障隔离。临时进程、fixture 与测试 Keychain 条目均由各次运行清理；所有下载、缓存、临时文件和制品在项目内。没有运行 Windows/Linux，也没有录制 PCM 或配对原始包。
