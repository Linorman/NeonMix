# NeonMix 凭证文件存储开发计划

日期：2026-10-02。状态：功能已实施、macOS 验证；跨平台与真实迁移验收待执行。设计依据：[凭证文件存储修改设计](05_NeonMix_凭证文件存储_修改设计.md)。

目标是让正式 Hub、Sender、桌面和 AirPlay 使用普通文件持久化凭证，并把系统凭证库依赖限定在一次性迁移工具中。完成后，常规测试在项目内创建独立目录即可执行，不需要访问真实用户的 Keychain、Credential Manager 或 Secret Service。

本轮按用户要求仅在 macOS 进行测试。已完成实现与验证的条目下方勾选；三端运行、真实 native 迁移和现有资料切换保留未完成。实施与证据见 [本轮交付记录](docs/evidence/credential-storage/implementation-20261002/README.md)。

## 1 执行顺序与交付条件

| 阶段 | 依赖 | 交付物 | 完成判据 |
|---|---|---|---|
| P0 冻结格式与调用边界 | 无 | schema、错误、fixture 约定 | Hub、Sender、AirPlay、桌面、实验格式和迁移均有明确分流 |
| P1 文件存储与基础 I/O | P0 | FileCredentialStore 与自动测试 | 三端真实文件测试通过，无 native vault 访问 |
| P2 Hub 与 Sender | P1 | 正式 CLI 文件流程 | setup、pair、pending 恢复、forget、重启保持身份 |
| P3 桌面与 AirPlay | P2 | 后台和接收端接入 | 正式文件 profile 可用、共享管理员受保护、接收密钥不变 |
| P4 旧资料迁移 | P0–P3 | 独立迁移二进制及故障测试 | 完整复制、身份不变、失败保留源、切换可验证 |
| P5 探针与依赖清理 | P2–P4 | 新 fixture、CI、干净的默认依赖图 | 常规探针不创建/删除系统凭证；迁移工具独立构建 |
| P6 三端验收与契约更新 | P1–P5 | 平台证据、文档、完成记录 | 下文发布 checklist 全部满足或明确列为未完成 |

建议按阶段拆提交，P1 可先增加模块而不改变运行入口，P2/P3 一起形成可用候选版本；P4 完成前不替换已有正式资料所用的程序。实施期间不把“新建资料可用”当作“既有用户升级完成”。

## 2 P0 格式和接口 checklist

- [x] P0.1 重新核对 `identity.rs`、`airplay.rs`、`daemon.rs`、`files*.rs` 的当前改动；记录本轮源码/制品版本，保留其他任务已有修改。
- [x] P0.2 在共享 `profiles.rs` 定义 Hub/Sender v2 和 AirPlay receiver v1；统一 `credential_store: "file"`，token profile 增加本地 `profile_kind`，明确旧 v1/无版本 receiver 的识别。
- [x] P0.3 定义 `SecretRef`、四种 SecretKind、秘密条目 v1、闭合字段、64 KiB 秘密读取上限；为其他 profile 设定独立限额并记录原因。
- [x] P0.4 定义 profile 父目录 `.credentials` 的固定解析规则、规范 UUID 文件名、完整引用身份和相对 `state_path`。
- [x] P0.5 明确正式格式、旧平台格式、旧实验格式三条读取路径；未知版本和混合字段不得落入实验兼容分支。
- [x] P0.6 固定错误码及桌面中文提示；错误字符串、Debug、CLI JSONL 和诊断不得包含 token、私钥、原始 JSON。
- [x] P0.7 创建独立于真实账户的 v1/v2 fixture 生成器；使用运行时生成秘密和证书，不复制用户现有资料进测试仓库。

验收：共享 schema 可被 Hub 与 desktop-service 复用，布局及版本规则与修改设计一致，没有通过全局环境变量选择 store 的隐含行为。

## 3 P1 文件存储 checklist

