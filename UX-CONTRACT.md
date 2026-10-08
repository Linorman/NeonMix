# NeonMix 桌面行为契约

P02 命令边界在 `commands.rs`，纯状态调度在 `intent.rs`。每个普通写意图创建时绑定 Hub/身份/runtime/context epoch 与完整 target 身份；按字段保留最多 128 个未发槽，连续绝对值在原槽合并，不同字段/目标公平排队。终止对象的离散操作取消其未发混音意图，重复终止点击合并；本地停止保持独立路径。房间/身份切换取消旧未发意图并清理该上下文草稿和 Undo；已发请求保留原 envelope，旧回复只记入原上下文记录。

Undo 只从确认成功的实际提交产生，窗口从确认时起算 8 秒。还原在创建、排队发送及兼容只读解析阶段核对相同 context/target、当前字段仍等于 after，仅提交相关 inverse 字段；不匹配显示冲突。结果 Unknown 保留草稿及冻结请求，使用“核对结果”重放原请求，禁止换 request_id/revision/payload，回执已不可得仍保持 Unknown。Mixer/现场/控制台显示通道命令状态；正在拖动的新草稿不被较早确认清除，session 移除/复用使旧草稿失效。

2026-10-07 键盘边界：现场、行式 Mixer 和控制台共用 `mixer_keys`。空 lanes 和失效 selection 先清空选择；空列表立即返回，M/S 不产生无目标操作。非空列表方向键可重新选择首路。文本框、IME、Modal 和快速操作继续拥有其键盘焦点，不触发通道操作。覆盖见 Desktop 的 `mixer_keyboard_handles_empty_removed_and_stale_selection_in_both_layouts` 和既有焦点/布局回归。

产品事实来源：`02_NeonMix_完整开发计划.md` §4.7、`01_NeonMix_设计方案与技术选型.md` §8，`docs/MEDIA-CONTROL-CONTRACT.md`、`docs/OUTPUT-BINDING-CONTRACT.md`、`docs/adr/ADR-011-discovery-and-pairing.md`。

## Canonical UI Map

| Capability | Canonical owner | Source of truth | Allowed variants | Verification |
|---|---|---|---|---|
| Button | egui::Button | DESIGN.md / theme.rs | action/confirmation | egui + native AX |
| Status | shell.rs 顶部栏房间胶囊 / 底部状态条 | 本文 | loading/ready/empty/error/stale | egui + native IPC |
| Device list | pages/devices.rs 本机音频设备面板 | native CLI devices JSON | read-only | native CLI + desktop |
| Room devices | Desktop::devices_page | Hub snapshot + /v2/airplay sources/sessions | native / AirPlay; search / state filter / count | egui AccessKit + macOS render |
| Select/Listbox | egui ComboBox + widgets::select_value / label_combo | DESIGN.md | authored | macOS popup/keyboard |
| Form | widgets::field / title_field | UX-CONTRACT.md | create/edit/inline title | validation/render |
| Gain | widgets::fader | 本文「Mixer」 | hero/row/strip（竖向） | egui test + AccessKit slider |
| Signal flow | flow.rs / pages/live.rs | 快照 + 诊断电平 + AirPlay 会话 | 左右三栏/上下紧凑；空态 | layout 不重叠测试 + AccessKit 节点名 + 截图 |
| Channel strip | pages/console.rs | 本文「Mixer」 | 控制台/行式回退 | 方向键测试 + 截图 |
| History | history.rs | 本窗口轮询（不导出、不写盘） | 电平历史带/趋势迷你图 | 单元测试 |
| Command | palette.rs 快速操作 / widgets::command_hint 平台提示 | 本文 | navigation/action | egui test (IME) |
| Scrollbar | theme.rs + egui ScrollArea | DESIGN.md | always visible | narrow screenshot |
| Toast | shell.rs 底部状态条（含还原） | UX-CONTRACT.md | success/error/pending/restore | render states |
| CRUD | desktop-service Request | shared protocol/API | explicit commit | macOS IPC probe |

## 业务依据与界面后果

权限来自 Hub `/v1/me` 和权威快照，服务器最终裁决（媒体控制合约）。成员仅调整自己的通道与停止发送；Solo/总控要求 Controller，撤销与重新允许要求 Admin。UI 未知或过期权限时禁用写入。

