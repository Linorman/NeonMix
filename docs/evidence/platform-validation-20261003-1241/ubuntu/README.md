# Ubuntu 同快照验证（2026-10-03）

**本轮构建、222项常规测试、身份/协议/解码和单路回归通过；四 AirPlay 稳态质量两次失败，尚未达到四路连续播放验收。** 3+1、2+2、1+3的30秒稳态以及0+4的60秒稳态通过新增质量断言。故障恢复的聚合定时跳帧必须单列，不能由存活lane计数零推导为全局零。

## 版本与执行边界

源为共同 `platform-validation-20261003-1241` 冻结快照，519文件，canonical manifest SHA256 `53ee66ce3ed30a46bb08dafa9a47dfe955aac41313e4d3b15e345f8dbe3d8e5e`。独立工作区 `/home/parallels/NeonMix/.local/tl3`，所有开发命令经 `.local/env.sh → tools/dev`，只复用项目内依赖缓存和SDK。Ubuntu 24.04.3 LTS / aarch64、Rust1.95.0、OpenSSL3.0.13、GStreamer1.24.2、PipeWire1.0.5；应用通过既有 `parallels` systemd user/GNOME会话运行，私有TMPDIR0700，显式选择 `pipewire:alsa_output.pci-0000_00_01.0.analog-stereo`，真实48kHz/1024帧周期，AirPlay观测设备积压约106.667ms。

正式质量结果来自用户要求的严格串行窗口：Mac本轮测试全部停止后，Ubuntu顺序执行low→matrix→GUI→专用LAN。`artifacts/.../ubuntu/remote/pre-serial/` 保留此前单路/后台/GUI结果；pre-serial ALAC一次timedlate118失败，串行原命令复验通过，未证明首败唯一原因，不抹除首败。

## 构建与原生功能

workspace tests **222 passed / 0 failed / 5 ignored**，54 suites；严格workspace all-targets Clippy `-D warnings`、release和当前4个CMake target通过。`cargo fmt --all`首败为嵌套工作区excluded vendor追溯外层Cargo目录；按cargo metadata枚举所有正式成员的格式检查通过，未改源码规避。

worker 23协议检查、5配对持久性场景、原生PCM/ALAC/AAC解码通过；17身份封装/错误启动检查通过，其中PKCS#8 v2当前OpenSSL下无崩溃。合法4种封装保持身份和签名，13类错误干净拒绝、不ready、不修改身份。

串行单路PCM/ALAC各2次暂停与重复SETUP恢复完整通过，timedlate=0、ingresslate=0，另同步模式通过。ALAC实时输入使用Mac只读FFmpeg生成的固定183字节合成包，Ubuntu真实worker解码，来源与SHA256见 [ALAC夹具来源](../../../../artifacts/platform-validation-20261003-1241/ubuntu/alac-fixture-provenance.json)；独立PCM/ALAC/AAC decoder检查不依赖该替代编码入口。

后台12场景、Mixer9个观察检查、管理4项检查通过；覆盖私有IPC/文件凭证、真实输出测试音、后台保持/权威重开、取消邀请、脱敏导出、崩溃隔离、定向增益/Mute/单多Solo、重连偏好、撤销与新PIN重新配对。

## 四路矩阵与质量

| 组合 | 稳态 | 聚合timedlate增量 | 存活lane欠载增量 | ingresslate增量 | 定向恢复 | 判定 |
|---|---:|---:|---|---:|---:|---|
| 4 AirPlay + 4读者首轮 | 30s | 3441 | 0/0/0/0 | 全0 | 未进入 | 失败：跳帧 |
| 4 AirPlay + 4读者唯一复验 | 30s | 0 | 133/234/176/42 | 全0 | 未进入 | 失败：欠载 |
| 3 AirPlay + 1原生 | 30s | 0 | 全0 | 全0 | 6 | 稳态通过 |
| 2 AirPlay + 2原生 | 30s | 0 | 全0 | 全0 | 4 | 稳态通过 |
| 1 AirPlay + 3原生 | 30s | 0 | 全0 | 全0 | 2 | 稳态通过 |
| 0 AirPlay + 4原生 | 60s | 0 | 全0 | 不适用 | 0 | 稳态通过 |

四路两次在新增质量门停止，未把未运行的故障恢复算作通过。状态读成功16193/16869次，均0错误。四路首轮queued blocks=7、lastsource rate44100，latencyadvance约1.718s、设备积压106.667ms，ingress接收仍有效；这些不能消除Mixer质量失败。

3+1/2+2/1+3合计12次恢复、36个baseline/fault/recovery窗口，存活lane underrun/ingresslate增量全零，但恢复窗口聚合timedlate合计445/269/170，单窗最大401/166/170。聚合计数不能精确归因于重新接入的victim或存活来源，因此不声明恢复期间无跳帧。启动累计值也不等于稳态增量：3+1启动timedlate1147和lane underrun2055保留，2+2启动timedlate257保留。

