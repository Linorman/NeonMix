# Ubuntu 原生复测与运行包（2026-10-04）

Ubuntu 新快照的构建、226 项常规测试、单路 AirPlay 功能与桌面/后台生命周期通过；多路及跨机媒体质量仍未通过。可独立运行的新目录包和归档已经留在 Ubuntu 并回传 Mac，本轮没有修改产品源码或系统关键设置。

## 留存包与手动启动

- 本机：[NeonMix-Ubuntu-ARM64-20261004.tar.gz](../../../../artifacts/packages/NeonMix-Ubuntu-ARM64-20261004.tar.gz)
- Ubuntu：`/home/parallels/NeonMix/artifacts/packages/u26-20261004`
- Ubuntu 归档：`/home/parallels/NeonMix/artifacts/packages/NeonMix-Ubuntu-ARM64-20261004.tar.gz`
- 大小：126,364,060 字节；1,114 文件。
- SHA256：`cba300f0886f033068ca90c9f8e9264352aa1566bf71f0aa81afb31b9d3ddd66`

在已登录的 Ubuntu 图形桌面，以 `parallels` 用户打开终端：

```sh
cd /home/parallels/NeonMix/artifacts/packages/u26-20261004
./start-desktop.sh
```

CLI 为 `./neonmix hub --help`、`./neonmix audio devices`。普通用户运行，不使用 root/sudo。包要求 Ubuntu 24.04 ARM64 与现有 PipeWire、图形桌面、D-Bus/systemd user 会话；不是 x86_64 包。整个目录一起移动，支持含空格路径，真实路径需不超过 80 字节。程序、非 glibc 运行库、GStreamer 插件和 scanner、中文字体、源码快照、许可与启动入口均已包含，不需要 Rust、Python 或原开发工作区。首次启动的资料/凭证/缓存/日志在包内 `.s`、临时文件在 `.t`；结束测试在应用中选择“退出后台”。

[包元数据](package.json)、[本机独立校验](local-package-integrity.json)确认归档 hash 与全部 1,113 个静态文件校验项一致，未带运行状态、凭证、registry、软链接。`README.txt`、`VERSION.json`、源码 archive/manifest 与完整许可材料随包留存。

## 版本和环境

机器 `192.168.100.112`：Ubuntu 24.04、aarch64、Linux `6.14.0-27-generic`，现有 `parallels` 登录会话。应用和音频经 transient `systemd --user` unit 运行；所有开发命令经 `tools/dev`。本轮独立源码目录 `/home/parallels/NeonMix/d26u`，包未覆盖旧版本。

共同冻结输入为 522 文件，canonical manifest SHA256：

```text
322fda1b52872be7e880f750a02ef01fe0034895ecba6aa10741c504078de860
```

源码 archive SHA256：`3cc83620794eceb5fabf635853d9ff56e194330a89e51ac69f4db76bffb08faa`。开始与结束逐文件均无差异：[开始校验](source-verification-start.json)、[结束校验](source-verification-final.json)。原生测试后只对包副本执行 `strip --strip-debug`，原 release 保留；`.text`、`.rodata` 及所有 ELF `SHF_ALLOC` section 的内容 hash、地址、大小和 flags 完全一致。[原/包二进制与 section 证据](package-binary-equivalence.json)。

## 构建和功能测试