- [x] P1.1 新增 `crates/identity/src/store.rs`，实现显式目录 `put/get/remove`；只在写入口创建目录。
- [x] P1.2 复用 `files.rs` 的排他新建；秘密落盘和读回校验完成后才返回引用，UUID 冲突不得覆盖旧条目。
- [x] P1.3 实现私有目录创建；Unix 0700/0600，Windows 受保护的当前用户 DACL。保留 `windows-sys` 所需 feature。
- [x] P1.4 增加有界、基于句柄的读取验证；拒绝坏 JSON、未知版本、错误 kind、错误所有者、过宽权限、链接、reparse point、非普通文件和 ADS。
- [x] P1.5 审查原子替换和同步顺序；Unix 同步必要的父目录；Windows 验证已有目标替换和共享冲突失败行为。
- [x] P1.6 补稳定 OS 锁 helper，使用非阻塞获取和明确 busy；锁文件不删除、不替换，异常退出不形成永久锁死。
- [x] P1.7 `remove` 对不存在条目幂等，对存在的错误 kind/不安全条目拒绝；仅删除指定条目。
- [x] P1.8 秘密封装不输出明文 Debug，序列化入口限定在持久化模块；明确读回失败和部分写入的自有文件清理。
- [x] P1.9 默认自动测试使用 `.local/tmp/credential-store-<uuid>`，并行目录相互隔离；fixture Drop 和脚本 finally 清理自有内容。
- [ ] P1.10 执行测试矩阵 T01–T06；按三端分别保存结果，不以交叉编译代替权限或文件替换运行验证。

验收：普通文件 round trip、故障恢复、隔离与权限测试不再 ignored，且不接触系统凭证库。测试缺少音频 SDK 时仍可单独运行 `neonmix-identity`。

## 4 P2 Hub 与 Sender checklist

- [x] P2.1 `setup` 先保存 TLS key 和 admin token，保证 `server.json/admin.json` 共享同一管理员引用，最后发布 `server.json`。
- [x] P2.2 保留空目录和初始化并发保护；正常错误仅回滚自建文件；异常退出后不覆盖残缺目录，提供清楚的 `setup_incomplete` 和显式清理方式。
- [x] P2.3 新 `state_path` 写为 `state.json`；loader 相对 profile 解析并限制在目录内，不因目录搬迁回写旧路径。
- [x] P2.3a 正式 v2 缺少/损坏 state 必须失败；调整 `server::serve` 当前缺失 state 时新建 Authority 的分支，避免意外更换 Hub UUID，并校验管理员身份。
- [x] P2.4 `config/credential` 读取新 store，正确区分 token kind；旧平台 profile 返回 `migration_required`，不尝试访问系统库。
- [x] P2.5 `pair` 在发网络请求前持久化 token 和 pending；已有 pending 使用同一 token/request UUID/invitation UUID，禁止隐式替换。
- [ ] P2.6 在响应丢失、Hub 重启、`/v1/me` 失败和最终 profile 替换失败后，验证恢复流程不重复登记设备。
- [x] P2.7 `pair/forget` 共用 profile 操作锁；并发操作返回 busy，避免删除后又被在途 pair 重新发布 profile。
- [x] P2.8 CLI 和桌面共用成员删除保护；禁止删除管理员 profile/管理员 kind，识别共享管理员引用和已知管理员设备身份。
- [x] P2.9 forget 先删成员秘密、后删 profile；中断后可重试；不改 Hub 的成员授权和撤销记录。
- [x] P2.10 将成功事件 `credential_store` 改为 `file`；新增 `credential-store-probe --directory <path>`，所有测试数据限制在其自建子目录。
- [x] P2.11 移除默认产品 `vault-probe`；旧自动化获得明确失败/升级提示，不保留会静默调用系统库的别名。
- [x] P2.12 运行 T07–T10；保留 `init` 实验兼容测试，并确认桌面仍拒绝实验凭证。

验收：正式 CLI 可连续完成 setup → serve → invite → pair → snapshot → restart → forget；错误和缺失秘密不会改变身份或生成替代凭证。

## 5 P3 桌面与 AirPlay checklist

