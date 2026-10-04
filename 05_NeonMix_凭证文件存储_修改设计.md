# NeonMix 凭证文件存储修改设计

日期：2026-10-02。状态：代码已实施，macOS 验证；其他平台及真实旧库迁移待验收。配套执行文档：[开发计划与 checklist](06_NeonMix_凭证文件存储_开发计划.md)。

建议把 macOS Keychain、Windows Credential Manager、Linux Secret Service 统一替换为项目状态目录中的普通 JSON 文件。保留现有 profile、UUID 引用、配对和认证流程，只替换秘密值的持久化方式。默认产品和常规测试不再读取系统凭证库；旧资料通过独立迁移工具保留身份后切换。

本文基于当前工作区源码，包括尚未提交的 AirPlay 和桌面改动。实施决策见 docs/adr/ADR-014-file-credentials.md，验收范围见 docs/evidence/credential-storage/implementation-20261002/README.md；尚未切换现有用户资料。

## 1 现状与修改理由

### 1.1 当前实现

| 范围 | 当前行为与依据 | 本次影响 |
|---|---|---|
| 设计与落地差异 | [主设计](01_NeonMix_设计方案与技术选型.md) §3 的选型表写 SQLite + 平台凭证；[ADR-011](docs/adr/ADR-011-discovery-and-pairing.md) 明确当前权威状态使用原子 JSON | 按当前代码设计，不把主设计表中的 SQLite 当作已经存在的基础设施 |
| 系统凭证封装 | [vault.rs](crates/identity/src/vault.rs) 提供全局 `put/get/remove/probe`，service 为 `com.neonmix.identity.v1`；[Cargo.toml](crates/identity/Cargo.toml) 为三端选择 `keyring 3.6.2` 后端 | 用显式目录上下文取代全局 OS store |
| Hub 身份 | [identity.rs](apps/hub/src/identity.rs) 的 `setup/config` 保存、读取 TLS 私钥和管理员 token；`server.json` 与 `admin.json` 共用管理员 token 引用 | 保留共享关系和现有身份 |
| Sender 身份 | 同文件的 `pair/credential/forget` 使用 `secret_ref`；token 在请求发出前保存，`pending` 支持响应丢失后的重试 | 保留先持久化后登记的顺序 |
| AirPlay 身份 | [airplay.rs](apps/hub/src/airplay.rs) 的 `Saved/run_worker` 用 `key_reference` 保存 Ed25519 私钥；`receiver.json` 同时保存允许和阻止的来源公钥 | 必须一起迁移；不能重新生成接收身份来绕过读取失败 |
| 桌面后台 | [daemon.rs](crates/desktop-service/src/daemon.rs) 的 `credential/profiles/HubStart/HubSettings/ForgetCredential` 按引用字段判定正式 profile | 改为严格的版本与存储格式识别，保留管理员删除保护 |
| 文件基础设施 | [files.rs](crates/identity/src/files.rs) 已有排他创建、随机同目录临时文件、文件同步与 rename；[files_windows.rs](crates/identity/src/files_windows.rs) 已有私有 DACL 和 ACL 文件系统检查 | 复用并补齐读取、目录和持久性约定，不另写一套不受限文件 I/O |
| 测试与探针 | [native_vault.rs](crates/identity/tests/native_vault.rs) 默认为 ignored；E05/E07/AirPlay 探针显式操作和清理 Keychain；Windows runtime 探针执行 Credential Manager 用例 | 换成默认执行的隔离文件测试，去掉系统账户和交互会话要求 |

### 1.2 已有证据

[当前状态](docs/STATUS.md) 与 [AirPlay 启动观察](docs/evidence/airplay/pause-resume-20261002/startup-observation.json) 记录了启动停在 `SecKeychainFindGenericPassword`，且 Hub 私钥、管理员 token、接收端密钥三个条目都存在。该证据说明当前流程确实受系统凭证访问影响，不足以断言具体的系统阻塞原因。

`docs/STATUS.md` 还记录 Windows 已登录桌面会话中的凭证测试通过，而 SSH 登录上下文返回 1312。这正是文件存储可以消除的一类运行环境差异。删除凭证库依赖不会消除音频设备、PipeWire、桌面托盘、驱动或网络发现自身的环境要求。