增益与 mute/solo 使用快照 revision，冲突后刷新并提示重试，不盲目覆盖。所有变更由后台确认后再显示为提交状态。设备和房间公开身份与名称同时可查。

发现结果不可信（ADR-011）；首次配对使用含固定身份的一次性邀请，私密邀请文本默认遮挡。正式长期凭证由相邻私有文件管理（ADR-014）。

Sender 的发送范围始终显示为绑定虚拟输出上所有应用的声音；系统默认选路由用户自己选择（输出绑定合约）。没有麦克风或应用级捕获快捷默认。

关闭窗口隐藏界面并保留后台，UI 崩溃不停止音频。停止发送只停止 Sender，停止共享只停止 Hub；退出后台明确结束本实例全部音频。没有自动重启被用户停止或管理员禁止的会话（规格 §8）。

## 状态与导航

快捷键提示统一由 `widgets::command_hint` 生成：macOS 使用 `⌘`，Windows/Linux 使用 `Ctrl+`，与 egui `COMMAND` 的实际行为一致。隐藏/退出快捷键当前仅 macOS 原生菜单提供，其他平台不展示提示。

左侧导航栏固定七个页面入口（现场、Mixer、Hub 设置、Sender、设备管理、诊断为 ⌘1–⌘6，关于无快捷键），默认打开「现场」；底部固定控制身份切换、「停止发送」（发送中）、并排的「隐藏窗口」「退出后台」，任何窗口尺寸下不随正文滚动，600×440 下与导航不重叠（有回归测试）。顶部栏显示房间及其状态，非 Mixer 页附带总输出电平、总音量与总静音；「快速操作」（⌘K）列出当前身份可执行的操作。各页共享底部状态条的 pending/error/success 消息。字段在切换页面、失败、后台断线时保留；不自动保存。刷新保留上次快照但显示过期，不允许以过期 revision 写入。单次 AirPlay 或诊断读取失败时保留同一房间的通道与上次有效数据，但立即标记不可用并停止绘制旧电平；连续 3 次失败才在底部状态条报告；本机 Hub 未共享时不以本地管理员身份轮询房间状态。每页空态均引导到所属入口；按钮忙时不可重复提交。

有界列表使用 ScrollArea；设备搜索本地即时过滤，带清除按钮。无匹配与未发现分别提示。

每页先给一个主视图与主操作，未完成的设置以步骤条展示，细节放在可折叠面板；默认展开跟随数据，用户选择优先。操作进行中不禁用整页，只有触发它的按钮显示进度；写入期间的输入保留最后一次意图，按下一份快照的 revision 发送。

危险确认使用 egui Modal，展示对象和影响，默认取消、Escape 取消，下一帧返回当前页导航；不引用已移除的控件焦点。全部动作由按钮触发，不依赖 hover。字段默认不因 Enter 提交，避免 IME 合成提交。

## Mixer

宽窗为通道条控制台（竖向电平与推子、总控条在右），放不下时每一路一行：电平、推子、增益读数、静音、Solo、详情。推子相对拖动，单击不改变增益；双击回到 0 dB；聚焦后沿行程方向的方向键步进；滚轮仅在聚焦或按住 Option 时生效。控制台 ←/→ 选择通道、↑/↓ 调音量；行式 ↑/↓ 选择通道、←/→ 调音量；M/S 静音/Solo，文本输入或对话框时不响应。「现场」页单击来源与 Mixer 共用所选通道，检查器内的推子、静音、Solo 与 Mixer 规则相同。混音调整 8 秒内可「还原」（⌘Z），以新 revision 发送反向操作；措辞与「撤销配对」区分。不可闻的通道淡出但仍可操作。快速操作在输入法组字时不因回车执行。

## 现场与动效

信号汇流图只画权威快照、诊断电平与 AirPlay 会话：未取得的电平画空心环、无会话状态画灰色虚线，不补齐、不演示。光点、雷达等连续动效只在对应真实量变化时运行（20 fps 上限），无信号、窗口隐藏或开启「减少动态效果」（关于页，或 `NEONMIX_REDUCE_MOTION=1`）时不申请连续重绘。房间动态与电平/指标历史只存在当前窗口内存，切换身份清空，不进入导出或状态文件；`ui-preferences.json` version 2 仅保存本机语言与减少动态效果设置（及兼容未知字段），不写用户/设备资料或秘密。

