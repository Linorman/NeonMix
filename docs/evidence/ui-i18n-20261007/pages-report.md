# UI i18n 页面与语义状态实施记录

日期：2026-10-07。所有迁移脚本、构建日志和开发产物均保存在项目目录，开发命令通过 `tools/dev`。

## 已完成代码

- [x] 七个导航页面、Mixer console 布局和 Hub 内嵌 AirPlay 面板的静态、动态、空态、tooltip 与可访问名称使用类型化 Message。
- [x] 页面所有 field / field_sized / title_field 调用使用独立稳定 ID。
- [x] 页面确认框保留完整对象与危险后果，并传递语义 Message。
- [x] 角色、Lane 状态和媒体网络状态以 Message 持有；Mute / Solo / 音量反馈与 Undo 标题保留语义参数。
- [x] 事件保留事件种类、事件时名称、语义状态及原始时间；渲染时翻译，差异比较不依赖译文。
- [x] 相对时间以及英文整数计数采用 Fluent 资源和 one / other 分支。
- [x] value_text 通过 Desktop Localizer 翻译缺失值与布尔值；JSON、设备 ID 和其他原始技术内容保持原值。
- [x] About → Display 接入 language_settings(ui) 入口；动效偏好保持既有业务调用。
- [x] Sender discovery 的 radar hash 直接使用原始 hub_id，避免显示语言成为动画身份。

## 资源数量

| 功能域 | 消息 |
|---|---:|
| live | 48 |
| mixer | 52 |
| hub | 99 |
| sender | 79 |
| devices | 26 |
| diagnostics | 69 |
| about | 16 |
| lanes | 34 |
| events | 13 |
| 合计 | 436 |

## 审校

停止共享保留已配对设备与设置；撤销设备配对明确立即终止媒体/控制连接并使旧凭证失效；AirPlay 撤销明确覆盖所有入口；重新配对明确不恢复其他入口的已撤销绑定；忘记本机配对明确停止发送、禁用本机输出且保留 Hub 记录；删除输出绑定保留系统音频驱动。中英文均保留这些后果。

保留的源码字面值属于精确例外：NeonMix / AirPlay / Hub / Sender 品牌或组件名；ID、dB、dBFS、ms、ppm、RMS、PLC、PCM 和 UTC/采样率等技术符号；JSON 键、命令 ID、稳定 widget ID、文件路径和协议枚举；AirPlay tests 的中文 UTF-8 边界、草稿、别名 fixture。普通 Solo 动作也已登记 Message。

## 验证状态

- [x] 指定 Rust 源文件经项目工具链 rustfmt 格式化。
- [x] 源码扫描确认页面、lanes 与 events 生产代码没有未解释的中文产品字面值。
- [x] cargo check --all-targets 中页面、lanes、events 自有代码无类型错误。
- [x] 八项 i18n UI 交互回归测试串行通过：`artifacts/i18n/interaction-tests.log`，8 passed，0 failed，0.85 秒。
- [x] 主 Agent 全量 targeted tests 通过，desktop 51 passed；包含精确光点端点回归。证据：`artifacts/i18n/final-tests.log`。
- [x] 主 Agent clippy 与严格资源检查通过；资源 AST / schema / Rust 呈现调用检查 712 messages、2 shipped locales，证据：`artifacts/i18n/clippy.log`、`artifacts/i18n/strict-report.json`。
- [x] Headless 中英文、七页与 AirPlay 的空/完整/在途/过期状态矩阵；当前帧 AX 名称与来源节点身份、长用户数据、600×440/1100×760/1600×1000 及 200% native scale 布局回归。
- [ ] 双语真实截图、原生菜单、真实 AX / IME 和三平台验证（主 Agent 汇总）。

新增事件回归测试 switching_locale_rerenders_existing_history_without_new_events：同一语义历史在中英文间重渲染，保留中文设备名与 Instant，确认无新事件及资源回退诊断。既有事件顺序/保留数量测试改为比较语义身份。

独占改动包括 apps/desktop/src/pages/*.rs（主 Agent 未分配页面之外除外）、lanes.rs、events.rs 与上述九域资源；保留原有 AirPlay 身份 v2、名称草稿、请求目标和 UTF-8 字节上限逻辑。

ProcessStatus 错误展示优先使用结构化 fault，旧后台仅允许稳定 machine code 映射。诊断 Engine.errors 不作为自然语言参数直接显示；仅非空数组显示本地化计数与输出恢复指引，避免空列表误报。

## 本轮回归发现及修复

1. About 语言菜单的 Space 打开正常，但 Escape 后焦点消失。Localization 持有菜单打开状态，菜单关闭后请求原 ComboBox 焦点；两语言键盘打开/关闭、原偏好不变回归通过。
2. 三组 AccessKit 回调迁移中，flow 图节点、viz pipeline / countdown、Mixer disclosure 的 Context 查询均移到 widget_info 之前。Mixer disclosure 的嵌套查询在完整页面矩阵中实际造成重入 egui Context 锁与阻断；最终回调只读取现成可访问文字，串行 128 场景矩阵全部完成。此类回调禁止嵌套 Context 查询。
3. Signal flow 的静音入口光点在 `t == end` 时，f32 `sin(PI)` 产生微负 envelope，触发 Color32 debug assertion。仅把光点 envelope 限制为 0–1，保留其他光效；新增精确 gate 端点绘制回归测试。
4. 测试缩放使用 RawInput viewport 的 native_pixels_per_point，保持 screen_rect 为逻辑坐标并显式断言逻辑窗口与 scale。200% 的 600×440 逻辑窗口对应 1200×880 物理像素。避免 pending zoom 改写 RawInput 与人为倍增视窗制造假越界。
5. Palette 的 egui selected WidgetInfo 在 AccessKit 表达为 toggled 属性；回归从当前帧选中节点验证真实查询、选中命令、切换语言、Enter 后正确 Discover 请求，不访问私有 Cmd 或索引。

`i18n_tests.rs` 包含 50 次切换保留草稿/目标/计时与零额外控制请求、中文发起英文完成的成功/失败、来源 AX ID 稳定、跨帧字段焦点与字符选择/展开/滚动、双语页面状态/长名缩放、语言菜单 Escape 与 Palette 命令身份，共八项测试。界面模型不会伪称原生菜单、真实读屏或输入法已验收。

最终矩阵口径：2 语言 × 8 展示面（七页 + AirPlay）× 4 状态 = 64 场景；2 语言 × 8 展示面 × 4 窗口/scale 组合（600×440@1×、600×440@2×、1100×760@2×、1600×1000@2×）= 64 个长名场景。128 场景均通过。新增最小窗口 200% 定向回归：`artifacts/i18n/long-scale-tests.log`，1 passed，0 failed，0.58 秒。

最后主任务工作区补验：共享操作意图收口后的联合113 tests、严格Clippy、格式与724/20资源检查已通过，日志final-worktree-*和summary.json。该结果证明新的工作区版本，不改变本报告已记录固定candidate/source包哈希的对应关系。