## 2 方案选择

| 方案 | 与现有项目的关系 | 测试与维护成本 | 结论 |
|---|---|---|---|
| 每个秘密一个 JSON 文件，profile 保留引用 | 可复用现有文件基础设施；避免在 `server.json` 和 `admin.json` 各存一份管理员 token | 用临时目录即可验证；需定义文件提交、权限和引用生命周期 | 推荐 |
| SQLite 保存秘密，其他状态继续 JSON | 需要新增数据库依赖、schema、锁与连接管理；数据库事务仍不能同时提交已有 JSON 文件 | 可隔离到临时数据库，但并未直接解决 profile 与秘密的跨存储一致性 | 当前不采用 |
| 配置、配对、权威状态整体迁入 SQLite | 可统一更多关联状态，但触及 Authority、revision、输出绑定和桌面设置 | 接近独立的持久化重构，超出解决凭证测试问题所需范围 | 留给有明确统一事务需求的后续工作 |
| 直接复用 `init` 的明文实验格式 | 代码表面改动较少，但正式 profile 的 pending、请求 UUID、固定 Hub UUID 等语义并不等同于实验配置 | 容易绕过正式流程限制 | 不采用 |

以上是依据当前架构做出的工程选择，不是容量或性能基准结论。不增加同时支持 file/sqlite/native 的运行时后端选择，也不做操作系统凭证访问失败后的自动回退。正式产品只有文件存储；兼容读取只存在于独立迁移工具。

普通 JSON 和未额外加密的 SQLite 都会使秘密以明文落盘。本方案接受这一属性，以当前用户的文件权限隔离其他普通用户；不声称达到系统凭证库的静态保护能力，也不引入与数据同目录保存密钥的额外加密层。

## 3 范围与不变量

本次涉及长期 Hub TLS 私钥、管理员 token、Sender token、AirPlay 接收私钥，以及它们的 profile、迁移和测试。邀请仍为短期文件或 UI 文本，Hub 仍只保存成员 token 摘要。AirPlay 的来源公钥集合、播放权限与播放模式仍在 `receiver.json` 中。

TLS 1.3、精确 DER 证书固定、Hub UUID 校验、邀请期限、重复请求语义、服务端角色、撤销、媒体认证和防重放全部保持现有规则。配对完成不自动发声，删除本地资料不等于撤销 Hub 端授权。文件 I/O 只发生于控制路径，不进入音频回调。

本次不迁移 `state.json` 到数据库，不改外部 HTTPS/WSS/媒体协议，不改驱动、系统默认路由或签名公证凭证。`tools/package_macos_hal.py` 的 `notarytool --keychain-profile` 是发布工具配置，不属于产品身份凭证，不能一并删除。

## 4 文件布局与格式

### 4.1 目录规则

每个 profile 的父目录固定拥有一个 `.credentials` 子目录；profile 只保存 UUID，不允许指定 store 的绝对路径或任意相对路径。CLI 从传入 profile 路径解析，桌面仍从 `--state-dir` 下的受限相对路径解析，不依赖当前工作目录或用户 HOME。

```text
<state-dir>/
  hub/
    server.json
    admin.json
    state.json
    .credentials/
      <tls-key-uuid>.json
      <admin-token-uuid>.json
    airplay/
      receiver.json
      .credentials/
        <receiver-key-uuid>.json
      runtime-key-<uuid>          # 仅 worker 生命周期内使用
  profiles/
    sender.json
    .credentials/
      <sender-token-uuid>.json
  output/binding.json
```

CLI 的 `--directory X` 对应 `X/.credentials`；`--credential X/alice.json` 对应 `X/.credentials`。多个 Sender profile 可以共用目录，但正常创建时每个 profile 生成独立 token 引用。AirPlay 使用其 `receiver.json` 所在目录，不依赖向上搜索 Hub 根目录。

引用的完整身份为“规范化 store 目录 + UUID”，不是单独 UUID。复制一个 profile 文件并不复制凭证；迁移和备份需包含相邻 `.credentials`。对共享同一引用的 Sender 副本执行 forget，会使这些副本也不可用，这与现有全局 vault 引用的本地共享行为一致。

