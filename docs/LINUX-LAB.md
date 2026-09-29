# 项目内 Ubuntu x86_64 运行实验

这个实验在 macOS arm64 上运行 QEMU TCG + Ubuntu 24.04 x86_64。它验证真实 Linux ELF、PipeWire 用户态 API 和数字音频路径，不验证实体声卡、USB、模拟输出延迟或 Windows/macOS 的虚拟驱动。

## 当前环境

- QEMU 10.1.5 源码、编译产物：`.local/qemu-build/`。
- libslirp v4.9.1 与 Meson 1.9.2：`.local/qemu-tools/`；已有系统 GLib、pixman 和编译器只读复用。
- Ubuntu 官方 release-20260911 amd64 cloud image：`.local/linux-vm/base.img`，SHA256 `612b2c0cc1bc413a6cb8c38fd611794caf0f2b436c50013d8b3794db12ad7354`。
- 可写 overlay、cloud-init、实验专用 SSH key、QMP socket：`.local/linux-vm/`。SSH只监听 `127.0.0.1:22282`，不使用用户已有密钥，不修改用户SSH配置。
- 访客中的所有安装、缓存和日志都存于项目内的qcow2文件；主机不安装QEMU、库或驱动到系统目录。

原始镜像和校验文件来自 [Ubuntu 官方目录](https://cloud-images.ubuntu.com/releases/noble/release-20260911/)，QEMU来自 [官方源码发布](https://download.qemu.org/)。启动前 `linux_vm.py` 校验镜像SHA256。

## 复用现有实验环境

```sh
tools/dev python3 tools/linux_vm.py start
tools/dev python3 tools/linux_vm.py status
tools/dev python3 tools/linux_vm.py configure
tools/dev python3 tools/cross_linux_check.py --build-audio --test-binaries
tools/dev python3 tools/pack_linux_lab.py
tools/dev python3 tools/linux_vm.py copy .local/linux-vm/payload.tar /home/ubuntu/neonmix/payload.tar
tools/dev python3 tools/linux_vm.py ssh 'cd ~/neonmix && tar -xf payload.tar && python3 tools/linux_runtime_probe.py'
tools/dev python3 tools/linux_vm.py stop
```

`pack_linux_lab.py`拒绝打包源码已变化或SHA256不匹配的旧二进制。`payload-manifest.json`列出当前测试可执行文件和哈希，运行测试时按此清单选择，不按目录里可能残留的旧文件计数。

访客为非root的 `ubuntu` 用户运行音频程序；cloud-init仅在安装PipeWire等系统依赖时使用访客root。为验证两种采样率，此专用VM的用户PipeWire配置允许44.1/48/96kHz；配置文件在访客 `~/neonmix/.local/config/pipewire/pipewire.conf.d/`。它不改变主机音频路由。

## 原生 Linux 上复现

在具备PipeWire/WirePlumber的目标机器构建release后运行：

```sh
tools/dev python3 tools/linux_runtime_probe.py --binary target/release/neonmix-audio
```

探针只创建独立命名的临时Sink，不修改默认路由；分别检查：

- 设备枚举（包括健康服务的空设备列表）；
- 48kHz与44.1kHz测试音频率、设备时钟和回调周期预算；
- Mono左声道输入的1/2幅度，以及左右反相输入下混后的静音；
- 暂停/恢复的epoch、首块位置和无数据统计（暂停3秒，保证1Hz监视器有完整空窗口）；
- 对实验Sink的系统Mute/恢复；
- 移除所选节点后的采集/输出失败及removed事件。

某采样率不被当前图支持会记录未通过，不偷偷改采样率。缺依赖、失败或异常会清理探针子进程，移除自己创建的非持久节点。报告在 `artifacts/linux-runtime/`，不保存PCM。

不要在运行回调期限探针时同时执行重型编译。TCG在主机繁忙时会被抢占，可能超过音频周期；本次保留了该失败记录，并新增 `callback_over_budget` 单独计数。该计数不冒充驱动Xrun事件。PipeWire 1.0.5的头文件尚无CPAL所检查的XRUN_RECOVER标记，因此“原生errors为0”不能独立证明无欠载。

## 交叉构建边界

`cross_linux_check.py`从固定Ubuntu仓库索引下载并校验头文件/运行库，保留在`.local/cross-linux`，使用Rust 1.95.0的标准库、LLVM归档器和链接器。macOS自带ar不能正确归档这些ELF对象，不能用仅通过cargo check代替实际链接。

此工具不是Ubuntu安装器，也不是原生Linux CI的替代。CI仍应在原生runner上执行workspace构建和测试，实体设备矩阵仍按 `ACCEPTANCE.md` 执行。
