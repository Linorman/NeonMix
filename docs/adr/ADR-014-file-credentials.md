# ADR-014：profile 相邻文件凭证与独立迁移

日期：2026-10-02。状态：实施；运行验证限于 macOS。

本决策取代 ADR-011 的平台凭证存储选择及 ADR-012 的对应产品依赖。发现、首次信任、精确证书 DER/Hub UUID 固定、邀请、服务端角色、撤销和媒体协议保持原契约。

正式产品仅使用 profile 父目录 `.credentials/<规范UUID>.json`。秘密条目为闭合 `version:1/kind/value` JSON，四种 kind 为 `hub_tls_key/admin_token/member_token/airplay_receiver_key`。接受明文落盘，Unix 目录 0700、文件 0600；Windows 受保护当前用户 DACL 与持久 ACL 文件系统。引用身份是规范 store 路径加 UUID；单独复制 profile 不会复制凭证。

Hub/token profile 为 v2，`credential_store:file`，token 增加 `profile_kind:admin|member`；receiver 为 v1。共享闭合 schema 限制 profile 256 KiB，条目 64 KiB；读取按句柄验证、拒绝链接/非普通文件/过宽权限/错误 owner。Unix store 以目录句柄和 openat/unlinkat 防止路径换链接。秘密排他新建、同步并读回后才能发布 profile；修改 profile 使用私有同目录临时文件替换。稳定非阻塞 OS 锁覆盖 setup、pair/forget、Hub owner 和设置写入，进程退出释放，锁文件不删除。

初始化保持空目录要求，先写两个秘密，再写 state/admin，最后发布 server。新 state_path 是相对路径；正式启动必须恢复已有 Authority，并验证管理员。缺损不会生成新身份。配对先持久化成员 token 和同 request UUID 的 pending，再登记；收尾失败可恢复。forget 先保护管理员及其别名，再删成员秘密、最后删 profile，缺失秘密允许幂等收尾；不代表服务端 revoke。

AirPlay 使用 receiver 的本地 store；旧无版本资料、缺失/坏密钥明确失败。仅全新 receiver 生成 key，保留信任集合和播放模式。运行 PEM 仅供 worker 临时读取，退出及下一次 owner 启动时清理本模块 UUID 文件，不清理持久 store。

旧 v1 平台资料只报告 migration_required。`apps/credential-migrate` 独立 workspace/lockfile，唯一保留 keyring 的工具，不由默认产品自动启动。离线复制 desktop/hub/profiles，保留秘密值、UUID、pending、Authority/revision/撤销、输出绑定和 AirPlay 信任。读取旧库用可终止 helper、超时/取消回收，秘密不经过参数、环境和报告。私有同父 staging 重新解析校验、复核源快照后排他发布；不覆盖目标，不自动删旧 vault。可能补建源协调锁，源持久数据保持不变。

备份应包含 .credentials 且维持权限；诊断与构建归档排除 store、运行 PEM 和含秘密 staging。新版目录发生新配对、撤销或设置写入后，禁止直接回到旧快照。Windows/Linux 原生权限与文件替换、真实 native 迁移另行验收。