- [x] P3.1 `daemon::credential/profiles` 改用共享 typed 元数据解析；仅检查字段存在的旧分支全部移除。
- [x] P3.2 `HubStart/HubSettings` 接受 v2；修改名称/输出时保留未知版本拒绝、store 标签、两个引用和 state 路径约束。
- [x] P3.3 profile 列表与状态轮询只读取元数据；真正执行认证的 CLI 才加载秘密，IPC 响应不返回秘密。
- [x] P3.4 修正 `ForgetCredential`：比较完整 store+UUID 和已知管理员身份，保留先停 Sender、再按 revision 禁用绑定、最后删除资料的顺序。
- [x] P3.5 为旧资料、缺失文件、权限错误、busy 提供可理解提示；不得以失败触发自动 setup、重新配对或自动发声。
- [x] P3.6 修改 `tests/lifecycle.rs` 中的假 vault profile，补新格式和错误格式场景，不把 JSON 字段存在当作验证通过。
- [x] P3.7 AirPlay `Saved` 加版本和 store 标签，保留 `playback_mode` 缺省兼容；已有旧记录返回迁移要求。
- [x] P3.8 `run_worker` 通过 receiver 本地 store 获取/创建 key；仅新建 receiver 可生成 key，失败不得改变原公钥。
- [x] P3.9 保留临时 PEM 的 worker 接口；退出清理和下次持锁启动清理只处理 `runtime-key-*`，不删除 `.credentials`。
- [x] P3.10 验证 AirPlay 重启后 `known_keys/blocked_keys/playback_allowed/playback_mode` 与 key 不变；被撤销来源仍拒绝。
- [x] P3.11 对日志、stderr、IPC、诊断导出注入测试秘密进行泄漏断言；保留现有白名单测试。
- [ ] P3.12 执行 T11–T13；已有音频行为仅做必要短回归，存储改动不要求重跑无关 DSP 长测。

验收：后台设置、读取、开始/停止、配对、删除和 AirPlay 身份持久化都使用文件；UI 关闭后后台生命周期保持原行为。

## 6 P4 迁移 checklist

- [x] P4.1 新增独立 `apps/credential-migrate` manifest/lockfile，从根 workspace 显式排除；keyring 的三端依赖只进入该工具。
- [x] P4.2 实现设计约定的 desktop/hub/profiles 三种 layout，拒绝目标存在、源目标重合/嵌套、外部 state 路径及未支持的持久文件。
- [x] P4.3 源读取前检查停机条件并获取适用的 background/state/binding 锁；记录旧独立 CLI 无法完全受新锁约束的离线要求。
- [x] P4.4 枚举 Hub、admin、Sender 和 AirPlay 引用；已知管理员别名保持 admin kind。独立 profiles layout 要求 `--profile-kind admin|member`，混合类型分批；与已知管理员引用冲突时拒绝，不能用 `name == admin` 猜测。
- [x] P4.5 native 读取放在受父进程约束的 helper 中，超时/取消可结束进程并回收；不把私钥/token 通过 argv、环境变量或日志传递。
- [x] P4.6 在同父目录私有 staging 中重建条目，完整读回；同 store 内共享引用去重，跨 store 的管理员别名保持保护。
- [x] P4.7 验证 TLS 私钥匹配证书、admin token 匹配 Authority 身份/摘要、AirPlay Ed25519 key；Sender 离线迁移不要求服务器在线。
- [x] P4.8 保留证书 DER、UUID、token 原值、pending 请求、Authority revision/撤销、输出绑定 ID/epoch、AirPlay 信任集合与模式。
- [x] P4.9 只迁移持久文件；排除 socket、PID、锁、日志、缓存、邀请和运行 PEM；将 `state_path` 转为新目录内部相对路径。
- [x] P4.10 发布前重新解析所有文件、校验引用并复核源快照未变化；实现不覆盖既有目标的排他目录发布。
- [x] P4.11 在 native 读取、条目写入、profile 写入、校验和最终发布各边界注入失败/强制退出；源保持不变，未完成目标不被宣布可用。
- [x] P4.12 实现重复执行与遗留 staging 清理规则，只清理本次归属的路径；报告脱敏，不自动删除旧 vault 条目。
- [x] P4.13 用 mock 旧读取器覆盖缺失条目、错误权限、超时、损坏、共享引用、pending 和已撤销设备；普通迁移算法测试不访问 native vault。
- [ ] P4.14 三端各做一次显式 native fixture 迁移验收，仅操作本次创建的旧库条目并清理；无权限时记录未完成，不把 mock 当作 native 成功。
- [x] P4.15 输出切换/回退说明：目标未发生新状态写入才可回到源；发生新撤销等状态变化后禁止直接恢复旧快照。
- [x] P4.16 对正在使用的实际资料，仅在后续明确执行切换任务时运行迁移；本设计交付不读取或改动用户秘密。

