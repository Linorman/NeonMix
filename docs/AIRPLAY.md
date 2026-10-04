# AirPlay Speaker 开发与验证

## 当前多入口实现（2026-10-02）

已接入多入口 profile v2、逐入口 worker、统一四路容量、定向控制 IPC v2 和桌面 v2 页面。此版本仍为 **macOS 开发 Alpha**；两路、四路加密数字源通过同一 Hub 和 CoreAudio BlackHole 输出，逐路断开、worker 退出及配对身份恢复得到短程探针验证。数字源的 FairPlay 接缝、Mixer 电平和计数不能替代真实 Apple 设备、模拟输出、P95、8/24 小时长测或发布验收。真实 Apple 占用入口消失、当前播放不中断及四台实机并发仍未验收。

下文历史记录保留当时的单路结果；当前接口与容量以 [行为合约](AIRPLAY-CONTRACT.md)、[控制 v2 / PCM v1 合约](AIRPLAY-IPC-CONTRACT.md) 和 [ADR-015](adr/ADR-015-multi-airplay.md) 为准。不要将早期 Keychain、单路配额或 v1 UI 描述作为现行多路行为。

### 使用多入口

1. 构建同一版本的 Hub、worker、desktop 和 desktop-service。升级现有房间前先停止对应后台，再用同一版本启动；仅关闭 UI 不会替换正在运行的后台。后台仍只拥有一个 Hub 和一个实体输出。
2. 在 Hub 设置选择 2–4 个入口，再开启接收。运行中可增加入口；减少前先结束被移除入口的来源与配对挑战。入口数表示可选择目标，不表示已连接来源数。
3. 每台 Apple 来源主动选择一个空闲入口。入口 1 保留原名与身份，新入口有独立密钥、UUID、名称和监听端口；首次连接使用该入口显示的 PIN。
4. 设备管理保留全部已配对来源，支持离线撤销、重新允许和无活动会话时修改播放方式。Mixer 每个活动来源独立成行，Mute/Solo 与原生输入共享房间规则。

配置升级不自动启动接收。原 v1 文件第一次经管理员启用或配置时，在持有 Hub owner 锁且入口停止的情况下验证并迁为单入口 v2；不自动增加或开启入口。未知版本、损坏资料或缺失秘密会报错，不重建身份。新建入口中断时私有pending日志用于恢复原UUID及密钥引用；缺少profile而已有凭证且无日志时拒绝创建。额外入口关闭后身份仍保留。

```sh
# 只读取认证后的 v2 状态；不会主动开启接收
tools/dev target/release/neonmix-hub airplay-v2 --credential .local/desktop/hub/admin.json
# 项目内命令文件：
# {"command_id":"UUID","expected_revision":N,"operation":{"action":"configure","receiver_count":2,"multi_receiver":true}}
tools/dev target/release/neonmix-hub airplay-v2 --credential .local/desktop/hub/admin.json --command .local/airplay-command.json
```

`configure` 省略 `receiver_count` 时默认建立 2 个入口；旧单路资料在显式配置多路前仍保持 1 个入口。所有写入使用独立 AirPlay revision。命令 ID 做有界去重，定向媒体操作还需要当前 source/session ID；客户端不能把旧来源操作映射到同入口的新连接。多入口模式的无目标 v1 接口返回升级要求。

### 离线迁移与回滚导出

下面命令只用于明确选择的资料目录，要求对应 Hub 已停止。工具解析 Hub profile 并取得与 Hub 相同的 state owner 锁；不会连接当前桌面房间或启动媒体。

```sh
tools/dev cargo build -p neonmix-identity --bin neonmix-airplay-profile
# LEGACY_NAME 必须是原接收器的完整发现名称；Hub UUID从server.json引用的state读取
tools/dev target/debug/neonmix-airplay-profile migrate .local/example/hub/server.json 'NeonMix — 原房间名'
# 输出名只允许同一airplay目录下的新文件名，保留原v2与凭证
tools/dev target/debug/neonmix-airplay-profile export-v1 .local/example/hub/server.json receiver-v1-export.json
```

