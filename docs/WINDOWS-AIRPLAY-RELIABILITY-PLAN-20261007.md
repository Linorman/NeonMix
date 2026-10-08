# Windows AirPlay 网络接入与进程退出改进方案

日期：2026-10-07。状态：A/B/C 已实施并形成未签名候选包；D 联合真机验收待执行。适用对象：Windows 安装器、AirPlay worker、Hub 和桌面后台。

本次应同时修复三条产品路径：安装后建立覆盖公用网络的受限入站规则；密钥文件从 Unicode 路径直接读取，消除 ANSI 代码页依赖；后台发起协作式停止，让 Hub 和 worker 完成资源清理，超时后才强制回收。三项均应作为下一版 Windows 安装包的验收条件，不能沿用关闭防火墙、纯英文路径或仅确认进程消失的测试结果。

本文保留原实施方案；当前实现、自动回归和未完成门槛见 [改进记录](WINDOWS-AIRPLAY-RELIABILITY-IMPLEMENTATION-20261007.md)。代码依据为 `aefb869` 基础上的当前工作区，包含此前未提交的 AirPlay、后台和 UI 改动。测试机“公用网络、防火墙关闭”来自本次问题描述；现有源码能够确认缺少安装规则、ANSI 路径转换及强制结束行为。既有播放修复和验收边界继续以 [实现状态](STATUS.md) 与 [Windows AirPlay 调查](WINDOWS-AIRPLAY-INVESTIGATION-20261007.md) 为准。

## 一 现状和影响

| 问题 | 已确认的实现 | 影响与验收缺口 |
|---|---|---|
| 安装没有配置入站规则 | `packaging/windows/neonmix.nsi` 使用 `RequestExecutionLevel user`，安装和卸载均无防火墙操作 | 关闭防火墙的测试不能证明正常系统策略下可以发现、连接并持续接收 UDP 音频；只放行专用网络不能覆盖公用网络 |
| 密钥路径依赖 ANSI | `platform::private_key_path()` 先通过 `CreateFileW` 校验文件，关闭句柄后按 `GetACP()` 转为窄字符串；协议层 `crypto.c` 再调用 `fopen` | 路径包含当前代码页无法表示的字符时启动失败。并非所有中文用户名必然失败：例如部分中文路径可被 CP936 表示，但无法被 CP1252 表示 |
| 后台立即强制结束 Hub | `Managed::stop()` 在 Windows 上先调用 `child.start_kill()`，再等待退出 | Hub 的正常收尾路径没有运行机会，运行 PEM 清理、发现注销和 worker 回收不能得到保证 |
| Hub 正常退出仍强杀 worker | `airplay.rs::run_worker()` 收尾直接 `kill()/wait()`，`ChildGuard::drop()` 也强制回收 | 仅修改后台仍不够；worker 已有 `stop` 命令和析构清理，应成为正常退出主路径 |
| 安装和卸载按进程名强杀 | NSIS `StopNeonMix` 对四种进程执行 `taskkill /F /T /IM` | 升级同样绕过收尾，而且可能结束其他安装目录或其他实例中的进程 |

