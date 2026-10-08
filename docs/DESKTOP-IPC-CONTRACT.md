# E07 desktop IPC and process ownership

## P02 bound command intents (2026-10-07)

`ServiceStatus.intent_version=1` advertises `Request::Intent`. The envelope binds the credential reference, expected Hub UUID and credential/device UUID, plus an immutable native/AirPlay command with its original request ID, runtime and conditional revision. The background validates the profile identity, freezes metadata beside its immutable secret reference, forwards the original command ID unchanged, and removes private `.neonmix-intent-*.json`/command fixtures on every completed path. Both servers also validate the authenticated device against `credential_id`, covering credential-path replacement after preparation. A missing capability/runtime produces `upgrade_required`.

`ResolveAirplayPatch` is a read-only compatibility preparation: refresh the same source/session/stream epoch, require matching runtime capability, preserve unspecified fields and return the exact resolved command plus actual before fields. Desktop cancels a result whose context changed, then freezes the resolved command before mutation. Unknown reconciliation resends this frozen command; it never re-runs resolution or adopts a new revision. Native/AirPlay domain versions remain separate; status completion does not imply audio configuration has been applied.

The UI links `neonmix-desktop-service` and calls its synchronous `Client` on a worker thread. The audio owner is the separate `neonmix-background` executable beside `neonmix-hub` and `neonmix-audio`. `Client::ensure_background()` detaches the background from the UI process group on Unix and starts it without a console window on Windows. No system service, background installation, or automatic media restart is involved.

The persistent state directory is supplied by `--state-dir`; the desktop default is project-local `.local/desktop`. Hub profiles live in `hub`, paired Sender profiles in `profiles`, and the UI's output binding in `output`. Profiles contain v2 file-store UUID references; plaintext product tokens and private keys exist only in adjacent .credentials files. Product IPC rejects laboratory plaintext credentials.

## Transport and authorization

macOS/Linux use a private Unix-domain socket `ipc.sock`. The real state directory must belong to the effective user and have mode `0700`; the socket has mode `0600`. Existing ancestors and request paths cannot contain symbolic links. The daemon verifies the peer effective UID using `getpeereid` on macOS and `SO_PEERCRED` on Linux. A private exclusive `flock` prevents duplicate daemons and protects stale-socket recovery.

Windows uses a local NamedPipe whose name derives from the current process user SID and the canonical state directory. The directory has a protected DACL granting access to that SID. Every pipe instance has the same protected SID-only DACL and rejects remote clients. Both endpoints query the connected peer process ID and verify its token user SID. A file opened with no sharing prevents duplicate daemons; the first NamedPipe instance additionally rejects preexisting pipe servers. Request paths cannot traverse reparse points or files owned by another user. The Windows source passes cross-target type checking and Clippy on the macOS host; Windows runtime behavior has not been accepted on a real machine in this task.

The wire frame is a four-byte big-endian unsigned byte length followed by UTF-8 JSON. `Envelope` contains `{version: 1, request: Request}`. `Request` is a closed Serde tagged enum. Unknown commands, fields—including extra fields on unit variants—and protocol versions are rejected. IPC carries no shell text, executable paths, arbitrary command arguments, roles supplied by a caller, or plaintext credential secrets. Command-line subprocesses are built from fixed argument vectors.

Frames are at most 262,144 bytes. The daemon admits eight requests concurrently; Windows reserves at most ten pipe instances, including accept/listener instances. Reading or writing a frame has a three-second deadline; acquiring the runtime has a five-second deadline. A CLI command has a 25-second deadline and bounded stdout/stderr reads. A complete operation has a 35-second deadline; client reads/writes have a 45-second timeout. Per-process JSONL telemetry is bounded to 16 KiB per line and retains only the latest event and statistics. Failure history retains 32 entries.

## State, stopping, and recovery

`Status` reaps exited children and reports actual process IDs, running/failure state, persisted Hub settings and pairing identities, active Sender options, and the persisted output binding. `Snapshot` comes from the pinned authenticated Hub and also includes `viewer` from `/v1/me`, including its server-authorized device ID and role. UI controls remain revision-checked through `neonmix-control::Operation` and the server's authoritative permissions. IPC rejects direct device registration and media-offer creation.

