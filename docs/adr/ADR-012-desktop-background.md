# ADR-012：桌面后台、受限 IPC 与 UI 状态

状态：E07 源码已实施，macOS 功能验证；其他平台运行门槛待补。日期：2026-10-01。

依据开发计划 §4.7、设计方案 §8.4。保留 egui/eframe 0.31.1 独立 UI；音频由独立 `neonmix-background` 管理同目录 Hub/Sender CLI，不让 UI 持有音频流。后台不随 UI 进程组退出，不自动重启被用户停止或异常结束的媒体，也不另建系统服务。

本地接口使用闭合 typed Request 集合、版本、限长/超时/并发限制。macOS/Linux 的 socket 与目录限制到登录用户；Windows NamedPipe 使用当前 SID 的受保护 DACL、拒绝远程与双方 SID 校验。输入不是 shell 命令，也不能指定任意二进制、角色或明文长期凭证。详细边界见 [IPC 合约](../DESKTOP-IPC-CONTRACT.md)。

复用 E04 权威状态/revision/权限、E05 固定 Hub 身份和平台 vault、E06 稳定输出绑定。UI 先读真实状态，再提交具体操作；失败保留输入，版本冲突刷新后由用户重试。地址/身份变化时丢弃旧读取结果，未知/过期权限禁用写入。普通轮询与操作忙碌分开，输入不会因周期刷新反复禁用。

本地停止与退出使用独立紧急通道，中断挂起的只读 CLI 查询；不取消可能已经提交的变更。开始与停止仍保持用户动作顺序：在途 SenderStart 后一定补发最终 Stop，防止先完成 Stop 再被旧 Start 覆盖。

实时 Mixer 新增预分配 50 ms Peak/RMS 窗口和原子快照，声明内部 48 kHz 口径。槽位身份与电平同时发布，无流、重开或输出不可用不显示旧电平。音频回调保持零分配；采集累计 Peak 与近期 Mixer 窗口分别标注。

macOS 标准 NSStatusItem 直接关联 retained NSMenu，提供托盘恢复与应用 Cmd+W/Cmd+Q。关闭窗口隐藏；退出需确认并先结束本实例音频。原生 AX 发现了 egui 的无效焦点崩溃，因此取消 Modal 后延至下一帧聚焦始终存在的页导航，并对 AccessKit 树的 focus membership 加入回归。下拉选项通过共享 primitive 显式关闭 popup，确保键盘/AX 选择完成。

脱敏导出采用独立、固定字段白名单，仅输出数字/布尔/null。它不是对任意错误字符串做关键字替换，邀请、令牌、证书、凭证引用、身份、名称、地址和路径全部排除。长期证据与未验收门槛见 [E07](../E07.md)。

2026-10-02：本文的平台凭证存储及对应依赖选择已由 [ADR-014](ADR-014-file-credentials.md) 取代；其他信任、协议与生命周期定义继续生效。


2026-10-07 可靠性修订：受管 Hub/Sender 以私有 stdin 协作式停止，生命周期任务独立于 IPC future；Windows 音频树使用挂起后入 Job、再恢复的创建顺序，kill-on-close 仅作异常兜底。Hub 正常停止对所有 worker 发出独立优先级 stop/EOF，在共享 2s 窗口内等待，随后才强制回收。运行 PEM 通过同一已验证句柄读取为小于 4096 字节的缓冲，解析完成和失败均清零；不再依赖 ANSI 路径，不改变持久身份与配对格式。新旧组件通过受管 CLI 与 ready 能力字段拒绝不兼容组合。实现、探针及剩余真机门槛见 [Windows AirPlay 可靠性记录](../WINDOWS-AIRPLAY-RELIABILITY-IMPLEMENTATION-20261007.md)。


## 2026-10-08 稳定性增量

后台 lifecycle 请求从接入配额和运行长锁中分离，使用同用户私有 endpoint；Stop 先接受 operation/实例代次，再由不随客户端断开取消的 owner 回收。Status 复制内存状态，不执行远端 CLI 或持久化。新桌面创建时固定 Start 的实例/停止代次与原选项，服务器接入和 spawn/install 都检查；旧实例停止命令也不作用于新后台。Ready 独立锁存，超时不作为成功，失去确认的退出不伪造成功。

Unix 协作 EOF 补齐独立 `neonmix-guardian`，不建立另一套 Stop 协议。guardian 持有创建时隔离的媒体组，5 秒协作/2 秒树退出核验；通过 waitid WNOWAIT 保留组 leader 的内核 PID，全部后代退出前不 reap，禁止 PID/PGID 复用误杀。后台正常等待保留 9 秒供 guardian 回收；Windows 使用原私有不可继承 Job。媒体 owner 是后台/guardian，而非 Desktop，UI 关闭与 UI 崩溃保持媒体的约束不变。

运行文件清理仅针对 live owner 捕获的 UUID 临时密钥，私有目录 pin、inode/dev 和 state owner 锁共同防止误删替换对象/其他实例。失败资源聚合为固定原因，媒体退出与清理完成分别报告；持久身份/配对资料保持。组件与 native 夹具证据不替代完整产品媒体、三端运行、最终签名包或 GUI 验收。
