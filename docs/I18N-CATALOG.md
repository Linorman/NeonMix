# UI 本地化文案目录与约束

日期：2026-10-07。源码基线及在途改动见 `evidence/ui-i18n-20261007/initial-worktree.json`。已有 AirPlay、Windows 网络与后台生命周期改动保持；本次不改变协议数值、控制权限、采样率、配对资料、音频生命周期或设备/房间数据。语言仅属本机 UI 偏好。

## 消息归属

| Owner | 使用位置 | 内容 |
|---|---|---|
| shell | main.rs / shell.rs | 导航、底栏、身份、确认、操作反馈 |
| lanes / events | lanes.rs / events.rs / pages/mod.rs | 角色、会话、网络、事件、相对时间 |
| palette | palette.rs | 分组、命令标题、搜索提示及别名 |
| widgets / flow / viz | 共用控件与绘图 | tooltip、AX、步骤、容量、图形节点 |
| live / mixer / hub / sender / devices / diagnostics / about | pages/* | 七页和嵌入 AirPlay 的全部文案 |
| faults | desktop-service FaultCode | 受限错误码和恢复指引；旧 error 保留兼容 |
| preferences / tray | localization.rs / tray.rs | 语言设置、偏好失败、原生菜单与 tooltip |

`crates/i18n/messages.toml` 是片段索引，`messages/*.toml` 定义语义 ID 与类型。资源 owner 与 schema 片段同名，两种语言随二进制嵌入。具体参数以 schema 为准：计数 u64、计量 f64、用户/系统原始字段 string、嵌套消息 message。用户字符串作为字面参数插入，保留 Fluent 隔离；复制来源保持原始字段。

## 文案分类与例外

产品标签、完整操作后果、空/错/过期/加载状态、剪贴板反馈、tooltip 和 AccessKit 名称全部使用语义消息。用户房间名、来源别名、系统设备名和事件发生时名称不翻译。协议 JSON 字段、UUID、路径、端口、版本、快捷键、dB/dBFS/Hz/ms/ppm、品牌与技术词保留；日志与测试夹具不属 UI 产品文案。精确源码调用点豁免见 `crates/i18n/ui-exceptions.toml`，不豁免目录。

## 稳定身份

field/title_field 使用独立业务 ID；Page enum、panel key、credential、stream/source/session ID、命令 Cmd、确认目标和还原 revision 不使用译文。ScrollArea 用 Page；胶囊与颜色动画使用业务 scope/稳定控件位置。Marks 存语义消息；事件、Undo 和异步结果保存语义后延迟渲染。状态条按 Message 身份去重。

## 错误来源

Reply/ProcessStatus 优先 fault，旧后台只精确识别已知机器码；未知 stderr 显示通用当前语言提示，不把原始秘密或错误串嵌入翻译/诊断。profile_error/classify_error 后台归类仍不依赖 UI locale。允许 fault 参数只有受限端口；IPC/脱敏证据见 service-report.md。

## 术语与危险动作

| 中文 | English | 后果 |
|---|---|---|
| 现场 | Live | 信号视图 |
| Hub 设置 | Hub settings | 房间设置 |
| 设备管理 | Devices | 配对设备 |
| 开始/停止共享 | Start/Stop sharing | 房间共享；不同于停止 Sender |
| 开始/停止发送 | Start/Stop sending | 本机发送会话 |
| 隐藏窗口 | Hide window | 后台音频继续 |
| 退出后台 | Quit background audio | 停止本实例共享、发送及全部音频连接 |
| 撤销配对 | Revoke pairing | 设备须重新配对；不等于还原混音 |
| 还原混音更改 | Undo mix change | 8秒内恢复上次值，使用新权威 revision |
| Mixer 队列估计 | Estimated Mixer queue | 不是实测端到端延迟 |

## 验证边界

P0 同 fixture/固定动效的600×440截图位于 `artifacts/i18n/baseline/600/`。性能基线、原生平台、输入法和辅助功能结果另见实施报告；未取得证据的项目保持未勾，检查器通过不代表原生验收。