### 4.2 版本规则

| 文件 | 新格式 | 兼容行为 |
|---|---|---|
| `server.json` | `version: 2`、`credential_store: "file"`；保留 `private_key_ref/admin_token_ref` 和配置字段 | v1 平台 profile 返回 `migration_required` |
| 管理员和 Sender profile | `version: 2`、`credential_store: "file"`、`profile_kind: "admin"或"member"`；保留 `secret_ref/hub_id/certificate/request_id/name/pending/invitation_id/device_id` | v1 平台 profile 返回 `migration_required` |
| `receiver.json` | 首次引入 `version: 1`、`credential_store: "file"`；保留 `key_reference/known_keys/blocked_keys/playback_allowed/playback_mode` | 无版本的旧格式须迁移；文件不存在才属于全新接收端 |
| 秘密条目 | `version: 1`、`kind`、`value`；文件名为规范 UUID 加 `.json` | 未知版本、错误 kind、损坏内容明确拒绝 |

秘密条目结构示意如下。占位值不能用作测试凭证或生产输入。

```json
{
  "version": 1,
  "kind": "member_token",
  "value": "<generated-secret>"
}
```

`kind` 仅允许 `hub_tls_key`、`admin_token`、`member_token`、`airplay_receiver_key`。普通 credential loader 可读取两种 token；Hub config 只接受管理员 token 和 TLS 私钥；forget 只允许成员 token；AirPlay 只接受对应的 Ed25519 私钥。`kind` 防止本地误用，不是服务端角色授权来源。

token profile 的 `profile_kind` 必须与秘密 kind 一致；它用于本地生命周期和缺失条目时的删除保护，不授予远程管理员权限。真正的角色仍来自 Hub Authority 和认证响应。

格式读取采用闭合 typed enum/struct，拒绝未知字段和混合格式。不能再用“缺少 `secret_ref/private_key_ref` 就当作实验格式”的回退方式。没有正式格式标记且完整匹配旧实验 schema 的文件才由 CLI 实验兼容分支读取；桌面仍拒绝该分支。未知正式版本不得进入实验分支。

### 4.3 接口建议

在 `crates/identity` 新增 `store.rs` 和共享 `profiles.rs`，把正式 profile 解析规则从 Hub 私有结构提升为 Hub 与桌面共用的接口。接口示意，不规定最终 Rust 命名：

```rust
FileCredentialStore::for_profile(profile_path)
store.put(kind, secret) -> SecretRef
store.get(reference, expected_kind) -> SecretValue
store.remove(reference, expected_kind) -> ()
load_profile_metadata(path) -> ProductProfile
```

`get/remove` 不隐式创建目录；仅写入入口创建 store。`SecretRef` 校验并规范化 UUID，禁止路径分隔符、ADS 和目录穿越。`SecretValue` 不实现明文 Debug/Serialize 输出；持久化由模块内部完成。普通测试直接传入临时目录，迁移算法使用可注入的旧秘密读取器验证故障，不为产品增加多后端配置系统。

错误至少区分 `migration_required`、`credential_missing`、`credential_corrupt`、`credential_version_unsupported`、`credential_permission_denied`、`credential_kind_mismatch`、`credential_store_busy` 和 `credential_io_failed`。对外错误不包含秘密值或原始 JSON；诊断白名单仍不导出任意路径和字符串。

## 5 文件操作和生命周期

### 5.1 文件保护与提交

新建 Unix 私有目录使用 0700，秘密及相关 profile 文件使用 0600；Windows 使用当前用户拥有的受保护 DACL，保留持久 ACL 文件系统要求。复用 `files_windows.rs` 与桌面 `transport` 的已有能力，抽出需要共享的最小接口。保留这些文件权限操作并不会重新引入系统凭证服务。

秘密读取限制为 64 KiB，超限先拒绝再解析；profile 同样使用有界读取并单独约定限额。拒绝秘密文件或 store 目录的 symlink/reparse point、非普通文件、非当前用户所有和过宽权限。使用句柄检查串联打开与验证，不能仅先检查路径再无条件跟随链接打开。迁移工具新建正确权限的文件，不擅自修改源目录权限。

