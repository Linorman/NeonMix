# Ubuntu / Windows 与 macOS 协同测试（2026-10-02）

本轮不能判定两端 AirPlay 支持通过。Ubuntu worker 在 Hub 集成启动时 SIGSEGV；Windows Hub 与 worker 的媒体 IPC 类型不兼容。桌面及原生音频的部分功能通过，Windows 的固定输出周期问题已修正并复验。

| 范围 | Ubuntu 192.168.100.112 | Windows 192.168.100.186 |
|---|---|---|
| 环境 | Ubuntu 24.04.3 ARM64 / Parallels / PipeWire | Windows x64 / WASAPI；administrator |
| 构建、自动测试 | workspace release；170 passed、5 ignored | desktop/background/Hub/audio/worker release；分组原生测试 150 passed、5 ignored |
| 桌面 | 正式进程和后台启动；UI 进程退出后后台保持；中文预览截图正常 | 桌面与后台构建通过，NamedPipe 和后台状态/关闭通过；SSH Session 0 阻碍可见窗口与实际交互验证 |
| 文件凭证 | 9 类探针通过 | 修正测试 fixture 的 owner/ACL 后，9 类探针通过 |
| 独立 AirPlay worker | 20 项数字协议、3 codec 解码、5 项配对场景通过 | MinGW 原生构建通过；运行时拒绝 NamedPipe 地址 |
| AirPlay Hub 接收 | **失败**：worker SIGSEGV，接收未就绪 | **失败**：worker 只接受 TCP，Hub 提供 NamedPipe |
| macOS → 原生 Hub | 两次 30 秒发送接通，但丢包/欠载可复现，质量未通过 | 修正输出周期后 30 秒发送结束正常；10 秒检查点 playing、非零输出，丢包/欠载/输出错误均为 0 |

`ignored` 未计作通过。Windows 为本轮实际执行的分组测试，不是声称完整 workspace 全部测试通过。macOS 协同使用项目原生 CLI 客户端及 −36 dBFS 合成音源，不是 Apple 系统 AirPlay 来源或麦克风采集。10 秒检查点是采样证据，不能外推长期稳定性、实听音质或端到端延迟。

## 主要问题与复现证据

1. **Ubuntu AirPlay 集成崩溃。** 同一 worker 在独立 TCP 数字协议探针中通过，但由真实 Hub 启动时退出码 −11，Hub 状态为 `control_reader_closed`，无 fatal JSON。根因未定位，不能仅凭独立协议通过宣布接收可用。见 [集成结果](evidence/platform-test-20261002/ubuntu/airplay-hub-final.json)、[退出码](evidence/platform-test-20261002/ubuntu/worker-exit.json)。
2. **Windows AirPlay IPC 不兼容。** Rust Hub 创建 `\\.\pipe\NeonMix.Airplay...`，C++ worker Windows 路径只接受 `127.0.0.1:port`。原生 worker 收到 NamedPipe 形状地址后退出 1，报 `media IPC must be IPv4 loopback`；真实 Hub 也未就绪。见 [受控原生复现](evidence/platform-test-20261002/windows/worker-ipc-check.json)。本轮没有修改这项协议行为。
3. **Ubuntu 原生跨机质量未通过。** 首次 10 秒检查点丢包 65、PCM sink dropped 1、Mixer underrun 8,640 帧；无编译任务的复验仍丢包 64、underrun 7,680 帧，两次均 `network_degraded`。输出设备可用、输出错误为 0 且电平非零，说明接通但存在媒体质量问题。见 [初测](evidence/platform-test-20261002/macos-client/ubuntu-lan.json)、[空闲复验](evidence/platform-test-20261002/macos-client/ubuntu-idle-retest.json)。
4. **Windows Hub 固定周期导致无输出，已修正。** 所测 WASAPI 输出接受 480 帧，原 Hub 请求 256 帧，被 `select_config` 拒绝；发送端退出 0 时远端仍为 `output_lost`。初始打开和故障重开均改为设备默认周期后，同一 VB-Cable 输出成功，macOS 复验进入 `playing`。见 [原生拒绝](evidence/platform-test-20261002/windows/vxe-period-256.log)、[修复前](evidence/platform-test-20261002/macos-client/windows-lan-retest.json)、[修复后](evidence/platform-test-20261002/macos-client/windows-period-fixed.json)。
5. **桌面交互验收仍有边界。** Ubuntu 有正式进程生命周期和预览截图，未操作真实关闭按钮/托盘恢复；预览中的快捷键仍显示 `⌘`。Windows Session 0 只有隐藏窗口与 WGL 初始化窗口，没有可见 GUI 证据，不计可见窗口通过。

