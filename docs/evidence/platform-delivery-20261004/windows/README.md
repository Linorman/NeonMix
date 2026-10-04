# Windows 当前快照构建与手测包验证（2026-10-04）

当前 522 文件快照在 Windows 原生构建、221 项测试通过、5 项忽略，正式成员格式与严格 Clippy 通过。最终 ZIP 在含空格路径和清洁开发环境中启动，CLI、WASAPI、独立后台、Hub 及集成 AirPlay 就绪通过；桌面程序保持运行 8 秒并只从包/Windows 系统加载模块。SSH 位于 Session 0，桌面加载检查不代表可见窗口、托盘或快捷键交互验收。

## 留存文件与启动

远端包目录：`E:\Desktop\NeonMix-test\packages\NeonMix-windows-x64-20261004-source322fda1b`。远端 ZIP：同路径加 `.zip`。本地下载文件：[Windows ZIP](../../../../artifacts/packages/NeonMix-windows-x64-20261004-source322fda1b.zip)。在实际登录的 Windows 桌面解压整个目录，双击 `Start-NeonMix.cmd`；完整说明在包内 `README-中文.md`。

ZIP 为 65,450,836 bytes，SHA256：`46823171e47bf939c70cad966b522fbb61989587ed41cad20786480dd5dfdfd7`。包含六个程序：Hub、音频工具、独立后台、桌面 UI、AirPlay worker，以及新增 AirPlay profile 管理工具；随包保留 MSVC/MinGW/GStreamer DLL、所需插件、scanner、许可证与实际源码归档。运行无需 Rust、Python、VS、MinGW 或 GStreamer 安装。中文字体使用已有 `C:\Windows\Fonts\msyh.ttc` / `msyhbd.ttc`，两者已确认存在。

包内 985 文件，`manifest-sha256.json` 覆盖另外 984 文件，本地逐文件审计零差异，六个二进制哈希与远端一致：[包信息](package.json)、[本地包审计](local-package-verification.json)。源码 canonical manifest SHA256 为 `322fda1b52872be7e880f750a02ef01fe0034895ecba6aa10741c504078de860`，输入源码归档为 SHA256 `3cc83620794eceb5fabf635853d9ff56e194330a89e51ac69f4db76bffb08faa` 的 source.tar.gz。本轮没有修改产品源码；末次远端源码与包内 tested-source 都与固定 522 文件逐项匹配：[首检](source-verification.json)、[末检和清理](cleanup.json)。

## 实际验证

