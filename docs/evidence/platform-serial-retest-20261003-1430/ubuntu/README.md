# Ubuntu 严格串行重测（2026-10-03）

**这一轮五种四路组合均通过稳态质量断言，28次定向恢复的存活通道保持；原生Mac→Ubuntu60秒仍有400次overflow PLC和12960帧Mixer欠载，质量未通过。** 与上一轮同源码、同二进制；新四AirPlay通过不能称为代码修复，也不能抹除上一轮两次失败。

## 版本、时序和执行方法

共同快照 `platform-serial-retest-20261003-1430`，519文件，canonical SHA256 `53ee66ce3ed30a46bb08dafa9a47dfe955aac41313e4d3b15e345f8dbe3d8e5e`，archive SHA256 `f0cdbce09265121b1257d99c996585618fa75271b1d1adf5d15b0d5be3152c18`。开始和最终逐文件校验均无差异，见 `source-verification-start.json` / `source-verification-final.json`。

本轮实际开始14:34:37，清理核对14:48:00（Asia/Shanghai）；`workflow-checks.json`、`checks.json`、`runtime-checks-low.json` / `runtime-checks-matrix.json` 保存每条真实新命令及开始/结束epoch。复用原工作区 `/home/parallels/NeonMix/.local/tl3` 与编译缓存，全部报告和制品写到新目录 `artifacts/s1430/`，没有沿用旧通过日志或覆盖旧记录。

主线程确认Mac本轮测试/编译停止后，Ubuntu严格顺序执行构建→低负载探针→五组合→宽窗GUI→全新私有资料的窄窗GUI→专用LAN。Mac仅在最后LAN窗口充当唯一发送端。全部Ubuntu测试停止确认后才允许Mac本机实验。

机器192.168.100.112，Ubuntu24.04.3 LTS/aarch64，Rust1.95.0、OpenSSL3.0.13、GStreamer1.24.2、PipeWire1.0.5。开发命令经 `.local/env.sh → tools/dev`；应用在既有parallels systemd user/GNOME会话运行。显式输出为 `pipewire:alsa_output.pci-0000_00_01.0.analog-stereo`，48kHz、1024帧周期，AirPlay观察到输出积压约106.667ms。TMPDIR在项目内，权限0700；缓存/SDK/构建产物均在项目范围。未改默认输出、路由、RTKit/PAM或系统服务设置。

## 新执行检查结果

| 检查 | 本轮结果 |
|---|---|
| 所有正式workspace成员格式 | 通过，1.230s；未扩展为excluded vendor格式验收 |
| workspace/all-targets严格Clippy | 通过，15.070s，`-D warnings` |
| workspace tests | 222 passed / 0 failed / 5 ignored，54 suites，59.268s；忽略项未计通过 |
| workspace release | 通过，124.838s，允许既有缓存增量但实际重新运行 |
| 当前CMake worker及3个probe target | 通过，4.237s；当前源码重新配置/构建 |
| 独立worker协议 | 23项通过 |
| 身份和PKCS#8 v2启动 | 17项通过：4种合法封装保持身份/签名，13种错误干净拒绝，无SIGSEGV、不ready、不重写密钥 |
| 配对持久性 | 5场景通过 |
| 真实worker解码 | PCM/ALAC/AAC通过 |
| 文件凭证 | 9场景通过，含归档排除秘密、回环读写、错误拒绝及失败清理 |
| 单路PCM/ALAC | 各2次pause+重复SETUP完整通过，timedlate=0、ingresslate=0；同步模式另行通过 |
| 独立后台 | 12场景通过，含私有IPC/文件凭证、测试音、崩溃隔离、重开、取消邀请、脱敏、显式shutdown |
| Mixer/管理 | 9个调音观察和4个管理检查通过，涵盖增益/Mute/单多Solo、重连、撤销、新PIN及独立播放方式 |

实时ALAC合成352帧输入仍使用只读Mac FFmpeg生成的固定183字节包，Ubuntu当前worker实际解码；复用的是输入夹具，不是旧测试结果。SHA256与生成命令在本轮 `ubuntu/alac-fixture-provenance.json`，独立3-codec probe不依赖该替代编码入口。身份/配对/解码和所有实时脚本均从当前冻结生产探针做Linux路径适配，适配只存本轮artifact，没有改生产源码。

`environment.json`中的辅助设备枚举曾沿用未支持的`--json`参数并返回exit2，仅为环境夹具记录；不计此项为产品枚举通过。真实Hub输出metadata、正式GUI显示的2个本机设备及所有播放实验为本轮实际输出依据。

## 五组合稳态与故障恢复

| 组合 | 稳态 | timedlate增量 | 各lane欠载/各AirPlay ingresslate增量 | 已完成定向恢复 | 恢复窗口聚合timedlate |
|---|---:|---:|---|---:|---:|
| 4 AirPlay + 4并发读者×5ms | 30s | 0 | 全0 | 16 | 123 |
| 3 AirPlay + 1原生 | 30s | 0 | 全0 | 6 | 226 |
| 2 AirPlay + 2原生 | 30s | 0 | 全0 | 4 | 51 |
| 1 AirPlay + 3原生 | 30s | 0 | 全0 | 2 | 788 |
| 0 AirPlay + 4原生 | 60s | 0 | 全0 | 0 | 0 |