验收：T14–T16 通过，迁移后身份一致性报告齐全；源资料、旧 vault 条目保持可用，正常产品无需加载迁移依赖。

## 7 P5 探针与依赖 checklist

下列文件是本次阅读发现的直接修改入口。开发时再次检索，避免并行新增脚本遗漏。

| 文件 | 具体改造 |
|---|---|
| `crates/identity/tests/native_vault.rs` | 从默认产品测试移除；file store 用例成为普通测试，native 迁移用例归独立工具 |
| `tools/e05_probe.py` | vault-probe 换文件探针；缺失条目注入改为 fixture 文件；清理去掉 `security delete-generic-password` |
| `tools/e07_background_probe.py` | 更新 store/版本断言和旧 `native_vault_profiles_without_plaintext_secrets` 名称；秘密只存在专用文件，不能再声称没有明文落盘 |
| `tools/e07_mixer_probe.py`、`e07_native_ui_probe.py`、`macos_sender_ui_probe.py` | fixture 和清理转为受控目录；保留控制、UI 和诊断断言 |
| `tools/airplay_hub_probe.py`、`airplay_mixer_probe.py`、`airplay_mac_source_probe.py` | 接收 key 文件清理、身份保持与错误注入，删除 fixture Keychain 操作 |
| `tools/airplay_manual_session.py` | start/restart/replace 保留 `.credentials`，clean 只删除所管 fixture；旧状态先明确迁移 |
| `tools/windows_runtime_probe.py` | Credential Manager 常规测试改为真实 Windows 文件 store 测试 |
| `.github/workflows/ci.yml`、`windows.yml`、`tools/check.py` | 接入新测试，避免常规 CI 构建/执行旧 native store |
| `tools/check_e05_cross.py`、构建准备与打包脚本 | 更新依赖说明，检查移除 keyring 后的实际依赖；排除秘密目录及迁移工具测试数据 |

- [x] P5.1 逐项完成上表改造；所有 fixture 明确 owner 目录，不使用 HOME 下默认凭证路径。
- [x] P5.2 新建无声卡的 `tools/credential_store_probe.py`，执行文件隔离、setup 持久化及错误场景；有媒体 SDK 的配对集成与声卡探针分层运行。
- [x] P5.3 默认 `neonmix-identity` 移除 `vault` 和 keyring 的三端依赖；再更新根 `Cargo.lock`，不手工删其他功能仍使用的 D-Bus/系统 API。
- [x] P5.4 检查 Hub、桌面、后台的实际依赖图不含 keyring；独立工具使用自己的构建入口与锁文件。
- [x] P5.5 代码检索无默认产品 `vault::` 调用，无常规探针系统凭证删除；仅迁移工具、发布公证工具和历史文档允许各自对应的引用。
- [x] P5.6 打包、诊断导出、日志归档显式排除 `.credentials`、运行 PEM 和含秘密的 staging；覆盖隐藏目录不会被意外归档的测试。
- [x] P5.7 原生测试失败日志不 dump 完整 profile/store；保留清理成功、计数、故障阶段等证据。
- [x] P5.8 清理本次无用临时目录和 fixture，不清理其他会话正在使用的 `.local` 数据。

验收：删除 fixture 根目录即可清理新存储；常规测试无需系统凭证服务，且仍实际覆盖正式配对而非全部退化为 `init` 实验测试。

## 8 测试矩阵