| 项目 | 当前结果与证据 |
|---|---|
| Cargo 常规测试 | 221 通过、0 失败、5 忽略；[tests.log](tests.log) |
| 格式 | `cargo fmt --all` 因 vendored 依赖格式失败，正式 workspace 成员单独检查通过；[suite.json](suite.json)、[首败](fmt.log)、[成员检查](fmt-members.log) |
| Clippy/release | `--workspace --all-targets --locked --offline -D warnings` 与 `cargo build --workspace --release --locked --offline` 通过；[Clippy](clippy.log)、[release](release.log) |
| AirPlay native 构建 | MinGW worker 与三个辅助 probe 构建通过；[worker](worker-build.log)、[probes](worker-probes-build.log) |
| worker 身份 | 17 类身份检查，v1/ring v2/legacy/CRLF 身份签名保持，错误输入干净拒绝；[identity](identity.json)、[suite](worker-suite.json) |
| worker 协议/配对/解码 | v2 fencing/协议与配对通过；PCM/ALAC/AAC 分别产生 24/40/51 个有界 PCM chunk；[协议](worker-protocol.log)、[配对](pairing.json)、[解码](worker-codecs.log) |
| NamedPipe | 私有父进程可用；broad DACL/unprotected DACL/wrong PID/missing pipe/invalid name 干净拒绝，六类通过；[pipe-security](pipe-security.json) |
| 媒体运行时 | 双流、丢包/重放、错误 fingerprint 拒绝和 GStreamer runtime 通过；[runtime](runtime.json)、[media-dual](media-dual.log)、[media-loss](media-loss.log)、[media-reject](media-reject.log) |
| 文件凭证 | 九类通过。初轮强制 UTF8 下 icacls 输出触发 Python reader 解码警告，原日志保留；原生代码页复验无警告、九类通过；[初轮](credentials.log)、[代码页复验](credentials-native-locale.log)、[复验说明](credentials-native-locale.json) |
| 后台 | 12 类流程通过，包括私有 NamedPipe、资料/凭证、邀请取消清理、诊断脱敏、UI 客户端退出、媒体崩溃、明确关闭；[background](background.json) |
| AirPlay/原生集成 | PCM 与 ALAC 各两次暂停恢复/重新 SETUP，通过稳定定时、1 AirPlay + 1 native 混音、worker 被杀后原生音频存活等九类断言；两种编码的 late_packets/timed_late_frames 无新增；[PCM](single-pcm.json)、[ALAC](single-alac.json) |
| 最终 ZIP 重定位 | 含空格路径、子进程 PATH 仅包内 bin + Windows，runtime/devices、低音量 3 秒 WASAPI、后台 status/shutdown、Hub 推进132,576帧且 errors/underrun 0、AirPlay ready/published 通过；[重定位验收](relocation-verification.json) |
| 依赖独立性 | PE 扫描无 missing DLL；运行中的后台/Hub/worker 和桌面模块均只来自包/Windows 系统；[PE依赖](dll-dependencies.json)、[重定位模块](relocation-verification.json)、[桌面加载](desktop-package-smoke.json) |

实际输出为 `CABLE Input (VB-Audio Virtual Cable)`，WASAPI 48kHz/f32/stereo、协商周期 480 帧。数字路径/设备写入证据不代表物理实听或音画延迟测量。本轮没有重新执行完整 Windows 多路故障矩阵、Mac→Windows 长时 LAN 或真实 Apple 并发。此前 Windows decoder_queue 间歇首败及 LAN 2 PLC/480 欠载首败仍作为已知问题保留，不能由本轮短检查判定已经根治。

## 首败与操作边界

首次包编排脚本沿用旧“五个程序”数量断言，本快照新增 profile 工具后 assert，包目录尚未写内容；修正交付脚本清单后成功，产品源码未改：[首败](package-first.log)。第一次含空格解包验收在最后清理时遇到 ffi DLL 的短暂句柄关闭延迟，检查日志保留；自有目录稍后无进程占用，清理并在同一最终 ZIP 复验，通过30次短重试处理清理延迟：[首败](package-verify-first.log)、[首轮夹具清理](verification-first-cleanup.json)、[最终结果](relocation-verification.json)。桌面加载额外检查只结束该解包夹具启动的桌面/后台，完整手测包保持未运行的干净状态。

开发源码、SDK、cargo cache、下载、target、日志、中间文件全部位于指定 `E:\Desktop\NeonMix-test`；本轮独立源码目录 `r05`，所有开发命令通过 `run.ps1` → `tools/dev.ps1`，只读复用已安装编译器/SDK。旧源目录和旧手测包保留。没有修改防火墙、默认音频设备、驱动、权限策略、计划任务、系统时钟或其他关键设置，也没有删除测试根目录外文件。包不含 data/state/logs/.credentials/PEM/key/registry；临时资料、邀请和自有进程已清理，源代码/依赖缓存/最终包保留：[清理](cleanup.json)。

递归回传原始日志曾被自动审批拒绝，原因是范围过宽且可能含临时凭证。改为远端明确53项白名单审查与脱敏后回传，PIN、token、bearer、PEM 与秘密字段均剔除，没有导出原始运行资料或凭证；原始本轮日志只留远端指定目录。[出口审计](export-audit.json) 记录审查文件与安全副本哈希，没有剩余审批阻塞。
