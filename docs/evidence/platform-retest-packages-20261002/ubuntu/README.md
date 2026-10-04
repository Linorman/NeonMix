# Ubuntu ARM64 重新测试与可运行包（2026-10-02）

本轮包可独立启动桌面、后台和原生 Hub，已留在 Ubuntu 并回传本机。**持续播放质量与 AirPlay 集成仍未通过**；这些限制也写进包内 README，不将可启动等同于完整功能验收。

## 交付

- 本机归档：`artifacts/packages/NeonMix-Ubuntu-ARM64-u26-211335.tar.gz`。
- Ubuntu 目录：`/home/parallels/NeonMix/artifacts/packages/u26-211335`。
- Ubuntu 归档：`/home/parallels/NeonMix/artifacts/packages/u26-211335.tar.gz`。
- 归档 125,292,442 bytes，SHA256 `8d0319681e94f3c5e0d2e1a51a329df4d1d3d07a256ce8c4d82e06d84017b79c`。
- 418 个文件，含五个程序、原生共享库闭包、媒体/AirPlay 插件、scanner、中文字体、许可材料、511 文件的真实源码快照、固定 UxPlay/libplist 源码归档与 patch。没有 `.s`、`.t`、`.credentials` 或测试身份。

在 Ubuntu 图形桌面以 `parallels` 用户打开终端：

```sh
cd /home/parallels/NeonMix/artifacts/packages/u26-211335
./start-desktop.sh
```

脚本通过已登录用户的 transient systemd unit 启动，不安装/enable 服务。不能使用 root 启动；完整目录路径最多 80 字节（按字节而非字符检查），移动时整目录一起移动。状态/日志/缓存写包内 `.s`，临时文件写 `.t`。包支持 `./neonmix hub --help`、`./neonmix audio devices`；声音测试应使用 `./neonmix session audio play ...`。详见包内 `README.txt`。

## 源码与环境

机器 `192.168.100.112`，Ubuntu 24.04.3 aarch64 / Parallels，Rust 1.95.0、GStreamer 1.24.2、PipeWire 1.0.5。工作区 `/home/parallels/NeonMix/.local/r26`；复用项目内缓存与 SDK，全部开发命令经 `tools/dev`。应用使用现有 parallels 登录会话，不修改系统权限/音频设置。

初始冻结 511 文件，canonical source manifest SHA256 `84b8e18d5499d666d859ff27e5531cbc614705a3fa14a3cdc970dc6dcb0cbadd`。本轮只追加主线程对 `apps/hub/src/media_worker.rs` 测试的最小修订：持锁时采集布尔结果，释放 report guard 后再 assert，避免断言失败时 destructor join 等待同一锁。最终 canonical manifest SHA256 `cddd52d25cca50e258e634389c1398b5797209b76e741ed9bf9521bcaebceb35`，逐文件见 `source-manifest.json`。产品 release 行为不受该 cfg(test) 修订影响。

## 结果

| 项目 | 结果与证据 |
|---|---|
| 常规测试 | 最终串行 workspace **182 passed / 5 ignored**，见 `tests-final.log`。不计忽略项为通过。 |
| 首轮挂起 | `terminal_report_survives_a_busy_diagnostic_reader` 在并行测试中挂住；GDB 记录测试线程 `MediaWorker::drop` join 与 worker `publish.lock` 相互等待。单项直跑通过；最小 guard 修复后完整串行通过。首轮与堆栈保留，没有删除失败记录。 |
| 构建 | workspace release 与当前 AirPlay worker/audio/crypto CMake 构建通过；见 `release.log`、`worker-build.log`。 |
| 独立 worker | 当前数字协议 20 项以及 PCM/ALAC/AAC 解码通过；见 `worker-protocol-final.log`、`worker-audio.log`。初次 harness 误指旧 build 导致 control_version 缺失，调整到本轮 build-r26 后通过；不归为产品故障。 |
| 包重定位 | 将目录复制到含空格路径，清空开发环境变量后设备枚举、桌面/后台/Hub 启动通过。`bundle-hub-maps.txt` 与 `bundle-desktop-maps.txt` 中旧工作区/SDK路径引用为 0；非系统库从包内加载。 |
| 真实归档解包 | tar.gz 解到另一个含空格目录，`hub --help`、devices、`start-desktop.sh` 均退出 0；GUI 活着、独立后台响应、终止 UI 后后台保持、shutdown 后 socket 删除。见 `archive-smoke.json`。最终只追加许可证文本，程序/脚本/资源未变；最终归档再逐项核对 SHA256。 |
| 60 秒 Mac → 包 Hub | 配对/发送成功，28 次 Mac 查询成功，输出 available、非零电平、output errors=0、callback overbudget=0。但约59.8秒 lost=500、PCM sink dropped=7、Mixer underrun=54240，状态 network_degraded，**质量未通过**。本机持续样本见 `bundle-lan-samples.jsonl`，Mac 侧见兄弟 `macos-client/ubuntu-relocated-bundle*`。本轮没有并行编译/worker诊断负载。 |
| 当前 AirPlay Hub | 最新 v1 Enable 两次均 **503 busy**，见 `airplay-hub.json`、`airplay-hub-second.json`；重定位包通过后台访问 v2 返回“权限或配对凭证不可用”，见 `bundle-airplay.json`。本轮未进入接收就绪，未成功启动实际接收 worker；不能据上轮较早源码的 SIGSEGV 断言本轮也崩溃。 |

桌面本轮验证正式进程与后台 API 生命周期，没有完成真实鼠标点击、托盘恢复、中文输入法、辅助功能交互矩阵。没有真实 Apple AirPlay 音源接收、实听、模拟端延迟、视频伴音、驱动安装、开机启动或签名发布验收。

## 清理与证据

`cleanup.json` 确认本轮 NeonMix 测试进程为 0；所有解包测试副本、邀请及生成凭证已删除，最终用户目录包和 tar.gz 保留。系统设置、默认音频路由未变。

完整编排日志/脚本在 `artifacts/platform-retest-packages-20261002/ubuntu/remote/`，精选证据在本目录；`package.json` 记录本机/远端路径、归档 hash 和本机逐文件校验数量。许可文本补齐后重新归档并回传，最终 SHA256 以上述值为准。