## 诊断与验证

四个健康磁贴概括播放与输出、媒体网络、缓冲与 Sender 采集，点击打开对应面板；面板显示独立指标。未知值显示“未取得”，累计计数明确标注。缓冲估计只按 48kHz 转换，不表示声音端到端实测。导出仅包含白名单数值和状态；排除邀请、令牌、证书、私钥、地址、设备名称和本地绝对路径。

键盘基础由 egui/accesskit 保留，真实中文 IME 与 VoiceOver 在 macOS 验证后记录证据，不能从编译成功推断完整辅助功能达标。

## 多路 AirPlay v2

新版桌面通过 `Request::AirplayV2` / `airplay-v2` / `/v2/airplay` 读写，旧单路 Request 保留兼容。来源列表以 source ID 关联，连接计数只统计 sessions。设备管理支持离线撤销；播放方式仅来源无活动会话时修改。Mixer 的实际 stream ID 对应电平，source/session ID 对应控制；过期会话不重试断开、撤销或混音。状态冲突刷新后由用户核对，不转向同入口的新来源。

接收关闭后可选择 1–4 个入口；入口故障只显示本行，其他来源仍可调音。恢复配对需明确选择入口并确认，其他入口的撤销状态不自动恢复。配对码只通过认证管理员回复逐入口保留，常规后台日志与诊断导出递归脱敏。macOS 单元与合成快照验证不等于真实 Apple 多设备验收。

管理员可在入口停止且无活动/预留时 `rename_receiver`（1–50 UTF-8 字节、非空白、无控制字符、名称唯一），下次启动使用新发现名，身份/配对不变。`alias_source` 为来源设置1–128字节别名或以null明确清除，不改变source/session控制键。UI行内保存只在合法且已修改时可用，服务端再次验证；失败和轮询保留草稿，仅匹配该对象/文本的成功回复结束编辑。

状态条固定36逻辑像素、保持单行；长错误只截断视觉文本，完整消息保留hover与可访问值，不能扩大底栏挤走正文。600×440最小窗口有回归测试。

## UI 语言与偏好

Owner：`crates/i18n` 管理 manifest、schema、Fluent、格式与回退；`apps/desktop/src/localization.rs` 管理系统候选、IME、帧边界提交与原生菜单通知；`preferences.rs` 管理私有目录、限长读取、并发读改写及同目录原子替换。产品边界见 [UI设计](docs/UI-I18N-DESIGN-20261007.md)、[桌面IPC](docs/DESKTOP-IPC-CONTRACT.md)。

默认 Auto，按完整系统偏好列表选择支持的语言，显式script优先；繁体系统不会自动误选简体。手选覆盖系统，回到 Auto 立即重查；启动、窗口重新激活和设置入口重查，不每帧查系统。检测异常保留当前语言，空/无匹配列表使用英文。选择框双语标签 `语言 / Language`，语言自称来自 manifest，shipped 才进入菜单。

切换在下一UI帧边界统一生效，IME合成期间延迟提交，不发送音频命令。保留页面、选择、展开、滚动、搜索、草稿、确认目标、还原Instant/revision与在途操作；现有动态历史和异步完成使用当前语言渲染。命令选择以Cmd身份保持；窗口标题仍为NeonMix。原生MenuItem原位更新，不重建托盘或重复注册handler。

语言和动效setter共用部分更新owner，不互相覆盖。旧文件缺language按Auto，未知language保留但提示恢复，未知高version拒绝写入。损坏文件启动不覆盖，明确保存时保留同目录损坏备份。保存失败保持当前窗口选择并提供重试，不把失败当持久化成功。预览模式只使用内存偏好与注入候选，不读写真实用户偏好、不连接后台。未知后台文字仅显示本地化通用提示；有效fault优先，允许参数受白名单限制。

辅助功能与绘图共用当前语言。AccessKit回调只取预先计算的名称，禁止在egui持锁回调中再次读取Context；重要长值可通过AX与tooltip完整读取。资源完整/自动测试/截图不替代各目标平台的真实菜单、输入法与读屏验证。


## 生命周期确认

Canonical owner：`main.rs::freeze_lifecycle/call/begin_urgent`、`desktop-service::Client` 与独立 lifecycle endpoint；依据稳定性计划 D04 和桌面 IPC 合约。Start 在用户操作时冻结后台实例与停止代次；操作选项不在发送时改取当前选择。Stop、停止共享及退出后台沿同一独立路径，启动期间也立即可达，关闭窗口仍保留声音。

