# E05 完整性检查

更新：2026-09-30。按完整开发计划 4.5 和设计方案 7.2–7.3 检查共享功能、平台后端和实际调用路径。Windows/Linux 没有运行测试；本轮在 macOS 上检查其条件编译和依赖分支。

## 需求与代码

| 要求 | 代码位置与实现 |
|---|---|
| DNS-SD 注册、浏览、解析 | `crates/identity/src/discovery.rs`；服务类型、TXT/SRV、解析事件及有界候选列表；Hub 创建 Publisher，CLI/连接选择使用 browse |
| 名称冲突、多网卡 | UUID/证书摘要分离发布名称；保留解析地址，按实际监听接口/地址族发布；并发验证地址，只有完整证书与 UUID 校验成功才选择 |
| 地址变化、正常/异常离线 | 接口定期检查、goodbye、SRV/A/AAAA 活性验证；WSS 断线重新发现、取快照再订阅 |
| 唯一首次信任机制 | 高熵一次性邀请文件；管理员开启/取消，客户端预存长期令牌，仅上传摘要，固定授予 Member |
| 超时、取消、单次使用和重复 | `pairing::Book` 单调期限和限额，完全相同请求返回原结果；取消/超时拒绝；落盘失败保持可重试；pending 资料可恢复已完成的服务端登记 |
| 假 Hub、换钥、错误证书 | `trust.rs` 固定 DER 证书并验证 TLS 签名；`identity.rs` 校验稳定 Hub UUID；从发现 TXT/名称/IP 不能取得授权 |
| 权限和撤销 | `pairing_api.rs` 由服务端 Admin Principal 签发；E04 Authority 校验成员控制、Solo、断开/重新允许/撤销；撤销关闭控制与媒体并持久化 |
| 用户停止后不自动发送 | 配对不创建媒体会话；发送只能显式启动；WSS 重连不创建新媒体会话，停止/撤销后的旧会话不能复活 |
| 平台凭证及私有文件 | `vault.rs`、`files.rs`、`files_windows.rs`；平台库错误不回退明文；JSON 只存公开身份和凭证引用，邀请及权威状态使用私有创建/替换 |
| 三端入口 | `apps/hub/src/main.rs` 的 setup/discover/invite/pair/cancel-invite/forget 和 Sender/控制使用相同共享代码，没有 E05 的仅 macOS 功能分支 |

桌面发现列表、邀请呈现、设备管理和可视交互属于 E07；媒体 SDK 打包和安装属于 E02/E09 的独立边界。

## 此次补齐

1. Windows 不再仅依赖目录继承权限创建邀请。新文件以 `CREATE_NEW` 和受保护的 OWNER_RIGHTS DACL 创建；写入之前检查文件系统支持持久 ACL。不支持时明确失败，不写秘密。路径支持标准库规范化的长路径，并拒绝会继承基础文件 ACL 的 alternate data stream。Unix 使用 0600。
2. Hub 的媒体连接保存完整本地 SocketAddr，保留链路本地 IPv6 scope；具体 scope 注册只使用对应接口。IPv6 通配注册只发布 IPv6 地址，适配 Windows 默认 IPv6-only 监听。
3. 权威状态及配对资料通过私有、独占、随机命名的同目录临时文件替换；旧临时文件不会阻断恢复，失败仅删除自己的临时文件。同步更新了 E04 的真实落盘失败探针，使它验证目标替换失败而不依赖旧临时文件名。
4. 正式资料的普通 serve 默认 `0.0.0.0:7443`，可被局域网 Sender 发现；实验资料保留 loopback 默认。`--listen` 仍可显式覆盖。
5. 平台凭证写入失败会尝试清理自己的条目，删除探针只把真正的 NoEntry 作为成功，不把凭证库错误当作已删除。

Windows 权限依据：[CreateFileW](https://learn.microsoft.com/zh-cn/windows/win32/api/fileapi/nf-fileapi-createfilew)、[SID strings](https://learn.microsoft.com/en-us/windows/win32/secauthz/sid-strings)。代码尚无 Windows 文件系统运行证据。

## 平台就位检查

| 平台 | 凭证库 | 文件保护 | 本轮证据 |
|---|---|---|---|
| macOS | Keychain | 私有文件 0600、原子替换 | 实际凭证读写/删除、完整功能探针、编译与测试 |
| Windows x64 | Credential Manager，`windows-native` | Win32 protected DACL；ACL 文件系统检查；长路径/ADS 处理 | macOS 宿主的 GNU target `cargo check/clippy --all-targets`，包括实际 Windows FFI 分支 |
| Linux x64 | Secret Service，`sync-secret-service/crypto-rust`，项目内 vendored D-Bus | 私有文件 0600、原子替换 | macOS 宿主的 Linux target `cargo check/clippy --all-targets`，包括实际 D-Bus 后端 |

交叉入口为 `tools/dev python3 tools/check_e05_cross.py` 和 `--clippy`。检查覆盖 `neonmix-identity` 与 `neonmix-control`；不执行任何 Windows/Linux 可执行文件。编译器/已安装 target 只读复用，所需头文件和缓存都在项目 `.local/`。

Windows/Linux 完整 Hub 的原生媒体 SDK 链接、加载、凭证库/ACL/网卡生命周期运行以及实体互通仍需对应平台验收。它们不属于本轮已经通过的结果。

## macOS 验证记录

macOS workspace 97 项测试、最新 E05/控制/Hub 30 项针对性测试、17 组真实场景通过，格式、Clippy 与 release 构建通过。Windows/Linux E05 核心的交叉检查和 Clippy 均通过。

最终结果、源码与二进制哈希、交叉检查日志见 [evidence/e05/review](evidence/e05/review/)。实际探针包含真实网卡的链路本地 HTTPS/WSS/DTLS-SRTP、正式默认监听，以及真实文件替换失败后的授权原子性和原邀请重试。

一轮旧 3 秒 IPv6 发送窗口未取得媒体包，原因没有确认；失败记录保留，随后相同参数的完整探针通过。该窗口短于协议的 5 秒握手故障期限，当前功能探针改用 10 秒，覆盖故障期限和有效 PCM。这个调整不作为延迟问题已解决的证据，发现/开始播放 P95 与长期稳定性仍待单独验收。

2026-10-02：本文原有系统凭证相关验收是历史记录。新文件方案实施与 macOS 验证见 [凭证存储交付](evidence/credential-storage/implementation-20261002/README.md)，不能据本审计计作 Windows/Linux 新方案通过。
