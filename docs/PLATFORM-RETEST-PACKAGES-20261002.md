# Ubuntu 复测与两端可运行包（2026-10-02）

Ubuntu 已使用新源码快照重新构建、测试，并从重定位后的运行包完成桌面/后台/Hub 启动及 Mac 跨机音频测试。Windows 已把上一轮验证版本制作成独立运行包，并重新验证包内运行库、输出和后台。两个包均用于继续手动测试；两端 AirPlay 目前仍未验收通过，Ubuntu 原生跨机音频质量也未通过。

## 下载与校验

两台远端机器已留存解压好的目录包和压缩包，本机也有经校验的副本：

| 平台 | 本机可下载归档 | 大小 | 启动入口 |
|---|---|---:|---|
| Ubuntu 24.04 ARM64 | [NeonMix-Ubuntu-ARM64-u26-211335.tar.gz](../artifacts/packages/NeonMix-Ubuntu-ARM64-u26-211335.tar.gz) | 125,292,442 字节（119.5 MiB） | `./start-desktop.sh` |
| Windows x64 | [NeonMix-windows-x64-20261002-tested-v1.zip](../artifacts/packages/NeonMix-windows-x64-20261002-tested-v1.zip) | 64,335,408 字节（61.4 MiB） | 双击 `Start-NeonMix.cmd` |

最终 SHA256：

```text
Ubuntu  8d0319681e94f3c5e0d2e1a51a329df4d1d3d07a256ce8c4d82e06d84017b79c
Windows 91e41e3ff1cdb1c172ab3053c011f6ca7fb7e10f19d376c89f12e8f82011a2fc
```

## 本轮版本与测试结果

| 项目 | Ubuntu 24.04 ARM64 | Windows x64 |
|---|---|---|
| 机器 | `192.168.100.112`，已有 `parallels` 桌面会话 | `192.168.100.186`，`administrator` |
| 版本 | 本轮冻结的 511 文件快照，加一项测试清理修正 | 上一轮实测、已修复 WASAPI 周期的版本；5 个 EXE 哈希保持 |
| 构建与常规测试 | workspace release 通过；串行 182 passed、5 ignored | 上轮 150 passed、5 ignored；本轮验证打包后的同一制品，不冒充新版全量测试 |
| 独立 AirPlay worker | 新版数字协议 20 项、PCM/ALAC/AAC 解码通过；不等于 Hub 集成通过 | 保留原生 worker 和依赖，重定位后能运行并复现已知 IPC 拒绝 |
| 包重定位 | 干净环境、含空格的新目录运行；设备枚举、正式桌面/后台/Hub 启动通过 | 从最终 ZIP 解包到含空格的新目录；runtime、设备枚举、3 秒实际输出、后台 status/shutdown、Hub 输出通过 |
| 原生跨机音频 | Mac → 包内 Hub 60 秒；配对/发送退出 0，但 `network_degraded`，质量失败 | 上轮 Mac → Windows 30 秒接通；本轮包内 Hub 输出 131,136 帧，输出错误/欠载 0 |
| AirPlay | 旧接口两次启用返回 `503 busy`；新版后台操作也被拒绝，未进入 worker 启动 | 同一 worker 仍拒绝 NamedPipe 地址，报 `media IPC must be IPv4 loopback` |
| GUI 交互边界 | 正式进程/后台可启动；不能据此声称全部按钮、托盘、辅助功能已验收 | SSH Session 0 无可见 GUI 验收；需在登录后的 Windows 桌面手测 |

两端不是同一源码版本：Ubuntu 覆盖本轮新快照，Windows 包保留上一轮已经验证的制品。后续并行开发不在这些版本的测试结论内。`ignored` 不计通过。两端均没有真实 Apple AirPlay 来源成功接收、视频伴音或模拟端延迟结论。

## Ubuntu 60 秒跨机复测

测试对象是重定位后的包，而不是开发目录中的程序。远端清空开发环境变量，经已有用户会话启动；测试期间没有远端 Cargo/Rust 编译，也没有同时进行 AirPlay 调试。Mac 使用独立固定副本的原生 CLI，以 437 Hz、−36 dBFS 合成音源发送，未采集麦克风或改变本机原有 AirPlay 房间。

全程取得 28 组快照/诊断，无查询失败。59.763 秒末次采样：

| 指标 | 结果 |
|---|---:|
| 会话状态 | `network_degraded` |
| 输出设备可用 | 是 |
| 输出帧数 | 4,388,864 |
| 输出 Peak | 0.0040467 |
| 输出错误 / 回调超预算 | 0 / 0 |
| 接收丢包 | 500 |
| PCM sink dropped | 7 |
| Mixer underrun | 54,240 帧 |

结论是接通且有非零输出，但媒体质量未通过。不能用发送进程退出 0 或设备回调无错误覆盖丢包、PCM 丢弃和 Mixer 欠载。见 [60 秒摘要](evidence/platform-retest-packages-20261002/macos-client/ubuntu-relocated-bundle.json) 与 [逐次原始采样](evidence/platform-retest-packages-20261002/macos-client/ubuntu-relocated-bundle-samples.jsonl)。

