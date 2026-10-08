# v0.2 安装包替换（2026-10-09）

按用户要求，从修复提交 `571cceaa3083f3a9a48b5b841fc3fb3bcc22f1f5` 重建 macOS ARM64 与 Windows x64 安装包，并替换 [GitHub v0.2](https://github.com/Linorman/NeonMix/releases/tag/v0.2) 的同名附件。两端 `--version` 均为 `neonmix-desktop 0.2.0 (571cceaa3)`。原 v0.2 标签保持，发布说明明确安装附件对应的新提交。

- 两端原生 `cargo build --release --workspace --locked` 完成；macOS worker 重建，Windows worker 使用匹配提交的现有构建目录重建检查。
- macOS DMG 解包的 55 个文件与已签名 `.app` 一致，`codesign --verify --deep --strict`、`hdiutil verify`、中文/空格路径搬移与包内双路媒体检查通过。
- Windows NSIS EXE 解包的 81 个文件与 manifest 一致；实体 UA2 上的包内后台→Hub→AirPlay 启动成功、ready/published，关闭后台后无运行密钥遗留。
- 原附件已下载并按旧 `SHA256SUMS.txt` 核验，保存在项目 `artifacts/release-v02-20261009/previous`。通过 `gh release upload --clobber` 替换两个安装包、两个 JSON 清单和 SHA256 清单，然后更新发布说明。
- GitHub API 返回的五项附件大小与 SHA256 digest 全部匹配本地，发布说明也逐字核对；见 `publication.json`。签名状态维持 macOS ad-hoc/未公证、Windows 未签名。

本地最终制品与日志在 `artifacts/release-v02-20261009`。Windows 构建、缓存、资料和输出均在 `E:\Desktop\NeonMix-test` 内，最终包位于其 `release-v02-20261009` 子目录。未执行安装、卸载或更改用户真实配对资料。解包临时目录与合成测试资料已清理；保留最终制品和回退备份。
