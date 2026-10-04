# 凭证迁移工具

这是独立、按需运行的离线工具；默认产品不包含 `keyring`。其 manifest 和锁文件不加入根 workspace。`tools/credential_migrate` 通过项目 `tools/dev` 执行 Cargo，把独立编译产物放在 `target/credential-migrate`；缓存、测试状态和所有产物留在项目中。

```sh
tools/credential_migrate test --locked
tools/credential_migrate build --locked --release
```

先停止使用源目录的 UI、后台、Hub、Sender 和 AirPlay worker；旧版独立 CLI 配对不会遵守新文件锁，必须停止。工具获取 `background.lock`、Hub state 对应 `.lock` 和 `binding.lock`；profile 锁同样保持稳定，不删锁文件。迁移会补建这些协调锁，持久状态文件与旧系统凭证条目保持原样。

```sh
tools/dev target/credential-migrate/release/neonmix-credential-migrate --layout desktop --source <old-state-dir> --destination <new-state-dir>
tools/dev target/credential-migrate/release/neonmix-credential-migrate --layout hub --source <old-hub-dir> --destination <new-hub-dir>
tools/dev target/credential-migrate/release/neonmix-credential-migrate --layout profiles --source <old-profile-dir> --destination <new-profile-dir> --profile-kind member
```

`profiles` 必须显式指定 `--profile-kind admin|member`，混合角色分批迁移；不会用 profile 名称猜测角色。desktop/hub 从 Hub 管理员引用与 Authority 身份判定管理员别名。

源和目标不得重合或嵌套，目标必须不存在；重复执行不会覆盖已发布资料。支持 desktop 下 `hub`、`profiles`、`output`、`outputs/main`，hub 下的 AirPlay，或 profiles 目录直接包含的正式配对 JSON。不支持的持久文件直接拒绝；锁、PID、socket、日志、缓存、临时命令/邀请、运行 PEM、手动会话根 bin 和接收端 gstreamer-registry.bin 不复制。源 state 必须在 source 中；新 Hub 使用目录内相对 `state.json`。

每次旧库读取由 helper 执行，默认 30 秒超时，可用 `--native-timeout-seconds 1..300` 设置；超时或 SIGINT/SIGTERM 取消会结束并回收 helper。秘密只经过匿名管道，不进入参数、环境变量、报告或日志。TLS 私钥必须与证书匹配，管理员 token 必须匹配 Authority 与设备角色，AirPlay 私钥必须为 Ed25519。Sender 离线迁移保留原身份，即使该设备已被服务器撤销也不重配。

目标同父目录的 `.neonmix-migration-<UUID>.staging` 以私有权限写入；发布前读回所有秘密、解析所有格式并复核源快照，再排他发布目录。正常失败清理本次 staging；强制进程退出后，可显式指定遗留目录清理：

```sh
tools/dev target/credential-migrate/release/neonmix-credential-migrate --cleanup-staging <exact-staging-path>
```

只允许清理名称、私有权限与 `.migration-owner.json` 标识一致的 staging，不扫描其他目录。该不含秘密的标识在最终目录保留，以避免发布前退出留下无法归属的暂存目录。staging 不可作为正式状态目录启动。

切换时明确让新二进制使用目标目录，并验证认证和身份保持。停止新实例后，只有目标尚未发生新的配对、撤销、设置写入时，才可以恢复旧二进制和源目录；目标发生新写入后，应保留并修复目标，直接恢复旧快照可能恢复已经撤销的授权。工具不自动删除旧凭证库条目。

普通自动测试只使用生成的隔离 fixture 与 mock reader，不读取真实系统凭证库。macOS 验证覆盖三个 layout、共享管理员引用、pending、撤销状态、绑定与 AirPlay 信任集合；缺失/超时、错误权限、密钥错配、源变化、目标竞争、五个失败与进程强退边界。真实 native 迁移及 Windows/Linux 原生权限验收尚未执行，应单独显式验收。

显式 native fixture 入口如下；本轮没有运行。普通 `cargo test` 和 `--all-features` 均将它忽略。它仅创建随机 UUID 的自有旧条目及 Hub/admin/AirPlay fixture，读取、创建、删除均由有 30 秒超时的 helper 执行；创建拒绝覆盖既有条目，删除要求条目原值与自有 fixture 匹配。秘密只经私有 stdin 管道。清理失败时保留项目内私有 `native-fixture-owner.json`，便于按本次清单恢复清理。

```sh
tools/credential_migrate test --locked --test native_fixture -- --ignored --exact native_fixture_migration
```