Closing, hiding, freezing, or crashing the UI does not own or end Hub/Sender media. `SenderStop`, `HubStop`, and `Shutdown` terminate and reap their owned processes. Unix sends `SIGTERM`, which Hub/Sender handle gracefully; after five seconds the daemon kills a child that has not exited. Windows terminates and reaps its owned child. Explicit stop clears active Sender options and never starts another session. Unexpected child exits are reported and require explicit restart. Existing output bindings and file profiles survive UI or background restarts.

Urgent stopping uses an epoch plus `Notify`. A validated stop/shutdown request interrupts a pending read-only Snapshot/Diagnostics/Discover/Devices/Airplay/AirplayV2 operation before waiting for the runtime mutex; cancellation drops and kills that operation's CLI subprocess. Registration, pairing, invitations, controls, and other mutations are not canceled by this mechanism because their remote commit may already have succeeded. Per-request temporary files have drop guards so deadline/cancellation paths also remove them.

One instance cannot run Hub playback and Sender transmission concurrently or pair/send against its own persisted Hub UUID. The Hub never falls back to a different selected output. Settings edits require stopped Hub playback and preserve the established Hub identity and native credential references.

## Pairing, output binding, and diagnostics

`Discover` returns one `discovery_complete` object containing the final `candidates` array, including an empty array when no rooms remain. CLI `resolved`/address-update events are transient and are not returned as the desktop command result. Missing completion or a malformed candidate list fails the request rather than displaying discovery success with an invented empty result.

`Invite` returns invitation text for temporary UI display/copy and removes its private output file after reading it. `PairText` writes a private temporary invitation, invokes the file-store pairing flow to `profiles/sender.json`, then removes the temporary invitation. Single-use, cancellation, expiration and device authorization remain server responsibilities.

`Output` uses persisted binding revisions for add/show/rename/enable/disable/remove. NeonMix add/rename attempts native display-name synchronization and explicitly returns success or a failure warning; the BlackHole provider is never renamed. Disabling/removing the binding is observed by the Sender's permission watcher.

On Linux, NeonMix `Output Add` starts the independent E06 `neonmix-audio virtual-output` owner and waits for the exact virtual device to become ready. A same-UID owner already holding E06's abstract-socket reservation is reused; it is never adopted or killed by this instance. Reopening with a saved NeonMix binding and explicitly starting Sender also restores its local owner if needed. Sender stop, disabling/removing bindings and forgetting a pairing keep the node owner alive; shutdown ends only an owner process created by this background. Native naming still uses E06 synchronization. This Linux path is cross-type-checked on macOS and has not been runtime-tested in this task.

`ForgetCredential` is a distinct user action for Sender profiles. It refuses Hub admin identity and references shared with that identity, stops Sender transmission, disables persisted output bindings by revision, and then deletes the Sender file-store secret and profile. The reply sets `output_authorization_required`: pairing again with the same Hub allows explicit Enable of the saved binding, preserving its stable output ID. Switching to another Hub requires removing the old binding and adding the new Hub binding. This removes no native audio driver.

Functional diagnostics retain numeric meters and ephemeral stream IDs so Mixer lanes can be associated with snapshots. Identity fields, names, routes, certificates, credential references and arbitrary error strings are redacted from diagnostics. `ExportDiagnostics` applies an additional strict schema-key whitelist and keeps only numeric, boolean and null telemetry, preserving array positions; unknown fields and every arbitrary string are excluded. It writes only `diagnostics-redacted.json` inside the private state directory with private file permissions. Local process and numeric command-fault telemetry can still be exported when a remote Hub is unavailable.

Estimated component delays remain component diagnostics; the product must not present them as a measured end-to-end latency. Native test tones explicitly select an output, last two seconds and use a fixed `-36 dBFS` source gain.

## macOS verification

`tools/dev cargo test -p neonmix-desktop-service` covers UI disconnect/reopen, actual child stop/kill/reap, unexpected child failure, single-daemon locking, private permissions, malformed/oversized/versioned requests, unknown nested fields, path traversal and symlinks, plaintext profile rejection, stalled-frame timeout, urgent read cancellation, repeatable ephemeral invitations, strict export whitelisting, binding restoration, and Sender forgetting. Native probes and evidence are recorded separately under `docs/evidence/e07`.

文件凭证的格式、备份、目录搬迁、显式迁移与回退见 [文件凭证使用与升级](CREDENTIAL-STORAGE.md)；存储决策以 [ADR-014](adr/ADR-014-file-credentials.md) 为准。系统凭证测试记录保留为历史证据，不能计作新文件方案的验收。

