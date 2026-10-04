# macOS 严格串行复测（2026-10-03）

本轮测试范围内全部检查通过；测试从 **2026-10-03 14:48:23 到 2026-10-03 14:57:35（Asia/Shanghai）**。Ubuntu新轮测试及清理完成后，Mac独占测试窗口；没有Ubuntu并行测试，没有自有编译/GUI/LAN与多路矩阵重叠。原用户四个NeonMix后台进程保持运行。

共同519文件快照canonical SHA256：`53ee66ce3ed30a46bb08dafa9a47dfe955aac41313e4d3b15e345f8dbe3d8e5e`。隔离源码在 `/Volumes/projects-mac/NeonMix/.local/tm3`，开始/结束核对均零差异，没有改动生产源码。复用项目内缓存与编译结果后，本轮实际重新执行fmt、Clippy、测试、release、CMake及所有运行探针，保留本轮命令和时戳，没有引用旧轮pass替代新执行。

## 构建与基础功能

- 显式workspace成员格式检查、严格 `Clippy -D warnings`、完整workspace测试和release通过；测试 **232 passed / 0 failed / 5 ignored**，忽略项不算通过。
- 嵌套快照下使用 `cargo metadata` 枚举正式workspace成员后执行 `cargo fmt -p ... -- --check`；不将此写成 `cargo fmt --all` 的本地路径依赖扫描通过。
- 当前快照CMake worker及crypto/audio/identity probes构建通过。worker **23协议、17身份启动、5配对持久化场景**通过，PCM/ALAC/AAC解码通过。
- 文件凭证 **9场景**与独立后台 **12场景**通过，无真实用户凭证访问。GUI自有后台/Hub与临时文件均已清理。

## 数字音频矩阵

| 组合 | 完整恢复数 | 质量观察窗 | 聚合timed late / 存活lane underrun / ingress late最大增量 | 整机CPU均值 / 峰值 |
|---|---:|---:|---|---|
| 4 AirPlay + 0原生 | 24 | 73 | 0 / 0 / 0 | 71.48% / 91.15% |
| 3 AirPlay + 1原生 | 6 | 19 | 0 / 0 / 0 | 55.69% / 68.36% |
| 2 AirPlay + 2原生 | 8 | 25 | 0 / 0 / 0 | 46.65% / 55.62% |
| 1 AirPlay + 3原生 | 2 | 7 | 0 / 0 / 0 | 38.63% / 51.00% |
| 0 AirPlay + 4原生 | 0 | 1 | 0 / 0 / 0 | 31.64% / 52.76% |

四AirPlay使用原强度 **4个并发状态读者，每次查询后5ms等待、30秒steady、3轮逐源disconnect/workercrash**；完成24次恢复，**32,495成功查询 / 0错误**。五组合共 **40次恢复、125个质量窗口**。各组合steady、故障前baseline、fault interval和recovery interval计数分别存入 `summary.json.matrix[].phase_quality`，原始逐路/逐窗计数在相应场景JSON。所有窗口的聚合定时跳帧、存活lane欠载、AirPlay ingress late均零增量，没有计数重置。

稳态硬断言和原强度没有放宽。新轮五组合及单路均首轮通过，没有进行运行复验。CPU仅1Hz整机观察，不设启动/中止百分比阈值、亲和性或硬上限，也没有停止其他用户后台。四AirPlay CPU峰值91.152%保留，未据此中止。

Mixer的9项真实电平/增益/Mute/多Solo/恢复偏好控制及设备管理场景通过。PCM和ALAC各完成两次1秒暂停、重复SETUP恢复，timed late与late packets为0；同步模式通过，保留协议PTS且latency advance为0。没有观察到 `decoder_input_queue` / `pcm_output_queue` / `decoder_metadata_queue` 故障；探针没有暴露独立decoder错误累计计数，因此不虚构该数值。

## 正式桌面窗口

正式非preview GUI **11项流程**通过：五页导航、中文粘贴和草稿保持、创建并启动自有BlackHole房间、权威Mute提交、脱敏导出、⌘W隐藏/托盘恢复、GUI崩溃音频继续/重开读保存状态、⌘Q确认/Escape取消、确认退出后台。正常与最小内容尺寸 **1100×760 / 600×440** 下真实⌘K、Escape、⌘2及快速操作底部边界通过；32px原生标题栏使AX外框为 **1100×792 / 600×472**，具体尺寸和截图均保留，最小窗截图已人工查看。

初次GUI夹具把正常尺寸按AX外框指定，流程通过但内容高度只有728px；修正仅夹具的标题栏尺寸后再次执行完整正式流程，最终记录在 `formal-ui.json`。初次完整证据留在artifact `ui-initial-outer-size/`，不归为产品失败。

## 证据与复现

[汇总](summary.json)、[精确命令及Asia/Shanghai时戳](all-checks.json)、[构建日志](build-checks.json)、[worker/凭证/后台](worker-checks.json)、[正式GUI](formal-ui.json)、[runtime日志索引](runtime-checks.json)、[CPU采样](cpu.jsonl)、[二进制路径与SHA256](binary-provenance.json)、[最终二进制核对](binary-verification-final.json)、[源码开始核对](source-before.json)、[源码结束核对](source-after.json)、[清理](cleanup.json)。完整artifact在 `/Volumes/projects-mac/NeonMix/artifacts/platform-serial-retest-20261003-1430/macos`。

开发入口示例（完整每项命令及初次GUI执行以all-checks.json为准）：

```sh
/Volumes/projects-mac/NeonMix/.local/tm3/tools/dev python3 /Volumes/projects-mac/NeonMix/artifacts/platform-serial-retest-20261003-1430/macos/harness/run_validation.py build
/Volumes/projects-mac/NeonMix/.local/tm3/tools/dev python3 /Volumes/projects-mac/NeonMix/artifacts/platform-serial-retest-20261003-1430/macos/harness/run_validation.py worker
/Volumes/projects-mac/NeonMix/.local/tm3/tools/dev python3 /Volumes/projects-mac/NeonMix/artifacts/platform-serial-retest-20261003-1430/macos/harness/run_gui.py
/Volumes/projects-mac/NeonMix/.local/tm3/tools/dev python3 /Volumes/projects-mac/NeonMix/artifacts/platform-serial-retest-20261003-1430/macos/harness/run_validation.py runtime
```

上述replay会新执行并写报告，不是本轮额外执行记录。当前测试制品在隔离根 `target/release`，媒体程序需同根项目内GStreamer，通过 `tools/dev`启动；未制作新安装包。

当前ps自有fixture/runner/编译 **0**、自有TMP子项 **0**，原用户PID **81271 / 98624 / 98625 / 98648** 均存活；源码519文件及全部记录的binary hash最终一致。没有修改默认音频路由、安装系统库或改变关键系统设置。

本轮是合成认证加密AirPlay经真实CoreAudio/BlackHole输出与Mixer数字计数验证。未控制真实Apple设备，未测模拟端实听/端到端音画延迟，未执行8/24小时长测，GUI未验证VoiceOver实听或真实输入法组字。跨机LAN数据由主线程报告单独列出。
