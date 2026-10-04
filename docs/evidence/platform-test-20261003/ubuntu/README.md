# Ubuntu 同版本测试（2026-10-03）

**构建、215 项常规测试、独立 worker 协议和后台通过；严格 Clippy、AirPlay Hub 接收与跨机播放质量未通过。** 本轮只使用三端共同冻结源码，没有修补产品后混用结果。

## 版本与环境

共同快照 512 文件，canonical manifest SHA256 `b0aa5d6bcf84e1d12dae4ff400cd885cb288f124107332907a9a1cb6cda7e9ef`；源码归档 SHA256 `c9d7f4682764dbad9cc0d22b9ded71aa8e1ea6e8393f6614e608cf636fb08704`。测试前后逐文件校验无差异，见 `source-verification.json`、`cleanup.json`。

目标 `192.168.100.112`，Ubuntu 24.04.3 aarch64 / Parallels、Linux 6.14、Rust 1.95.0、GStreamer 1.24.2、PipeWire 1.0.5。工作区 `/home/parallels/NeonMix/.local/t03u`；编译缓存通过项目内链接复用旧 target，SDK/cargo 缓存复制后在本工作区使用。应用以已有 parallels 桌面用户的 systemd user 会话运行。所有开发入口均经 `tools/dev`，没有修改系统设置或默认音频路由。

Hub SHA256 `62b1aa415841407f45ebe2c7cd9aedb3f56ba9201b12eb330e2c623a44a983c4`，worker SHA256 `1ee873f18102d6f4ca78fc4baacbf2e8ba1c6b8166b3e25f640b753cf863ae79`。其余制品见 `binary-sha256.txt` 和 `binary-architecture.txt`。

## 检查与运行结果

| 项目 | 本轮结果 |
|---|---|
| fmt | `cargo fmt --all --check` 被嵌套工作区布局阻断：excluded vendor manifest 向上找到了旧项目 workspace。按显式 workspace 成员执行格式检查通过，源码未改变。见 `fmt.log`、`fmt-members.json`。 |
| 严格 Clippy | **失败**：`crates/airplay-ipc/src/local_unix.rs:154` 的 `unsafe { std::mem::zeroed() }` 缺前置 SAFETY 注释，触发 `undocumented_unsafe_blocks` deny。见 `clippy.log`。未把它描述为已证实内存错误。 |
| workspace tests | 串行执行 **215 passed / 0 failed / 5 ignored**；忽略项不计通过。见 `tests.log`。 |
| release / CMake | workspace release、当前 worker、crypto probe 和音频 probe 编译通过。见 `release.log`、`worker-build.log`。 |
| 独立 worker | 数字协议 **23 个检查项**通过，PCM/ALAC/AAC 实际解码通过，5 项配对持久性场景通过；使用合成源，不代表 Apple 客户端互操作。见 `worker-protocol.log`、`worker-audio.log`、`pairing.json`。 |
| 后台与桌面 | 12 类后台场景通过，包括私有 IPC/profile、测试音、真实认证快照、客户端崩溃隔离、邀请取消、脱敏导出、媒体崩溃报告和显式 shutdown。正式 GUI 启动并建立 Hub；终止 GUI 后 Hub 保持，随后 shutdown 成功。见 `background.json`、`gui-lifecycle.json`。未验证真实鼠标、托盘恢复和输入法矩阵。 |
| Mac → Ubuntu 60 秒 | 同快照 Mac release 发送退出 0、28 次查询成功，输出 available/非零、output errors=0。但状态 **network_degraded**，累计 Mixer 欠载 53,760 帧。Mac 27 个活动样本的最大 lost=445、PCM sink dropped=6（约58.149秒）；末采样约60.283秒 receiver 已移除，空数组不能解释成丢包0。Ubuntu 在此期间没有编译、多 AirPlay 或 GDB 负载。见 `lan-samples.jsonl` 及主报告的 Mac 跨机采样。 |
| AirPlay 1/2/4 路 | 按当前 `/v2/airplay` configure → enable_receiver → pair_receiver 入口执行。configure/API 与真实输出正常，但 worker 在 ready 前退出，来源尚处 created 阶段，收到/释放块均为0；无法进入数字混音、活动路断开/恢复矩阵。见 `airplay-1.json`、`airplay-2.json`、`airplay-4.json`。没有用旧 v1 拒绝来判断本轮失败。 |

