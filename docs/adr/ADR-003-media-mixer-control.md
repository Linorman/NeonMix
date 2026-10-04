# ADR-003 / ADR-004：媒体边界、输出时钟与控制事务

状态：E02–E04 核心原型实现、macOS 基础功能验证；工作包未完全验收，缺口见 [审计](../E02-E04-AUDIT.md)。补充并取代 ADR-001 中 E03 尚未实现的描述，不改变 E01 原生 I/O 合约。

采用 gstreamer-rs 0.23.x 的 1.24 API 基线，macOS 原生运行时锁定 GStreamer 1.28.7 官方 universal archive，项目内解包，不执行安装脚本。选用 app、opus、rtp、rtpmanager、dtls、srtp、coreelements、audioconvert 等明确插件；UDP 由 Rust 非阻塞工作线程管理，便于将已认证控制会话绑定到实际 socket。使用一个 40 ms rtpjitterbuffer 和及时交接、不执行时钟等待的有界 PCM appsink，不使用 rtpbin 后再串联第二个 jitterbuffer。

选择独立 Rust Mixer，GStreamer 不持有输出声卡，不执行房间混音或长期采样率补偿。RTP/PTS 通过有界锚点映射；Opus PLC 没有 RTP 包时使用实际输出样本数。RTP 时间由编码样本钟统一，规避首包编码前瞻产生的特殊步长。证书绑定和密钥 ready 同时检查，再进入解码及反馈。源码不读取或打印 DTLS 导出的密钥；认证与重放保护交给成熟媒体库。

Mixer 固定 16 个预分配槽位，以设备拉取的内部帧数为消费基准；SincFixedOut 支持动态比率。保持每路独立控制器、SPSC 和统计。槽位增删仅切换配置并重置状态，回调不析构流；关闭原生 stream 后在 owner 回收 Mixer，原 UID 重开时清旧积压并重置输出映射。

控制服务用 TLS 1.3、服务端认证 Principal、单权威状态、幂等和 revision。统一命令 envelope 用于所有写操作；事件采用有界增量而非不断复制完整会话历史。持久化事务与 SPSC 单生产者容量检查串行，先保证可落盘再提交。实验入口使用明确的凭证文件，不把该入口当作 E05 完整配对或系统凭证保护。

字段、错误、等待职责、限额和复现见 [媒体与控制合约](../MEDIA-CONTROL-CONTRACT.md)。Windows/Linux 新媒体运行时打包与实机互通尚未验证，自动发现、桌面 Mixer 界面和安装发布不是本 ADR 的已交付内容。


2026-09-30 调度修正：PCM appsink 的 `sync=true` 会阻塞上游 streaming thread，真实双路中出现超过 100 ms 的驻留和源位置缺口；改为 `sync=false`，包期限由 jitterbuffer 负责，实际呈现由设备驱动的 Mixer 负责。appsink 同时限制 buffer 数、字节与时间。Thread QoS 不能单独声明进程正在做用户音频工作，Sender/活动接收线程另持有 Foundation 活动 token，允许显式休眠并在退出时释放；不使用扩大缓冲来吸收后台定时器停顿。


解码时间线修正：jitterbuffer 内仍保留接收 PTS 与期限，但输出到 depay/decoder 前统一按扩展 RTP 样本位置重建 PTS，丢包 timestamp/duration 使用同一源钟。容量丢弃缺少原生 loss event 时补发最多 120 ms 的 loss event，实际 Opus PLC 数与额外包损独立公开并参与反馈；更长断点重新缓冲。测试音源按固定样本钟有界追赶，不以跳过周期压缩源时间。


Sender 的媒体泵保持在 QoS 普通线程，内核 sleep 节奏与 Tokio 控制反应器分开；停止信号每轮非阻塞检查。周期诊断用容量 2 的队列交给普通写线程，慢管道/文件只丢统计，不阻塞音频。真实故障探针额外堵住 B 的 stdout、检查期间 B 的 PCM/欠载、解堵后要求 `diagnostic_drops > 0`，证明测试确实产生了诊断背压。


2026-09-30 续轮：分阶段墙钟与线程 CPU 观测抓到两路 Sender 在 1 ms 等待中停留约 123 ms，送编码器仅约 36 μs；事件唤醒与 critical timer 单独改动后仍有约 53 ms 额外等待。独立检查确认 CLI 请求 QoS 成功但实际基础优先级为 31。Apple XNU 的 [task policy](https://github.com/apple-oss-distributions/xnu/blob/main/osfmk/kern/task_policy.c) 对未声明应用角色的进程设置 QoS ceiling，与本机读回一致。

媒体泵现改为 socket/appsink 就绪通知及有界期限，macOS 用 kqueue 单次 `NOTE_CRITICAL` timer；原生采集和预算耗尽保留有界重试。专用 Rust/GStreamer 媒体线程使用 `SCHED_OTHER`、基础优先级 47，并读回实际策略；没有设置 Mach 时间约束或抢占系统音频回调的实时优先级。线程 guard 不可跨线程移动，离开时恢复普通策略与基础优先级；显式 pthread 调度会退出 QoS 模型，不能用于共享的 UI/Dispatch 执行器。70 ms Mixer 水位与 80 ms 源期限不变。最新实测以稳定性记录为准，不把调度配置成功本身当作长测通过。


2026-09-30 E03 淡入审计：原统一绝对步长只在 unity 增益上对应 5 ms，低/高增益切换时长不同；buffering 输出的补零会提前消耗恢复淡入。由实际 Mixer 波形回归证明后，改为预分配的 240 内部帧线性 envelope，目标变化从当前值插值，PCM 尚不可消费时保持恢复 envelope。初始、FIFO 欠载/断点 reset 与输出重开共用同一规则，实时路径分配/重分配/释放仍为 0。原 8 小时运行受控中止，修订后另起固定制品验证，不把旧制品计为新制品已通过。

2026-09-30 Ubuntu 补验：GStreamer 1.24.2 缺少较新版本的 `dropped` 属性，改用缓冲私有序号追踪真实队列缺口。Sender 源队列缺口在编码前关闭上下文，不依赖 Opus/payloader 传播 `DISCONT`；接收 PCM 队列仍导出丢弃统计。TLS acceptor 的真实 local/peer SocketAddr 传给媒体事务，UDP 绑定实际控制本地地址，并保留 IPv6 scope，避免通配监听在多网卡下回复到另一个地址。Mac/Ubuntu 原生基线及短时互通见 [Ubuntu 补验](../UBUNTU-ARM64-E02-E04.md)；Ubuntu 长时音频仍未通过，未以增加队列或放宽判据声明稳定。

2026-09-30 Linux 调度/周期续修：媒体线程使用普通 SCHED_OTHER/nice −10，读回真实状态并恢复；音频回调使用 CPAL realtime 路径（已有 RLIMIT_RTPRIO 授权优先，否则 RTKit），PipeWire 内部数据线程加载 `client-rt.conf`。Linux 采用系统默认周期，不把 480 帧包长或固定 256 帧当作设备周期。恢复 VM 系统最小 1,024 帧后，五分钟跨机和控制回归全通过；40 ms jitterbuffer 与 70 ms Mixer 水位保持原值。详见 [根因和证据](../UBUNTU-ARM64-E02-E04.md)。
