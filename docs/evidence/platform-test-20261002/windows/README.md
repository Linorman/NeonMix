# Windows 原生测试（2026-10-02）

机器：`192.168.100.186`，`administrator`，Windows x64。所有测试源码、依赖、缓存、临时文件、日志与编译制品均落在 `E:\Desktop\NeonMix-test`；没有安装驱动、修改防火墙、系统默认音频设备或时钟。只读复用已有 VS 2022 Community、Rust 1.95.0、Python 3.13、MinGW GCC 15.2；Rust 固定版本入口使用目标目录内的 toolchain junction。命令经项目 `tools/dev.ps1` 运行。

## 结果

- Rust `audio`、`desktop`（含 screenshot feature）、`background`、`hub` release，以及 MinGW AirPlay worker 原生构建通过。
- 唯一测试用例共 **150 通过、0 失败、5 默认忽略**：identity/core/audio/AirPlay IPC/adapter 84；desktop/background 20；control 11；Hub/media 35。不是全 workspace 无条件验收；初始构建和测试失败日志另存。
- GStreamer runtime、双路混音、丢包/重放、错误身份拒绝 4 组运行探针通过。文件凭证探针经过 Windows fixture ACL 适配后 **9 类全部通过**；fixture 已删除。
- 独立后台的 NamedPipe status、shutdown、退出码 0 通过；自动测试另覆盖双 owner 拒绝与关闭重开。产品自行创建私有状态目录；Python 手工预建目录的 owner 不符合产品约束，原始失败保留。
- **Mac → Windows 30 秒原生 LAN 发送通过**。修复固定 256 帧周期后，同一个 VB-Audio Virtual Cable 输出协商为 48kHz、480 帧，保持房间身份与配对。播放中采样：output_frames=3,843,936，peak=0.004014、RMS=0.002817；输出错误、Mixer underrun、接收 lost/late、PCM sink drop 均为 0。该结果是 WASAPI 数字输出，不是扬声器实听、模拟延迟或长时稳定性验收。
- **AirPlay 接收未通过**：真实 Hub 启用后 `worker_failed / control_reader_closed`。同一原生 worker 的受控运行明确返回 `media IPC must be IPv4 loopback`（exit 1），而 Rust Windows Hub 传入 NamedPipe endpoint。该 IPC 接口不匹配仍需实现；没有 Apple 来源播放通过的结论。
- **GUI 未验收**：SSH 为 Session 0；5 页 screenshot 预览各等待 20 秒均未得到截图。独立 EnumWindows 只找到隐藏的 NeonMix、WGL dummy、NVIDIA OpenGL 窗口，无可见窗口，stderr 空。向现有 console 用户 token 调用 CreateProcessAsUser 因缺少进程特权失败；CreateProcessWithToken 成功但仍处于 Session 0。未改变持久用户权限或创建计划任务。桌面单测与原生构建通过，不能替代 console 窗口、托盘和交互验收。

## 修复前证据及边界

1. `PIPE_ACCESS_INBOUND` 从错误的 Windows API 模块导入，导致 E0432；改导入路径后构建通过。
2. worker helper `boolean` 与 Windows RPC typedef 冲突；改为 `boolean_field` 后 MinGW 构建通过。
3. Hub 对 Windows 强制 256 帧，实体 VXE 原生复现 `period 256 outside 480..480`。初始 LAN `output.available=false`、帧数 0；Windows 使用默认周期后同一 VB 输出及跨机发送通过。
4. Hub 搬迁测试比较 Windows `\\?\` canonical 路径与未 canonical 路径，属于断言问题；规范化 expected 后 23 项 Hub 测试通过。
5. Python 3.13 `mkdir(mode=0o700)` 在 fixture 显式加入 SYSTEM/Administrators ACE。fixture helper 先单独设置 owner，再移除这两条 grant，再限制继承和当前用户权限；生产权限校验未放宽。组合 icacls 参数 exit 87、后续 ACL 拒绝均保留。
6. VB-Cable 单独 capture loopback 探针曾报 Xrun（error code 5），输出本身成功；此采集路径没有复验通过。Shanling UA2 系统默认格式为 384kHz，超出项目 44.1/48/96kHz 范围，未修改设备格式。
7. Windows UTC 时钟比 Mac 约慢 79 秒，使 120 秒邀请在客户端只余约 40 秒有效时间；重新生成后配对通过。未调整系统时钟。

## 源码与证据

初始源码快照包含当前未提交源文件，共 503 个文件，排除真实连接配置、证书、缓存和历史证据；tar SHA256 为 `29c1bbd197dbb9b5488c0deb4c8e94badcbf88a42399a3527b2c558ccc25606c`。初始逐文件清单见项目 `artifacts/platform-test-20261002/windows/source-manifest.json`，源码 archive 保留于同目录。复验仅叠加本轮修复；远端 `server.rs` 精确修改两处周期条件，未同步其他工作流新增的多 AirPlay 接收实现。最终实际源文件与五个 EXE 哈希见 [source-and-binary-hashes.json](source-and-binary-hashes.json)，周期修改前后哈希见 [period-fix.json](period-fix.json)。这些结果只覆盖该测试快照，不代表测试期间其他并行源码改动。

关键证据：

- [核心测试](independent-tests-retest.log)、[桌面/后台测试](desktop-tests.log)、[Hub/media 复验](hub-media-tests-retest.log)、[control 与初始 Hub 失败](hub-media-tests.log)
- [文件凭证最终结果](credentials-final.log)、[原生后台](background-smoke.json)、[媒体运行](runtime.json)
- [初始无输出](initial-lan-before.json)、[修复后启动](lan-hub.log)、[远端结束诊断](lan-after.json)、[Mac 播放中诊断](../macos-client/windows-period-fixed-diagnostics.json)、[Mac 30 秒发送](../macos-client/windows-period-fixed-sender.jsonl)
- [真实 worker IPC 拒绝](worker-ipc-check.json)、[Hub AirPlay 失败](airplay-enabled.json)
- [5 页预览](desktop-smoke.json)、[原生窗口与 Session 检查](window-check.json)、[实体周期拒绝](vxe-period-256.log)、[采集 Xrun](capture.stderr)

远端保留源码、项目内依赖/缓存和 `target/release` EXE/PDB，便于复现。已停止本次全部 NeonMix 进程，删除测试房间、后台状态、文件凭证、invite、dummy key 以及一次性传输/不完整下载文件；本地一次性依赖中转也已删除。[清理记录](cleanup.json)确认目标路径内没有剩余 NeonMix 测试进程。没有删除目标目录外的用户文件。