创建秘密使用随机 UUID、排他新建、完整写入、`sync_all`、读回校验，再发布引用该秘密的 profile。秘密正常不原地覆盖：需要新值就生成新引用。可变 profile 和 `receiver.json` 使用同目录私有临时文件替换。Unix 增加必要的父目录同步；Windows 原生验证覆盖已有目标替换、共享句柄和失败保留旧文件。现有 `files::replace` 不应仅凭名字就被认定已经满足所有崩溃持久性要求。

同一 profile 的 `pair/forget/设置写入` 使用稳定锁文件上的 OS 文件锁，失败快速返回 busy；同一个配对请求的锁覆盖本次有界网络操作，避免两个进程复用同一 pending profile 同时提交。setup 使用目录级锁，AirPlay 沿用 Hub 单 owner 并串行保存。锁文件保持稳定，不在持锁期间被替换或删除，进程异常退出由 OS 释放。读取看到完整旧文件或完整新文件；读取后已进入内存的凭证不会因本地 forget 自动撤销。

仅适配本机常规文件系统；网络盘和同步盘不作为首轮支持目标。磁盘满、权限拒绝、格式损坏、缺失条目都报错，不能生成新身份、尝试系统 vault 或创建替代秘密。

### 5.2 Hub 初始化

保留空目录要求，检查时仅排除本模块认可且未被持有的稳定锁文件；获取初始化锁后生成证书、管理员 token 和 Authority。先持久化两个秘密，再保存 `state.json` 和 `admin.json`，最后发布 `server.json` 作为可启动配置。`server.json` 与 `admin.json` 必须指向同一个管理员 token 条目。

正常失败只删除本次创建的条目和文件。异常退出留下不完整目录时，重新运行 setup 不覆盖它；检查工具报告 `setup_incomplete`，由显式清理本次未完成初始化后重试。禁止因为缺少 state 或某个秘密就重新生成 Hub UUID。锁标记与初始化完成状态须分开，不能把陈旧空锁文件当作活跃进程。

新 v2 `state_path` 写为相对 `server.json` 的 `state.json`，loader 按该 profile 父目录解析并检查边界；内存中的 `ServerConfig` 仍可使用绝对路径。这样复制完整目录不会让新实例回写源目录。旧 v1 的绝对路径由迁移工具显式处理。

正式 v2 启动要求该 state 文件存在且可恢复，并校验管理员身份。当前 `server::serve` 在 state 不存在时可新建 Authority 的分支只能留给显式实验配置，不能使损坏的正式目录获得新 Hub UUID。

### 5.3 Sender 配对与删除

首次配对顺序为：验证邀请和证书 → 持久化成员 token → 持久化包含同一 request UUID 的 pending profile → 发起登记 → 验证 `/v1/me` → 原子设置 `pending: false`。落盘失败时不发出首次登记；服务器成功但本地收尾失败时保留 token、request UUID 和 pending，后续沿用当前恢复逻辑，不重复登记新设备。

forget 先验证为成员条目，桌面仍先停止 Sender、禁用输出绑定并阻止删除管理员身份。随后删除 token，最后删除 profile；删除缺失 token 视为成功，支持上次在两步之间中断后的重试。条目存在但 kind 不是成员时拒绝；缺失条目时仍需根据 profile 身份和管理员引用检查阻止误删。CLI 与桌面共享这一保护规则，不能只依赖 UI 按钮隐藏。

跨目录比较管理员引用时使用完整 store 身份，同时检查已知管理员 `hub_id/device_id`；迁移工具遇到管理员引用别名时保留 `admin_token` kind，不能转成成员。普通流程不做全目录自动垃圾回收。写入 profile 前崩溃产生的孤立秘密保留待显式离线扫描清理，避免扫描不完整时删除仍被使用的引用。

### 5.4 AirPlay

把 `run_worker` 中的两个 vault 调用替换为接收端目录下的 FileCredentialStore。首次建立身份时先写秘密，再提交 `receiver.json`；已有记录缺少/损坏 key 时停止启动并报告错误，不重新生成。