旧v1全局禁止播放在迁移后变为各旧来源的播放禁止，管理员可逐来源重新允许；撤销来源仍不能通过允许动作恢复。导出使用入口 1 的当前密钥引用、配对和全局撤销状态，不恢复旧快照。工具输出恢复所需的 UUID/device ID/名称；运行旧 Hub 前须保持同一 Hub 身份及发现名称。导出文件不是自动安装或自动回滚；原 v2 及额外入口凭证继续私有保留。旧平台凭证格式仍需先走 [ADR-014 文件凭证迁移](CREDENTIAL-STORAGE.md)。

### 本轮可复现验证

```sh
tools/dev cargo test -p neonmix-identity -p neonmix-airplay-ipc -p neonmix-airplay-adapter -p neonmix-hub -p neonmix-desktop -p neonmix-desktop-service --locked
tools/dev python3 tools/airplay_multi_source_probe.py --sources 4 --output coreaudio:BlackHole2ch_UID
```

初始候选的[双路数字源报告](../artifacts/airplay/multi-source/65dece189792/result.json)与[四路数字源报告](../artifacts/airplay/multi-source/7faceabb6034/result.json)保留对应二进制哈希；后续候选需重新运行，不能沿用旧哈希结果。

数字探针创建项目内隔离身份，显式选择 CoreAudio 输出，测试后回收自有进程和 fixture；它不连接已有用户房间，不录音。输出报告绑定 Hub/worker 二进制 SHA-256，范围以具体报告为准。macOS 桌面合成状态验证见 [UI 证据](evidence/multi-airplay-ui-20261002/expanded-verification.json)，覆盖空/单/四来源、600/1600边界、长名、入口改名与别名草稿/错误；先前1100/720双AirPlay加双原生截图一并保留；截图预览不启动后台或播放声音。

## 单路与早期调查记录

2026-10-02 暂停恢复调查：用户确认低延迟正常，但 iPhone 暂停后恢复会断开。已从固定 UxPlay 方案和数字源找到两个接收端可复现缺陷，完成保持身份/控制连接的媒体时间线及重复 SETUP 修订，五次恢复、短暂停、FLUSH、同步和预缓冲验证通过。当前实例候选启动等待 macOS 钥匙串读取完成，真实 iPhone 结果仍待确认；不能将数字通过解释为 iOS 27 全面兼容。[本轮方案与边界](evidence/airplay/pause-resume-20261002/README.md)。

2026-10-02：默认低延迟音乐模式已实现，可在接收器就绪且来源断开时切换为音画同步。每个媒体 epoch 固定平移 PTS，首包接收后保留 120ms 余量；不会逐包按到达时间重排或跳过采样。macOS release 的合成加密 ALAC→真实 worker/Hub/CoreAudio 对照：音画同步待播约 1,938ms，低延迟 30 秒稳定段约 118ms，96 包预缓冲约 136ms，迟到包与定时跳帧均为零。这里的待播时间是软件估计，不是 Apple 来源或扬声器的端到端测量。

房间设备页已显示当前/最近 AirPlay 来源、连接状态、公钥摘要 ID；支持搜索、计数及既有管理员控制，活动来源排在最前。它不取得原生房间控制凭证。相关测试、截图、源码/制品与复现边界见 [本轮记录](evidence/airplay/latency-devices-20261002/README.md)。新制品位于 `target/release`；已有手动测试实例使用私有二进制副本，更新时保留身份与配对记录：

```sh
tools/dev python3 tools/airplay_manual_session.py restart --refresh-hub --refresh-worker --refresh-desktop
```

这会结束当前接收，之后从来源重新选择原房间。未执行该命令前，已有运行实例继续使用旧副本。

