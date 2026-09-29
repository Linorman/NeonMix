# E01 音频接口与实时边界

## 统一格式与源

`AudioBlock` 预分配 480 个双 lane float32 帧，不含堆对象。`frame_count` 决定有效范围，尾部不是音频。Mono 原生采集复制到左右 lane，`channel_layout` 保留原生 Mono 含义；输出 Mono 使用 `(L+R)/2`。暂不猜测多声道顺序，>2 声道明确拒绝。

头字段包含 `stream_id`、`stream_epoch`、源帧位置、原生 sample rate / channel layout、frame count、discontinuity flags、原生 capture timestamp 和进程内 arrival timestamp。时间戳只在同一流时钟域内比较；不能跨电脑相减。源采样率支持 44.1/48/96 kHz，PCM 表示支持 f32/f64/i16/i32/u16，其他格式返回可见错误。

`StereoSource` 是 48 kHz stereo 的输出源接口。测试源支持 sine、每秒 impulse、silence及双声道/单侧/反相模式；Mute 继续推进源帧。`NativeBackend::open_output` 按真实回调帧数 pull，不以固定10 ms 调度。非48 kHz输出通过 rubato SincFixedIn 预分配转换，报告群延迟帧数；E03 将另行实现长期ppm反馈控制，当前只有名义采样率转换。

## 时间、周期与事件

源位置基于实际收到的帧计数；原生 capture 时间戳识别 >2 ms 时间空洞并跳过空洞，不保存“补播”数据。时间倒退、格式重建、暂停后恢复采用新 epoch。进程重启后 epoch 可重新从1起；E02 必须同时用新 session_id 绑定，不能单独把 epoch 当全局身份。

输出同时报告 submitted_frames、原生 playback timestamp、timestamp-derived presented_frames，以及原始 `native_device_frame_position`、按epoch归零的 `native_stream_frames` 和换算到48kHz的 `native_internal_frames`。原始位置来自WASAPI IAudioClock、Core Audio有效sampleTime和PipeWire graph ticks；借助有来源记录的CPAL最小扩展公开，没有新增一套后端。HAL值是I/O周期的sample坐标，PipeWire值是graph时钟坐标，不能与模拟端到端测量混称。缺少有效原生位置时报告null，绝不以提交帧数冒充设备位置。

`presented_frames`仍独立表示根据设备报告待播时长推算的播放位置，不是原始计数。使用整段帧数有理数换算避免逐块取整漂移；重开流构造新OutputClock，原生帧位置/时间戳回退或超过1s断点更换epoch。所有硬件基准仍需三端真机对照。

设备 ID 使用 WASAPI endpoint ID、Core Audio UID、PipeWire node.name；拒绝 PipeWire 合成默认别名。不绑定设备名或枚举序号，不在设备丢失时切到其他输出。CPAL 负责活动流的原生通知，CLI `watch` 在控制线程以1Hz比较设备快照，生成 added/removed/changed；枚举故障作为错误报告，不假装设备列表为空。

## 队列与实时约束

| 边界 | 容量/单位 | 满/空处理 | 调度 |
|---|---|---|---|
| 原生 capture → 消费者 | 32块；每块最多480帧 | 满时丢当前块并换队列 generation；消费者清除旧generation，保留新数据；超过100ms过期 | 原生回调生产，非实时消费者读取 |
| 输出源 → rubato | 每次256个源帧；输入/输出预分配 | 不等待其他线程；转换错误置码 | 设备回调同步拉取 |
| 诊断 | 原子计数，固定字段 | 不存PCM或错误字符串 | 控制线程格式化JSON |

容量单位是块，不是固定320ms；小周期可能每块远小于480帧。捕获桥接会把大原生回调拆成多个块，不依赖实际周期等于480。非实时CLI每5ms排空队列，诊断每秒采样；音频由原生事件/回调驱动，不在音频回调睡眠或轮询。

