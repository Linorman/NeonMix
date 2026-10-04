# Ubuntu ARM64 E00/E01 验证

日期：2026-09-30。用户提供的节点为`192.168.100.112`，Ubuntu 24.04.3 LTS、aarch64，`systemd-detect-virt`确认运行在Parallels中。本轮按用户选择只测试当前工作区的E00/E01；E02–E04未纳入验收。

## 结果

| 项目 | 结果 |
|---|---|
| 原生构建 | Rust 1.95.0在ARM64 Ubuntu上成功构建release音频CLI和egui诊断界面，保留调试信息 |
| 自动测试 | 30项通过：核心21、I/O 2、音频CLI 6、诊断界面1；不运行E02–E04集成测试 |
| 工程检查 | 所选包的格式检查、Clippy `-D warnings`、44.1k→48k离线模拟通过 |
| PipeWire原生场景 | 9组通过：设备枚举、端点能力、48/44.1k桥接、Mono左侧下混、Mono反相抵消、暂停恢复、设备Mute、节点移除 |
| 仿真声卡输出 | 显式打开`pipewire:alsa_output.pci-0000_00_01.0.analog-stereo`，48kHz、1024帧周期，输出142336帧，时钟有效推进，错误/回调超时均为0 |
| GUI启动 | 在`parallels`用户的Wayland桌面成功运行5秒后主动终止；这是启动检查，不是完整视觉或辅助功能验收 |
| 状态恢复 | 临时测试Sink已清理，PipeWire允许采样率恢复为原来的`[ 48000 ]`；未修改默认输出配置 |

证据：[自动检查](evidence/ubuntu-arm64-peer/e00-checks/result.json)、[9组原生场景](evidence/ubuntu-arm64-peer/linux-runtime/result.json)、[声卡输出](evidence/ubuntu-arm64-peer/hda-result.json)、[GUI启动](evidence/ubuntu-arm64-peer/desktop-result.json)、[环境和包哈希](evidence/ubuntu-arm64-peer/environment.json)。

此节点提供真实Linux进程、PipeWire和ARM64原生运行证据，但声卡由Parallels仿真。没有据此宣布物理声卡/USB、模拟端延迟、Linux x86_64或E02–E04通过。

## 源码与修复

传入的是工作区快照，以已提交的`1fc338f`为基线，包含当时未提交文件。编译期间源码保持固定，仅同步了本轮两个构建脚本的修复：`tools/dev`和`tools/prepare_linux.sh`不再固定使用`x86_64-linux-gnu`库目录，而是通过`cc -dumpmachine`使用当前编译目标的目录。本机原有GStreamer和后续模块改动得到保留。

测试期间本地E02–E04相关文件继续变化，结果绑定到[测试源码哈希](evidence/ubuntu-arm64-peer/tested-source-manifest.json)，不覆盖随后发生的修改。[快照对照](evidence/ubuntu-arm64-peer/local-snapshot-comparison.json)列出当时已变化的文件。

环境准备中发现原系统软件源包含不提供ARM64包的地址；只在项目内配置了ARM64源及包索引，未改`/etc/apt`。Clang共享库已安装，但缺少内置C头文件；将`libclang-common-18-dev`解包到项目并设置bindgen资源目录后，构建成功。Rust包、原生包、Cargo缓存和编译产物均位于项目内；已有编译器、Clang共享库和中文字体只读复用。

## 远端落点与复现

远端项目目录：`/home/parallels/NeonMix`。测试使用拥有桌面音频会话的`parallels`用户，连接其`/run/user/1000`下的PipeWire和D-Bus服务。开发环境入口`.local/env.sh`设置项目内Rust、原生库、Clang资源和音频会话，最终调用`tools/dev`。

```sh
cd /home/parallels/NeonMix
runuser -u parallels -- .local/env.sh python3 .local/e00-checks.py
runuser -u parallels -- .local/env.sh python3 .local/runtime.py
```

第二条命令临时允许44.1/48/96kHz，运行9组场景，并在退出时恢复原来的采样率列表。原生时序测试与重型编译分开执行。

本地归档位于`artifacts/ubuntu-peer-192.168.100.112/`：`neonmix-e01-linux-aarch64.tar.gz`包含两个ELF、独立debug文件、运行日志、锁文件和构建哈希；`tested-source.tar.gz`保留精确源码快照。二进制和符号SHA256已在取回后核对。依赖许可文件是工作区解析目录，含未构建模块，不表示那些模块已经验收或随包分发。

这是开发构建归档，不是独立安装包。远端完整开发环境和可复用依赖保留在项目目录中；没有向系统安装开发包，也没有保存SSH密码。