2026-10-01：已实现 macOS 音频接收原型、Hub 定时接入和桌面控制。真实 Mac 系统声音列表可发现接收器，已通过一次原生 PIN、签名、FairPlay 与音频 SETUP 并收到解码 PCM；修复会话 ID 截断后，加密数字发送端已通过实际 Mixer 和 CoreAudio 的 1+1 混音；iPhone 16 Pro / iOS 27 / Spotify 已经由用户确认实际可听，但初版出现爆音与音质损耗。定时采样连续性修订后的一次实机复测在开始播放前失败；已补充有界媒体背压和失败阶段诊断，通过 macOS 突发数字源回归，等待再次实听。真实 Mac 最终播放、视频伴音和 Windows/Linux worker 构建仍未验收。

## 构建与运行

所有命令在项目根目录经 `tools/dev` 执行。GStreamer 原生运行库先按主项目入口准备；AirPlay 额外源码、开发头文件、补丁、音频插件清单和 CMake 输出都在 `.local/airplay`。

```sh
tools/dev python3 tools/prepare_gstreamer.py
tools/dev python3 tools/prepare_airplay.py
tools/dev cargo build --release -p neonmix-hub -p neonmix-desktop -p neonmix-desktop-service
```

`prepare_airplay.py` 验证固定下载哈希，并把音频 worker 复制至 `target/debug`、`target/release`，与 Hub 同目录。Hub 自动使用该同目录程序；不会接受远程客户端提供的程序路径。

启动已有桌面后台/房间，选择实体输出并开始共享。在「Hub 设置」或「Mixer」的「AirPlay 扬声器」中开启接收。管理员可显示源设备配对码；源 Mac 从系统声音输出列表选择 `NeonMix — 房间名`，在原生输入窗输入码。窗口关闭由独立后台保持接收；关闭 AirPlay 接收、断开来源、重新允许、静音/Solo 和撤销配对有独立入口。

此版使用上游 `AppleTV3,2` 协议兼容型号：实机对照证明自定义型号导致 Mac 走未实现的文本 ANNOUNCE，而兼容型号进入已有纯音频 PIN/SETUP 路径。型号不代表视频能力；features 不含视频、照片、镜像或 HLS，所有相关媒体请求仍被拒绝。必须从音频输出列表选择，不能使用屏幕镜像。

CLI 可读取接收状态或发送有 revision 的管理员命令：

```sh
tools/dev target/release/neonmix-hub airplay --credential .local/desktop/hub/admin.json
# 先读取上述 AirPlay revision，再创建项目内命令文件：
# {"expected_revision":N,"operation":{"action":"enable"}}
tools/dev target/release/neonmix-hub airplay --credential .local/desktop/hub/admin.json --command .local/airplay-command.json
```

AirPlay revision 与原生房间 revision 分开。源码入口、权限及字段见 [行为合约](AIRPLAY-CONTRACT.md)、[IPC 合约](AIRPLAY-IPC-CONTRACT.md) 与 [ADR-013](adr/ADR-013-airplay-audio-worker.md)。

## 已运行验证

- macOS workspace 最新常规测试 141 项通过；随后增加输出 epoch/PIN 窗口回归。后台/Hub/IPC/adapter/identity/I/O 严格 Clippy 通过，桌面 release 构建与原生窗口预览通过；桌面现有未用组件等警告单独保留。
- 独立 worker 正向数字发送端覆盖正确/错误 PIN、20 字节实际 SRP proof 与旧 64 字节格式、公钥绑定、签名验证、请求 ID 准入、FairPlay 接缝、NTP、加密 RTP 回绕与 PCM IPC。越界配置、无效密钥、第二连接、错误 SRP 阶段及视频/HLS/镜像拒绝通过，AddressSanitizer 通过。数字发送端不冒充 Apple 客户端。
- 真实 Hub/CoreAudio 输出的两次启停、管理员 PIN 可见/成员不可见、成员写入和旧 revision 拒绝通过。杀死 worker 后原生 Sender、Hub 输出帧和实际电平保持。见 [Hub 证据](evidence/airplay/hub-macos.json)。
- Mac 27.0（26A5416b）同机系统声音选择器的发现、原生正确 PIN/SRP、签名和音频协商已得到证据；实际 source proof 为 20 字节，已修正原上游只接受 64 字节的检查。首次收到 644 个 PCM 块全部被拒时，定位到 libplist 对大于 INT64_MAX 的 JSON ID 静默截断；Hub 和 worker 控制 ID 现均限制于 `1..2^53-1`，二进制字段仍为 u64。