| 编号 | 场景与故障点 | 预期结果 | 执行范围 |
|---|---|---|---|
| T01 | put/get/remove、重复 remove、UUID 碰撞 | 原值一致，删除幂等，既有条目不覆盖 | 三端普通自动测试 |
| T02 | 两个目录使用同 UUID、改变 cwd、并行 fixture | 数据隔离，定位由 profile 决定 | 三端普通自动测试 |
| T03 | 错误 kind/版本、超限、损坏 JSON、缺失文件 | 稳定错误，无 native fallback/身份重建 | 三端普通自动测试 |
| T04 | symlink/reparse、ADS、目录代替文件、过宽权限、错误 owner | 读取或写入在不泄漏/覆盖的情况下拒绝 | 各平台适用项；无法构造的权限项注明未执行 |
| T05 | 写失败、读回失败、替换失败、提交前后强退 | 旧 profile 完整；未发布秘密不被使用；只清理自建临时文件 | 故障注入加真实子进程退出 |
| T06 | 同 profile 并发 pair/forget、锁 owner 崩溃、Windows 共享句柄 | 冲突有界失败，锁可恢复，目的文件保留 | 三端真实文件/进程测试 |
| T07 | setup 中断/重跑、目录已有数据、秘密缺失 | 不覆盖、不再造身份；明确不完整状态 | CLI 集成 |
| T08 | 正式配对、登记后丢响应、Hub 重启、收尾落盘失败 | token/request/device 保持，不重复登记 | HTTPS 正式流程 |
| T09 | forget 中途退出、管理员及共享别名删除、Sender 副本 | 幂等恢复；管理员拒绝；共享成员引用影响符合文档 | CLI 与桌面后台 |
| T10 | 异证书/异 UUID、邀请失效、撤销后的 WSS/媒体、旧实验格式 | 原信任/权限语义保留；桌面拒绝实验格式 | E05 相关集成探针 |
| T11 | Hub 设置、列表、SenderStart/Stop、绑定 disable、诊断 | 新 profile 正常；无秘密进入 IPC/导出；revision 语义保持 | desktop-service 和 E07 探针 |
| T12 | AirPlay 初建、重启、缺 key、坏 key、worker 退出/强退 | 私钥稳定、缺损不换钥、临时文件可清理 | Hub/worker 数字协议探针 |
| T13 | AirPlay 已知/阻止来源、播放模式、PIN 窗口 | 信任与模式保持，撤销仍有效 | 合成协议与已有真实来源短回归 |
| T14 | 完整 Hub/desktop/Sender/pending/无 AirPlay 资料迁移 | 所有引用可读；逐字段身份与授权相同 | mock 原库自动测试 |
| T15 | 原库超时/缺 key、源变化、目标竞争、复制中强退 | 无源写入、无已发布半成品、可重试且不覆盖 | 迁移故障测试 |
| T16 | 同平台真实旧库迁移、目录搬迁、切换与回退 | 原认证有效，不回写旧 state；回退限制可复现 | 三端单独显式验收 |
| T17 | 无凭证服务/无桌面凭证会话启动纯文件探针 | 不出现凭证库访问、弹窗、等待 | macOS、Windows SSH、Linux 无 Secret Service 环境 |
| T18 | 默认依赖、日志/诊断/打包/临时清理 | keyring 隔离、秘密不导出、不残留测试账户条目 | CI 与归档检查 |

`T05` 的注入测试验证操作边界，不能代替三端真实文件替换；`T17` 只验凭证路径，不要求无桌面环境同时通过声卡、GUI 或托盘测试。跨平台 CI 尚未具备的项目保持未勾选。

## 9 执行命令与证据

所有下载、缓存、测试状态和构建产物留在项目目录。以下为开发入口；实际 macOS 执行记录与未执行项以本轮交付记录为准。

```sh
tools/dev cargo fmt --all -- --check
tools/dev cargo test --locked -p neonmix-identity
tools/dev cargo test --locked -p neonmix-hub -p neonmix-desktop-service
tools/dev cargo clippy --locked -p neonmix-identity -p neonmix-hub -p neonmix-desktop-service --all-targets -- -D warnings
tools/dev cargo build --locked --release -p neonmix-hub -p neonmix-desktop-service -p neonmix-desktop
tools/dev cargo tree --locked -p neonmix-identity
tools/dev python3 tools/check_e05_cross.py --clippy
```

