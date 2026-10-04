# E06 完成度审计

依据：完整开发计划 4.6、设计方案 4.1–4.5；用户最初限定 macOS，后来明确授权指定 Ubuntu 主机测试。这里保留三端完整目标，不以 BlackHole 或交叉编译替代成品验收。

| 要求 | 当前证据 | 结论 |
|---|---|---|
| 三端系统可选虚拟输出 | macOS BlackHole 与自研 NeonMix HAL 系统枚举、默认输出选路及 afplay 实测；Ubuntu 桌面普通 pw-play、API 枚举与操作者确认系统列表；Windows WaveRT 源码及 PE 交叉链接 | macOS 与指定 Ubuntu 通过；Windows 系统列表仍缺运行证据 |
| 稳定身份，不因房间名/IP/网络变化重建 | 固定原生身份及共享 hub_id/output_id 持久绑定；真实 BlackHole 重开、改名、Hub 地址变化、停用/重新绑定探针 | 持久绑定和本地名称已实现；自研标准 Name 跨进程写入被拒绝，已修订为 nmna CFString setter；SDK 宿主及修订实际安装/跨进程名称同步、绑定撤销与恢复通过，Ubuntu 原地名称与服务重建稳定逻辑身份实测通过；Windows 原生名称源码与 macOS 交叉检查通过，Windows 身份/改名实测仍待补 |
| 端点声明与真实格式能力一致 | macOS 自研实际 48k stereo f32/44.1k 应用转换与拒绝不支持设备率；BlackHole 44.1/48k 实测；Windows 单 48k stereo PCM16 声明与编译；Ubuntu 实际 48k stereo f32 和 44.1k PCM16 应用转换 | Windows 真实格式协商未验证 |
| 系统时钟独立于 Sender/Hub | BlackHole 与自研 HAL 无源、Sender 停止、普通应用继续实测；HAL 多客户端/时钟测试；Windows QPC/WaveRT timer；Linux 独立 owner/driver clock | macOS 自研与外部通路、指定 Ubuntu 均有运行证据；Windows 仍需运行验证 |
| 有界桥接及停止后无旧音频 | 自研/BlackHole mute、无源、停止及新会话实测；HAL 时间标签、长度限制、零分配；Windows DMA 256KiB、通知最多2；现有用户态队列合约 | Windows 实际 loopback/无活动应用行为未验证 |
| 音量/Mute 只应用一次 | BlackHole 标量到 dB 实测；自研实际标量 0.5 振幅比 0.5000000000000553、Mute/恢复通过；Ubuntu 原生 channelVolumes 0.5 振幅比约 0.49998、Mute/恢复/无源静音通过；Windows 声明软件音量 | Windows 音量作用点仍需测量 |
| 启停、节点移除及服务重建 | Mac Sender SIGINT/SIGTERM、改率故障与明确新会话、自研实际服务进程更换及重新打开通过；HAL 首/末客户端、异常退出与并发排空；Ubuntu 实际 PipeWire/WirePlumber 重启、节点删除与同逻辑身份数字桥接恢复；Windows timer/list teardown 源码 | macOS 与 Ubuntu 的上述故障/恢复通过；完整生命周期尚未齐全 |
| 休眠/恢复、禁用、更新/卸载 | Windows PortCls/WDF/PnP 源码；其他平台安装原型尚无完整生命周期产品 | 未完成实机矩阵与 E09 集成 |
| 最小接口、访问权限、实时边界 | 自研 HAL 原子参数与分配测试；Linux 私有文件锁/非root owner；Windows 无自定义 PCM IOCTL/落盘/网络代码，保留 DRM 管理 | 系统加载后的权限、Verifier/HLK/HVCI 与故障隔离审查未完成 |
| Windows 合法驱动来源与 WDK 入口 | MS-PL 微软简化 SysVAD，commit/文件哈希、SDK SHA256、10个 COFF 单元及 unsigned PE；MSVC project/build script | 真实 MSVC/WDK、签名及运行验收未完成 |
| Linux 用户会话与注销恢复 | UID 1000 正常用户 manager 启动、NoNewPrivileges 基线、独占拒绝和停止后清理通过；用户级 service 模板 | Ubuntu 桌面有运行证据；正式 unit 未安装/启用，注销恢复未验证 |
| macOS Apple Silicon ABI 与设备实现 | arm64 bundle、独立 SDK 布局/常量对照、全部可调用接口槽、控件和多客户端、ad-hoc strict 校验 | 本机源码/离线及实际 CFPlugIn bundle 宿主通过；系统安装签名/权限/哈希核对通过，Core Audio 真实隔离宿主/系统设备/数字桥接通过；名称修订实际安装、跨进程读回与服务恢复通过 |
| 签名、公证、正常安全配置和安装清理 | BlackHole 使用已有厂商签名驱动；自研 HAL ad-hoc；Windows 映像 unsigned，无安全设置改动 | 不能据此标记 E06/E09 发布门槛通过 |

源码与复现入口见 [E06](E06.md)、[ADR-009](adr/ADR-009-virtual-outputs.md)、[Windows 合约](../drivers/windows/wavert/DRIVER-CONTRACT.md)。所选证据在 `docs/evidence/e06/`，完整日志/失败记录和二进制在项目 `artifacts/`。

## 当前验收阻断

自研实际系统桥接证据为 `docs/evidence/e06/neonmix-native.json`，通过普通应用、混音、音量、Mute、无源、离线设备及新会话；不能继续沿用“未系统加载”的旧结论。标准 Name 的历史跨进程拒绝保留在 `artifacts/e06-binding/20260930-175759`。

修订采用 `nmna` 自定义 CFString setter，`cust` 描述符与标准 Name 通知在 SDK 宿主和实际系统调用中验证通过。已安装和项目 bundle 的二进制 SHA-256 均为 447a28242cd8253e55f6f18812918a806030bec87e20d77595add732a5ed3e5b。名称更新读回一致、音频持续、UID 不变；绑定禁用/删除撤销旧会话、地址变化和不同 Hub 拒绝通过。服务恢复探针也已通过。详见 neonmix-binding.json 与 neonmix-service-recovery.json。

用户追加授权指定 Ubuntu 主机，实际桌面、音量、加密发送、绑定及服务恢复已通过，见 [Ubuntu E06](UBUNTU-ARM64-E06.md)。Windows 运行矩阵仍未执行；正式驱动/插件/安装包签名、公证、Verifier/HLK/HVCI、完整生命周期与干净机器分发也没有完成证据。因此整体 E06 尚未达到开发计划全部门槛。

自研服务恢复证据见 `neonmix-service-recovery.json`：Core Audio PID 从 76818 换为 86446；旧输入/输出明确故障退出，原 UID 恢复，重新打开后的数字桥接及原生时钟通过，输入/输出错误与回调超预算为 0。系统声音设置列表可见 NeonMix 已由操作者确认。实机睡眠由操作者明确暂缓；实际睡眠探针已准备好，要求新的完整 Wake 记录、至少十秒真实睡眠、同 UID 唯一和非零数字重开；不会以显示器熄屏、DarkWake 或操作者确认代替证明。

macOS 开发安装包已构建并实际展开验证载荷/权限/签名/哈希，但未执行 Installer/干净机器矩阵。正式模式已检查并拒绝缺少 Developer ID 的开发 bundle；证据见 macos-package.json 与 artifacts/e06-macos-release-gate.log。

2026-10-02：本文原有系统凭证相关验收是历史记录。新文件方案实施与 macOS 验证见 [凭证存储交付](evidence/credential-storage/implementation-20261002/README.md)，不能据本审计计作 Windows/Linux 新方案通过。
