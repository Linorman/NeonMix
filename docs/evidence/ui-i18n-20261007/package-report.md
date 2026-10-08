# macOS 本地开发候选包与离线迁移验证

日期：2026-10-07。当前交付是 Apple Silicon macOS 的 **固定 UI 开发候选包**，未安装、未启动 wrapper / 原生 GUI、未替换真实身份或配对资料。签名为 **ad-hoc，未 notarized**。

最终候选位于 `artifacts/i18n/package/NeonMix.app` 与 `NeonMix-0.1.0-macos-arm64.dmg`。DMG 30,440,206 字节，SHA-256 `d54ede1c27a500f10183fd35b0ccfd8f435f23dc450518b20bd92889239260d7`。包含最新旧 0644 动效偏好兼容、偏好初始化、Space / Enter 事件消费与 ComboBox 关闭、英文字体恢复及 724 条 / 每语言 20 模块资源；开发工具 i18n-bench 不作为产品程序打包。

## 来源与后续工作区差异

- [x] 普通 desktop / background release 构建通过，40.30 秒，证据：`candidate-product-build.log`。输入文件清单 `candidate-source-manifest.json` SHA-256 为 `e13e897fda5a59b7ab417d0eb18dbce2a06cfd5e7be062ff9b8f6165a29dca2c`；构建开始与结束时清单一致。
- [x] 主 Agent 冻结的 feature UI 候选 SHA-256 `b05318078859552336309ab67a8406ad914b4d6004d7f1d15b9be292c1da5da5` 在普通构建与打包后保持，证据：`preview-feature-final-before.json`、`preservation-after.json`。
- [x] 版本 0.1.0 / Git HEAD `aefb86903` 仅是标识；未提交工作区实际二进制以 `bundle-manifest.json` 和 DMG 哈希为准。

构建结束后、重包完成前，其他稳定性工作继续改动了 commands / intent、main / shell / mixer、control / desktop-service 等 12 个文件，具体 before / after 哈希记录在 `post-build-source-drift.json`。因此此包关联构建时固定清单和冻结 UI 候选，**不等于之后正在变化的完整工作区**。

主 Agent 说明 foreign-intent 整理中的完整旧测试当前仍有 3 FAIL、Clippy 有 5 条告警。此前绿色测试不是这份扩大源码的全绿证据。本轮本地候选普通 release 编译成功，不宣称全套当前测试 / Clippy 全绿，也不宣称完成三端正式发布。

## 最终包检查

- [x] `--output` 限定项目内输出，默认仍为 `artifacts/installers`；`--keep-app` 保留验证后的 app，未覆盖旧 installer。项目外路径拒绝测试返回 2，未创建输出。原 Mach-O / RPATH 测试 7 项通过。
- [x] `NeonMix-0.1.0-macos-arm64.dmg` 经 `hdiutil verify` 通过。没有挂载或安装到系统应用目录。
- [x] bundle 54 文件、52 arm64 Mach-O、29 Frameworks 动态库。desktop / background 两个变化的 Mach-O 新执行签名与相对依赖解析；50 个未变化 Mach-O 逐文件 SHA 匹配此前完整验证结果，复用其 SDK / worker / plugin 验签证据，没有扩大媒体验证。
- [x] 新 app 与项目内中文 / 含空格搬移副本均 `codesign --verify --deep --strict` 通过；迁移后全文件 SHA 匹配，非系统依赖解析无错误。
- [x] 去掉开发用 DYLD / GStreamer 搜索路径后，新 desktop `--version`、background `--help` 在搬移目录只读运行成功，不进入 GUI 或服务工作流。其他三个未变化 CLI 的帮助 / 版本成功证据复用此前 `previous-bundle-verification.json`。
- [x] 新 desktop 二进制逐模块包含完整英文 20、中文 20 个 FTL 原始内容；不是只查几个关键词。外部 `.ftl` 文件数量零，所有 724 条已声明消息属于完整模块内容与构建期 schema / Fluent 校验。
- [x] 旧 `artifacts/installers` 的 Mac DMG、metadata、SHA256SUMS 哈希保持；冻结 b053 UI 副本保持。

SDK / worker 没有改动，前一轮完整 workspace release、实际 worker 构建、52 个 Mach-O 独立签名与全部相对加载引用检查、5 个 CLI 搬移检查均保留历史证据，不冒称本轮重新执行。前一轮 release Localizer 测试副本在非 ASCII / 含空格且无 FTL 的目录执行 50 次切换与参数隔离两项测试通过，机制证据见 `embedded-offline-tests.json`，副本已清理。本轮新 40 个完整资源模块嵌入验证已新执行，没有新的文件路径语言包依赖。

## 未执行

- [ ] 最终包 Finder / LaunchServices 启动、包内 GUI 切换、TCC、原生菜单 / AX / IME。
- [ ] 真实安装、旧安装升级、真实身份与偏好迁移、安装回退。
- [ ] Windows / Linux 本次最终安装包与原生安装升级验收。
- [ ] Developer ID / notarization / Gatekeeper 下载信任。

wrapper 固定使用真实 HOME 下的 Library 数据目录。为避免读写真实偏好 / 身份及项目外中间产物，本轮没有执行 wrapper、没有改写 HOME，只运行 Clap 早退出入口。Info.plist 中已有的中文系统权限用途说明仍保留，未触发 TCC 展示；不能据此声称系统权限展示面已双语验收。

## 复现与证据

```sh
tools/dev cargo build -p neonmix-desktop -p neonmix-desktop-service --release
tools/dev python3 tools/package_macos.py --output artifacts/i18n/package --keep-app --skip-build
tools/dev python3 artifacts/i18n/package/verify_package.py --changed-products-only
tools/dev hdiutil verify artifacts/i18n/package/NeonMix-0.1.0-macos-arm64.dmg
```

验证脚本需要先建立 `搬移 含空格/霓虹混音.app` 副本；变化产品验证复用 `previous-bundle-manifest.json`。最终数据为 `bundle-verification.json`、`bundle-manifest.json`、`candidate-source-manifest.json`、`post-build-source-drift.json`、`preservation-after.json`；最终打包日志 `build-package-candidate.log`，签名信息 `codesign-display.txt`。此前候选 metadata / manifest 以 `previous-` 前缀保留。

本次按 [packaging-notarization 技能](/Users/linorman/.codex/plugins/cache/openai-curated-remote/build-macos-apps/0.1.4/skills/packaging-notarization/SKILL.md) 检查 bundle / nested code 与分发边界。当前候选未 notarized，未做真实安装 / 三端发布，不声称公开分发就绪。

最后主任务工作区补验：共享操作意图收口后的联合113 tests、严格Clippy、格式与724/20资源检查已通过，日志final-worktree-*和summary.json。该结果证明新的工作区版本，不改变本报告已记录固定candidate/source包哈希的对应关系。
