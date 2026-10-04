# ADR-010：Windows E06 最小 WaveRT render 驱动

状态：源码已收敛，macOS 交叉编译/链接与 Windows x64 MSVC/WDK 原生构建已通过；系统加载与音频运行验收未执行。2026-10-01。

设计要求优先评估现成驱动。当前没有已取得产品再分发许可的现成二进制；第三方驱动不进入 NeonMix 安装包。采用微软官方 [Simple Audio Sample](https://github.com/microsoft/Windows-driver-samples/tree/main/audio/simpleaudiosample)，它是从 SysVAD 收敛出的最小 WDM/WaveRT 框架，不把完整多端点/offload/关键词/APO 示例带入产品。

这个内核模块保留 C++，因为 PortCls/WaveRT、WDF 迷你端口及微软样例 ABI 是当前可审查的实现起点。边界严格限于单个 render endpoint、格式、合成时钟、DMA 缓冲、PnP 和电源生命周期；网络、认证、编码及 Mixer 仍为 Rust。代码及衍生修改使用原仓库 MS-PL，版权及许可保留；仓库 commit、原始文件哈希与 SDK 包 SHA256 均已锁定。

删除采集端、测试音、PCM 文件写入和可重新启用写入的注册表路径。虚拟端点没有真实放大器，删除样例“硬件音量/Mute”节点，使用 Windows 软件音量；后续须在 Windows 对 WASAPI loopback 实测其作用点，不能凭源代码推断不会重复或遗漏增益。

稳定硬件 ID 为 `ROOT\NeonMixAudio`，驱动服务及 KS reference strings 固定。首次安装/更新必须只保留一个 root devnode；不能用重复执行 sample `devcon install` 代替幂等安装。安装、签名、公证/认证和正常安全设置下的加载属于尚未通过的发布与平台门槛，当前脚本不修改安全设置。

完整源码在 `drivers/windows/wavert/Source`，差异在 `patches/windows-simpleaudio-neonmix.patch`；接口、上限和编译证据边界见 [驱动合约](../../drivers/windows/wavert/DRIVER-CONTRACT.md)。2026-10-01 Windows Server 2022 / VS 2022 x64 MSBuild 使用原始 WDK 头文件通过编译、链接、INF 验证与 Inf2Cat。补齐自有 `.sys` 打包项，删除零采集端点的不可达安装路径，并显式选取 x64 SDK 工具；没有绕过检查或降低警告门槛。Windows 音频与 Driver Verifier/HLK/HVCI、签名及系统加载仍待验收。
