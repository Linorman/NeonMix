# Ubuntu ARM64 E06 验证

用户追加授权的 `192.168.100.112` 上，Ubuntu 24.04.3 LTS / aarch64 / Parallels、PipeWire 1.0.5、WirePlumber 0.4.17 已通过本轮桌面虚拟输出功能验证。音频进程运行于已登录桌面的 UID 1000；root 只用于 SSH 传输、启动用户服务及权限拒绝检查。操作者确认系统设置 → 声音的输出列表可见 NeonMix。此结果不覆盖实体声卡、Linux x86_64 或其他发行版。

源码固定在远端项目的 `.local/e06-20260930/workspace`，没有覆盖其他任务的源码。最终 299 个文件的 SHA-256 见 [源码清单](evidence/e06/ubuntu-source-manifest.json)；初始/最终源码归档、构建日志和所有失败记录在本机 `artifacts/e06-ubuntu/`。远端依赖与构建缓存仍位于 `/home/parallels/NeonMix` 内，复用项目已提取的 SDK，没有向系统安装开发依赖。

| 验证 | 结果与证据 |
|---|---|
| 普通用户启动 | 用户 systemd manager 启动，不需要额外调度授权；最终基线还启用 `NoNewPrivileges=yes`，[原生基线](evidence/e06/ubuntu-native.json) |
| 独占与清理 | 同 UID 第二 owner 被拒绝；owner 停止后节点消失，Sender 不拥有或删除节点 |
| 格式、桥接和时钟 | 固定 `pipewire:neonmix.sink.default`，48k stereo f32；普通 `pw-play` 的 44.1k PCM16 转换后可读；437 Hz 与原生输出时钟通过 |
| 音量与应用混音 | 原生线性增益 0.5 的实测振幅比约 0.49998；两种测试音来自两个普通应用，RMS 比约 1.4139；Mute 和停止所有源后的 RMS 均为 0，[绑定/音量报告](evidence/e06/ubuntu-binding.json) |
| 名称、绑定与加密媒体 | description/nick 原地更新，运行中 global ID 不变，node.name/output_id/hub_id 稳定；Opus/SRTP 接收有非零 PCM；disable/enable 和 remove 撤销旧 Sender，新发送采用新会话与媒体上下文 |
| 服务与节点故障 | WirePlumber 重启、PipeWire PID 83497→114204、显式删除节点后均由同一 owner 重建唯一逻辑输出；每次重新打开后数字桥接通过。PipeWire 重启使旧输入/输出以 code 1 退出，[恢复报告](evidence/e06/ubuntu-service-recovery.json) |
| 权限 | root 启动被拒绝；root 与 UID 65534 发给 UID 1000 owner 的名称请求被 SO_PEERCRED 拒绝，节点未变，[权限报告](evidence/e06/ubuntu-permissions.json) |
| 构建与针对性检查 | ARM64 release、Linux 格式检查、音频/Hub 原生 Clippy `-D warnings`、5 项绑定持久化测试及 2 项 owner 锁测试通过 |

最终功能探针的采集错误、回调超预算、丢弃和过期帧均为零。这是短时数字功能检查，不代替长期稳定性、模拟端延迟或注销恢复。

## 本轮发现与修复

导出本地 adapter 后，Core roundtrip 与 proxy bound 的顺序并不固定。原实现在第一个 roundtrip 时未收到 bound 就提前失败。现等待两件事都完成，并兼容 version 1 的 bound_props 事件；真正失败仍走超时/错误及有界退避。

WirePlumber 重启后原节点仍有端口，但新的播放/采集流没有回调，日志出现 pending linkable 未激活。现监听已观察到的主 WirePlumber client 移除，退出本次 export，由 owner 退避重建。逻辑 node.name 和保存名称不变；运行时 global ID/object.serial 可以在服务重建后变化，不能当作持久身份。修复前失败和修复后实际读回均保留。

初次音量探针把 `wpctl` 的立方 UI 刻度误当线性振幅，还混入了变更前的累计样本。已改为直接设置并读回 SPA channelVolumes `[0.5, 0.5]`，跳过两次统计后再测量；没有修改实现去适配错误断言。刻度机制也见 [WirePlumber mixer 源码](https://github.com/PipeWire/wireplumber/blob/0.4.17/modules/module-mixer-api.c)。

SSH 下 `runuser` 创建的进程最初被 RTKit 拒绝，有限调度额度下曾通过；最终证据改用正常用户 manager 启动，无额外额度。单独检查 adapter 时项目 SDK 缺少 dbus-1.pc，失败日志保留；最终使用实际音频/Hub 构建图的 vendored D-Bus 特性，原生 Clippy 和测试通过，没有安装系统开发包。

## 复现入口和清理边界

在登录的桌面用户会话通过项目 `tools/dev` 运行；从 SSH 管理入口启动用户 manager 时采用以下形式，所有源码、日志、缓存和生成 WAV 均在项目内：

```sh
systemd-run --user --machine=parallels@.host --wait --collect --pipe \
  --property=NoNewPrivileges=yes --unit=neonmix-e06-user-probe \
  /home/parallels/NeonMix/.local/e06-20260930/dev \
  python3 tools/e06_linux_probe.py --desktop-session
```

`tools/e06_linux_binding_probe.py` 需要先以相同桌面用户启动持久 owner，绑定目录为项目 `.local/e06-binding`，并确认没有已有 binding.json。它建立独立测试 Hub Sink，恢复原生音量/Mute，删除临时 WAV/凭证和测试绑定。`tools/e06_linux_recovery_probe.py` 会实际重启测试桌面的音频服务，需要独立测试窗口及名为 `neonmix-e06-owner` 的用户服务；不得在有其他音频验收进程时执行。`tools/e06_linux_permissions_probe.py <uid>` 只由测试主机管理入口运行，音频 owner 保持非 root。

临时桌面 owner 已停止，最终基线确认节点清理。SSH 认证曾短暂被拒绝，连接恢复后已核对没有 NeonMix 进程/节点，删除本轮私有状态目录、临时资料与清理脚本。PipeWire、WirePlumber、pipewire-pulse 均正常，默认输出为原 HDA Sink，见 [清理证据](evidence/e06/ubuntu-cleanup.json)。没有将此次临时 unit 安装或设为永久自动启动；源码、构建缓存、SDK 和验收日志保留在项目内。

未执行注销/重新登录、整机睡眠、干净机器安装/升级/卸载及跨发行版矩阵；Linux 分发和 E09 门槛保持未完成。