Hub 测试/构建需要现有 GStreamer 开发环境，按 README 准备；不要因这一需求把纯文件测试也绑定到音频 SDK。交叉脚本只覆盖其现有 identity/control 范围，不能计作 Windows/Linux 完整运行成功。

已新增以下入口（独立迁移工具推荐使用 tools/credential_migrate 保持 target 隔离）：

```sh
tools/dev python3 tools/credential_store_probe.py
tools/credential_migrate test --locked
tools/credential_migrate build --locked --release
```

独立工具首次确定依赖时生成其 lockfile，之后使用 `--locked`。Windows 使用同参数的 `./tools/dev.ps1` 前缀；Linux 使用 `tools/dev`。迁移工具常规 tests 使用 mock，真实旧库验收必须单独显式调用专用测试入口，不让 `--all-features` 意外触发系统凭证访问。

针对性检查通过后，在候选合并前运行一次 `tools/dev python3 tools/check.py --native-media --only format clippy tests release`，Windows 替换前缀。正式媒体、E05/E07/AirPlay 探针按本次影响运行；已有长测仅在新失败指向媒体行为时扩大。运行记录保存于 `artifacts/credential-storage/<run-id>/`，归档脱敏摘要至 `docs/evidence/credential-storage/`。

证据至少包含源码/二进制哈希、平台、命令、退出码、各 T 编号结果、未执行理由、故障注入位置与清理结果。身份一致性比较可记录布尔结果和公开标识，不能把私钥/token 原值加入报告。

## 10 P6 文档与交付 checklist

- [x] P6.1 新增 ADR，声明取代 ADR-011 的平台存储选择及 ADR-012 中对应依赖；首次信任、配对协议和撤销规则保持原定义。
- [x] P6.2 更新 `01_NeonMix_设计方案与技术选型.md` 的 SQLite/凭证选型和相关文字；更新 `02_NeonMix_完整开发计划.md` 的验收边界。
- [x] P6.3 更新 `03/04` AirPlay 设计与计划的 key 存储；更新 `docs/AIRPLAY-CONTRACT.md`、`AIRPLAY.md` 和 worker README。
- [x] P6.4 更新 `README.md`、`docs/E05.md`、`E07.md`、`DESKTOP-IPC-CONTRACT.md`，说明文件位置、备份范围、迁移、forget/revoke 区别和相对路径。
- [x] P6.5 在 `docs/STATUS.md` 新增实际实施结果及三端证据，不改写旧 Keychain/Windows 测试历史；审计文档新增说明而非冒充历史已使用新方案。
- [ ] P6.6 三端 T01–T06/T17/T18 完成；正式流程 T07–T13 按平台能力记录通过与未完成；关键未完成项不能计作三端交付。
- [ ] P6.7 T14–T16 验证迁移保持身份和撤销记录；至少覆盖正在使用的 Hub + admin + AirPlay 组合以及独立 Sender/pending。
- [x] P6.8 默认产品和普通测试不含 native vault 依赖或调用；迁移工具独立构建且无自动启动入口。
- [x] P6.9 验证旧 profile 有清楚迁移提示、未知格式拒绝、凭证缺失不重建；桌面 IPC 仍不传秘密。
- [x] P6.10 所有相关探针完成目录清理；没有遗留本轮创建的 native 迁移测试条目，没有把秘密目录加入 Git、打包或诊断。
- [x] P6.11 开发交付说明列出实际改动、验证范围、剩余问题及资料切换步骤；本计划只勾选已有证据的项。

完成定义：新建、日常运行、测试和故障恢复全部使用同一文件方案；旧资料有保留身份的可执行迁移路径；平台凭证库只剩独立、按需运行的历史迁移职责。

2026-10-02 本地切换补验：当前 macOS AirPlay Hub/admin/receiver 与默认桌面实际迁移、身份核对和 HTTPS 认证通过；历史测试房间原库条目已丢失，未重建。见 [本地迁移记录](docs/evidence/credential-storage/local-migration-20261002/README.md)。P4.14/P6.6/P6.7 含三端或其他迁移场景，仍不据本次局部验收全部勾选。
