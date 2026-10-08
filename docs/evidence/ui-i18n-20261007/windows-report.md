# Windows 原生与 Linux 交叉验证（2026-10-07）

Windows 原生 Rust 构建、75 项完整测试（desktop 52 + i18n 23）、严格资源/展示调用点检查、伪资源生成及真实系统语言偏好探针全部通过。Linux host 不可达，已完成 macOS→Linux 的离线 cargo check；此记录不把交叉编译写成 Linux 运行验收。

## 隔离与来源

原生 host：`linorman@192.168.100.186`，Windows 11 build 26100，PowerShell 5.1.26100.4768。仅使用 `ssh -F /dev/null -o BatchMode=yes -o ConnectTimeout=5 -o StrictHostKeyChecking=yes -o UserKnownHostsFile=/Volumes/projects-mac/NeonMix/.local/ssh/known_hosts`。

独立副本：`E:\Desktop\NeonMix-test\ui-i18n-20261007`。创建前确认该目录不存在；既有项目、fix 目录及生产状态不被覆盖。Cargo cache/index 从既有项目复制为独立可写缓存，target、TEMP/TMP、Python cache 与日志均为副本内路径。通过 `tools/dev.ps1` 执行开发命令；会话级 rustup resolver 仅返回既有 Rust 1.95.0 编译器，不安装或更改全局 toolchain。只读复用既有 VS2022 Community/MSVC 14.44.35207 和项目内 GStreamer 1.26.10 SDK。

源码 tar SHA256：`824949c95d46ef0a641f3ad7721fe61dc756cae0e6ba31bc3e40438e758fa42a`，上传后逐个验证 659 个文件哈希。初次 desktop 编译发现 tar 排除 docs/evidence 时遗漏 compile-time full.json 夹具；补入并验证 7 个基线 fixture JSON。随后同步并验证 5 个必要最终 overlay 文件：Python dev 编排、Windows 例外路径规范化、native provider 的空 iterator 保守处理、pages 数值格式代理及开发基准工具。

完整组装清单共 667 个唯一文件，见 `artifacts/i18n/windows-final-source-manifest.json`。最初 tar 与两个 overlay 的独立清单均保留；报告不把更新后的 overlay 误认为最初 tar 的内容。传输 tar 与临时 i18n fixture 已清理，隔离源码/cache/target 保留用于复验。

## 实际命令与结果

| 原生命令（通过 tools/dev.ps1） | 结果 |
|---|---|
| `cargo test -p neonmix-i18n --locked` | 初次 23 tests 全部通过；编译 15.04s，含 SSH 共 17.95s |
| `cargo test -p neonmix-desktop --locked` | 补 fixture 后 52 tests 全部通过；编译 7.91s，测试 0.67s |
| `cargo test -p neonmix-i18n -p neonmix-desktop --locked` | provider/工具 overlay 后最终完整 75 tests 全部通过；编译 4.45s，desktop 0.65s，含 SSH 共 8.40s |
| `cargo test -p neonmix-desktop --locked i18n_tests::` | 最后的 pages formatter 代理同步后 8 项定向 tests 通过；编译 3.88s，测试 0.63s |
| `python tools/check_i18n.py --strict --pseudo --report artifacts/i18n/windows-strict-pseudo-report.json` | 712 消息、19 模块、2 shipped locale 及 Rust 展示调用点检查通过；14 个精确品牌/技术例外；4.15s 含 SSH |
| `cargo run --manifest-path artifacts/i18n/windows-locale-probe/Cargo.toml` | 真实 Windows UI 偏好列表与双语渲染探针通过；编译 6.46s |
| `cargo build -p neonmix-desktop --locked` | 最终 debug 原生应用构建成功；6.42s |

测试包括模拟 IME 状态、AccessKit 树、焦点/草稿/展开/滚动、语言 popup Escape、50 次切换、后台请求计数、参数/资源故障、偏好读写及字体字节/隔离字形回归。这些自动测试不替代实际 Windows 输入法、读屏软件或原生菜单交互。

实际原生探针 stdout：

```text
system_candidates=["zh-CN", "en-US"]
auto_locale=zh-CN reason=SystemMatch { index: 0 }
render zh-CN: 保存
render_diagnostics zh-CN: []
render en: Save
render_diagnostics en: []
```

直接调用 sys-locale 平台实现读取有序偏好，没有修改系统语言、时区或键盘设置。此 positive probe 证实正常读取；上游 iterator 不暴露错误原因。生产 adapter 将无候选视为检测不可用，首次回退英文、已有 Auto 保留本次语言；不能宣称原生 API 错误与真实空列表已可区分。

最终 debug binary：`E:\Desktop\NeonMix-test\ui-i18n-20261007\target\debug\neonmix-desktop.exe`，20,196,352 bytes，SHA256 `c0cc111994d39e236cc333b9a23100969b859e649b14497031f9520af6bf10dc`。本次未启动该应用、未停止既有 desktop/background/audio。