继续保留 `runtime-key-<uuid>` 临时 PEM 文件供现有 worker 接口读取，进程退出清理；异常退出留下的运行文件在下一次启动持有 Hub owner 后清理，仅匹配本模块受控文件名。持久秘密与运行临时文件不能混删。无需因此修改 C/C++ worker 协议或让 worker 管理永久身份。

迁移和重启须保持 `known_keys`、`blocked_keys`、`playback_allowed`、`playback_mode`。旧记录没有 `playback_mode` 时沿用当前 Serde 默认值，不改变既有行为。

## 6 旧资料迁移与回退

### 6.1 独立的一次性迁移工具

已新增独立构建的 `neonmix-credential-migrate`，使用旧 `keyring` 后端读取原引用，依赖隔离于默认产品构建之外。可放在显式排除出根 workspace 的 `apps/credential-migrate`，拥有自己的 manifest/lockfile，并复用共享 schema 和文件写入模块。默认 Hub、后台、桌面及其常规测试依赖图不含 `keyring`。

正常产品遇到旧格式只报告需要迁移，不自动访问 OS vault。这样不会把已有的启动阻塞带入新版本。迁移本身仍需原平台、原账户能够读到旧秘密；通过可终止的 helper 子进程为每次 native 读取设置超时，不能仅在线程外围设置超时后让阻塞线程继续持锁。

若旧库无法读取，保留源资料并报告迁移未完成。丢失的私钥或 token 无法从公开证书、引用或服务端摘要恢复；此时只能继续排查旧库，或由用户明确选择重新建立身份与配对。实现不能把重置伪装成迁移成功。

### 6.2 复制迁移而非原地改写

建议 CLI 合约如下，现有命令：

```text
neonmix-credential-migrate --layout desktop --source <old-state-dir> --destination <new-state-dir>
neonmix-credential-migrate --layout hub --source <old-hub-dir> --destination <new-hub-dir>
neonmix-credential-migrate --layout profiles --source <old-profile-dir> --destination <new-profile-dir> --profile-kind <admin|member>
```

源和目标不得相同或相互嵌套，目标必须不存在。

旧 Profile 没有可靠的角色字段，不能按 `name` 判断管理员。desktop/hub layout 用所包含的 `server.json` 管理员引用和 Authority 判定；独立 profiles layout 要求显式指定该批资料的类型，混合类型分批处理。已知管理员引用与指定 member 冲突时拒绝，不能强行改类。

迁移流程如下：

1. 停止使用源资料的 UI、后台、Hub、Sender 与 worker；工具获取可用的 background/state/binding 所有权锁。旧版本独立 CLI 配对并不识别新锁，因此必须离线执行，不能声称文件锁已覆盖所有旧写入者。
2. 按 layout 识别持久文件、引用和共享关系。desktop 覆盖 `hub`、`profiles`、`output` 及当前支持的 `outputs/main`；hub 覆盖 Hub 与其 AirPlay；profiles 覆盖目录内正式配对 JSON。遇到未知持久资料先列为不支持，不静默漏复制。
3. 在目标同一父目录创建随机私有 staging 目录。仅复制所需持久状态，不复制 socket、锁文件、PID、日志、缓存、临时邀请和 `runtime-key-*`。
4. 逐个读取源秘密，保持原字节值和引用 UUID；同一目标 store 内去重。跨 store 的旧共享引用可以各存一份，但保持管理员 kind。校验 TLS 私钥与证书匹配、AirPlay key 算法、管理员 token 与 Authority 摘要及身份一致。Sender 只做本地结构校验，是否已经被服务端撤销不依赖迁移网络请求决定。
5. 写新版本 profile 和所有秘密；保持 Hub UUID、证书、request/device/invitation UUID、pending、Authority revision、撤销状态、输出绑定 ID/epoch 和 AirPlay 信任集合。将 v1 绝对 `state_path` 转为目标内部相对路径；源 state 在 source 之外时明确拒绝，要求先把相关文件纳入完整迁移范围。
6. 校验所有引用均可读取，重新解析 Authority 与绑定；源快照在复制前后变化则失败。同步文件及目录，将 staging 发布为目标目录；发布必须保证不覆盖另一进程创建的目标目录，不能用“先 exists 再无条件 rename”。实现平台对应的排他发布或目标占用协议并测试。
7. 输出脱敏结果和新目录位置。启动新二进制显式指向目标目录，确认身份保持和认证正常；源目录与旧库条目保持原样。