`server.rs::serve()` 已有正常收尾基础：信号触发 HTTP graceful shutdown，随后停止 AirPlay、等待接收线程、停止媒体线程并释放输出。当前 Windows 子进程通过 `CREATE_NO_WINDOW` 启动，不能假定存在可接收 Ctrl+C 的控制台。微软对该标志的定义明确说明没有设置控制台句柄，故不把广播控制台信号作为本方案的停止机制。[Process Creation Flags](https://learn.microsoft.com/en-us/windows/win32/procthread/process-creation-flags)

## 二 防火墙规则随安装维护

### 规则对应实际网络进程

正式包将 Hub 放在 `$INSTDIR\bin\neonmix-hub.exe`，将 worker 放在 `$INSTDIR\airplay\bin\neonmix-airplay-worker.exe`。启动器和桌面 UI 不是 AirPlay 接收 socket 的 owner，为 `NeonMix.exe` 单独添加规则不能解决问题。

Hub 持有 mDNS/DNS-SD 发布和原生媒体接入；worker 持有 RTSP TCP、音频及控制 UDP、NTP timing。Hub 启动 worker 时传入 `rtsp_port=0`，协议层也会分配媒体端口。因此不能只放行常见的 5000、7000 或 UDP 5353。

建议使用以下规则模板。所有规则都是入站 Allow，作用于 Domain、Private、Public，远端地址限定 `LocalSubnet`，不配置边缘穿越放行。这里的 Public 是 Windows 网络类别，不代表允许来自互联网任意地址的连接。

| 规则后缀 | 完整程序路径 | 协议与本地端口 | 用途 |
|---|---|---|---|
| `Hub-Control-TCP` | `$INSTDIR\bin\neonmix-hub.exe` | TCP，桌面默认 7443 | 原生配对、控制、状态和事件 |
| `Hub-Media-UDP` | 同上 | UDP，任意本地端口 | mDNS 5353 及原生动态媒体端口 |
| `AirPlay-Control-TCP` | `$INSTDIR\airplay\bin\neonmix-airplay-worker.exe` | TCP，任意本地端口 | 动态 RTSP 控制监听 |
| `AirPlay-Media-UDP` | 同上 | UDP，任意本地端口 | 音频、控制、重传及 timing |

“任意端口”仅对表中的完整程序路径生效，不建立全系统端口放行。若正式产品允许更改 Hub 控制端口，规则维护工具应接收经过范围校验的实际端口并更新对应规则；CLI 自定义部署应提供同一维护入口。默认不额外创建出站规则，受管环境限制出站时由网络管理员按同一程序清单部署。

应用规则需要完整路径，不支持路径通配符；显式阻止规则优先于允许规则；组织策略还可能禁用本地规则合并。因此“本地规则已写入”与“当前有效策略允许连接”必须分别报告。[Windows Firewall rules](https://learn.microsoft.com/en-us/windows/security/operating-system-security/network-security/windows-firewall/rules)

`LocalSubnet` 覆盖 Windows 判定的本地子网，多网卡时可能涉及多个接口。该方案默认支持同子网连接；跨 VLAN、路由子网或 VPN 不自动放宽为 `Any`，由显式配置提供所需远端网段。IPv4 与实际支持的 IPv6 路径均需实测，不能用 IPv4 成功代替双栈验收。

### 保持每用户安装并单独提升规则维护权限

继续使用每用户安装。增加固定功能的 `neonmix-network-helper.exe`，由安装器通过 Windows UAC 单独提升，使用 Windows Firewall COM API 维护规则。安装器明确说明将允许同子网设备在包括公用网络在内的网络类别上连接 NeonMix。不要把整个安装器直接改成管理员身份：标准用户输入另一管理员的凭据时，管理员的 HKCU 和 LOCALAPPDATA 不是原安装用户的目录。

helper 仅支持 `install/repair/remove/inspect` 等闭合操作。它根据经验证的安装根目录推导两条固定 EXE 路径，不接收任意程序、任意 shell 命令或任意规则正文。提升后的操作应保留原安装用户和安装实例信息，不重新从管理员环境变量推算路径；验证调用方、安装清单、预期文件以及路径中的重解析点，跨权限通信使用受限本地通道。正式包还需把 helper 纳入现有签名和产物哈希流程。

规则标识采用 `NeonMix.<安装实例ID>.<规则后缀>`，版本号仅放在描述字段中。实例 ID 随安装记录持久化；升级复用，另一目录的新安装使用另一 ID。不要按展示名称匹配所有 NeonMix 规则，也不要使用会随用户重命名变化的用户名作为唯一键。

| 操作 | 规则生命周期 | 失败行为 |
|---|---|---|
| 首次安装 | 文件落盘并校验后创建规则，读回比对路径、协议、端口、profile 和地址范围 | UAC 取消或权限不足仍可完成程序安装，但显示“网络配置未完成”，提供重新配置入口 |
| 原路径升级 | 对同一实例执行幂等 upsert，不累积版本规则 | 记录哪些规则更新成功，失败时恢复本次修改前的自有规则 |
| 更换安装目录 | 新路径规则生效后，清理有安装记录可归属的旧路径规则 | 无法归属的规则不批量删除；显示需清理的旧安装记录 |
| 修复安装 | 对照模板补齐自有缺失或错误规则，检查有效策略 | 显式 block 或组织策略阻止时显示具体原因，不循环重复添加 Allow |
| 卸载 | 正常停止本实例后删除本实例规则，再移除程序文件，保留用户资料 | 提升失败时明确提示遗留规则，保留可用于管理员清理的实例信息 |

持久化规则变更清单，支持中断后重试。默认不删除用户或组织创建的阻止规则。若确需修复先前系统弹窗产生的阻止项，应展示精确规则及程序路径，并由明确的修复操作处理。

### 诊断区分配置和网络实况

新增只读检查，分别给出当前接口网络类别、该 profile 的防火墙启用状态、自有规则是否匹配实际运行路径、有效策略是否可判断。状态至少包括 `configured`、`missing`、`path_mismatch`、`policy_blocked`、`unknown`。防火墙服务不可用、第三方防火墙或信息读取失败均不能当作“已经放行”。

发现成功、TCP 连接成功、收到 UDP、生成 PCM、输出回调有音频是不同阶段。诊断应关联现有 worker ready、协议会话和音频计数，帮助区分“能找到但没声音”和“根本找不到”。无 UDP 只能定位到接收链路，不能单凭零包计数断言防火墙是根因。

本地界面可以展示必要路径帮助修复；脱敏导出继续遵守固定白名单，新增项使用数字、布尔和枚举数值，不导出用户名、地址或完整路径。

## 三 使用 Unicode 文件句柄读取密钥

### 推荐实现

采用“平台层打开并校验一次，同一句柄读出有界 PEM，再交给协议层解析”的方式。Windows 路径只做 UTF-8 到 UTF-16 的严格转换，随后全程使用宽字符文件 API，不再调用 `GetACP()` 或 `WideCharToMultiByte()` 生成密钥路径。

当前 `CreateFileW` 校验成功后关闭句柄，再由 `fopen` 按路径重新打开，也存在校验对象与读取对象分离的问题。推荐实现同时消除这个窗口，而不只是替换一个文件打开函数。

实施步骤：

1. 将 `private_key_path()` 拆分为路径验证和 `read_private_key()`，以 RAII 管理句柄及秘密缓冲区。拒绝空路径、嵌入 NUL、无效 UTF-8 和备用数据流路径。
2. Windows 用 `CreateFileW` 打开现有普通文件，保留当前 SID owner、受保护 DACL、持久 ACL 文件系统及重解析点检查。同一句柄用于权限验证和 `ReadFile`；父目录的链接/重解析点策略与现有身份文件模块保持一致，不能把 `FILE_FLAG_OPEN_REPARSE_POINT` 误当作检查了所有父目录。
3. 保持当前运行 PEM 的大小边界：现有 4096 字节缓冲加 EOF 检查实际接受小于 4096 字节的文件。新读取器最多读取 4096 字节用于判断边界，达到上限即拒绝，不扩大到凭证 JSON 的 64 KiB 上限。
4. 在受维护协议补丁中新增接收 `const unsigned char *pem, size_t len` 的初始化路径，将数据传至现有严格 PEM/DER Ed25519 校验。建议命名为 `raop_init2_from_pem`，名称为拟新增 API；不得误当作当前上游接口。
5. 复用已有 `BIO_new_mem_buf` 路径，继续拒绝错误算法、非 PKCS#8、多个对象和非法尾部数据。解析结束后清零临时 PEM，失败分支也必须执行清理。
6. Unix 使用打开后的文件描述符验证并读取，保持 owner、普通文件及 0600 规则；不因 Windows 修复放松其他平台的权限边界。

PEM 仅在 worker 进程内由平台层传入协议层；Hub→worker 控制 JSON 仍传文件路径，不把长期密钥放到命令行、环境变量、控制日志或普通 JSON 事件中。持久 store、receiver UUID、公钥和已有配对记录不迁移、不重建；缺失或损坏密钥继续明确失败。

当前协议补丁已经特意避免把 MinGW 的 `FILE*` 交给另一套 CRT 的 OpenSSL。改动必须保留这一约束，不能回退到 `PEM_read_PrivateKey(FILE*)`，否则可能重新出现 `OPENSSL_Applink` 问题。

### 最小补丁的取舍

若需要先交付范围更小的修复，可保持 UTF-8 路径，协议层在 Windows 分支转换为 UTF-16 后调用 `_wfopen(..., L"rb")`；这是代码页兼容修复，但仍需单独解决校验后重新打开的问题。推荐直接实施同句柄读取，避免短期内维护两次改造。

微软文档确认窄字符 `fopen` 默认按 ANSI 代码页解释文件名，`_wfopen` 接受宽字符文件名；`ccs=UTF-8` 控制文件内容编码，不能修复路径编码。[fopen and _wfopen](https://learn.microsoft.com/en-us/cpp/c-runtime-library/reference/fopen-wfopen?view=msvc-170)

不采用改变系统区域设置、要求启用系统 UTF-8 Beta、依赖 8.3 短路径或把密钥复制到公共英文目录作为产品解决方案。也不把支持中文路径等同于支持任意长路径；长路径应单独验证启动器、DLL 和插件加载、profile 读写全链路。

### 错误分类和补丁落点

保留对用户可操作的错误类别：路径编码无效、文件不存在、读取失败、权限不符合要求、密钥格式错误。通过固定错误码分类，不把完整路径或 PEM 内容写入 fatal 事件；桌面错误映射避免把所有情况压成“后台操作失败”。

源码改动应进入 `apps/airplay-worker/platform.h`、`main.cpp`、`patches/airplay-audio-only.patch` 及相应 probe。`.local/airplay/patched` 是生成目录，只修改该目录不能形成可重建的交付。补丁必须能重新应用到锁定的 UxPlay 源码，生成与已测构建一致的文件。

## 四 建立完整的协作式退出链路

### 后台到 Hub 的停止通道

推荐为后台持有的长生命周期 Hub/Sender 子进程增加显式启用的私有 stdin 控制通道。现有 `audio_command()` 默认 `stdin(null)`；受管启动模式改为 `stdin(piped)`，并为对应 CLI 增加拟议参数 `--managed-control-stdin`。普通 CLI 不启用该参数，继续保留 Ctrl+C 和 Unix SIGTERM 行为，避免终端 stdin EOF 误停止手动启动的 Hub。

第一版协议只需要一个闭合命令，例如：

```json
{"version":1,"type":"stop"}
```

命令使用 LF 分隔、最大 1024 字节；拒绝未知字段和版本。管道由直接父进程创建，不对 LAN 或任意本地进程开放。父端句柄不可继承到其他进程；子进程再启动 worker 时不得泄漏这个控制句柄。受管模式下父端 EOF 视为 owner 丢失，触发同一停止流程；不重启媒体。

读取由可取消的专用读取器完成，接入 Hub/Sender 的统一停止通知。Windows 阻塞读必须有显式取消和线程回收路径；不要依赖无法中断的 stdin 阻塞任务在 Tokio runtime 退出时自行消失。格式错误应生成受限错误并进入可终止状态，不能无限等待下一条消息。

stdout 的 `shutdown_started` 和 `shutdown_complete` 可用于诊断，但不是唯一停止依据；后台必须等待持有的进程句柄确认退出。现有 stdout 仅保留最近事件，也不适合拿某条曾出现的日志代替进程状态。

### 状态与超时

将 `Managed` 的停止表示为 `Running → Stopping → Stopped`，或者在等待期限耗尽时进入 `ForceStopping → Stopped/StopFailed`。重复停止同一进程应返回同一个结果；进入 Stopping 后拒绝同实例新启动，防止在途 Start 覆盖 Stop。

建议首版预算如下，使用单调时钟和共享截止时间，实施后按实测校准：

| 层级 | 建议预算 | 到期动作 |
|---|---|---|
| 后台写入停止命令 | 250 ms，计入总预算 | 标记通道失败，继续有限等待；不能卡在写管道 |
| 所有 AirPlay worker 正常退出 | 并行等待共 2 s | 仅强制回收未退出的自有 worker，再核验退出 |
| Hub 完整正常收尾 | 共 5 s，包含 worker 预算及现有 HTTP 1 s drain | 后台对该子进程树执行强制回收 |
| 强制回收后的核验 | 共 2 s | 未确认全部退出则返回失败，不发布 Stopped |
| 一次后台 Shutdown | 停止执行阶段总计不超过 10 s | 聚合所有 owner 的结果，不因首个失败跳过剩余清理 |

这里 10 s 是执行停止操作的预算；请求还可能等待已有变更完成和 runtime 锁。现有 IPC 的 5 s 锁等待、35 s 操作、45 s 客户端超时需统一检查。排队超时只能报告“停止尚未完成/后台忙”，不能提前标记成功。紧急只读取消继续保留，已经可能提交的变更不能被任意取消。

不要在停止开始就 `self.child.take()` 并仅由临时 future 持有 Child。当前请求超时会丢弃执行 future，配合 `kill_on_drop(true)` 可能再次绕过正常收尾。应由生命周期管理任务持续持有 Child，IPC 请求只订阅停止结果；即使请求取消，停止任务仍继续到退出或明确失败。客户端断开也不得传播为停止任务取消，保持当前退出请求不依赖 UI 读取回复的语义。

此外，当前 `timeout(..., child.wait()).await.is_err()` 只检查外层超时，未传播内层 `wait()` 错误。实现时分别处理超时、wait 失败及实际退出状态。未确认退出时保留可再次查询的 owner 记录。

### Hub 和 worker 的收尾顺序

1. Hub 原子设置 stopping，停止接受新会话、准入和会改变运行状态的新命令；停止后不再进入输出恢复或 worker 重启分支。已接受的持久化变更按原事务规则完成，不在退出时重新初始化身份。
2. 立即关闭 AirPlay 媒体门、失效 Mixer producer，撤销本实例发现发布。原生 Hub 与 AirPlay 的发布对象都需要覆盖。
3. 所有 receiver 并行发出已有 worker `stop` 命令。该命令已经在 `main.cpp` 中设置 `running=false` 并唤醒等待；让 worker 退出主循环，停止媒体写、销毁 RAOP/decoder/socket 并退出。
4. 控制队列满或 writer 阻塞时，停止请求不能依赖普通队列腾出空间。提供独立高优先级停止标志与可取消写入路径；无法发送时受管 stdin EOF 可作为备选信号。等待期间继续排空 stdout，避免子进程被事件写入反向阻塞。
5. 对共享期限内未退出的 worker 执行 kill/wait，随后回收 reader、writer、PCM 线程，移除对应 `runtime-key-<UUID>`。4 个 receiver 共用 2 s 等待窗口，不能逐个累计为 8 s。
6. Hub 完成 HTTP/WSS drain、原生媒体线程停止、输出释放及 owner 锁释放，返回退出状态。WSS 或磁盘异常不能造成无界 join；失败应记入收尾结果。

`ChildGuard::drop()` 继续作为异常兜底，但正常路径应显式 disarm，避免二次终止。`FileGuard` 保留；清理失败须有脱敏计数，不能静默宣称无遗留。下次 owner 启动时清理本模块 UUID 命名的运行文件仍有必要，因为断电和强制终止无法保证执行析构。

停止结果建议包含 `graceful`、`forced`、`elapsed_ms`、`exit_code`、`cleanup_complete`。只有进程退出且清理核验成功才报告完整完成；强制退出可表示“已停止，使用强制回收”，不能伪装成正常退出。既有 `HubStop`、`SenderStop`、`Shutdown` 请求名称保持，返回结构和状态字段的兼容性在 IPC 合约中明确。

### 异常退出时回收整个进程树

Windows 增加每个受管音频进程树独立的 Job Object，由后台持有唯一有效 owner 句柄并配置 `JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE`。Hub/Sender 及其 worker 属于该 job，桌面 UI 和后台本身不加入此音频 job。正常退出不靠关闭 job 触发；只有超时、崩溃或 owner 消失时才用它兜底。

创建子进程时应先挂起、成功加入 job，再恢复运行，消除 Hub 在入 job 前先派生 worker 的窗口；这需要受控的 Windows spawn 封装，不能在普通 spawn 后忽略 `AssignProcessToJobObject` 失败。job 句柄不得被子进程继承，禁止无意 breakaway；已有外层 job 的环境需验证嵌套兼容，失败时终止未托管子进程并报告启动失败。

微软说明 `TerminateProcess` 是无条件终止，外部调用返回时不代表进程已经退出，必须继续等待进程对象。因此强杀只保留为后备，且强杀后仍需核验。[TerminateProcess](https://learn.microsoft.com/en-us/windows/win32/api/processthreadsapi/nf-processthreadsapi-terminateprocess) Job 的 kill-on-close 行为见 [JOBOBJECT_BASIC_LIMIT_INFORMATION](https://learn.microsoft.com/en-us/windows/win32/api/winnt/ns-winnt-jobobject_basic_limit_information)。

后台正常处理 Shutdown 时须逐个尝试所有 owned 资源并聚合错误，不能因为 `sender.stop().await?` 失败就跳过 Hub；服务主循环退出后的第二次清理也应幂等。崩溃时 job 可以确保进程树结束，但不承诺完成 PEM 删除或协议收尾。

### 安装和卸载复用同一退出机制

增加安装维护入口，通过现有受限 desktop IPC 发送 Shutdown。当前 `neonmix-background` CLI 只有启动后台的入口；不能把尚不存在的 `--shutdown` 参数写成现成调用，需新增固定维护命令，并确保连接失败时不会自动启动新后台。

安装器在文件替换前调用旧安装的维护入口或兼容 IPC 客户端，目标由旧安装记录、原登录用户和明确 state-dir 确定。等待后台及音频进程退出后，再让该安装 UI 通过显式维护消息退出；普通后台崩溃不应被解释为要求关闭所有 UI。

旧版本没有协作式停止协议时允许明确记录强制兼容路径，但只能定位到该安装根目录、该用户、相应创建时间/句柄的进程。不能继续使用 `/IM` 全局匹配。无法核实归属或仍有程序占用时停止文件替换并提示重试，避免产生半旧半新的运行树。新版本正常升级和卸载必须走协作式退出。

## 五 实施拆分和交付文件

建议按以下顺序交付，最终通过同一安装包联合验收。每一项都有独立可审查结果，但不把其中任一项通过记成整体完成。

| 批次 | 主要改动 | 完成条件 |
|---|---|---|
| A 密钥兼容 | worker 平台读取、协议补丁、identity probe | 不依赖 ACP；权限及严格密钥解析回归通过；已有公钥及配对保持 |
| B 生命周期 | `desktop-service` Managed、Hub/Sender 停止通知、AirPlay worker 收尾、Windows job、生命周期测试 | 正常 stop 不强杀；卡死有期限；取消和崩溃不遗留进程；可立即重启 |
| C 安装维护 | NSIS、网络 helper、安装维护 CLI、`tools/package_windows.py` | 公用网络规则可安装/修复/删除；跨用户提升正确；升级卸载仅影响本实例 |
| D 联合验收 | Windows 真机探针、安装包证据、真实 iPhone 回归 | 防火墙开启、非 ASCII 用户目录、安装及退出完整链路通过 |

协议初始化补丁应同步更新 `apps/airplay-worker/README.md` 和身份 probe；退出行为更新 [桌面 IPC 合约](DESKTOP-IPC-CONTRACT.md)、[AirPlay IPC 合约](AIRPLAY-IPC-CONTRACT.md) 及 ADR-012/013 的适用段落。若引入新的公开字段，先记录字段语义和兼容策略，再更新 UI。完成后才更新 `docs/STATUS.md` 的实现及验收状态。

网络规则维护代码需有独立规则模板和读回比较逻辑，便于安装器、修复入口、静默部署共用。静默安装返回区分“安装成功且网络就绪”和“程序已安装但需配置网络”的结果；具体 exit code 写入部署文档，不能依赖交互式弹窗。

开发命令统一通过 `tools/dev` 或 Windows `tools/dev.ps1` 执行，源码准备、缓存、构建、报告和临时测试资料全部落在项目内。安装器验收产生的正式安装目录和系统规则属于明确的安装测试结果，应在隔离测试用户/机器中管理并按测试记录还原。

## 六 测试与验收矩阵

### 防火墙和安装

| 场景 | 方法 | 通过条件 |
|---|---|---|
| Public 且防火墙开启 | 无历史 NeonMix Allow 的干净测试用户/机器安装正式包，真实 iPhone 首次配对、重连、播放 | 无需关闭防火墙或切换网络类别；UDP、PCM 和输出计数持续增长，声音正常 |
| Private 及切换 | 播放前后切换 Public/Private，并重新连接 | 对应 profile 的规则持续有效，不依赖首次安装时的网络类别 |
| 负对照 | 仅撤销本测试实例 worker UDP 规则，排除其他放行规则，保持防火墙开启 | 能检测预期的接收失败；恢复规则后同配置恢复，证明测试真正覆盖入站限制 |
| 路径和实例 | 中文及空格安装路径、原路径升级、改路径安装、两个独立实例 | 规则指向实际 EXE；不重复、不误删另一实例；升级仅停止目标实例 |
| 权限失败 | 标准用户取消 UAC、另一个管理员完成 UAC、修复重试 | 安装目录及 HKCU 仍属于原用户；失败可见；重试幂等 |
| 策略阻止 | 预设显式 Block、禁用本地规则合并 | 不误报网络已就绪，不删除外部策略；报告可操作原因 |
| 卸载 | 活动播放中卸载、取消提升、重复清理 | 停止结果可见；删除本实例规则，保留配对资料；遗留项可准确清理 |
| 多接口与地址族 | Wi-Fi/有线、多网卡、实际支持的 IPv6 连接 | 使用正确程序及地址范围；超出 LocalSubnet 的失败与默认支持边界一致 |

测试前记录有效 profile、规则、默认入站策略及历史弹窗规则。验收期间保持防火墙开启；若有远程维护通道，先保证其既有规则有效，结束后只还原本次测试改变的状态。

### 路径和身份

| 场景 | 方法 | 通过条件 |
|---|---|---|
| 非 Unicode 系统代码页 | CP1252 + 中文用户/目录；CP936 + 该代码页不可表示的字符 | worker ready 成功，同一密钥公钥一致；不出现有损转换 |
| 系统 UTF-8 | ACP 65001 环境重复同组测试 | 无单独系统设置依赖或行为分叉 |
| 完整真实路径 | 中文 Windows 账户首次安装、已有账户升级，含空格、重音及非 BMP 字符 | launcher、profile、插件与密钥全链路可用；不是只对独立 C 函数测成功 |
| 权限拒绝 | 错误 owner、过宽 DACL、目录、重解析点、ADS、无持久 ACL 文件系统 | 与现有权限边界一致，明确失败且不生成新身份 |
| 密钥拒绝 | 缺失、损坏、错误算法、多个 PEM、边界大小及尾部垃圾 | 无 ready；无自动生成或覆盖；秘密不进入日志 |
| 文件替换 | 在权限校验与读取之间注入可控替换 | 读取保持绑定到已验证句柄，或明确失败；不能读取未经验证的新对象 |
| 配对保持 | 升级前后比较稳定身份，已配对 iPhone 重连 | 不要求重新 PIN 配对，receiver UUID/公钥保持，原持久资料不被迁移 |

代码页由测试环境和进程记录确认；`chcp` 只反映控制台代码页，不能代替 `GetACP()`。使用现有 identity、pairing、protocol 探针扩展场景，并在 Windows 正式 MinGW worker 与匹配 DLL/runtime 下执行。macOS/Linux 跑相应权限及协议回归。

### 退出和恢复

| 场景 | 通过条件 |
|---|---|
| 空闲/活动播放 HubStop | 收到协作式停止，Hub、全部 worker、相关线程退出；正常分支 `forced=false`；运行 PEM 消失 |
| 1 至 4 个 receiver | 所有 receiver 在共享期限内停止，不按数量累计超时；发现服务已发起注销，端口和 owner 锁释放 |
| 控制/PCM 背压 | writer 阻塞、控制队列满、消费者不读均不能让退出无界等待 |
| 卡死 worker/Hub | 在预算内升级为强制回收；结果明确 forced；只结束本实例进程树 |
| 后台异常退出 | job 兜底无遗留 Hub/worker；下次启动清理本模块运行 PEM，不删除持久 store |
| UI 关闭/崩溃 | 音频继续；仅显式停止或退出后台才触发停止流程 |
| 请求取消及竞态 | 客户端断开、操作超时、重复 Stop、Start/Stop 交错后，管理任务仍完成停止且不自动重启 |
| 资源操作失败 | stop 写入、wait、文件删除、单个 owner 清理失败不会被吞掉；其余 owner 仍尝试清理 |
| 连续恢复 | 至少 50 次启动→播放→停止；存活进程数、句柄数和运行文件数不持续增长，可立即重开同一 profile 和端口 |
| 升级/卸载 | 新版正常退出；旧版兼容强制路径有记录；其他目录、其他用户或手动 CLI 实例不被误杀 |

停止验收同时检查进程对象、运行文件、监听端口、profile owner 锁和重连结果。mDNS 客户端可能缓存旧条目，不能把 iPhone 选择器是否瞬时消失当成唯一收尾判据。正常退出要求本地注销动作和资源释放证据；远端显示延迟另外记录。

### 联合放行条件

最终使用可重建的完整 Windows 安装包，在防火墙开启的 Public 网络和非 ASCII 用户目录下完成真实 iPhone 已配对播放，连续播放至少 30 分钟，并执行暂停恢复、停止重启和一次原路径升级。保留启动、稳态、停止阶段的质量计数增量；稳态不得出现本次改动引入的接收中断、队列失控或异常进程退出。既有起播 malformed 和停播欠载须分阶段记录，不能隐藏后宣称全程零异常。

该 30 分钟是本次联合回归门槛，不替代项目既有的多设备、延迟及长期稳定性验收。合成探针不能替代 iPhone，防火墙关闭的播放不能替代规则验收，进程消失不能替代正常收尾。

证据放到 `docs/evidence/windows-airplay-reliability-<实际日期>/`，包含安装包/二进制哈希、源码版本与补丁差异、Windows 版本和 ACP、脱敏规则前后差异、各场景结果、退出耗时与方式、运行文件数量及真实播放确认。失败和未执行项保留，不能合并为 pass；清理本次无后续用途的临时夹具。

## 七 升级和回退边界

三项改动均不需要改变持久身份格式。升级应先正常停止原实例、保留用户资料、替换匹配的 Hub/worker/helper/DLL/plugin 组合，再维护规则并验证启动。此前调查使用过混合 SDK 构建，不能单独复制一个 worker EXE 就视为完成安装修复。

回退按完整已验证安装包执行；规则 helper 应根据目标版本的安装清单恢复本实例规则，避免残留指向已删除目录的 Allow。若升级后已经发生新的配对、撤销或设置提交，保留最新资料，不恢复旧用户目录快照；这继续遵守 ADR-014 的身份持久化约束。

对新后台配旧 Hub、新 Hub 配旧 worker，应在启动能力检查或包完整性检查阶段明确拒绝不兼容组合，或使用已定义的旧版强制停止兼容路径；不得静默宣称支持协作式退出。正式放行要求上面的三条产品路径在同一安装版本中同时成立。

## 八 代码定位

| 文件或符号 | 本次审查依据 |
|---|---|
| [Windows 安装器](../packaging/windows/neonmix.nsi) | 每用户安装、无规则维护、`StopNeonMix` 按名称强杀 |
| [Windows 打包脚本](../tools/package_windows.py) | Hub/worker 的实际路径以及 MSVC/MinGW runtime 分离 |
| [worker 平台层](../apps/airplay-worker/platform.h) | `private_key_path` 的宽字符校验及 ANSI 转换 |
| [worker 主程序](../apps/airplay-worker/main.cpp) | `raop_init2`、动态 RTSP、`stop` 与 Worker 析构 |
| [受维护的协议补丁](../patches/airplay-audio-only.patch) | `crypto.c` 密钥文件读取和严格 PEM/DER 解析 |
| [后台进程管理](../crates/desktop-service/src/daemon.rs) | `audio_command`、`Managed::stop`、Shutdown 与请求超时 |
| [后台入口](../crates/desktop-service/src/bin/neonmix-background.rs) | 当前仅支持启动，需要新增安装维护入口 |
| [Hub 服务](../apps/hub/src/server.rs) | `serve` 停止信号及收尾顺序 |
| [AirPlay owner](../apps/hub/src/airplay.rs) | worker 控制线程、强制回收、`FileGuard`、运行密钥清理 |
| [生命周期测试](../crates/desktop-service/tests/lifecycle.rs) | 已有 UI 隔离、停止和异常恢复测试扩展入口 |
| [身份探针](../tools/airplay_identity_probe.py) | 公钥一致性、非法密钥和 ready/退出判定扩展入口 |
