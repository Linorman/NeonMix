# 自动换曲无声修复（2026-10-02，macOS）

用户报告 iPhone 上 Spotify 自动切换下一首后无声，暂停再播放可恢复。已复现并修复一个与该症状吻合的定时 Mixer 缺陷；真实 iPhone/Spotify 连续换曲仍须实听确认。

## 根因与修复

AirPlay 来源可以在曲间空档继续推进 RTP/PTS，不发送 FLUSH 或重新 SETUP。两者的时钟映射保持一致，因此已有 `timestamp_jump` 检测不会申请新 epoch。Hub ingress 的内部 PCM 位置只累计实际解码帧，而定时 Mixer 在缺少 PCM 时仍推进播放游标。下一曲 PCM 到达后，旧游标已领先数秒，±1000 ppm 的连续漂移修正无法及时追上，导致有效音频持续被跳过。暂停/播放触发媒体重置时会清除此状态。

修复在定时 lane 已耗尽 PCM 后接到新块时，清除旧 SRC 游标、相位与残留 FIR 状态，按新块的呈现时间重新开始，并沿用已有 5ms 淡入。未来音频仍等待 PTS，真正过期的采样仍丢弃；持续有数据时保持原有连续 sinc SRC。没有增加按无音频超时断开连接的逻辑，也不要求重新配对或重建协议会话。

## 证据

- 新增 `timed_audio_recovers_after_a_track_gap_without_an_epoch_change`，旧逻辑在下一曲第一个块失败。修复后通过，并补充未来 PTS 等待、过期音频不重放检查。
- [旧版本数字复现](baseline-gap.json)：真实 Hub/worker/CoreAudio 接收 677 块、释放 669 块，连接仍 active，但恢复后输出电平达不到验收条件。输入继续被接收，未出现时间线倒退；它不是 Apple 客户端的抓包。
- [修复后连续五次空档](fixed-five-gaps.json)：加密 ALAC、44.1kHz、每包 352 源帧；每次输出 RMS 恢复约 0.084–0.086，`media_resets=0`、`timed_late_frames=0`，无需 FLUSH/SETUP 或新 epoch。原生来源混音和 worker 故障隔离同时通过。
- [音画同步空档](fixed-sync-gap.json)、[既有暂停恢复](fixed-pause-resume.json) 和 [96 包预缓冲](fixed-prefetch.json) 均通过，定时跳帧均为零。
- 长空档后 GStreamer resampler 会释放一个带旧 PTS 的残留块，Hub 正确将其丢弃。因此五次空档共计 5 个迟到拒包；恢复稳态没有新增拒包，不能把这项结果表述为全程零迟到包。探针对每个边界最多允许这一块，其他拒绝或持续迟到均失败。
- 61 项 core/adapter/IPC 常规测试及额外两分钟加速定时质量测试通过；相关 core/adapter/IPC/Hub 严格 Clippy、workspace 格式检查和 Hub release 构建通过。

当前手动测试实例已更新 Hub 并重启，接收器就绪、输出帧持续推进，receiver key 与配对/信任列表在更新前后保持一致；[替换记录](replacement.json) 包含旧、新制品 SHA256。实际 iPhone/Spotify 复测仍待用户确认。

## 复现

```sh
tools/dev cargo test -p neonmix-core -p neonmix-airplay-adapter -p neonmix-airplay-ipc --release
tools/dev cargo test -p neonmix-core --release --test timed_quality -- --ignored
tools/dev python3 tools/airplay_mixer_probe.py --profile release --codec alac --native-start-order hot --pause-seconds 2 --pause-cycles 5 --track-gap --report .local/tmp/track-gaps.json
```

所有数字探针使用临时文件凭证、合成认证来源和 BlackHole/CoreAudio 输出，没有录制歌曲或 PCM，结束后移除自有 fixture 和进程。没有运行 Windows/Linux，也不将本机数字验证视为真实 Spotify 客户端的完成验收。