接受停止显示“正在停止”；只有同一 operation 的真实回收确认才显示“发送已停止”或“共享已停止”，退出后台也等完成确认。IPC 丢失/超时显示“停止结果待确认，请刷新状态或重试停止”，保持窗口及原状态。双 IPC 失败不当作退出成功。已确认停止代次比在途旧 Status 更新；旧轮询不恢复运行标记。继续使用共享状态条/按钮进度、双语资源、tooltip 与 AccessKit，不新增颜色或布局。

共享启动期间，Hub 开关、现场主操作和快速操作均可发出停止；停止同时取消尚未发送的同类启动。开关的进度与可点击性独立，不用等待启动完成才能停止。普通后台操作执行期间最多保留一个冻结参数的后续操作，同类重复点击不重复提交，容量已满明确反馈；停止仍走独立通道。混音意图继续使用 P02 按字段调度。

刷新与交互：轮询先读取 AirPlay/诊断，最后读取控制快照，各自保留真实接收时间；旧 runtime 的辅助数据不用于新房间。正常请求在途时，已有控制状态的输入可用期最多延续到 IPC_TIMEOUT（45 秒），只接受绑定原上下文的待提交意图；实际混音意图派发仍要求 4 秒内的权威快照。短暂快照读取失败保留原显示，但立即停止基于该次失败发送命令；原状态超过 4 秒后禁用，单纯开始重试不能重新启用。权限/身份错误、停止和上下文改变立即使控制不可用。诊断失败独立标记，不使成功的控制快照失效。


## 输出绑定对象条件（P06）

保存名称、启用、停用和移除使用读取时的 output UUID/revision；确认框携带同一对条件。后台不会把旧指令的对象条件改取当前绑定。对象重建即使revision复用也显示“此输出绑定已被替换，请刷新后再修改”。缺少有效UUID的旧只读资料不能发管理动作。名称保存后的原生应用使用原保存回执条件，另一个对象已经取代它时保留回执并显示同步未完成。


## 保存与音频应用

Canonical owner为commands.rs的应用观察与command_note，仍使用widgets/fader、固定底栏、共享双语Message。已保存不当作已可闻：同runtime的新callback序号到达目标前显示待应用；两秒未到显示检查输出/重试共享提示，诊断未取得则进度未确认。通道行内保留文本，完整长值可经tooltip/AX读取，不只依赖底栏；行高16/底栏36不因文案改变。新操作继续按单字段调度，不禁用整页，应用观察不发第二次写。目标/房间改变使旧观察失效，其他管理者更新当前值时不把旧目标宣称为已应用。


## 独立诊断与发送目标（P08）

Canonical owner：measurement.rs的ObservationClock/CounterHistory、lanes.rs的共享通道模型与pages/diagnostics.rs的健康解释。控制快照、远端诊断、AirPlay会话、本机Sender采集各自计时；超过三次对应轮询周期为Stale，一次读取失败即不可用。详情保留上次数据时明确标出年龄；必要字段缺失显示未取得，合法零值与无新采样/有效静音分别显示。累计错误与相邻有效样本增量分开，历史错误不永久标为当前故障。Mixer的Starved来自当前诊断，不从worker交包推断；现场和Mixer复用同一Lane状态。维持原磁贴、note、tone、正文滚动及tooltip/AX。

Sender真实目标来自后台锁存的sender_started认证Hub/设备/房间名；当前控制房间与身份不覆盖它。目标未确认显示正在确认发送目标，远端输出/通道/电平只采用相同Hub的快照和目标设备。旧组件缺少metrics_age_ms时不把反复读取的累计值当作新采样；相同采样序号不制造新计数增量，Sender/后台实例变更清空计数基线。脱敏导出只新增数值age/sequence字段，不导出sender_target、Hub/设备身份或房间名。实际IME/读屏与其他平台仍单独验收。

电平的sample_age_ms另取实际音频publication时钟；诊断GET持续成功不能刷新冻结callback的样本。measurement.rs::meter_measurement统一服务现场/顶栏/行式/控制台，年龄超限显示空电平与未知输出健康，不影响独立有效网络/采集读数。
