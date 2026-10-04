# NeonMix 桌面行为契约

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

发现结果不可信（ADR-011）；首次配对使用含固定身份的一次性邀请，私密邀请文本默认遮挡。正式长期凭证由平台 vault 管理。

Sender 的发送范围始终显示为绑定虚拟输出上所有应用的声音；系统默认选路由用户自己选择（输出绑定合约）。没有麦克风或应用级捕获快捷默认。

关闭窗口隐藏界面并保留后台，UI 崩溃不停止音频。停止发送只停止 Sender，停止共享只停止 Hub；退出后台明确结束本实例全部音频。没有自动重启被用户停止或管理员禁止的会话（规格 §8）。

## 状态与导航

快捷键提示统一由 `widgets::command_hint` 生成：macOS 使用 `⌘`，Windows/Linux 使用 `Ctrl+`，与 egui `COMMAND` 的实际行为一致。隐藏/退出快捷键当前仅 macOS 原生菜单提供，其他平台不展示提示。

左侧导航栏固定七个页面入口（现场、Mixer、Hub 设置、Sender、设备管理、诊断为 ⌘1–⌘6，关于无快捷键），默认打开「现场」；底部固定控制身份切换、「停止发送」（发送中）、并排的「隐藏窗口」「退出后台」，任何窗口尺寸下不随正文滚动，600×440 下与导航不重叠（有回归测试）。顶部栏显示房间及其状态，非 Mixer 页附带总输出电平、总音量与总静音；「快速操作」（⌘K）列出当前身份可执行的操作。各页共享底部状态条的 pending/error/success 消息。字段在切换页面、失败、后台断线时保留；不自动保存。刷新保留上次快照但显示过期，不允许以过期 revision 写入。每页空态均引导到所属入口；按钮忙时不可重复提交。

有界列表使用 ScrollArea；设备搜索本地即时过滤，带清除按钮。无匹配与未发现分别提示。

每页先给一个主视图与主操作，未完成的设置以步骤条展示，细节放在可折叠面板；默认展开跟随数据，用户选择优先。操作进行中不禁用整页，只有触发它的按钮显示进度；写入期间的输入保留最后一次意图，按下一份快照的 revision 发送。

危险确认使用 egui Modal，展示对象和影响，默认取消、Escape 取消，下一帧返回当前页导航；不引用已移除的控件焦点。全部动作由按钮触发，不依赖 hover。字段默认不因 Enter 提交，避免 IME 合成提交。

## Mixer

宽窗为通道条控制台（竖向电平与推子、总控条在右），放不下时每一路一行：电平、推子、增益读数、静音、Solo、详情。推子相对拖动，单击不改变增益；双击回到 0 dB；聚焦后沿行程方向的方向键步进；滚轮仅在聚焦或按住 Option 时生效。控制台 ←/→ 选择通道、↑/↓ 调音量；行式 ↑/↓ 选择通道、←/→ 调音量；M/S 静音/Solo，文本输入或对话框时不响应。「现场」页单击来源与 Mixer 共用所选通道，检查器内的推子、静音、Solo 与 Mixer 规则相同。混音调整 8 秒内可「还原」（⌘Z），以新 revision 发送反向操作；措辞与「撤销配对」区分。不可闻的通道淡出但仍可操作。快速操作在输入法组字时不因回车执行。

## 现场与动效

信号汇流图只画权威快照、诊断电平与 AirPlay 会话：未取得的电平画空心环、无会话状态画灰色虚线，不补齐、不演示。光点、雷达等连续动效只在对应真实量变化时运行（20 fps 上限），无信号、窗口隐藏或开启「减少动态效果」（关于页，或 `NEONMIX_REDUCE_MOTION=1`）时不申请连续重绘。房间动态与电平/指标历史只存在当前窗口内存，切换身份清空，不进入导出或状态文件；`ui-preferences.json` 只保存减少动态效果开关。

## 诊断与验证

四个健康磁贴概括播放与输出、媒体网络、缓冲与 Sender 采集，点击打开对应面板；面板显示独立指标。未知值显示“未取得”，累计计数明确标注。缓冲估计只按 48kHz 转换，不表示声音端到端实测。导出仅包含白名单数值和状态；排除邀请、令牌、证书、私钥、地址、设备名称和本地绝对路径。

键盘基础由 egui/accesskit 保留，真实中文 IME 与 VoiceOver 在 macOS 验证后记录证据，不能从编译成功推断完整辅助功能达标。

## 多路 AirPlay v2

新版桌面通过 `Request::AirplayV2` / `airplay-v2` / `/v2/airplay` 读写，旧单路 Request 保留兼容。来源列表以 source ID 关联，连接计数只统计 sessions。设备管理支持离线撤销；播放方式仅来源无活动会话时修改。Mixer 的实际 stream ID 对应电平，source/session ID 对应控制；过期会话不重试断开、撤销或混音。状态冲突刷新后由用户核对，不转向同入口的新来源。

接收关闭后可选择 1–4 个入口；入口故障只显示本行，其他来源仍可调音。恢复配对需明确选择入口并确认，其他入口的撤销状态不自动恢复。配对码只通过认证管理员回复逐入口保留，常规后台日志与诊断导出递归脱敏。macOS 单元与合成快照验证不等于真实 Apple 多设备验收。

管理员可在入口停止且无活动/预留时 `rename_receiver`（1–50 UTF-8 字节、非空白、无控制字符、名称唯一），下次启动使用新发现名，身份/配对不变。`alias_source` 为来源设置1–128字节别名或以null明确清除，不改变source/session控制键。UI行内保存只在合法且已修改时可用，服务端再次验证；失败和轮询保留草稿，仅匹配该对象/文本的成功回复结束编辑。

状态条固定36逻辑像素、保持单行；长错误只截断视觉文本，完整消息保留hover与可访问值，不能扩大底栏挤走正文。600×440最小窗口有回归测试。