| 项目 | 本轮结果 | 证据 |
|---|---|---|
| 正式 workspace 成员格式 | 通过 | [构建检查](checks.json) |
| `cargo clippy --workspace --all-targets --locked -- -D warnings` | 通过 | 同上 |
| Workspace 串行测试 | 226 passed、5 ignored；ignored 不计通过 | 同上，原始 `tests.log` 留存 artifacts |
| Workspace release、worker/crypto/identity/audio probes | 通过 | [worker 构建](worker-build-final-checks.json) |
| 独立 worker 数字协议和鉴权 | 通过；23 项协议检查 | artifacts 的 `worker-protocol.log` |
| 配对持久化/重启、17 类身份输入、PCM/ALAC/AAC | 通过 | [identity](identity.json)、[pairing](pairing.json)及日志 |
| 文件凭证和后台 | 通过；12 个后台场景 | [credentials](credentials.json)、[background](background.json) |
| PCM、ALAC 暂停及重复 SETUP | 各两次通过，单路定时跳帧增量 0 | [PCM](single-pcm.json)、[ALAC](single-alac.json) |
| 音画同步模式功能 | 通过；不等于物理音画延迟验收 | [同步](synchronized.json) |
| Mixer 增益/Mute/Solo、来源管理/权限 | 通过 | [Mixer](mix-control.json)、[管理](management.json) |
| 可选旧凭证迁移工具 | 18 passed、1 ignored、strict Clippy通过；release 链接磁盘不足，未交付此 helper | [独立检查](migrate-checks.json)、[原始链接失败](migrate-release.log) |

独立 worker/单路通过只证明合成认证加密数字路径，不是 Apple 实机互操作结论。独立迁移工具不属于主运行包；新包创建文件凭证资料不需要它。

## 五组合质量和故障恢复

全部使用固定二进制、真实 PipeWire 输出和最新恢复期硬断言，未降低负载或放宽断言。四 AirPlay 为四独立认证数字来源、四并发读者每 5 ms 查询、30 秒稳态及两轮预定故障；其他 AirPlay 组合 30 秒稳态、0+4 为 60 秒稳态。

| AirPlay + native | 结果 | 具体失败 / 完成范围 |
|---|---|---|
| 4 + 0 | 失败 | 30 秒稳态跳帧增 **4,086**，四 lane 为 826/1,170/917/1,173；存活 lane 欠载及 ingress late 为 0；0/24 恢复。16,155 次查询零错误。 |
| 3 + 1 | 失败 | 稳态通过；三个断开/恢复通过，第4个观察（source1 worker crash恢复）窗口跳帧增 **20**；未完成6次质量验收。 |
| 2 + 2 | 失败 | 稳态跳帧增0，AirPlay lane2欠载增 **62**，其余 lane 0；未进入恢复。 |
| 1 + 3 | 通过 | 稳态及2次恢复、7个质量窗口均无跳帧/存活欠载/ingress late增量。 |
| 0 + 4 | 首轮失败、唯一诊断复验仍失败 | 首轮在等待60秒稳态结果时native Sender已退出；首夹具未保留退出码/日志。唯一同强度诊断中四Sender存活60秒，native lane欠载增960/1,440/960/960，共 **4,320**，仍质量失败。 |

[汇总和观察窗口径](native-matrix-summary.json)、[四路首败](four-contention.json)、[3+1](3plus1.json)、[2+2](2plus2.json)、[1+3](1plus3.json)、[0+4首败](0plus4.json)、[唯一诊断复验](0plus4-diagnostic.json)、[诊断命令](0plus4-diagnostic-check.json)均已留存。诊断只补退出码和脱敏原生 stderr，原二进制/断言/负载不变，不把后来来源存活当作早期退出已修复。

四 AirPlay 本段整机 CPU 平均23.99%/峰值29.55%；其余组合也未出现整机CPU满载。[CPU分段统计](cpu-runtime-summary.json)。这不排除单线程、锁竞争、VM调度或夹具发包节奏，不据平均CPU指定唯一根因。

## 最终包重定位与 Mac 协同

最终包复制到 `/home/parallels/NeonMix/p26 space/u26-20261004`，清空开发环境变量，`PATH=/usr/bin:/bin`。CLI和设备枚举、Hub/worker启用、实际合成加密AirPlay播放、正式GUI启动、GUI终止后后台保持和显式shutdown通过。扫描后台、Hub、worker、GUI映射文件：全来自包目录或系统，无开发目录引用。[重定位原始结果](relocation.json)、[分列判定](relocation-verdict.json)。

本包单路5秒质量区间仍失败：输出帧增244,736，timed late增0、Mixer欠载增 **48**、输出错误0，RMS0.01416。重定位报告的 `passed` 仅指可重定位/生命周期；`airplay_quality_passed=false`，整体质量判定为失败。没有用能启动或有声音覆盖欠载。

