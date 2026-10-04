# 最新三端复测问题修复与 macOS 验证（2026-10-03）

依据 [02:23 快照报告](PLATFORM-RETEST-20261003-022352.md) 定位。此次保留工作区已有改动，在当前项目源码上修复；没有覆盖冻结快照或旧运行包，没有连接 Ubuntu/Windows 执行测试。

## 已确认原因与改动

### AirPlay 低延迟余量被输出设备积压消耗

旧算法将 epoch 首包的呈现目标压到接收后 120ms，却没有计入原生设备的待播放数据。Ubuntu 单路报告记录 `output_latency_ns=106645833`，1024 帧回调还需约21.33ms。Mixer 必须在回调内预先生成整块将来播放的 PCM，因此首包之后几乎没有解码/提交余量，接入层按墙钟判断仍未迟到，Mixer 却已消耗完可用 PCM并反复重新获得时间线。

新增确定性测试直接连接真实 `Ingress → BlockProducer → Mixer`，以理想网络到包、相同106.645833ms积压和1024帧回调运行3秒：旧实现跳过 **19,092帧**，稳态有36,485个采样偏离应有的常量波形。修正后这些计数归零。另覆盖256帧/10ms及2048帧/160ms设备形状，全部无跳帧、欠载或稳态波形缺口。

新公式：`advance=max(协议目标−接收时间−设备输出积压−120ms,0)`。设备积压来自同次原生时钟观测的 `playback−callback`，在首包进入前更新；不再从启动期被饱和截断的累计帧数反推。首包之后保持该epoch固定平移，不随逐包到达或设备积压变化重定时。同步模式仍保留协议PTS；8块短队列和4秒/2MiB待播上限未扩大。

这证明并修复了一个与 Ubuntu 证据匹配的公共算法缺陷，不等于已经在 Ubuntu 原生复测通过。

### 定时通道缺PCM时漏记欠载

旧 `timed_next` 在缺PCM时输出零，但没有增加 `underrun_frames`/逐路计数；原生通道则有计数。因此旧报告的 AirPlay 逐路 underrun=0 不能证明连续播放。现在已开始播放后耗尽PCM会计入欠载，等待未来首个PTS的静音不计入。回归覆盖断流、恢复、逐路归属，并保持定时路径的实时约束。

多路探针原先只记录定时质量计数，仍可能将有声音和功能恢复计为 `passed`。现在稳态必须同时满足聚合 `timed_late_frames`、逐路欠载和各AirPlay ingress late均无增量。故障期间聚合late仍不能单独归因于存活来源。

### 诊断与开发入口

- Windows旧 `decoder_queue` 混合了压缩输入队列和PCM发送队列；现在区分 `decoder_input_queue`、`pcm_output_queue`、`decoder_metadata_queue`，保留脱敏错误分类。旧报告未保留足够信息，不能反推首败属于哪一种。
- 快速操作面板在600×440窄窗底部会被裁切，现按实际窗口高度计算顶部偏移与可滚动列表高度。
- 修正侧栏、快速操作、还原等快捷键提示：macOS显示`⌘`，其他桌面显示`Ctrl+`。只有macOS原生菜单实现的隐藏/退出快捷键不在其他平台展示。
- 本次标准Cargo测试暴露Hub和media测试可执行文件的项目内GStreamer运行库搜索路径缺失，已补充`profile/deps`对应rpath。没有向系统目录安装库。

## 尚未定位的问题

原生Mac→Ubuntu的398次overflow PLC与Windows首次2次overflow PLC仍缺少能确认唯一原因的原始时序。已在macOS实际DTLS/SRTP/Opus链路增加30ms批量到包验证，100个认证包、47,520 PCM帧，未发生lost/overflow/late/PCM丢弃或时间线缺口；因此不能把普通批量到包直接认定为原故障原因，也未盲目扩大40ms jitterbuffer或关闭容量约束。

Windows三worker同轮`decoder_queue`故障尚无确定复现；此次仅改善错误阶段辨识。Windows Session0没有可见桌面属于原测试会话限制，Linux/Windows正式GUI、真实Apple多设备互通、模拟端声音质量和8/24小时长测均未在本轮完成。

## 本轮验证