## Multi-receiver AirPlay v2 (2026-10-02)

The outer desktop envelope remains version 1. A new closed request variant, `airplay_v2`, contains `credential`, optional `hub`, and optional `command`. A missing/null command is a read; a command uses `AirplayCommandV2 { command_id, expected_revision, operation }` from `neonmix-airplay-adapter`. The daemon invokes the fixed sibling `neonmix-hub airplay-v2` CLI, which calls `/v2/airplay`. Command JSON uses a private project-local temporary file with a drop guard. The legacy `airplay` request remains available only for compatible single-receiver clients; the new desktop never sends an implicit-current-source command.

The reply carries `revision`, `enabled`, `multi_receiver`, `receivers[]`, persistent `sources[]`, active `sessions[]`, and `capacity { limit, active, reserved, available }`. Sources and sessions are not added together to count connections. Receiver rows remain visible when their discovery service is hidden. `configured_enabled` records durable selection separately from runtime `enabled`; retained disabled identities are not counted as configured inputs. Per-entry enable/disable persists its selection, while the global off switch only stops runtime reception. Public replies omit source public-key material and credential references. PINs are returned by the Hub only to administrators; the daemon preserves only the top-level legacy PIN or the per-receiver PIN matched by receiver ID in an authenticated AirPlay control reply. Nested history, process logs, diagnostics and exported state continue to redact PINs.

The desktop keys Mixer state and meters by actual stream ID, then targets writes using source ID plus session ID. Receiver array order, lane reuse and names are never control identities. Gain/mute preferences persist per source; Solo is session-only and participates in the entire native/AirPlay room. Restoring a mix or dispatching a queued intent checks that the original source/session still exists; a changed session is not silently retargeted. Offline revoke is explicit with null session ID and remains subject to server revision and session checks.

The UI uses the authoritative write reply and refreshes on conflicts. `configuration_partial`, `saved_session_ended`, and `profile_durability_unconfirmed` are shown as distinct outcomes instead of announcing unconditional success. A full Mixer command queue may report `media_pending`; durable results remain committed while the owner retries the newest configuration. Command ID replay is bounded to 128 process-local receipts at the Hub; the desktop must not infer durable replay of media actions across Hub restarts.

Hub settings owns receiver count, per-entry enable/disable, pairing and visibility. Devices owns persistent sources, offline allow/revoke and playback mode; Mixer owns active source gain/Mute/Solo with session-targeted disconnect/revoke. One unavailable receiver does not disable the entire Mixer. Read-only AirPlay polling is cancelable by urgent stop; mutations retain the existing no-cancel-on-unknown-commit behavior.

macOS verification includes desktop AccessKit/state tests, session-target rejection, meter mapping under receiver reordering, cross-source Solo, per-receiver PIN redaction, existing daemon lifecycle tests, and native preview screenshots at 1100/720 widths. Evidence is under `docs/evidence/multi-airplay-ui-20261002`. This does not establish real Apple source compatibility, native VoiceOver acceptance, four-device hardware concurrency, or long-duration reliability.

## Optional public faults and UI localization (2026-10-07)

The outer envelope remains `version: 1`. `Reply` and `ProcessStatus` add `fault: Option<Fault>` alongside the legacy `error`. Success and healthy status use null/no fault. Missing fields deserialize to None. `Fault` has a closed snake_case `FaultCode` and `params`, with unknown fields denied. `hub_port_in_use` alone permits and requires `port`, an integer in 1..65535; every other code requires an empty parameter object. Unknown future codes or malformed optional faults deserialize to Some(GenericFailure) at the Reply/ProcessStatus boundary, leaving legacy fields usable while preventing unsafe parameter failures being reinterpreted as a legacy code. Deserializing Fault directly rejects them.

```json
{"ok":false,"data":null,"error":"Hub 端口 7443 已被占用；请先退出其他 NeonMix Hub 或占用该端口的程序","fault":{"code":"hub_port_in_use","params":{"port":7443}}}
```

The canonical public vocabulary is `crates/desktop-service/src/fault.rs`; `FaultCode::ALL` and `as_str()` enumerate it. It includes existing control/AirPlay machine codes, file-credential errors, service lifecycle failures, and Windows network-helper errors. The desktop maps these to the corresponding typed `fault-*` message in `crates/i18n/messages/faults.toml`. This service has no locale input or translation-engine dependency.

