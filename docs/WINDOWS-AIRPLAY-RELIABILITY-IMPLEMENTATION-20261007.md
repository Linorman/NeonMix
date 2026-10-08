# Windows AirPlay 可靠性改进记录

2026-10-07。A/B/C 源码与自动回归已实施，完整未签名候选包已构建；D 的 Public 防火墙开启、非 ASCII Windows 账户和真实 iPhone 30 分钟联合验收仍待执行。依据 [原方案](WINDOWS-AIRPLAY-RELIABILITY-PLAN-20261007.md)。本次未使用子 Agent。

## 实现

密钥由平台层从同一已验证句柄读出，Windows 使用严格 UTF-8→UTF-16 转换、祖先目录句柄固定、普通文件/owner/受保护 DACL/持久 ACL 校验和 ReadFile；Unix 使用 no-follow fd、fstat 与 0600。PEM 必须小于 4096 字节，解析调用新增 `raop_init2_from_pem`，保持原有 Ed25519 PKCS#8 严格校验及公钥一致性；所有退出分支清零秘密缓冲。libplist 的受维护补丁补齐 JSON surrogate pair，嵌入 NUL 不再被字符串提取截断。持久身份、配对格式及 PCM v1 布局不变。

后台的生命周期任务持续持有 Child，IPC 超时/取消不会丢掉进程所有权。受管 Hub/Sender 通过限长私有 stdin stop/EOF 收尾，手动 CLI 保留信号行为。Windows 以挂起→入不可继承 Job→恢复的顺序启动，异常 owner 退出回收整个音频树。Hub 在停止时拒绝新准入、关闭媒体门、失效 Mixer producer、注销发现；worker 的停止独立于普通控制队列，所有 receiver 共用 2 秒窗口。PCM 和 stdout 持续排空到 worker 退出，再回收线程与运行 PEM，避免读端提前关闭将正常停止变成媒体故障。退出、强制方式、耗时、退出码和清理核验均通过新增结果字段报告。新旧组件以受管 CLI 和 ready 能力字段拒绝不兼容组合。

安装器保持每用户安装，网络 helper 单独 UAC 提升。helper 只接受 install/repair/remove/inspect、范围校验的端口及验证过的调用进程；路径由自身安装根目录推导，两条网络 EXE 的哈希编译进 helper。原调用方的进程对象、同一 helper 文件对象、创建时间与 SID 保持原用户身份。实例 GUID、原用户 SID、目录和端口持久化，journal 通过固定、拒绝重解析点和硬链接的 UTF-16 文件句柄写入。规则按实例精确 upsert/删除，失败恢复本次已有规则，不删外部 Block。

四条规则都是程序路径限定的入站 Allow，Domain/Private/Public、LocalSubnet、无 edge traversal；Hub TCP 默认 7443，其他媒体/RTSP 端口按程序允许动态端口。只读检查区分配置、路径错配、已知策略阻止和有效策略 unknown，列出防火墙 profile、连接网络类别和 Block 检出。未知有效策略不宣称网络可达；零 UDP 也不单独判定防火墙根因。

升级/卸载用固定维护客户端验证安装目录、同用户及持有的进程句柄，先发 Shutdown，再要求本安装 UI 退出。未核验、手动 CLI 仍占用或旧组件不支持协议时拒绝替换文件；不再全局按进程名强杀。UI 维护回调只设置退出标志并在窗口所属线程销毁原生窗口，让 winit 正常结束事件循环和释放资源，不依赖额外渲染帧，也不强杀 UI 进程。升级复用 GUID，改路径成功配置新规则后才尝试删除有旧安装记录的规则；卸载保留用户资料，网络清理失败时保留 helper 与实例记录供重试。

## 自动验证

证据目录为 [windows-airplay-reliability-20261007](evidence/windows-airplay-reliability-20261007/)。