日志见 `artifacts/i18n/windows-final-tests.log`、`windows-overlay-targeted-tests.log`、`windows-strict-check.log`、`windows-locale-probe.log`、`windows-desktop-build.log`、`windows-binary-hash.log`；首轮漏 fixture 的编译日志保留以解释修复过程。探针、native env 与 validation 脚本均保留，probe Cargo.lock 已取回。

## Linux 验证范围

`linorman@192.168.100.112` 和 `parallels@192.168.100.112` 均无法 SSH 连接（后者明确 `No route to host`）。因此 Linux native UI/菜单/AX/IME/音频实测未执行。

复用现有项目 `.local/cross-linux` Rust std、Ubuntu headers/sysroot 和 LLVM archives，使用 `tools/dev python3 artifacts/i18n/linux-cross-check.py`；只读缓存、不拉 package index、不安装全局工具。`cargo check --locked --offline --target x86_64-unknown-linux-gnu -p neonmix-i18n -p neonmix-desktop` 成功：首次 core 14.12s、desktop 1m05s；provider 更新后最终联合增量检查 9.38s。

## 尚未在 Windows 实际执行

中文用户目录下的安装/偏好升级；真实输入法组字；Windows 屏幕阅读器朗读；原生托盘/菜单切换、点击/隐藏恢复；真实 200% 系统 DPI 的窗口检查；播放中 20 次切换与进程/会话/音频录制；长时内存与字体/菜单句柄测量。上述项目以主报告限定发布范围，不能由 native 编译或模拟交互推断成功。


## 最终功能快照补验（724 消息 / 20 模块）

前述 75 项完整通过及旧 binary hash 对应报告明确列出的先前快照。晚期加入旧 Unix 0644 动效偏好兼容、偏好先于 UI 初始化、shared popup 的 Space/Enter 消费与焦点关闭时，工作区另一并行 owner 同时重构 intent/commands 依赖。中间同步快照出现编译错误，日志已保留；没有覆盖外部 owner 的实现或把中间态写为成功。

随后以主任务生成的不可变 `artifacts/i18n/final-source` 完成最终补验：671 个文件逐一 hash 验证全部一致，source archive SHA256 `694979b4f4b1826a47b918eaff0e7dcd0489b232de952cbf0c0d3dd08b716c4f`，详情见 `windows-immutable-source-archive.json`。本阶段完全停止追踪继续变化的 working tree，仅验证该固定源。

| 固定快照原生检查 | 结果 |
|---|---|
| `cargo test -p neonmix-desktop --locked widgets::localization_tests::keyboard_combo_selection_closes_popup_and_returns_focus` | 1 项通过，涵盖 request_discard 重复 pass、Space 消费、popup 关闭和焦点返回；编译 11.97s，测试 0.02s |
| `cargo test -p neonmix-desktop --locked preferences::tests` | Windows 可用的 3 项通过，0.02s；Unix 0644/UID/非链接兼容测试仅在主任务 Mac 日志验证，未在 Windows 假称执行 |
| `cargo test -p neonmix-desktop --locked i18n_tests::` | 8 项功能交互通过，0.61s |
| `cargo build -p neonmix-desktop --locked` | 最终 native debug 构建成功，6.63s |
| `python tools/check_i18n.py --strict --pseudo --report artifacts/i18n/windows-immutable-strict-report.json` | 724 消息、20 模块、2 shipped locale、Rust 展示调用点全部通过；含 SSH 2.42s |

冻结源原有 14 项例外的 AirPlay 行号仍旧，首轮 strict 因精确位置不匹配失败。仅根据该固定 Windows source 的 Rust AST 刷新相同 file+value 的位置，未新增豁免或改生产源；单独记录 `windows-immutable-ui-exceptions.toml` 与 `windows-immutable-exceptions-overlay.json`。最后 source 定义为 base671 + 此一个审查位置 overlay，而非声称原 tar 未改变。

最终 native binary 为 20,682,752 bytes，SHA256 `7d9b8d3d4e7226700efa07c6e8c072661e84553c54dd6b80fa8a47cb15448783`。见 `windows-immutable-binary-hash.log`。传输 tar 两端已清理，固定源码、可写缓存及必要证据保留。

日志：`windows-immutable-{popup,preferences,interaction,build}.log`、`windows-immutable-strict-final.log`、`windows-immutable-verify.log`；完整 JSON 见 `windows-immutable-strict-report.json`。测试 runner 编译了当前 58 个可用 Windows test，但只执行本功能的 12 项定向 tests；不宣称外部 intent/控制重构的当前完整 suite 或 clippy 全通过。原生 GUI、真实 IME/托盘/读屏及播放测试仍按前述未执行范围记录。