## AirPlay 启动崩溃证据

无调试器的三次数字源探针均记录 `worker_failed / failure_stage=worker_exit`。单独受控诊断用 SIGSTOP→exec 原 worker 的包装入口，保持父 Hub 看到的 PID；GDB attach 后继续，未通过另起子 worker 改变 expected PID。当前冻结 worker 捕获到 **SIGSEGV**：

```text
ed25519_key_get_raw       crypto.c:398
pairing_get_public_key   pairing.c:105
raop_init2               raop.c:726
main
```

完整栈见 `worker-gdb.log`，禁用了参数值打印，没有保存密钥。固定上游实现中，Ed25519 密钥加载失败可返回 NULL，而 pairing 调用者继续读取公钥；该路径与堆栈相符。**具体 PEM 导入失败原因未定位**，不把 OpenSSL 版本/PKCS8 兼容猜测写成确定根因。独立 worker 使用 openssl 生成测试密钥通过，不能替代 Hub 生成接收身份的接缝验收。

## 入口生命周期与隔离边界

`entry-lifecycle-final.json` 记录配置 1、2、4 个入口后逐入口启动、观察7秒、停止：全部 worker 最终 `worker_exit`，全部可停，Hub 输出帧持续推进且 output errors=0。

1/2 入口观察期间原生发送进程和电平保持。首个 1→2→4 矩阵的四入口阶段，原生发送已退出、电平归零；该 fixture 总时长23.997秒，而原生参数为 `--seconds 600`，不能解释为正常时长结束。原始通用探针删除了子进程日志，因此当时退出码/原因未知。

针对新疑点，仅追加一次四入口24秒取证：12个2秒样本原生 poll 均为 null，电平持续非零，退出前仍运行；清理阶段明确记录 `sender_stop_requested` → `sender_stopped`。见 `four-native-retake.json`、`four-native-1.log` 和 `native-early-exit-boundary.json`。**此复验没有复现提前退出，也不能消除首轮未知项；更不能算四路 AirPlay 媒体隔离通过。**

## 制品与清理

本轮五个可运行二进制保留在：

```text
/home/parallels/NeonMix/.local/t03u/artifacts/t03u/bin/
```

它们是独立副本，不依赖共享 target 中随后被重编译的文件；运行库仍在本工作区 `.local`，不是新的可移动分发包。需要手测时，以 parallels 桌面用户执行：

```sh
cd /home/parallels/NeonMix/.local/t03u
.local/env.sh systemd-run --user --collect \
  "$PWD/.local/env.sh" "$PWD/artifacts/t03u/bin/neonmix-desktop" \
  --state-dir "$PWD/.local/manual03"
```

旧手测包 `/home/parallels/NeonMix/artifacts/packages/u26-211335` 没有覆盖。

`cleanup.json` 确认本轮 NeonMix 进程为0，临时身份/邀请和调试入口已删除，AirPlay fixture 目录为空。完整原始日志与平台适配 harness 位于本机 `artifacts/platform-test-20261003/ubuntu/remote/`；适配仅涉及 Linux guard、PipeWire ID、lsof 路径、动态库扩展名、PID 查询和本轮独立夹具，没有产品源码修改。

本轮没有真实 Apple 音源接收、四台实机并发、模拟声音/视频同步/P95、8/24小时长测或发布验收。平台移植测试中的夹具失败保留在完整 artifacts，未算作产品失败。
