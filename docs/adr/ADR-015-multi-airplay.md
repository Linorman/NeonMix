# ADR-015：多 AirPlay 入口、统一准入与会话隔离

- 日期：2026-10-02
- 状态：已采纳并实施 macOS 开发 Alpha；数字源验证通过，真实 Apple 多设备与发现隐藏验收未完成
- 依据：[多入口设计](../../07_NeonMix_多AirPlay设备混音_设计方案.md)、[开发计划](../../08_NeonMix_多AirPlay设备混音_开发计划.md)、[ADR-013](ADR-013-airplay-audio-worker.md)、[ADR-014](ADR-014-file-credentials.md)
- 当前行为：[AirPlay 合约](../AIRPLAY-CONTRACT.md)、[控制 IPC v2 / PCM v1](../AIRPLAY-IPC-CONTRACT.md)、[桌面 IPC](../DESKTOP-IPC-CONTRACT.md)

## 问题

旧实现只有一个接收身份、worker 和隐式当前来源，不能让多台设备独立进入 Mixer。多个已配对记录不等于多个活动输入；内部 Mixer 有 16 个槽位也不能成为 AirPlay 并发承诺。将所有设备引导到同一个传统 AirPlay 目标无法可靠地按来源分流，接收协议与 decoder 仍需单 owner。

## 决策

一个 Hub 发布 1–4 个独立稳定接收入口，每个入口运行一个音频 worker、占用时只接受一个来源。所有原生和 AirPlay 输入共享 Hub 权威容量与同一个实体输出。多入口模式的活动输入和建立中预留合计最多 4；每入口和每个已验证 source ID 各最多一个活动/预留。

入口、来源和会话使用独立身份：入口是持久 UUID/device ID/密钥引用；来源是规范解码后公钥 SHA-256；绑定是入口与来源的二元授权；session/stream/epoch/lane 为运行期对象。不同物理设备使用相同显示名不会合并；同一硬件使用不同配对公钥也不会自动合并权限。

profile v2 保存入口、来源、绑定、播放允许、多路模式与容量字段，仍使用 ADR-014 私有文件凭证，不新增 Keychain/vault 路径。来源上限 64、绑定上限 256，撤销记录计入上限。新入口独立随机生成 Ed25519 私钥，不能从公开 ID 派生秘密或复制入口 1 密钥。

首次接入旧 v1 时，在 Hub owner 锁和接收停止的条件下检查资料并升级，保持入口 1 的旧身份、名称、密钥、配对和播放方式；不自动启用多入口或恢复媒体。新增身份先关闭保存，部分失败后保留已保存的关闭入口供重试。私有pending操作日志先保存目标身份和引用，再写秘密与原子profile；强制退出后恢复相同身份而不泄漏无归属秘密，已发布秘密不被重建或删除。无日志且旧凭证存在时不能因profile缺失生成新身份。未知版本、缺失秘密或损坏资料报错，不创建替代身份。离线工具提供显式迁移和入口 1 旧格式导出；回滚导出反映最新全局撤销，不能恢复旧快照。

每路独立媒体 IPC、控制管道、Ingress、短 SPSC、连续 sinc 和 RTP/PTS 映射。Hub 锁只保护准入与权威提交；媒体/磁盘/发现等待在锁外，完成后重查原 owner。预留到期先关 gate、清媒体并回收实际 lane，再允许复用。公共输出时钟/声卡故障仍是全房间故障域。

控制 JSON 升级为 v2，绑定 worker generation、connection/request ID、trust generation 和 session/epoch；新增 `grant_applied`。PCM 保持 v1 的 112-byte header、最多 480 帧和 3952 bytes，避免不必要的媒体布局改动。控制与媒体可能乱序，确认前首包仅进入有界待播；正确确认后开 gate。旧事件、旧包和错误连接不能使另一会话重建或结束。

占用入口在活动 grant 确认后撤下公开服务对，保留 worker、listener 与现有媒体。释放后仍以相同身份恢复发布。独立 hostname 避免误撤其他入口；发现任务按代际串行，有界重试；隔离 mDNS fork 的旧 goodbye 重发不得作用于新注册。API 的 `hidden` 表示本地状态，不能宣称所有 Apple 列表即时消失。

新增 `/v2/airplay`，读模型分离 receivers/sources/sessions/capacity；写操作显式指定入口或 source/session。独立 revision 和最多 128 个进程内 command ID 回执防止重复提交；持久化成功后的会话消失返回“已保存、原会话已结束”，不回滚已提交设置或重定向新会话。多路模式下无目标 v1 调音、断开和撤销拒绝并要求升级。

桌面保留现有 egui 视觉系统，扩展入口设置、持久来源列表、多行 Mixer 和逐来源诊断。实际 stream ID 关联电平和推子；播放偏好/trim/mute 按来源保存，Solo 仅属于当前会话，覆盖原生和 AirPlay 全部输入。PIN 仅管理员短暂交互展示，日志、诊断与持久 UI 状态不保存。

## 保持的边界

仍为纯音频、经典 UDP/NTP profile；不新增视频、镜像、HLS、AirPlay 2 buffered audio/PTP、多房间或一个公开目标自动分流。默认低延迟按 epoch 首包保留设备输出积压之外的 120ms 接收余量，synchronized 保留协议目标。每路有界待播为 4秒/2MiB，四路合计 8MiB；这不是进程 RSS 上限。短队列、limiter 和实时 callback 约束保持。

旧单路模式保留原先 1 AirPlay +1 原生约束；只有显式多路配置才使用四路总配额。多路配置与旧 receiver v1 文件升级是两个动作，避免升级意外改变现有房间容量、发现名称或开始接收。

## 验证与未完成项

macOS 两路与四路独立加密数字源已通过同一个 Hub、动态不同 lane 和真实 CoreAudio BlackHole 输出。短程探针逐来源执行断开、对应 worker 退出及配对恢复，并检查其他来源继续、身份保持与新 session。报告绑定测试二进制哈希；合成 FairPlay、输出电平和计数不等于 Apple 互通或模拟声音测量。

自动测试覆盖持久身份/迁移/撤销/限额、IPC 代际与 context、准入竞争、控制去重/持久化边界、桌面稳定 stream 映射/权限与 PIN 脱敏。原生 macOS 预览覆盖双 AirPlay 加双原生状态与窄窗口，使用合成 fixture，不启动接收或录音。

真实两台及四台 Apple 来源并发、占用后 Apple 观察设备目标消失且当前播放不中断、完整 4+0/3+1/2+2/1+3/0+4 实机矩阵、物理音画 P95、8小时健康网络播放与24小时资源长测、Windows/Linux worker 运行和正式分发仍未完成。不能以入口 UI、本地 goodbye 确认、数字源或单设备历史结果关闭这些验收。