四路首轮本次通过，故没有触发仅一次原强度复验。四读者29997次成功、0错误。总共28次恢复、84个baseline/fault/recovery观察窗口，存活通道的欠载和ingresslate均无增量；**恢复期间聚合timedlate不为零**，不能把新victim允许的接入时间线处理扩大为“全局无跳帧”。表中最后一列为全部这些恢复观察窗口的聚合增量和；聚合计数不能唯一归因于victim或存活来源。

启动累计值继续保留：四路稳态起点lane0 underrun1958；3+1和2+2稳态起点timedlate329/749。它们不等于本表的稳态增量，也未被隐去。

[上一轮同源码证据](../../platform-validation-20261003-1241/ubuntu/README.md)中四路30秒首轮timedlate增3441、唯一复验四lane欠载增133/234/176/42，均失败且未进入故障恢复。本轮通过仅证明这一次严格串行窗口，不能认定间歇问题已经消失；本次没有修改产品代码、放宽断言或追加无限复验。

## 原生Mac→Ubuntu60秒

主线程pair/send均exit0，28个活动查询无query失败；末59.858s为network_degraded、输出电平非零：lost400=overflowPLC400，派生jitter lost为0，Mixer欠载12960。PCM sink dropped、PCM timinggap、queue dropped、late、timedlate、output errors及callback overbudget均0。**这个60秒质量窗口失败。** 不能把lost直接称作已证实的物理网络丢包，不能把发送结束后的空receivers当作丢包零。

Ubuntu独立采样在每次请求后sleep1s，实际36个活动样本、跨度55.439s，最后一个较早样本lost397=overflowPLC397、Mixer欠载12960；PCM drop/timinggap等仍0。主线程较晚400与本地397按各自时序保留，未拼为同一份样本。完整末活动remote输出/接收器在 `native-lan-summary.json`；全部原始样本在 `artifacts/platform-serial-retest-20261003-1430/ubuntu/remote/lan-samples.jsonl`，主线程证据在同轮 [cross-platform目录](../cross-platform/)。

本地分段最大wire→authenticated4.591ms、authenticated→jitter32.513ms、jitter→PCM48.117ms；native scheduling configured/entered4、failed0。这些观测不足以确定overflow PLC唯一原因。与上轮末456lost/448overflow、14PCMdrop、21120欠载相比本次计数较少，但仍达不到零丢弃/零欠载目标。

本轮actual endpoint先写`lan-ready.json`，主线程启动自动消费等待器后才生成120s直接Invitation对象，权限0600；未重现上轮包装JSON/协调过期问题。专用LAN期间没有其他Ubuntu探针或Mac本机测试。

## CPU、正式GUI及留存

CPU仅用1Hz `/proc/stat`观察，4逻辑核，无百分比起步/停止门槛，编译和实时实验不重叠。完整各场景mean/P95/max在 `cpu-summary.json`，不是host CPU或模拟声音质量证明。

| 窗口 | guest CPU均值 | P95 | 峰值 |
|---|---:|---:|---:|
| 4 AirPlay / 4读者完整场景 | 21.786% | 25.000% | 26.055% |
| 3+1 | 13.456% | 14.467% | 18.500% |
| 2+2 | 13.372% | 14.358% | 15.960% |
| 1+3 | 12.951% | 14.610% | 16.080% |
| 0+4 | 12.696% | 13.995% | 16.332% |
| native LAN本地活动窗 | 4.233% | 5.000% | 5.263% |

正式release Xwayland GUI在宽窗和全新私有state窄窗分别完成Ctrl1..5五页导航、CtrlK快速操作、AltF4关闭请求后Hub继续、终止GUI后后台保持、重开同Hub PID、CtrlK搜索quit并Tab/Enter确认真正退出后台。宽窗实际1100×699（GNOME可用高度约束，未声称请求的760已实现）；**窄窗实际600×440**，XWD header和xwininfo均核实，快速操作底部提示及退出确认按钮均在窗口范围内。原始截图与PNG逐页检查，均为正式UI，不是preview。

部分早期Hub/Sender/Mixer截图仍在后台状态/房间身份异步刷新阶段，显示空态；设备/诊断及快速操作后续截图显示真实已共享房间。本轮没有从导航截图推导完整房间配置或真实手机发送的GUI操作验收。AltF4后xwininfo仍IsViewable，只证明关闭请求处理及后台保持，未扩大为托盘隐藏形态验证；Wayland鼠标/IME/托盘完整交互仍未覆盖。

本轮5个aarch64 ELF（desktop/background/hub/audio/worker）留在 `/home/parallels/NeonMix/.local/tl3/artifacts/s1430/bin/`，哈希/架构见 `binary-sha256-final.json` / `binary-architecture.txt`，依赖项目内SDK，不宣称新便携分发包。当前Hub SHA256 `8d9cd78b0b907d5ef37f634ce697e2128e3e1f823ce4b2050ffc24cda59169eb`，worker `b0aa5ce7d1194e5e3476a3eae852da5ef144972d76df36fda39ff54d7a196a84`；本轮与上一轮5个制品哈希全部相同，fresh命令和fresh运行结果不同。`credential-migrate`为excluded独立工作区，本轮未列为运行制品或构建验收。

最终 `cleanup.json` 保存三个unit均MainPID0/inactive、无NeonMix测试进程、私有TMPDIR为空、邀请已移除、路由与基线相同、源519文件无差异的证据。旧包和资料保持；本轮未安装驱动/系统库，未创建Git提交。不覆盖真实Apple互操作、物理音画/实听和8/24小时长测。
