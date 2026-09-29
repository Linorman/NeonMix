# 工程诊断 UI 行为契约

业务来源：`01_NeonMix_设计方案与技术选型.md` §2、§8.4 与 `02_NeonMix_完整开发计划.md` §4.1。
本界面为 E00 工程壳，不提供配对、权限、录音、删除、输出切换等 E04/E07 操作。

| Capability | Canonical owner | Source of truth | Allowed variants | Verification |
|---|---|---|---|---|
| Button | egui::Button | DESIGN.md / theme.rs | refresh | egui frame tests |
| Scrollbar | egui::ScrollArea | theme.rs | always visible | desktop smoke |
| Status | Desktop::show | 本文 | loading / ready / empty / error | egui frame tests |
| Device list | Desktop::show | native CLI devices JSON | read-only | CLI + desktop smoke |

默认语言 zh-CN；无日本市场流程。设备名称保留系统内容，技术字段可使用英文。
启动异步查询同目录 `neonmix-audio devices`。最多一个枚举任务，10 秒超时、失败后可重试，成功结果来自退出码为 0 且 JSON 可解析的子进程；错误时保留上次数据并标明不是最新状态。
设备列表为空时解释需要连接设备及刷新；没有音频程序时说明先构建 workspace。没有假成功值。Tab / Enter 使用 egui 标准键盘行为，AccessKit 已启用；辅助功能实机验收单独记录。
E00 不存在持久化设置、表格选择、下拉框、认证或 CRUD；不为这些能力添加空 UI。
