# UI 翻译维护

首期语言为简体中文与英文，默认 Auto。开发入口必须使用 `tools/dev`（Windows使用dev.ps1），缓存、临时fixture和产物在项目内。

文案归 `crates/i18n/locales/{tag}/{module}.ftl`，语义ID、参数类型与必要属性归 `messages/{module}.toml`，统一索引为 `messages.toml`。新增文案先登记业务语义，再写完整句子和参数，不以原文作key、不动态拼接key、不在页面增加locale判断。生成Rust Message接口，构建同时校验完整FTL AST。参数string是原始用户/系统字段，message是可重新渲染的语义消息，计数u64，技术计量f64；不得传token/PIN/私钥/stderr。

1. 在manifest新增语言，先draft，填写标准tag、自称、direction和明确alias。编译期根据shipped登记生成ResolvedLocale及菜单数据，draft不进入UI或Auto。
2. 补齐全部模块、必要属性、完整操作后果和参数含义。中英文复数分支可不同；恢复指引和危险对象不能因短标签省略。
3. `tools/dev python3 tools/check_i18n.py --strict` 检查重复key、缺失、传递参数、引用循环、函数白名单及Rust展示调用点。允许函数必须实际实现后加入白名单；不能因FTL可解析就使用NUMBER/DATETIME。精确例外必须记录file/line/value/category/reason，不豁免目录。
4. `--pseudo --report artifacts/i18n/pseudo-report.json` 生成独立英文AST文本扩展资源，保留占位符、品牌、SI及selector。伪本地化只在测试目录，不覆盖shipped或设备名。
5. 覆盖字体/塑形/混合脚本、最小窗口与200%缩放、键盘/IME、原生托盘/菜单/读屏以及播放期间切换；RTL必须单独验证egui塑形和布局，不能仅改direction发布。
6. 完成母语审校和目标端验收，才改shipped；补Auto匹配、LocaleFormat/字体适配测试和证据。新语言无需逐页修改业务文案。

删除key、错参数、重复key和循环引用的破坏性测试由Rust临时catalog运行，不能修改正式资源后忘记恢复。资源随include_str嵌入，build.rs追踪manifest/schema/每个FTL；增量修改必须触发重建。回退用于故障恢复，不能替代发布前资源完整度100%。

回退使用完整已验证旧包，保留Hub身份、配对与输出资料。旧客户端可能忽略或重写UI语言字段（重新采用旧版中文行为），这一偏好限制不改变音频协议与身份资料。发布范围由当前checklist/STATUS和平台证据决定。