A failed CLI operation first accepts a valid structured stdout fault, then a recognized stdout legacy machine error, then reduces stderr to a bounded public fault category. Parameters from structured faults survive the IPC boundary. Arbitrary stderr, credential details, paths, names, PINs, PEMs and tokens never become translation parameters or legacy failure text. Existing public control errors remain their original legacy machine strings; newly coded service/profile failures retain actionable compatibility messages. Persistent process status records structured faults before redacting events, processes a final non-newline-terminated log line, and keeps a specific fault when later stderr is generic. An unexpected exit does not replace a recorded specific failure. Explicit normal stops do not synthesize `process_exited`; failed runtime-key cleanup reports `runtime_cleanup_incomplete`.

New clients prefer fault, then use `Fault::from_machine_code` for an old backend's recognized error, then display a localized generic failure. Machine-code compatibility accepts the exact code or a code followed by the documented colon/detail separator; it never translates Chinese prose or exposes the detail suffix. A client must not display an arbitrary legacy error as its primary message. Faults do not bypass diagnostic/export whitelists.

`tests/fault_contract.rs` verifies all four old/new Reply combinations using real Serde round trips through the exact pre-fault structs, and both status directions (including the old/old and new/new status cases). The old structs ignore the added field; the new structs accept its absence. Fault-schema rejection and unknown-data fallback are verified separately. `tests/lifecycle.rs` additionally verifies actual macOS Unix-socket/CLI behavior for structured-fault precedence, parameter retention, machine-code fallback, generic failure, credential/network/key-cleanup failures and secret exclusion. This establishes additive JSON compatibility and macOS fixture transport behavior, not archived release-binary interoperability or Windows/Linux native behavior. Detailed limits and validation are in [the service evidence report](evidence/ui-i18n-20261007/service-report.md).


## 2026-10-07 受管停止

Hub/Sender 受后台管理时显式使用 `--managed-control-stdin`，只接收最大 1024 字节、LF 分隔的 `{"version":1,"type":"stop"}`；未知字段、版本、命令、过长输入和 owner EOF 都进入停止流程。手动 CLI 不读取 stdin，以 Ctrl+C / Unix SIGTERM 停止。读取线程可取消并回收；Windows 的无控制台模式只依靠私有管道。

生命周期任务持续持有 Child。停止请求被取消或 35 秒操作超时后，任务仍执行退出；Stopping 期间拒绝同一 owner 再次启动。所有 owner 并行停止：写入预算 250ms，正常等待 5s，强制回收后核验 2s，Job accounting 与日志排空各最多 250ms。IPC 的 5s 排队、35s 操作与 45s 客户端超时保留；排队失败不发布停止成功。Windows 先挂起进程，加入不可继承的独立 Job Object，再恢复主线程；job 持有整个音频进程树，后台异常退出会触发 kill-on-close。

`ProcessStatus` 增加可缺省的 `stopping` 和 `stop_result`。`HubStop` / `SenderStop` 保持原事件名并附加 `stop_result`；`Shutdown` 附加 Hub/Sender 的结果。字段 `graceful/forced/elapsed_ms/exit_code/cleanup_complete` 分别表示正常退出、是否用过强制回收、单调耗时、实际退出码及运行 PEM 清理核验。只有已确认进程/Job 退出才清除 running/pid；未核验的 owner 保留并可再次 Stop。进程已经退出但清理未完成，返回失败并保留停止结果。重复 Stop 复用同一结果。

`ServiceStatus.managed_control_version=1` 是新增能力字段；旧客户端可忽略，缺失表示不支持安装维护。安装器的 `--shutdown-installation --installation-root <root> --state-dir <original-user-state>` 客户端不启动新后台，校验实际 NamedPipe server PID、用户和安装路径；保留进程句柄，等待音频退出后向本安装 UI 发送 `NeonMix.Maintenance.Exit.v1`。回调在窗口所属线程销毁目标原生窗口，winit 正常结束事件循环并释放 UI 资源；不等待隐藏窗口渲染帧。其他安装、手动 CLI 和普通后台故障不会触发 UI 维护退出。旧后台不兼容时拒绝替换文件，要求手动退出，不按进程名强杀。