完整workspace测试 **232 passed / 0 failed / 5 ignored**；忽略项未算通过。严格Clippy、`cargo fmt --all -- --check`、workspace release、CMake worker构建通过。最后界面高度调整另行通过desktop 16项测试、严格Clippy与release构建。

| macOS真实CoreAudio/BlackHole数字场景 | 稳态 | 完整恢复 | 观察窗 | 定时跳帧/存活通道欠载/ingress late增量 | CPU均值/峰值 |
|---|---:|---:|---:|---|---|
| 4 AirPlay + 4状态读者×5ms | 30s | 24 | 73 | 0/0/0 | 68.73%/83.04% |
| 3 AirPlay + 1原生 | 20s | 6 | 19 | 0/0/0 | 57.01%/75.46% |
| 2 AirPlay + 2原生 | 15s | 8 | 25 | 0/0/0 | 50.60%/64.16% |
| 1 AirPlay + 3原生 | 15s | 2 | 7 | 0/0/0 | 37.59%/50.63% |
| 0 AirPlay + 4原生 | 60s | 0 | 1 | 0/0/0 | 30.15%/53.43% |

五组合合计40次故障恢复、125个采样窗口，窗口内质量增量均零。四读者28,909次成功查询、零错误。CPU为1Hz整机采样，只观察，无百分比起步/中止门槛或硬cap；编译和音频场景串行执行。窗口计数不等于连续模拟端实听测量。

单路PCM和ALAC各两次暂停/重复SETUP恢复通过，聚合timed late为零；同步播放模式通过。worker PCM/ALAC/AAC解码探针通过。

另行完成原生双Sender 60秒数字读回：**2,880,000帧、0静音帧**；两路lost/PLC/PCM sink dropped/queue drops/timing gaps和Mixer underrun均零，原生输出/采集错误、回调超预算均零。读回报告的capture discontinuities=1为保留的启动累计值，未将所有计数统称为零。此项为同机数字链路，不代表跨机或模拟声音质量。

macOS实际窗口在1100×760及600×440下，⌘K打开快速操作、Escape关闭、⌘2导航均通过；最终AX检查确认面板底部提示位于窗口范围内，截图已检查。Linux/Windows提示只做静态条件分支检查，未原生运行。首次UI夹具误用搜索框占位文字查找AX对象；实际AX提供无名text field，改按实际菜单项识别后快捷键检查通过，未修改快捷键行为。实际截图另发现并修复窄窗面板裁切，保留前后截图。

证据：[汇总](evidence/platform-retest-fixes-20261003/summary.json)、[原始场景及命令](../artifacts/platform-fixes-20261003-followup/runtime-checks.json)、[修复前确定性失败](evidence/platform-retest-fixes-20261003/ingress-before.log)、[修复后确定性回归](evidence/platform-retest-fixes-20261003/ingress-after.log)、[60秒数字读回](evidence/platform-retest-fixes-20261003/native-dual-soak.json)。完整原生采样/日志还在 [双路探针制品](../artifacts/e02-e04/20261003-035950/result.json)。

## 留存与范围

所有开发命令经`tools/dev`，缓存、临时目录、运行库、测试凭证及编译产物均在项目`.local/`、`target/`和`artifacts/`内；复现脚本保存在本轮artifacts目录。重要证据选择进入`docs/evidence/platform-retest-fixes-20261003/`。未安装系统库，未操作远端节点、系统默认音频路由或驱动。

本轮未创建Git提交。工作区包含原有未提交开发内容；[修复相关源码哈希](evidence/platform-retest-fixes-20261003/source-sha256.json)是相关文件清单，不冒充完整仓库冻结快照。运行制品见[二进制哈希](evidence/platform-retest-fixes-20261003/binary-sha256.json)。


最终[窄窗截图](evidence/platform-retest-fixes-20261003/ui-palette-600.png)与[修正前裁切](evidence/platform-retest-fixes-20261003/ui-palette-600-before-fit.png)、[UI检查](evidence/platform-retest-fixes-20261003/ui-result.json)、[清理核对](evidence/platform-retest-fixes-20261003/cleanup.json)已保存。自有测试进程和临时凭证已清理，测试前原有四个NeonMix后台/Hub/worker进程均保持存活。
