# Core Audio 虚拟输出（E06）

当前开发入口使用已安装的 BlackHole：`neonmix-hub send --virtual-output --virtual-output-provider blackhole`。它按精确 UID `coreaudio:BlackHole2ch_UID` 选择 stereo input side，日志明确记录 external_driver；不会把别的设备当作缺失端点的替代。普通应用在系统设置选择 BlackHole 2ch。真实默认输出/afplay、双应用混音、音量/Mute、无源、停止发送及改率后新会话均已验证，复现和证据见 [E06](../../docs/E06.md)。

`hal/` 是 NeonMix 自研 Rust AudioServerPlugIn，固定 UID `com.neonmix.audio.virtual-output`，48 kHz stereo float32。系统混音后的 WriteMix 写入插件内逐帧带时间标记的有界环形缓冲；ReadInput 读取同时间位置，无源、覆盖或新时间线返回静音。标准主音量/Mute 控件、属性通知、多客户端启动/停止与非法输入验证已经实现，网络和编码全部留在 Sender。

依赖采用本地修订的 `tympan-aspl 0.1.0`。上游有错误的 I/O 操作码、缩减的 I/O 周期结构和不完整的接口表；本地版本按本机 Apple SDK 修正并通过独立布局/常量对照。细项见 [本地修订](../../patches/tympan-aspl-0.1.0-realtime-clock.md)。数据与时钟回调 1000 周期测得零分配、零重分配、零释放。

`tools/build_macos_hal.sh` 生成项目内 `artifacts/macos/NeonMixHAL.driver`，加入依赖许可证并完成整个 bundle 的 ad-hoc 签名、strict 校验、arm64 和工厂符号检查。它不需要 Developer ID 身份，**尚未系统安装或测试加载**。此前把上游 CI 的拒绝直接套用为本机不可加载结论不成立。BlackHole 的实测也不能当作自研插件加载证据。

自研插件的系统加载/控件界面、音频服务重启、休眠、额外应用兼容及 E09 安装/公证/卸载仍待完成。BlackHole 仅作为用户明确指定的开发提供者，不随 NeonMix 分发。

自研设备的标准 Name 属性已可写；绑定标签通过 `output sync-name` 或绑定 Sender 启动同步到已加载设备。CFString 校验、UID/时钟/音频不变的离线 ABI 测试通过，尚无系统加载后的改名证据。BlackHole 原生名称保持厂商定义。

本轮补充共享 Sync 回调、控制线程排空、整对立体声原子发布、版本化时钟与异常客户端退出；实际 bundle 的 Apple CFPlugIn/SDK 宿主验证见 `tools/macos_hal_bundle_probe.py`。系统安装入口 `tools/install_macos_hal.sh install|uninstall` 只操作自有 bundle、不自动重启音频服务；安装副本的目录例外已经由用户明确授权，用户已完成安装副本，签名与项目文件哈希一致；当前仍等待服务重启与系统加载测试。

真实系统设备/默认输出/afplay、双应用混音、音量一次、Mute、无源、Sender 离线仍可用及加密发送已通过。标准 Name 对客户端只读；跨进程改名使用通过 `cust` 声明 CFString 的 `nmna` setter，通知标准 Name，实际绑定改名、撤销与服务恢复复验通过。实机休眠暂缓，未记通过。默认签名为 ad-hoc；`NEONMIX_HAL_SIGN_IDENTITY` 可使用已有正式身份，当前无有效签名身份，不能把开发安装视为正式发布。