失败只清理本次 staging；进程崩溃遗留 staging 不作为可启动 profile 交给用户，清理由显式恢复入口按本次迁移归属处理。目标已存在时不覆盖；重复调用可验证已完成结果后退出，或要求使用新目标。迁移是离线复制操作，不需要跨文件原地事务日志。

### 6.3 回退边界与旧条目清理

切换验证期间，新旧实例不能同时运行同一身份。目标尚未接受新的配对、撤销或设置写入时，可停止新版并恢复旧二进制与源目录。目标已有新写入后，源目录已过时，不能直接回退，否则可能恢复已撤销授权；应保留新目录并修复，反向迁移是另一个显式任务。

初版迁移工具不自动删除旧 vault 条目，避免破坏保留的源目录。完成验证并决定放弃旧版回退后，单独按迁移清单清理旧条目和不再使用的源资料；不能扫描删除整个 `com.neonmix.identity.v1` service。报告只记录计数、状态和必要的本地迁移标识，不包含 token、私钥或原始文件内容。

完整复制文件目录现在等于复制本地身份，备份必须包含 `.credentials` 并维持访问权限；诊断包与构建归档必须排除它。普通删除不承诺介质级安全擦除，备份中的令牌仍应通过服务端撤销失效。

## 7 代码与文档改动清单

| 位置 | 修改内容 |
|---|---|
| `crates/identity/src/store.rs`、`profiles.rs`、`lib.rs` | 新 store、typed schema、共同校验与错误；替换公开 vault 模块 |
| `crates/identity/src/files.rs`、`files_windows.rs` | 私有目录、受限读取、句柄校验、可靠替换和必要同步 |
| `apps/hub/src/identity.rs`、`main.rs` | setup/pair/config/credential/forget，明确格式分流，文件探针替代 vault-probe |
| `apps/hub/src/server.rs` | 正式 v2 必须恢复已存在的 Authority，阻止缺失 state 时意外创建新身份 |
| `apps/hub/src/airplay.rs` | receiver 版本、文件 key、故障保留身份、运行文件清理 |
| `crates/desktop-service/src/daemon.rs`、`lib.rs` | 产品格式识别、列表、Hub 设置、删除保护、可解释错误；IPC 不传秘密 |
| `crates/identity/Cargo.toml`、`Cargo.lock` | 移除默认 keyring 后端；Windows 文件 ACL 依赖保留 |
| `apps/credential-migrate`、根 `Cargo.toml` | 独立工具与 workspace 排除规则，三端旧库读取只留在此处 |
| tests、E05/E07/AirPlay scripts、Windows runtime、CI | 文件测试、fixture 和清理改造；独立迁移测试 |
| README、主设计、E05/E07、IPC/AirPlay 合约、ADR | 实施时更新当前契约；历史证据保持原样 |

删除 keyring 后不能按包名批量删除全部 D-Bus 依赖；其他桌面能力可能仍需要它。依据实际依赖树清理。旧测试日志中的 Keychain/Credential Manager 是历史事实，不应改写为新方案证据。

## 8 验收标准

新建 Hub、正式配对、重启认证、pending 恢复、forget、撤销、桌面控制和 AirPlay 均使用文件凭证。正常产品与常规测试不调用 native vault、不创建系统凭证条目，不需要钥匙串确认、Secret Service 或 Windows 凭证会话。

macOS、Windows、Linux 都运行真实文件读写与失败测试；无桌面会话时仍可完成纯文件测试。迁移验证旧身份和权限状态逐项保持，故障不会产生半成品正式目录或触发身份重建。三端 ACL/权限差异仍分别验收；纯文件通过不代表三端音频、驱动、GUI 或实际 Apple 来源验收通过。

详细阶段、测试矩阵、实施命令和完成判据见[配套开发计划](06_NeonMix_凭证文件存储_开发计划.md)。