当前源端重复 PIN 输入窗的自动化未稳定，最近未成功完成最终播放复测；最近失败见 [Mac 源端证据](evidence/airplay/mac-system-source.json)，早期错误 profile 的结果另行保存。没有通过的视频伴音、物理 P95、长测或三端发布结果。

## 复现与证据边界

```sh
tools/dev cargo test -p neonmix-airplay-ipc -p neonmix-airplay-adapter -p neonmix-hub
tools/dev python3 apps/airplay-worker/probe.py
tools/dev .local/airplay/build/neonmix-airplay-audio-probe
tools/dev python3 tools/airplay_hub_probe.py
tools/dev python3 tools/airplay_mac_source_probe.py
```

所有探针 fixture 位于 `.local/tmp`，有自身进程和相邻 .credentials fixture 清理；源端探针暂时切换系统输出，结束恢复原来的输出。常规运行不录音、不保存视频或原始配对包。`NEONMIX_AIRPLAY_TRACE=1` 仅开启固定方法、路由、响应码和 SRP 长度/失败类别，不输出密钥、PIN、公钥负载、IP 或原始头/body。

定时 ingress 的 4 秒/2 MiB 限额、原生短队列、epoch、跳钟和 protocol gain 去重已测试。callback 呈现锚点、Mute/Solo 消耗及 44.1/96 kHz nominal conversion 坐标有数字测试与实时零分配回归；这些不能代替扬声器输出与源屏幕的共同测量。

加密数字源的动态身份、定时释放、真实 Mixer 电平、播放中加入原生 Sender 的 1+1 混音及 worker 故障隔离已通过，见 [Mixer 证据](evidence/airplay/mixer-macos.json)。原生建会话持有控制锁曾拖住 AirPlay 统计发布，现将调度统计保持在本线程、通过非阻塞快照发布，未增大媒体队列。

后续继续完成真实 Mac 系统输出与本地视频播放，确认 pause/seek/切回及 1+1 混音，再按用户约束只在 macOS 验证接收端。原生 PIN profile、当前用户 IPC 产品化、三端 worker/干净安装、许可和签名发布仍按专项计划推进。来源与分发边界见 [vendor/airplay](../vendor/airplay/README.md)，不改变 Rust workspace 的专有许可。

## iPhone 实听与音质修订

用户使用 iPhone 16 Pro / iOS 27 / Spotify 已连接并确认 Mac 扬声器可听。初版收到 8,773 个 PCM 块、释放 8,623 块，接入层各拒绝计数为零，但定时 Mixer 跳过 6,085 帧；用户报告爆豆声、明显音质损耗与稍高延迟。该结果只证明接通，不表示音质通过。

定时 lane 现改为连续 fractional cursor、共享预计算 64-tap sinc，以及 PTS 斜率/平滑相位控制。原生 lane、8 块交接与绝对 deadline 预算保持。±100/500ppm、±100µs/1ms callback jitter、480/383 混块的 440/1000Hz 数字波形尖峰残差由旧约 0.999 降到最高 8.71e-6；120 秒加速稳态、8k/18kHz 幅度、回调零分配与原生漂移/停顿回归通过。修订 release 的加密数字源→真实 CoreAudio 1+1 测试 `timed_late_frames=0`。这些数字证据不代替同一源设备的实听和视频伴音。