新版明显减少旧四路30秒的999658跳帧，但两次四路质量失败足以否定“已全部消除”。未修改生产代码，未追加无界重试或放宽断言。

## Mac → Ubuntu 原生跨机60秒

主线程同快照Mac native Sender退出0；28个活动查询样本无query失败，最后活动时长60.736s，nonzero meter。主线程最后活动样本：lost456、overflowPLC448（差8为派生jitter lost，不能据此证明物理网络丢包）、PCM sink drop/timing gap各14、Mixer欠载21120；queue drop/late/timedlate/output errors/callback overbudget均0。**质量失败。** 不能把发送结束后空receivers解释成全部零。

Ubuntu独立1Hz采样保存50个活动样本、跨度58.958s，末次更早样本lost452/overflowPLC444、PCM sink drop/timing gap各13，与主线程后续末值按各自时序分别保留，不拼接为同一个样本。原始分段最大wire→authenticated30.711ms、authenticated→jitter50.740ms、jitter→PCM38.442ms，native scheduling configured/entered4、failed0；这是诊断观察，不足以确定overflow PLC的唯一原因。guest该活动窗CPU均值4.019%、P95 6.030%、峰9.067%。

首个LAN专用Hub等待协调时600采样上限自动结束，全段receivers为空；保存 `lan-idle-first/` 与自动shutdown记录，为协调空闲，未计产品失败或音频通过。新LAN2保持同源码专用Hub，首次邀请包装 `{"invitation":"Invitation JSON string"}` 与主线程等待器预期直接Invitation对象不符，未消费即过期；修正测试夹具JSON解包、原Hub刷新120s后才真正完成60s发送。全部试验邀请最后已清理；LAN没有与本机matrix/GUI或其他Mac测试重叠。

主线程完整发送与晚一采样证据在 [跨机目录](../cross-platform/)，Ubuntu末次活动完整输出/receiver见 `native-lan-summary.json`；所有1Hz样本在原始 `ubuntu/remote/lan-samples.jsonl`。

## CPU与GUI

按1Hz `/proc/stat`观察guest 4逻辑核CPU，无强制起步/停止百分比门槛，编译与音频顺序执行。四路首轮/复验均值24.559%/23.921%、峰28.0%/27.068%；3+1/2+2/1+3/0+4均值14.753%/15.238%/14.719%/13.694%。不是host CPU或声音质量证明，详情见 `cpu-summary.json`。

正式release Xwayland GUI完成Ctrl1..5五页导航，CtrlK快速操作，截图确认Linux均显示Ctrl+提示；活跃Hub的AltF4关闭后GUI存活且Hub继续；终止GUI后后台保持，重开同Hub PID；CtrlK搜索quit、确认窗口Tab/Enter实际退出后台成功。截图逐张已检查，均为真实正式UI。Hub/Sender初始截图处于资料轮询尚未显示的空态，随后Mixer/设备/诊断及重开显示真实已共享房间。

`gui-palette-600`仅是调整尺寸请求的截图文件名，GNOME实际窗口1158×699，不能作为600×440验收；报告不声称窄窗通过。AltF4后窗口仍IsViewable，证明Hub保持与关闭请求处理，未证明托盘隐藏/最小化形态正确。Wayland原生鼠标/IME/托盘完整流程未验收。

## 原始结果、留存与限制

当前制品副本保留 `/home/parallels/NeonMix/.local/tl3/artifacts/tl3/bin/`，包括desktop/background/hub/audio/worker/credential-migrate，哈希见 `binary-sha256.json`，均为aarch64 ELF。它们依赖该工作区SDK，不宣称新便携分发包。旧手测包和旧用户资料未覆盖；默认输出、路由、RTKit/PAM及系统服务设置未改。

本轮不覆盖真实Apple设备互操作、物理音画/听感、8/24小时长测。首败夹具记录保留：漏复制credential_fixture后import失败；ROOT未随搬迁更正导致cleanup/receiver_snapshot异常；补充当前助手并固定ROOT后原命令复测。初次background output故障发生在这个首败窗口，现串行背景探针通过，未删除首败或声称确认唯一原因。

核心证据在本目录JSON与截图；完整日志、1HzCPU、Linux适配脚本与pre-serial结果在 `artifacts/platform-validation-20261003-1241/ubuntu/remote/`。最终 `cleanup.json` 证明自有NeonMix进程为空、TMPDIR为空、邀请已移除、路由与基线相同；LAN和CPU的systemd unit均MainPID0/inactive，旧包保持。源码最终逐项519文件无差异，规范canonical及archive SHA256见 `source-verification-final.json`。
