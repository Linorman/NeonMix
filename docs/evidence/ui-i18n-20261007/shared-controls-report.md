# P3/P5 共用 widgets、flow、viz 实施记录

日期：2026-10-07。沿用原有 Neon Console 的 egui 视觉、布局、动作和动效；无新增 UI 框架。

已完成：27 条共用消息的 schema + 中英文 Fluent 资源；全部生产文案迁移；字段内部 ID 与 caption 分离；card rail/pill 的动画 ID 不再用标题/显示文字；toggle 已由原有 next_auto_id 形成稳定 ID；panel/slider/flow/pipeline 已有独立业务/步骤 ID 保持。kv_grid 支持 owned 翻译标签；Source facts 改为 Vec<(String,String)>。flow 来源、房间、离线汇总的 AccessKit 名称即时翻译，新增实体输出节点的 AccessKit 名称。You 角标按翻译后实际文本宽度预留空间；用户名称、数字、dB/dBFS、AirPlay/Sender 等技术标识保持来源语义。

接口：`field(ui,id_salt,label,value,secret)`，`field_sized(ui,id_salt,label,value,secret,max_width)`，`title_field(ui,id_salt,label,value,hint)`；id_salt: impl Hash 必须由调用方给稳定业务 ID。`Source::accessible_label(&Localizer)` 供纯方法延迟翻译；`kv_grid<L:AsRef<str>>` 同时接受(&str,String)与(String,String)。现有 Hub 数据类型没有 facts 字段，没有擅自扩展。

验证：生产文件中中文 grep 只剩注释；没有显示文案参与 make_persistent_id。新增3项 widgets 跨frame回归测试验证普通字段/行内标题切语言后的ID、focus、draft保持，同caption不同业务ID不冲突；新增 flow 双语访问名称与用户字面参数测试。完整 desktop --tests 已可编译；`tools/dev cargo test -p neonmix-desktop localization_tests -- --nocapture` 的4项本分工回归测试全部通过。主 Agent 最新整suite为43项通过。修复了本分工的临时String借用、AX Fn捕获，以及egui AccessKit回调已持ctx锁时不得再次调用text(ui)/renderer(ctx)的问题：全部访问名称现在在widget_info之前计算，回调仅返回已经翻译的String。cancel_modal/sidebar原生AX树测试单跑通过；live_graph测试完成，原断言已由主 Agent按Fluent隔离符规范更新。

不能以此记录宣称真实IME组字、VoiceOver/Windows/Linux读屏或托盘验收完成；这些由整体P6实机验收记录。布局状态由现有项目native/egui测试和最终截图检验。


2026-10-07原生回归补充：真实macOS发现ComboBox键盘Space选择后弹出列表未关闭。共用select_value现在返回Response并消费已处理Space/Enter、显式关闭Memory popup和menu_state。新增 `keyboard_combo_selection_closes_popup_and_returns_focus`：4帧及request_discard模拟通过，值改变/关闭/focus/keyup和Space消费均检查。fresh debug及release冻结包b0531807...da5da5的54次原生切换均已通过；详情见native-macos-report.md。主Agent最新whole suite数量以其最终日志为准。