新增 `timed_drift_ppm`、`timed_phase_error_ns` 与实际协商 codec/源采样率诊断；空 error 不再在脱敏时转成字符串，避免健康接收器出现假的故障文案。配对码显示字号增加，仍默认遮挡、管理员可明确揭示。

## 启动失败修订

音质修订后的 iPhone 复测协商为 ALAC、44.1kHz、每包 352 源帧，接收 55 个 PCM 块后 worker 退出，尚未释放播放块。用户确认无法播放。这次旧版本只报告通用 `worker_failed`，无法据此确定全部退出原因。

发现媒体 reader 的 8 块交接队列曾把暂时满载直接当作致命错误。现保持该队列大小，在 reader 线程保留一个待交接块并等待最多 200ms，通过 TCP 向 worker 施加有界背压；独立控制管道和停止标志不受等待影响。新增 `failure_stage` 固定类别，区分媒体拥塞、校验、关闭及解码故障，不转发原始异常或密钥材料。

macOS release 的 96 块初始预缓冲（500pps，随后恢复源时钟）通过真实 worker、Hub、定时 Mixer、CoreAudio 和原生来源混音，接入拒绝为零、`timed_late_frames=0`；见 [预缓冲证据](evidence/airplay/prefetch-macos.json)。这一探针使用合成加密 PCM，仍需 iPhone ALAC 实听确认。

真实设备首次测试通过 `tools/dev python3 tools/airplay_manual_session.py start --name 'AirPlay 复测'` 启动。它在项目内复制不可变二进制，再创建独立文件凭证 fixture；编译不会替换这次测试的运行文件。后续 `start` 复用同一身份，省略 `--name` 时保留原名。`status` 读取脱敏状态，不包含配对码或凭证；`stop` 停止自有进程并保留配对，再次 `start` 恢复接收。只有明确执行 `clean` 才删除本脚本拥有的 .credentials 和 fixture 文件。`restart` 保留身份、配对、名称和输出，重新开启 PIN 窗口；旧平台凭证资料先通过独立迁移工具复制到新目录，不能直接覆盖；历史实测读取 receiver key 曾阻塞在 macOS Keychain。

2026-10-02 用户确认 iPad 弹出 PIN 输入窗并配对成功；当前实例协商 ALAC 44.1 kHz，收到并释放 24,777 块，接入拒绝为零。iPhone 早期成功的接收身份与后来重建的测试实例不同；已修复脚本重复启动删除身份的行为，当前 iPad 身份及配对经真实 stop/start 验证保持不变。新增 `tools/dev python3 apps/airplay-worker/pairing_probe.py` 验证接收端公钥/签名绑定、持久记录重连、记录缺失、撤销与换钥，五项通过。iPhone 恢复、当前版本的实际音质和视频伴音仍需实机确认；详见 [配对调查](AIRPLAY-PAIRING-INVESTIGATION.md)。

后续 iPhone 选择接收器时没有 PIN 提示，播放自动退回 iPhone；Hub 无握手、无已配对公钥、无音频，换名后仍然如此。实际 DNS-SD 查询发现 Wi-Fi 发布了其他网卡的 `fe80::/64` 地址，mdns-sd 0.21.4 按网段匹配 IPv6，没有保留这些地址各自的接口 scope。Speaker profile 现暂时禁用 IPv6 发现，原生 NeonMix 发现保持原有行为。新实例的实际广播已无 AAAA 地址记录，广播出的非 loopback IPv4 地址均得到 RTSP `/info` 200；见 [发现与监听证据](evidence/airplay/discovery-ipv4-macos.json)。这是发现缺陷的本机验证，尚不能证明它是此次手机退回的唯一原因，仍等待真实来源复测。IPv6-only 网络和完整 scope 修订未验收。

文件凭证的格式、备份、目录搬迁、显式迁移与回退见 [文件凭证使用与升级](CREDENTIAL-STORAGE.md)；存储决策以 [ADR-014](adr/ADR-014-file-credentials.md) 为准。系统凭证测试记录保留为历史证据，不能计作新文件方案的验收。