只读诊断 `local.network` 区分 `configuration_code` 与 `effective_policy_code`（0 configured、1 missing、2 path_mismatch、3 policy_blocked、4 unknown），同时给出 profile、防火墙启用值、连接网络类别、自有规则匹配及显式 Block 检出。规则已配置不能证明实际网络可达；无法完整判断有效策略时保持 unknown。脱敏导出只允许对应数字、布尔和数组，不导出路径、SID、地址或规则名称。


## P05 启动确认（已实施部分）

`ProcessStatus.running` 表示 Child 尚在本实例管理范围；`ready` 独立锁存本 Child 的业务 Ready。Hub 需要 `hub_started`（配置/输出/listener 初始化完成），Sender 需要 `sender_started`（会话、媒体和采集初始化完成）。普通日志不覆盖 Ready，前一个 Child 的 reader 不会更新新 Child 的状态。自然退出、Stop 或 fault 清除 Ready；迟到 reader 不恢复失败/停止状态。

HubStart 与 SenderStart 最多等待十秒业务 Ready；spawn、尚在运行、等待到期都不作为成功。失败/超时先通过既有托管停止 owner 回收 Child，再返回具体失败；不会留下“启动请求失败但仍采集”的进程。五种真实独立 Child 及真实 IPC Ready→stats 已通过。本节不代表独立生命周期 accept/operation、UI 双 IPC 失败或三端强退已完成，相关门槛仍保留在开发计划 P05。


## P05 独立生命周期入口

后台在同一私有目录建立独立 `lifecycle.sock`（Windows 同 SID/目录哈希的独立 `.lifecycle` NamedPipe），认证规则与普通 IPC 相同，普通八连接配额不占用它的八个短连接配额。请求最多 1024 bytes；只接受 Status、HubStop、SenderStop、Shutdown、实例绑定的 LifecycleStop 及 LifecycleOperation，对其他命令明确拒绝。Status 从内存复制 metadata 与 live Child 状态，不等远端 CLI、profile 锁或 fsync；metadata 在普通事务完成后更新。

Stop 返回 `{operation_id, instance_generation, state:accepted}`，只证明后台锁存请求；独立 owner 继续回收 Child，不随 socket 断开取消。重复在途 Stop 合并；最多保留 32 个 operation，先淘汰完成回执。LifecycleOperation 在同实例查询 accepted/completed 和真实 stop_result/fault。客户端库等待相同 operation 的完成，再向桌面返回完成值；IPC 失败/超时保持结果未确认。Shutdown 在 owned 资源确认回收后保留最多 500 ms 供完成查询，然后即使客户端消失也退出；超过该窗口的调用者可能得到未知结果，不能把连接丢失自动当作成功。

`ServiceStatus.lifecycle` 含版本 1、实例 UUID 和 Hub/Sender 各自的停止代次。Desktop 创建 Start 时生成 LifecycleStart，冻结所见实例/对应停止代次及原目标选项；服务在接入与真正 spawn/install 两处校验。Stop 一经接收就推进对应停止代次；Shutdown 同时推进两种代次并拒绝新 Start。创建早于 Stop、发出晚于 Stop 的 Start 也不能生成 Child。Desktop 的 LifecycleStop 同样固定实例，旧实例命令不能作用于新后台。旧原始 Start 仅保留接入时绑定的兼容语义，新桌面要求 lifecycle 能力；完整旧/新制品矩阵由 P10 验收。

桌面停止不与普通 worker 排队，启动过程中也立即发送。只有带同实例 operation ID 且目标 cleanup_complete 的确认才显示成功；未确认保留原状态与重试入口。停止代次较旧的轮询不能覆盖已确认停止。HubStop、SenderStop、Shutdown 的结果范围分别为共享、发送、本实例全部资源。


## Unix guardian 与进程树回收

macOS/Linux 包内新增 `neonmix-guardian`，后台媒体必须使用同目录组件；每个 Hub/Sender/本地虚拟输出 owner 由一个 guardian 持有。后台只持有其私有 stdin 写端，后台异常死亡触发 EOF；guardian 仍能执行 250 ms 停止写入、5 秒媒体协作及最多 2 秒组退出核验。后台正常停止的外层等待为 9 秒，以免提前强杀正在回收后代的 guardian；完整停止预算维持 10 秒。Windows 继续 Job Object，不依赖此二进制。