## 本轮修正与 AirPlay 边界

Ubuntu 首轮常规测试中的 `terminal_report_survives_a_busy_diagnostic_reader` 挂起。GDB 显示测试清理在 `MediaWorker::drop` 等待线程，而该线程等待读取者仍持有的报告锁。单项独立执行可通过，不能把挂起直接归为音频实现故障。

本轮只修正该测试的清理顺序：持锁采集两个断言条件，释放锁后再断言；断言条件未放宽，生产行为不变。单文件格式检查和后续原生测试通过。[补丁](evidence/platform-retest-packages-20261002/test-cleanup-fix.patch) 与 [前后哈希](evidence/platform-retest-packages-20261002/test-cleanup-fix.json) 已保留。

Ubuntu 新版 AirPlay 本轮在启用阶段就被拒绝，没有到达 worker 启动。因此本轮既不能宣称旧 SIGSEGV 已修复，也不能把上轮 SIGSEGV 当作新版实际复现。旧版证据保留在 [上一轮完整报告](PLATFORM-TEST-20261002.md)。Windows 包仍是上一轮版本，本轮重定位 worker 再次明确复现 NamedPipe/TCP 不匹配。

## 手动启动

### Ubuntu

机器上已保留目录：`/home/parallels/NeonMix/artifacts/packages/u26-211335`。

在已登录的 Ubuntu 桌面，以普通用户 `parallels` 打开终端运行：

```sh
cd /home/parallels/NeonMix/artifacts/packages/u26-211335
./start-desktop.sh
```

CLI 入口为 `./neonmix`，例如 `./neonmix audio devices`、`./neonmix hub runtime`。包包含程序、非系统运行库、音频插件、字体和启动入口，不需要开发工作区、Rust 或 Python。仍需要匹配的 Ubuntu 24.04 ARM64、图形桌面和 PipeWire 用户会话；不要用 root 直接运行音频桌面程序。将整个目录保留在较短的真实路径，避免 Unix socket 的路径长度限制。运行状态和临时文件写入包内 `.s`、`.t`。

### Windows

机器上已保留目录：`E:\Desktop\NeonMix-test\packages\NeonMix-windows-x64-20261002-tested-v1`。

在登录后的 Windows 桌面双击 `Start-NeonMix.cmd`。命令行入口分别为 `NeonMix-CLI.cmd`（Hub）、`NeonMix-Audio.cmd` 和 `NeonMix-Background.cmd`，例如：

```bat
NeonMix-CLI.cmd runtime
NeonMix-Audio.cmd devices
```

不需要安装 Rust、Python、Visual Studio、MinGW 或 GStreamer。第一次运行由程序创建私有状态/凭证，日志在包内 `logs`，运行资料在 `data`。不要只复制单个 EXE；完整移动目录才能保留依赖。关闭窗口可能仍保留后台，请使用应用中的“退出后台”结束本包实例。包不安装虚拟音频驱动，也不修改默认音频设备。

## 包验证与证据

Ubuntu 最终归档含 418 个文件，主线程核对归档哈希及 417 个文件校验记录全部吻合；启动脚本保留可执行权限，无 `.s`、`.t`、`.credentials` 或 registry。含空格路径解包后 CLI、设备枚举、桌面与后台生命周期通过，原开发工作区运行库引用为 0。见 [Ubuntu 包元数据](evidence/platform-retest-packages-20261002/ubuntu/package.json) 及平台详细报告。最终归档仅在已验证的程序/脚本/资源上补齐许可证文本，随后重新逐项校验。

Windows 最终 ZIP 为 64,335,408 字节，982 个归档文件。158 个 PE 的静态 DLL 依赖扫描无缺失；运行中检查后台 13、Hub 74、worker 36 个模块，均来自解包目录或 Windows 系统目录，没有开发工作区依赖。主线程另外校验 ZIP 哈希和全部 981 个 manifest 文件，全部吻合，且无运行 state、凭证、registry 或日志。见 [Windows 包元数据](evidence/platform-retest-packages-20261002/windows/package.json)、[重定位验证](evidence/platform-retest-packages-20261002/windows/relocation-verification.json)、[归档完整性审计](evidence/platform-retest-packages-20261002/package-integrity.json)。

完整平台明细：[Ubuntu](evidence/platform-retest-packages-20261002/ubuntu/README.md)、[Windows](evidence/platform-retest-packages-20261002/windows/README.md)。源码清单、制品哈希、依赖来源、许可材料和已知问题随包留存。

本轮测试进程、解包验证副本、邀请和生成凭证已清理，最终目录包、压缩包和开发构建产物保留。Windows 的全部本轮文件操作限定在 `E:\Desktop\NeonMix-test`，未删除其他目录文件，未更改系统关键设置。清理时发现原开发目录另有一个非本轮后台实例，已保留；没有将“清理测试进程”扩大为停止用户其他会话。