Mac原生合成Sender使用同快照，对此最终重定位包目标60秒。主线程确认没有Mac编译/压缩/其他自有音频；Ubuntu也无编译和其他音源。新120秒邀请配对成功，25个Mac活动采样无查询失败，但Sender约55.369秒报 `Connection refused (os error 61)`，不能称完成60秒。最后53.701秒Mac活动采样及Ubuntu最后活动采样均为：

| 指标 | 结果 |
|---|---:|
| lost / overflow PLC / late / queueDrop | 0 / 0 / 0 / 0 |
| PCM sinkDropped / timingGap | **93 / 8** |
| Mixer underrun | **15,360** 帧 |
| 输出错误 | 0 |
| 输出电平 | 非零 |

Ubuntu完整采样250次；活动receiver/media_worker仅32次，epoch范围 `1791045949.568`—`1791046000.689`，随后会话消失。Hub仍可查询，输出时钟epoch1、无设备错误，一直采样到 `1791046210.421`；显式 `lan-done` 后才在 `1791046211.566` shutdown。unit `RuntimeMaxUSec=infinity`、夹具3600轮上限未耗尽且无运行异常，**不是夹具到期/Hub整体提前关闭**。`Connection refused`的具体链路与会话终止原因仍未定位，不能直接判发送器本身故障。会话消失后的最终累计欠载16,320包含后续阶段，质量判断使用上表活动期15,360。

[完整采样](lan-samples.jsonl)、[最后活动](lan-last-active.json)、[变化区间](lan-receiver-changes.json)、[结束前最后状态](lan-receiver-final.json)、[生命周期](lan-lifecycle.json)保留。background诊断API按产品规则脱敏；会话移除后未保留可唯一解释退出的终端worker错误，因此目前不能还原唯一终止原因。没有再发邀请重复试图覆盖首败。

## 正式桌面与范围

正式 release GUI 经Xwayland捕获，实际执行Ctrl+1…5、Ctrl+K、关闭、重开并核对相同后台Hub、确认退出。600×440窄窗大小核实通过；普通窗请求1100×760但本机桌面可用高度使实际内容1100×699，未声称请求760已实现。[普通窗](gui-x11.json)、[窄窗](gui-narrow-x11.json)、[窄窗快速操作截图](gui-narrow-palette.png)、[窄窗诊断截图](gui-narrow-diagnostics.png)。

该测试覆盖导航和生命周期；不代替所有按钮/输入法/托盘恢复/辅助功能、真实Apple来源、多Apple设备并发、物理听感/端到端音画延迟、8/24小时长测、系统驱动或签名发布验收。

## 空间、夹具修正与清理

开始仅余3GiB。第一次独立target的格式/Clippy通过后，测试依赖编译因空间下降被主动停止，未声明测试完成；只删除本輪未完成target，复用项目内既有target后完整重跑，保留[cache切换记录](cache-switch.json)。可选迁移工具release真实出现磁盘链接失败。仅清理本轮冗余bin副本及复制的Cargo缓存，原target/SDK/旧包保留。

包装第一轮因误将独立迁移工具当workspace成员而缺文件，日志保留；重定位入口错误传 `devices --json` 已被保留并修正为真实CLI。第一归档校验器错误用hash作索引，使相同内容文件被覆盖；只改为按路径索引，同一归档所有成员重新核验通过。上述入口/环境问题与媒体质量失败分列，未修改产品源码/断言。

所有本轮应用进程、测试凭证、邀请和含空格验证副本已清理；新目录包、归档、source和原target保留。网络路由前后相同，没有改默认音频设备、RTKit系统配置或持久系统设置，旧包继续存在。[清理与取证边界](cleanup.json)。原始完整日志、可重放入口位于本机 `artifacts/platform-delivery-20261004/ubuntu/remote/` 与 Ubuntu `d26u/artifacts/d26u/`；长期JSON/主要日志与截图在本目录。