媒体处于创建时隔离的进程组，guardian 通过 WNOWAIT 保留未 reap leader，保证停止期间该 PID/PGID 不复用。后代确认退出后才 reap；不从日志 PID、进程名或磁盘 PID 决定强杀目标。`ProcessStatus.pid` 是媒体进程，`owner_pid` 是 guardian，`guardian_result` 单独锁存 version/forced/cleanup_complete/原始 child_exit_code 与固定清理失败码。缺最终回收确认不能升级为完成。日志不覆盖媒体最近业务事件，阻塞的日志消费者也不阻止停止。

回收临时 `runtime-key-<canonical UUID>` 文件时，live owner 先捕获私有目录和 inode/dev。媒体树退出后取得同一 state owner 锁，只删除仍匹配的捕获对象；持久 profile、receiver 身份与其他实例目录不删除。任何目录权限、对象替换、owner 锁或同步错误都保留为 cleanup incomplete；其他 owned 资源继续尝试回收。下一次 Hub 启动仍在其 owner 锁下按既有规则清理遗留文件。

本机原生进程夹具结果与实际音频/跨平台/签名包验收分开。调试构建可由私有测试环境 `NEONMIX_GUARDIAN_AUDIT_DIR` 记录有限终态数字/固定码回执，未包含参数、profile 或密钥；release 不启用该夹具入口。


## P06 HubSettings 本地提交

本地 HubSettings 仍要求先停止 Hub，保持设置的显式保存语义。保存由 `identity::hub_settings` 协调 profile/state 日志，不再分别 replace 后回滚。后台启动及 HubStart 先恢复未完成事务，再读取严格配置；恢复成功但旧请求目标已一致时，不再次递增 revision。

`profile_durability_unconfirmed` 表示提交结果尚未完成确认，`configuration_recovery_required` 表示现有事务阻止新的冲突配置或启动；二者都有双语固定 fault，不回显路径/原始异常。界面保留草稿并提示刷新/重试恢复。配置日志内容与暂存文件不进入脱敏导出。


## P06 输出绑定管理身份

OutputAction Rename/Enable/Disable/Remove 含必需 `expected_output_id` 和 `expected_revision`。UUID 创建于读取 snapshot 的对象作用域，后台原样转发 CLI；缺失、nil或旧对象不会替换为当前对象。Local Forget 在读取需停用的绑定时捕获同一对条件，不在发出时重取 UUID。Add/Show 保持原语义。

保存名称后的 native sync 传递保存回执中的 UUID/revision；如另一个客户端在此间重建对象，只记录 `native_name_synced=false` 与固定替换错误，已保存的原对象回执不被篡改为新对象。Desktop 使用 `output_object_replaced` 双语 fault 提示刷新，危险确认框保持打开时捕获的对象条件。

## 诊断样本与Sender目标（P08）

ProcessStatus新增向后兼容的metrics_sequence/metrics_age_ms；只有媒体stats事件推进序号并记录后台单调时间，Status返回采样年龄而不是Status读取年龄。metrics_observed_at只驻留内存，不序列化。Sender进程变化使客户端计数基线失效；旧后台未提供年龄时采样为未知。诊断远端返回available/sample_age_ms/runtime_epoch，控制快照不刷新诊断时间。

sender_target为当前Sender子进程在sender_started时发布并锁存的已认证hub_id、device_id和可选room_name，普通stats不能改写；UI运行目标与控制身份无关。它仅用于本地功能Status，不进入数值白名单导出。导出可包含available、sample_age_ms、metrics_age_ms、metrics_sequence；未知键、目标身份、名称、地址和路径继续剔除。

## Sender运行期限与本机反馈（P09）

后台发送使用显式--until-stopped，不再传--seconds 86400；到期不会触发每日重连。实验CLI的--seconds限制1..86400，与持续选项互斥；无选项保留10秒实验默认，0不代表无限。sender_stopped携带user_stopped/duration_elapsed/failed结束原因，失败不会因远端Stop对账不可用被改为成功。正常停止与强制回收仍使用生命周期operation。

真实TLS连接的peer地址经IPv4-mapped/loopback/本机接口及IPv6 scope归属校验，固定Hub身份认证后比较后端endpoint ID；同机同端点在open_capture/play之前返回local_feedback_loop。接口或身份不足返回local_feedback_check_unavailable；远端同名或本机不同ID允许。Linux生产binding另核对UID owner协议/实例/UUID。三个新增fault只带闭合code，双语UI提供可解释恢复文案。