## 为继续测试所做的修正

改动保留在工作区，未提交。精确到本轮快照的 [补丁](evidence/platform-test-20261002/test-unblocking-fixes.patch) 和 [文件哈希](evidence/platform-test-20261002/test-unblocking-fixes.json) 可独立审查：

- `crates/airplay-ipc/src/local_windows.rs`：从正确模块导入 `PIPE_ACCESS_INBOUND`，移除未使用导入，解除 Windows 编译阻断。
- `apps/airplay-worker/CMakeLists.txt`：只按文件名排除 plist 测试源，避免含 `test` 的项目路径删空源码；为链接共享协议探针的静态库启用 PIC。两项在 Ubuntu ARM64 原生复验。
- `apps/airplay-worker/main.cpp`：`boolean` helper 改为 `boolean_field`，避免 Windows RPC typedef 冲突；Windows MinGW 构建复验通过。
- `apps/hub/src/server.rs`：Windows 初始输出与重开使用设备协商周期；原生 WASAPI 和跨机发送复验。
- `apps/hub/src/identity.rs`：搬迁测试比较 canonical path，正确处理 Windows `\\?\` 前缀，不修改身份逻辑。
- `tools/credential_store_probe.py`：仅对本轮自有测试 fixture 设置当前用户 owner，并去掉 Python 3.13 创建的额外显式 ACE；不放宽产品权限检查。

测试期间工作区另有并行功能修改。两台机器以开始时内容完全一致的 503 文件快照为基线，只同步本轮指定修正；结果不覆盖随后并行开发的全部代码。[两端源码比较](evidence/platform-test-20261002/source-comparison.json) 与各平台最终哈希记录了这一边界。

## 环境与操作边界

Windows 所有主动创建的测试源码、依赖、缓存、临时资料、日志和构建产物均位于 `E:\Desktop\NeonMix-test`，未删除其他目录文件。Ubuntu 资料位于既有项目的 `.local/platform-test-20261002/workspace`，部分 Unix socket fixture 使用项目内短路径。开发入口为 `tools/dev` / `tools/dev.ps1`，系统编译器和 SDK 只读复用。

Ubuntu 经 SSH/runuser 直接启动音频时被实时调度权限拒绝；改用已登录用户的 `systemd --user` 会话后正常，未更改 RTKit/PAM/polkit。长临时路径超过 Unix socket 上限后，仅调整了项目内 fixture 路径。Windows 比 Mac 慢约 79 秒，120 秒邀请在客户端视角只剩约 40 秒；重新签发后配对成功，没有修改系统时间。没有安装驱动、修改防火墙、默认音频路由或持久权限策略。

完整平台报告及清理记录：[Ubuntu](evidence/platform-test-20261002/ubuntu/README.md)、[Windows](evidence/platform-test-20261002/windows/README.md)。两台测试房间均被 [macOS DNS-SD 发现](evidence/platform-test-20261002/macos-client/native-discovery.json)。本轮没有真实 Apple AirPlay 来源成功接收、物理实听/模拟延迟、视频伴音或长期稳定性结论。

两端本轮 NeonMix 进程均已停止，自有测试房间、临时凭证和邀请已清除；源码、依赖缓存、构建制品及证据保留。macOS 协同客户端的临时凭证和邀请也已清除，本机原有 AirPlay 房间未操作。
