# UI 多语言实施结果

2026-10-07：Auto、简体中文、English 已完成开发。默认 Auto 按有序系统偏好匹配；七个页面、AirPlay、动态历史、还原与异步反馈、托盘、应用菜单、tooltip/AX 共用当前帧语言，音频实时模块不依赖翻译。偏好 version2 合并写入并保留旧动效设置，含旧 Unix 0644 布尔文件的严格兼容；损坏原件保留、未知高版本拒绝覆盖、失败当前窗口继续生效并可重试。

资源/语言枚举由 manifest 与 schema 生成。724条消息、20个双语模块、14个逐位置技术/品牌例外通过同一 Fluent AST、传递引用/参数/循环/函数/代表分支校验；开发入口已接 tools/check.py。LocaleFormat、解析缓存、字体文件缓存与有界诊断完成；英语仍保留 CJK。FTL方向隔离不画占位框，但未宣称RTL塑形。

本轮实际修正：译文与 ID/Marks 解耦、AX callback 先计算文字避免 egui 重入死锁、Escape 与选择后焦点恢复、Space/Enter 消费避免 discard pass 重开 popup、英文窄窗顶部栏导致正文偏移、长房间胶囊边界、光点端点负浮点透明度、真实旧偏好文件模式迁移。

## 验证

当前工作区针对 desktop/service/i18n 的 **113 项测试通过、2 项已有专用原生fixture测试忽略**，严格 Clippy、格式与资源检查通过。期间另一操作意图改动曾造成临时编译/旧断言失败，最新联合验证已经收口；历史日志不被当前结果替代。

macOS 冻结release 54次真实选择/语言切换、popup关闭/焦点/Escape、双语原生菜单/单托盘、Cmd+W隐藏恢复、Cmd+Q唯一确认及3组Auto启动通过，RSS首尾下降约22.8MiB，无持续增长。Windows固定源码原生编译、核心/桌面测试及最新12项定向测试、真实系统候选 `[zh-CN,en-US]` 与Auto通过；未将headless AX当真实读屏。Linux离线交叉类型检查通过，原生节点不可达。

64个状态场景及64个长名/缩放场景、包括600×440逻辑点200%通过；48张双语三种窗口尺寸截图位于 `artifacts/i18n/{zh-CN,en}/{600,1100,1600}`。后续keyboard-only修复与旧偏好兼容不改变这些正常绘制截图，实际菜单/选定态截图见native-macos目录。

## 性能测量

下表是同机1100×760的空/四路公开fixture预览，3秒预热后4次1秒采样，CPU为一个核心百分比。它证明UI观察值，没有运行真实媒体；短样本受焦点/系统负载影响，不能由下降宣称音频优化或长期门槛通过。

| UI | 场景 | 平均CPU | 最后physical footprint |
|---|---|---|---|
| baseline | idle | 0.55% | 289.6 MiB |
| baseline | four-inputs | 5.77% | 304.9 MiB |
| zh-CN | idle | 0.47% | 290.6 MiB |
| zh-CN | four-inputs | 2.67% | 305.2 MiB |
| en | idle | 0.47% | 310.3 MiB |
| en | four-inputs | 4.31% | 292.1 MiB |

双语资源冷解析 0.445 ms；1,144个代表消息首次格式化约0.26–0.31 ms，暖格式化约0.12–0.14 µs/条；50次切换并全量代表渲染 9.89 ms，fallback诊断0。独立基准测量见 resource-performance.json，不含egui帧提交。

## 开发候选与发布边界

`artifacts/i18n/package/NeonMix-0.1.0-macos-arm64.dmg` 包含40个完整FTL嵌入模块；52个arm64 Mach-O验签/相对依赖、非ASCII移置、离线资源与只读CLI、DMG验证通过。签名仅ad-hoc，未notarized、未安装/启动wrapper。包严格关联构建时 source manifest；随后共享源码改动及最新工作区绿测试单独记录，不能宣称该包就是持续变化工作区的所有最新提交。

**本轮交付限定为开发源码和macOS开发候选，尚不宣称三端正式发布。** 真实IME/VoiceOver及Windows/Linux读屏、Windows真实菜单和中文用户目录升级、Linux GUI、真实播放期间20次切换、完整安装升级/签名仍待对应环境验收；checklist这些项保持未勾。后续增加语言是流程模板，不是首期欠项。

证据：[汇总](evidence/ui-i18n-20261007/summary.json)、[核心](evidence/ui-i18n-20261007/core-report.md)、[Windows](evidence/ui-i18n-20261007/windows-report.md)、[macOS原生](evidence/ui-i18n-20261007/native-macos-report.md)、[包](evidence/ui-i18n-20261007/package-report.md)。
