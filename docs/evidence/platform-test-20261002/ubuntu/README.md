# Ubuntu ARM64 实机测试（2026-10-02）

结论：当前源码的原生构建、170 项常规测试、文件凭证和后台生命周期通过；桌面预览正常渲染。AirPlay 独立数字协议/解码通过，但 Hub 集成 worker 以 SIGSEGV 退出，接收未就绪。Mac → Ubuntu 原生发送已接通，初测和空闲复验均降级，不能判定播放质量通过。

## 环境与源码

目标 `192.168.100.112`，Ubuntu 24.04.3 aarch64，Parallels VM，PipeWire 1.0.5、GStreamer 1.24.2、Rust 1.95.0。SSH root 仅用于编排，应用/构建以已有桌面用户 `parallels` 运行。

工作区 `/home/parallels/NeonMix/.local/platform-test-20261002/workspace`。源码快照含未提交修改，排除私有连接配置、凭证、缓存和旧证据：503 文件，归档 SHA256 `f20bbcf615f7388b0c9e48fd1888249d3ef9ea8f873daf7ef783bbe4e2d32536`；逐文件哈希见 `source-manifest.json`。两端共同原始 manifest canonical hash 为 `00505e6429b7fb930055954f1a1af3589d136e7300f196a5166fc590ab58411f`。

本轮原生复测同步了 `apps/airplay-worker/CMakeLists.txt` 的 basename 过滤与静态库 PIC 两项修复。Windows 专用 Rust import 及 worker `boolean_field` 重命名未同步进本 Linux 快照。最终二进制与 CMake 哈希见 `final-hashes.txt`；桌面最终制品另启用 screenshot feature。

## 已通过

- `tools/dev cargo test --workspace --locked`：170 passed、5 ignored；忽略项不计通过。首次缺少 appsrc 插件导致两个 Hub 测试失败，补全项目内插件后通过，完整首败保存在 artifacts 的 remote 目录。
- workspace debug 与 release 构建通过；没有声称本轮执行完整 Clippy。
- 文件凭证 9 类场景通过，fixture 清除；见 `credentials.log`。
- E07 后台 12 类场景通过：私有 IPC/profile、低音量测试音、认证快照、客户端崩溃后 Hub 保留、邀请取消/自连拒绝、脱敏导出、媒体崩溃报告、设置及显式 shutdown；见 `background.json`。
- worker 数字协议 20 项、PCM/ALAC/AAC 实际解码和 5 项配对保持场景通过；见 `worker-protocol-final.log`、`worker-audio-final.log`、`pairing.json`。这些是合成源，不是 Apple 客户端互操作验收。
- 正式桌面（`--state-dir`，非 preview）可启动并创建独立后台；杀掉 UI 后后台仍存在，之后可通过 IPC 停止。未自动化真实窗口关闭按钮、托盘恢复、鼠标/键盘交互。
- `desktop-preview.png` 是独立预览渲染截图，中文正常；快捷键仍显示 `⌘K/⌘1`，属于 Linux 平台文案问题，未判断实际按键行为。

## 失败与边界

1. CMake 原过滤 `REGEX "test|plistutil|cnary.c"` 命中整个绝对路径中的 `platform-test`，把 libplist 源文件全部删掉；修为 basename 匹配后配置通过。Linux ARM64 的 crypto shared probe 还需要 protocol/plist 开启 PIC；修复后 worker 和探针成功构建。首败与修复日志已保留。
2. 经 SSH/runuser 直接运行音频出现 `RealtimeDenied`（error 9）；通过已有登录用户 `systemd-run --user` 启动相同程序则正常。后续音频测试均使用该用户 manager；没有修改 RTKit、PAM、polkit 或系统限制。
3. 测试长路径超出 Unix socket 108-byte 上限；后台 fixture 和项目内 TMPDIR 缩短后继续。短临时路径仍位于 `/home/parallels/NeonMix/.local/`，未落系统 `/tmp`。
4. AirPlay Hub 集成最终未通过：真实 PipeWire 输出/TLS 成功，但 Speaker ready 超时，状态 `failure_stage=control_reader_closed`。临时诊断 wrapper 观测同一 worker 二进制的子进程 `exit_code=-11`（SIGSEGV），没有 fatal JSON。见 `airplay-hub-final.json`、`airplay-failure-state.json`、`worker-exit.json`。独立 TCP 媒体 worker 探针通过，不能外推 Unix Hub 集成通过；崩溃根因尚未定位。GDB 已安装，但没有属于本轮 worker 的 core/Apport crash 文件，没有修改系统 core 设置。最终 worker 已恢复原二进制，诊断 wrapper 不作为产品产物。
5. Mac CLI 配对后向 Ubuntu Hub 发送 437 Hz、−36 dBFS、30 秒原生媒体，成功发送且接收有非零电平；首次 10 秒快照 lost=65、PCM sink dropped=1、Mixer underrun=8640。停止编译后的第二次 30 秒复验仍 network_degraded（10 秒 lost=64、underrun=7680）。完整远端每 2 秒记录见 `native-lan-diagnostics-retest.jsonl`（45 样本，末段累计 underrun 最大 25920，output errors=0）；Mac 同段证据位于兄弟 `macos-client` 目录。没有把发送端退出 0 作为质量通过。

本轮没有真实 iPhone/iPad/Mac AirPlay 来源成功接收、端到端实听/模拟延迟、视频伴音、长测、驱动安装或三端发布验收。

## 清理与复现

开发命令由工作区 `.local/env.sh` 设置项目内 SDK、缓存和插件，再调用 `tools/dev`。正常音频运行使用：

```sh
runuser -u parallels -- "$WORKSPACE/.local/env.sh" systemd-run --user --wait --pipe \
  "$WORKSPACE/.local/env.sh" target/release/neonmix-audio play \
  --device pipewire:alsa_output.pci-0000_00_01.0.analog-stereo --seconds 2
```

平台适配 harness、全部原始日志与准备脚本在本地 `artifacts/platform-test-20261002/ubuntu/remote/`；仅改变探针的系统 guard、openssl 路径、动态库扩展名及 fixture 路径，未把这些测试脚本作为产品源修改。

`cleanup.json` 确认本轮 NeonMix 进程全部结束，自有临时状态、邀请和凭证已删除；项目构建产物与证据保留。没有更改默认音频路由、系统配置或既有项目资料。