回调不操作UI、磁盘、网络、数据库或Tokio；项目回调只用栈数据、预分配数组与原子状态。`realtime`集成测试对采集桥接、队列和sinc输出检查零分配/重分配/释放。该证据不等于所有原生依赖内部都无锁或硬实时保证。原生流由调用线程销毁，回调panic在项目边界捕获并锁定故障，错误回调只记录数字码。

## 日志与故障码

JSONL包含 event、version（错误/版本事件）、device_id和StreamInfo、stream_id/stream_epoch（块头）、帧数、静音/无数据、drop/stale、原生时间戳、回调最长耗时和实际周期。无录音文件、密钥或令牌。

| 数字码 | 含义 |
|---|---|
| 1 | 设备不可用 |
| 2 | 其他原生后端错误 |
| 3 | 不支持格式、样本缓冲类型不符或转换失败 |
| 4 | 项目回调panic，后续静音/停止采集 |
| 5 | Xrun / 系统报告音频期限未满足 |
| 6 | 设备配置变化 |
| 7 | 流失效 |
| 8 | 权限拒绝 |
| 9 | 实时调度权限不足 |
| 10 | 进程内epoch编号耗尽；拒绝复用旧时间线 |

CLI打开失败/回调故障返回非零退出码；采集无数据本身不是打开失败，按统计报告。端点/应用静音行为由平台提供，项目不会再叠加一份端点增益；实验性的 `--mute-at` 只控制本进程的采样/输出，不能等同于系统音量验收。

## PipeWire 边界补充

项目扩展订阅所选节点的global_remove事件，包括只进入Paused而不会进入Unconnected的移除路径；主动析构期间的断开事件不记为运行故障。一个已连接但没有音频端点的PipeWire服务返回空列表，枚举超时/连接失败仍报错。应用流节点不作为可选设备，避免不稳定/重复node.name。

SPA输入chunk的offset按maxsize取模，size受映射长度与预分配8192帧scratch限制，检查帧对齐并支持跨尾部的环绕。EMPTY标记生成相应PCM格式的静音，CORRUPTED标记报故障；不读取无效尾部。scratch在建流时分配，回调不分配或扩大它。

epoch使用进程内统一分配器；重开不能复用某个旧流在暂停/断点重置中已经使用的编号。进程重启仍须由E02的新session_id隔离。采集频率是带幅度门限的正向过零估计，用于确认测试音，并非完整频谱/音质测量；PCM不落盘。

回调的 `callback_over_budget` 统计墙钟耗时超过当前帧数/采样率的次数（包括调度抢占），与驱动报告Xrun的错误码5分开；二者不互相替代。

## 一致的输出时钟观测

`RunningOutput::latest_position()` / `position_reader()`提供同次回调的完整`OutputPosition`：`clock_timestamp_ns`与`native_device_frame_position`配对，`sample_rate`、本次`callback_frames`和48kHz换算值也在同一观测中。`device_timestamp_ns`保留旧字段名，表示预测播放时间，不能作为原始帧位置的观测时间。CLI的`clock`对象使用这一路径；独立原子诊断计数不作为同步时钟事务。

发布器只有一个可变拥有者，读者可克隆；全序原子字段和奇偶版本排除撕裂读，读取最多尝试3次，竞争或未发布时返回None。发布和读取均无分配、无锁或等待。返回值是最近完整观测，不自动证明当前仍在播放；要结合故障/回调状态。原生流先销毁，控制侧读者随后销毁，避免在音频回调清理时释放最后一个快照Arc。

macOS采集在打开设备前查询原生录音权限。虚拟输入也受Microphone权限控制；未授权返回PermissionDenied和明确修复提示，而不是将操作系统清零的音频当作有效静音采集。权限归属可能是启动CLI的宿主应用或终端。该查询只在控制线程执行，不在回调中检查或请求授权。

频率诊断会在连续50ms低于幅度门限时结束当前信号分段，再从下一段重新计数；统计仍保留最长有效段。停源期间的真实静音帧仍计入总帧数与RMS，不再计入跨段频率跨度。