| 范围 | 结果 |
|---|---|
| macOS / Windows 身份 | Unicode（中文、空格、重音、非 BMP）、4095/4096 边界、非法密钥及公钥/签名一致性通过；同句柄替换测试通过；Windows 实际 ACP 为 936，未改变系统代码页 |
| Windows 规则维护 | 双实例、三次幂等 repair、端口范围、路径错配修复、外部 Block 保留、重复 remove 和缺失规则报告通过；测试规则已清理 |
| 生命周期 | macOS / Windows 50 次取消 Stop→等待完成→立即重启通过，卡死 owner 有期限强制回收；Windows owner 异常退出后两个子孙进程以持有句柄确认退出 |
| macOS 1–4 路活动收尾 | 合成加密 PCM 输入；正常 Stop 约 0.67–0.80s，四路不是逐个累计超时；所有 worker exit=0、forced=false、运行 PEM 清零。原 profile/端口重开、显式重新启用后，owner EOF 约 0.88–0.97s 收尾，身份文件哈希不变 |
| 协议补丁 | 重新应用锁定 UxPlay 源码，20 个修改文件与已构建源码一致；libplist 2.6.0 的 JSON 补丁同样重新应用核对 |
| 候选包 | Windows x64 release 构建与 NSIS 编译通过；包内 worker 身份及 7 项加密 RTP→PCM/SETUP 边界回归通过。Hub runtime、空闲维护、不启动新 owner、目标 UI 正常退出且其他目录 UI 存活已由 package smoke 验证 |

较早的一轮四路 mix/fault 探针通过；新增 managed 场景曾分别在 debug 的起播期出现 media_frame_timeout、release 的稳态检查出现 431 timed_late_frames。失败报告保留，未计作质量通过。独立 shutdown 场景只验资源收尾，保留当时质量计数，不替代 steady/mix 的质量门槛；这些现象尚未唯一归因，不能据此宣称长期或全程零欠载。工作区同时含其他未提交的控制、Mixer 与界面改动，候选包以实际构建快照和源码哈希为准。

Windows 实验机的防火墙三个 profile 原本均关闭，本次未改变开关或网络类别，也未替换原 D 盘安装和原配对资料。规则维护通过不能计作 Public 防火墙开启的播放通过。CP1252 / ACP65001、另一个管理员完成 UAC、取消提升、组织策略、真实非 ASCII 账户、双栈/多接口、活动安装升级卸载及真实 iPhone 联合测试仍待对应环境执行。

## 构建和维护

所有显式依赖、缓存、中间文件和产物在项目内；Windows 测试节点使用本项目镜像 `E:\Desktop\NeonMix-test\r06` 的 tools/dev.ps1。已有 MSVC、MinGW 编译器和系统 SDK 只读复用。失败的额外 MinGW GStreamer 下载未用于成品；候选包使用现有已校验 MSVC GStreamer 1.26.10 的匹配头文件与 DLL，MinGW pthread worker 的编译器运行库单独收集到 airplay/bin，与 Hub 的 bin 隔离。这是完整组合的候选构建，不是把一个 EXE 覆盖到原 MinGW 安装。

```powershell
./tools/dev.ps1 cargo build --release --locked -p neonmix-desktop -p neonmix-hub -p neonmix-audio -p neonmix-identity -p neonmix-desktop-service
# worker 按 apps/airplay-worker 的构建入口准备，必须指定与其构建相同的 SDK/runtime。
./tools/dev.ps1 python tools/package_windows.py --mingw C:/msys64/mingw64 --worker .local/airplay/build-reliability/neonmix-airplay-worker.exe --worker-gst .local/gst --compiler-runtime C:/msys64/mingw64/bin --makensis .local/nsis/nsis-3.13/makensis.exe
```

开发者需在已设置项目内 TEMP/TMPDIR 的 MSVC 环境调用打包脚本。NSIS 输入显式指定 UTF-8。无 Git 的测试镜像记录 commit=null 与真实源码哈希，不伪造版本。helper 在组件签名之后生成固定组件哈希，并须参与最终签名；当前候选包未签名，未作发布放行。

静默安装退出码：0 表示程序与本实例规则创建/读回成功，仍需按 effective_policy 与实际流量判断可达；20 表示程序已安装但网络配置/清理需重试；30 表示实例尚未完整停止、文件替换未执行；31 表示目标目录不是可接受的专属安装目录。修复入口为安装目录的 `bin\neonmix-network-helper.exe repair`（自定义 Hub 端口须传 `--port`），inspect 不提升权限。卸载网络清理失败时中止删除 helper/记录，恢复后可再次卸载。

下一步联合放行仍按原方案 D 执行：同一完整签名包、Public 防火墙开启、非 ASCII 用户目录、已配对 iPhone 连续 30 分钟及暂停恢复、停止重启、原路径升级，并记录分阶段计数；当前自动证据不替代这一门槛。
