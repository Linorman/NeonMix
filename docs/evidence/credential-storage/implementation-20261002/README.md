# 文件凭证实施与 macOS 验证（2026-10-02）

已实现正式 Hub/Sender/桌面后台/AirPlay 的相邻文件凭证、闭合共享 schema、稳定 OS 锁和独立旧资料迁移工具。普通产品及普通测试不访问系统凭证库；秘密明文落在私有 `.credentials` JSON 内，诊断和打包排除它。旧资料不会自动回退或重建身份。用户本轮仅要求 macOS 测试，现有手动会话资料和系统默认路由未变更。

完整 workspace 的 format、严格 Clippy、tests、release 通过；最后的针对性检查覆盖新补充代码，57 项测试通过（identity 17、Hub 26、desktop-service 14）。独立迁移工具 18 项普通测试、Clippy、fmt、release 通过；真实 native fixture 的 1 项显式 ignored 测试入口已经提供，本轮未运行。纯文件 CLI 9 类、E05 16 类、AirPlay Hub 6 类、E07 后台 12 类探针通过。源码和制品哈希、命令、退出码和脱敏场景结果见 [summary.json](summary.json)；详细日志在项目 `artifacts/credential-storage/implementation-20261002` 及各 regression 子目录。

实际故障验证包含：排他新建和替换失败保旧；目录被换链接仍由句柄锚定；同 UUID 跨目录、cwd 变化隔离；进程强退释放锁；保留克隆描述符时显式解锁；setup 在 secrets/state/admin/server 四个提交阶段强退后不覆盖旧目录；pending 收尾因目录不可写失败仍保留秘密与请求；迁移五个阶段失败和强退、源变更、目标竞争。并发 spawn 暂时继承锁描述符造成的 busy 竞态已修复，8 线程并发测试连续 10 轮通过。

| 矩阵 | 本轮结果及边界 |
|---|---|
| T01–T03 | macOS 文件操作、UUID 碰撞不覆盖、目录隔离、版本/kind/损坏/限额拒绝通过 |
| T04 | macOS 链接/目录/FIFO/宽权限拒绝通过；跨账户错误 owner、Windows reparse/DACL 未执行 |
| T05 | macOS 实际替换失败、setup 强退和 pending 写失败通过；所有秘密写入/读回故障边界尚未逐个注入 |
| T06 | macOS OS 锁冲突、子进程强退和继承描述符竞争通过；两路真实网络 pair/forget 竞争和 Windows 共享句柄未执行 |
| T07 | setup 中断/重跑/非空与缺损身份拒绝通过 |
| T08 | E05 正式配对/重试、Hub 重启、pending 恢复、收尾写失败通过；/v1/me 独立故障注入未执行 |
| T09 | CLI/后台管理员及共享设备别名保护、成员 forget、缺秘密断点收尾和绑定禁用通过（部分为隔离单测） |
| T10 | 异证书/UUID、取消/过期邀请、撤销 WSS/媒体、实验分流通过 |
| T11 | E07 真实后台设置/列表/重启/脱敏导出通过；SenderStart/Stop 和 forget/disable 由 lifecycle 单测覆盖；完整 GUI/Mixer 未重跑 |
| T12 | AirPlay 真实 worker 初建/重启/缺坏 key/worker 强退、不换身份和运行 PEM 清理通过 |
| T13 | 信任集合、播放模式、管理员控制、PIN/revision 保持通过；实际 Apple 来源迁移后的重新认证未执行 |
| T14–T15 | 生成的 Hub/admin/AirPlay/Sender/pending/撤销/绑定 mock 迁移、身份保持、超时/取消和故障/强退通过 |
| T16 | 真实旧库迁移未执行；显式 native fixture 入口已实现，现有资料未切换 |
| T17 | macOS 纯文件路径及默认依赖不访问原生凭证通过；Windows SSH、Linux 无 Secret Service 场景未执行 |
| T18 | 默认产品无 keyring、归档隐藏凭证排除、诊断逐值泄漏断言与 fixture 清理通过 |

三端存储与迁移代码已就位，但 Windows/Linux 权限、文件替换和原生运行验收不能计作完成。当前状态也不代表 iPhone/macOS 实际来源或跨机音频长测。06 计划的 P1.10、P2.6、P3.12、P4.14、P6.6/P6.7 因上述范围保留未勾选；其余勾选代表实现及本轮证据。

本轮没有创建原生凭证条目。普通 fixture 均由 Drop/finally 清理，仅保留源码、缓存、制品和脱敏证据。早期探针只因旧错误文本断言失败的日志保留为诊断历史，重跑记录为验收依据。

现有资料切换是后续明确任务：停止源进程，使用 `tools/credential_migrate` 独立构建，再按 desktop/hub/profiles layout 复制到不存在的新目录。完整使用见 [文件凭证使用与升级](../../../CREDENTIAL-STORAGE.md)。目标发生新配对、撤销或设置写入后禁止直接回退旧快照；工具保留旧库和源持久数据。
