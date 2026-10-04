# Windows 可移动手测包（2026-10-02）

## 交付

- Windows 目录：`E:\Desktop\NeonMix-test\packages\NeonMix-windows-x64-20261002-tested-v1`
- Windows ZIP：同目录旁的 `NeonMix-windows-x64-20261002-tested-v1.zip`
- 本机副本：`artifacts/packages/NeonMix-windows-x64-20261002-tested-v1.zip`
- ZIP：**64,335,408 字节**；SHA256：`91e41e3ff1cdb1c172ab3053c011f6ca7fb7e10f19d376c89f12e8f82011a2fc`
- 解压后约 175,114,918 字节、982 文件；详细路径、大小及校验见 [package.json](package.json)。

在登录后的 Windows 桌面双击 **Start-NeonMix.cmd**。命令行：`NeonMix-CLI.cmd runtime`、`NeonMix-Audio.cmd devices`；两个入口完整转发全部参数。`NeonMix-Background.cmd` 可独立启动本包后台。中文使用说明和已知限制随包提供。

首次运行由产品生成 `data/state` 私有状态；缓存/临时文件在 `data`、日志在 `logs`。原始目录包及ZIP均未运行，**没有测试凭证、配对、state、registry或日志**。不要只复制单个EXE；完整目录可移动。不需要Rust、Python、VS、MinGW或GStreamer安装，也不需要调整ExecutionPolicy。

## 版本与内容

沿用上一轮 Windows 原生验证、已修复256帧固定周期的五个 EXE；逐个SHA256与上轮记录一致。**本包不是后来并行开发的最新多AirPlay源码构建**。实际源文件/EXE hash见包内 `tested-version.json`，完整逐文件hash见 `manifest-sha256.json`。

包内提供5个NeonMix EXE、app-local MSVC/MinGW运行DLL、GStreamer所需插件和scanner、依赖notice/license、实际测试源码及固定UxPlay/libplist源码、补丁与构建入口。未包含测试profile/系统凭证/SDK/编译缓存。包内环境脚本设置运行库/插件/registry位置，并清除开发环境的GStreamer/PKG变量；不会写系统PATH或安装服务。

## 本轮从ZIP重定位后的实际验证

验证目录为 `E:\Desktop\NeonMix-test\package verification with spaces\NeonMix-windows-x64-20261002-tested-v1`，从最终ZIP新解包，子进程无tools/dev环境或开发工具PATH。

| 检查 | 结果 |
|---|---|
| 包内runtime、设备枚举 | 退出码0 |
| 3秒WASAPI输出 | 48kHz、默认480帧；146,016帧；错误0、callback超预算0。经独立Audio CMD转发超过8个参数，未截断 |
| 独立后台 | NamedPipe status/shutdown成功、退出0 |
| Hub原生输出 | 131,136帧；输出错误0、Mixer欠载0 |
| PE依赖扫描 | 158个PE文件，导入DLL缺失0；允许Windows系统DLL |
| 已加载模块核查 | background 13、Hub 74、worker 36项，全部来自解包目录或Windows系统目录；没有原开发目录或外置SDK依赖 |
| 五个EXE | 均与上一轮已测SHA256一致 |
| AirPlay worker | 能从移动后包加载全部依赖并运行；仍返回已知`media IPC must be IPv4 loopback`，exit1；不是加载失败，也不宣称AirPlay可用 |

证据：[重定位验证](relocation-verification.json)、[DLL导入清单](dll-dependencies.json)、[Hub诊断](relocated-hub-diagnostics.json)、[原生输出](relocated-output.log)。

GUI仍受SSH Session 0限制，本轮未声称真实桌面窗口/托盘交互通过；为用户保留真实console双击入口。上一轮150项自动测试、文件凭证9类、Mac→Windows30秒LAN通过属于此包相同EXE的历史证据，不冒充本轮重复运行。VB-Cable采集Xrun、Shanling384kHz格式与时钟偏差的既有边界在包内说明中保留。

## 留存与清理

原始目录包和ZIP保留在上述Windows路径；本地ZIP已核对SHA256。一次性含空格解包副本、其测试状态/凭证/dummy key/registry已删除，验证进程全部退出。现存源码、构建制品和缓存继续保留。所有本轮Windows文件操作都限定于`E:\Desktop\NeonMix-test`；未更改防火墙、用户权限策略、默认输出、驱动、系统服务或系统时间，未删除目标外文件。

清理核对见 [cleanup.json](cleanup.json)。

复核另观察到原开发目录 target/release 下的后台进程（PID 33236）；它不是本轮解包验证创建的进程，因此保留，未误杀其他手动会话。包验证路径下没有残留进程。
