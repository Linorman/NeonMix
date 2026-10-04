# 文件凭证使用与升级

正式 Hub/Sender profile 是 v2，receiver 是 v1，均有 `credential_store: "file"`。秘密只存在 profile 父目录 `.credentials/<UUID>.json`，以明文 JSON 保存，访问限于当前用户。Hub `server.json` 和 `admin.json` 共用管理员 token；AirPlay `airplay/receiver.json` 使用 `airplay/.credentials`。profile 元数据不会通过 IPC 提供秘密。

新建 Hub 继续用 `setup`，Sender 继续用 `invite/pair`。新 state_path 为 `state.json`，随完整目录搬迁定位，损坏或缺失不会重新生成 Hub UUID。`credential-store-probe --directory <私有目录>` 在自己的随机子目录验证并清理；`vault-probe` 已移除。setup 发现非空或残缺目录返回 setup_incomplete；先检查内容，只在确认属于本次未完成初始化后显式清理，再重试，不覆盖原身份。

备份/搬迁要复制整个资料目录，包括隐藏的 `.credentials`，并保持 Unix 0700/0600 或 Windows 当前用户 DACL。只复制 Sender profile 不足以认证。同目录共享同一引用的 Sender 副本执行 forget 后都会失去本地 token；已在内存里的 token 不会因此失效，需 Hub 管理员 revoke 撤销远程授权。管理员及其共享别名禁止 forget；服务端角色仍由 Authority 决定。

旧平台资料启动只返回 migration_required。不要覆盖正在运行的程序和资料；先停止源 UI/后台/Hub/Sender/worker，使用独立工具：

```sh
tools/credential_migrate test --locked
tools/credential_migrate build --release --locked
tools/dev target/credential-migrate/release/neonmix-credential-migrate \
  --layout desktop --source <旧目录> --destination <不存在的新目录>
```

独立 Hub 使用 `--layout hub`；独立 profile 批次使用 `--layout profiles --profile-kind admin|member`。源/目标不能相同或嵌套。新目录必须不存在，未知持久文件、外部 state 路径、旧库不可读取均失败并保留源。默认 30 秒 native 读取超时，helper 可结束并回收；旧库条目不自动删除。完整说明见 [迁移工具](../apps/credential-migrate/README.md)。

用新二进制明确指向目标目录后，核对原 Hub UUID、认证、输出绑定及 AirPlay 已配对来源。目标未发生新状态写入时可停止新版回到旧二进制和源目录；发生新配对、撤销、设置变更后应保留并修复新目录，不能直接恢复过时授权快照。遗留 staging 只能用工具的 `--cleanup-staging <确切路径>` 验属清理，不能作为正式目录运行。

常规诊断、日志、构建包不包含 .credentials、runtime-key 或迁移 staging。文件删除不代表介质安全擦除；备份中的成员 token 需通过 Hub revoke 失效。当前运行验证限于 macOS；Windows/Linux 原生 ACL/权限及真实旧库迁移未计作通过。
