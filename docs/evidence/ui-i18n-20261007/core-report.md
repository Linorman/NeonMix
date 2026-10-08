# P1 本地化核心与检查器验收（2026-10-07）

已实现离线嵌入的 Fluent 双语资源、类型化 Message、纯函数语言解析、共享并发 Localizer、统一 LocaleFormat、资源/调用点检查器、原位刷新的原生菜单以及有界字体文件缓存。最初资源检查为 712 条 schema 消息、19 个功能模块；并行 intent 整合后的固定功能快照为 724 条消息、20 个模块、zh-CN/en 两个 shipped 语言；当前 typed 代表消息渲染诊断为零。

## API 与维护入口

`LanguagePreference::{Auto, Explicit(String)}` 按字符串 serde，默认 auto，未知值原样保留。`resolve(&preference, &[String])` 返回 `Resolution { locale, reason }`，理由区分手选、系统列表命中位置、未知手选值及全列表无匹配。语言扩展先经过 language-tags BCP 47 解析；显式 script 优先，繁体候选不误映射简体，未知首候选继续下一项。

`ResolvedLocale` 及 `tag/native_name/shipped/from_tag/direction` 全部由 manifest 生成，保留首期 `ZhCn/En` 兼容名称。draft 不进入生成的 shipped enum、UI 列表、Auto 和嵌入资源。匹配别名唯一来源为 `locales/manifest.toml`，更改 manifest/schema/FTL 及添加文件均触发构建检查。

Message schema 使用 `messages.toml` 索引及 `messages/*.toml` fragment。支持 string、u64、f64、message；最后一项生成 Box<Message>，渲染时递归使用当前 Localizer，避免还原/状态反馈在切换后冻结为旧语言。属性生成独立类型化 variant。

Localizer 是 Send+Sync，可由 egui Context 分享 Arc。所有实例从 OnceLock 分享已解析并发 bundle；Clone 不重新解析。静态缓存最多 1024 条，动态参数不缓存，切换才递增 generation 并清空静态缓存；诊断去重并限制 128 条，只保存 locale、schema identity 与原因，不保存用户参数或格式化错误内容。回退有界为当前语言→英文→固定英文失败提示。

LocaleFormat 提供整数、带符号整数、小数（最多 6 位）、百分比、相对时间及普通日期。非 finite 数值、溢出百分比和非法日期显示当前语言 Unknown。公历日期使用 ISO 顺序，由调用方提供已换算日期及明确 timezone；模块不读取实际系统 locale/时区。相对时间消息复用 Events 类型，由调用方持有的 Localizer 渲染；计量两语言共用点号小数与 SI 符号，不改变 JSON/端口/复制值。

## 验证结果

- `tools/dev cargo test -p neonmix-i18n` 在开发机阶段全部通过；Windows 最终完整原生执行为 23 tests 全部通过，覆盖候选矩阵、未知偏好 round-trip、manifest/menu 元数据、嵌套重渲染、缓存/回退、字面参数隔离和检查器破坏性夹具。
- `tools/dev cargo clippy -p neonmix-i18n --all-targets -- -D warnings` 通过。
- `tools/dev python3 tools/check_i18n.py --strict --pseudo --report artifacts/i18n/final-strict-pseudo-report.json` 最初通过，712 消息/19 模块；最终固定 Windows 快照同入口通过 724 消息/20 模块（见 windows-immutable-strict-report.json）。Windows 下 Python 正确复用 dev.ps1 环境，例外路径比较兼容反斜杠。
- 删除英文键、重命名参数、重复键、语法错误、未知引用、message/term 循环、遗漏必要属性、数值参数错类型、未知函数和 term 绑定契约错误均被同一 Rust AST 检查稳定拒绝；CLI 返回非零。
- Rust syn 对展示调用点进行检查，包含 egui 文本/tooltip/ComboBox、WidgetInfo、Painter 和原生菜单。14 个例外均有精确文件、行、值、类别与理由，仅品牌、标识/单位及 Mute 技术标记；不豁免目录。此检查不宣称任意变量的数据流全程序证明。
- 伪资源由 Fluent AST TextElement 扩展约 50%，保留 selector、参数、品牌和技术词。生成前对整个伪 Catalog 运行相同 schema/引用/代表参数渲染验证；仅写入 artifacts/i18n/pseudo/en，未进入 shipped 菜单。
- 50 次核心切换证明 bundle Arc 指针保持、generation 正确、动态缓存为空。桌面更完整的 50 切换状态/请求测试见 Windows 报告与主验收报告。

## 原生菜单、字体与方向隔离

Tray::new 接收 Localizer，托盘与 macOS 应用菜单保存 MenuItem 句柄；update_locale 原位更新 label/tooltip，不重建图标或注册新 handler，保持原菜单 ID、Cmd+W/Cmd+Q 与 Windows 稳定窗口标题/唤醒通道。Mac/Windows 编译通过；人工菜单点击、隐藏恢复及读屏属于单独原生验收，不能从编译推断。

字体 cache 按 canonical PathBuf 分享一次读取的 process-lifetime bytes，OnceLock+Mutex 最多 8 个文件；普通/粗体同文件分享字节，不因 locale 重新 leak。重复读取修改后 fixture 的回归测试在 Mac 和 Windows 通过。继续只读复用系统字体，保留英文缺字恢复入口。

Fluent 插入的 FSI/PDI 在实际 egui layout 中 advance 为零、没有 glyph texture；拉丁测试边界 kerning 有约 0.2px 差异。epaint 0.31.1 源码仍明确 TODO heed bidi characters，因此本结果确认隔离符不画占位字形，不证明完整 RTL 塑形/重排；首期仍仅 LTR zh-CN/en。

## 证据与边界

最终严格/伪资源报告见 `artifacts/i18n/final-strict-pseudo-report.json`，依赖精确版本/许可见 `dependency-licenses.md`。Windows 原生记录与 Linux 交叉检查结果见 `windows-report.md`。夹具在项目 .local/tmp 中创建后自动清理；依赖缓存、target、报告与截图均位于项目。

sys-locale 的 iterator 不提供原生失败原因。生产适配器现把没有候选视为不可用：首次初始化使用英文，已有 Auto 窗口保留当前语言；注入的真实 Ok(empty) 路径仍按纯规则回退英文。该保守适配不宣称能够原生区分空列表和 API 错误。真实平台 IME/AX/播放/托盘、CPU/帧耗时及发布支持范围以主报告实际执行结果为准。

最终原生补验已覆盖 shared select_value 在 request_discard 重复 pass 时消费 Space/Enter并关闭popup、返回焦点；偏好先于 UI 初始化、旧 Unix 0644动效文件的限长/UID/非链接兼容规则由主任务记录。原始不可变 source 保留，14个技术例外位置更新为独立Windows overlay，不修改并行owner业务实现。

最后主任务工作区补验：共享操作意图收口后的联合113 tests、严格Clippy、格式与724/20资源检查已通过，日志final-worktree-*和summary.json。该结果证明新的工作区版本，不改变本报告已记录固定candidate/source包哈希的对应关系。
