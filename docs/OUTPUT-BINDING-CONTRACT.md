# E06 本地输出绑定合约

实现：`crates/output-binding`，非实时线程持久化；`neonmix-hub output` 管理，`send --output-binding <directory>` 使用。2026-09-30。

每个状态目录最多有一个输出。`binding.json` 保存 schema/revision、稳定 `output_id`、经过 TLS/服务端权限校验取得的 `hub_id`、提供者、精确原生 `device_id`、可编辑 `display_name`、enabled 和 authorization_epoch。地址/IP 不进入身份，房间名和显示名变化不生成设备/输出新身份。重复 add 同一 Hub/设备返回原记录；换 Hub 或设备须先明确 remove，再 add 新输出 ID。

管理写入取得进程间文件锁，使用 revision 比较防止覆盖并发修改；私有目录/文件在 Unix 为 0700/0600，拒绝符号链接，JSON 限制为 16 KiB。新记录先在同目录临时文件写入并 sync，原子 rename 后同步目录。失败或冲突不发布半份 JSON，不把损坏记录视为“未绑定”并覆盖。

`enabled` 控制发送授权，不等同于卸载底层 OS 设备。每次 true→false 增加 authorization_epoch；重新 enable 不把代际改回。Sender 捕获启动记录，在独立控制线程每 200ms 检查身份、设备、授权代际和可读性；停用、删文件、替换、损坏、私有目录/文件权限丢失或无法读取都不可恢复地终止旧 Sender。即便 disable/enable 在一次轮询之间发生，旧授权也失效。名字和 revision 的普通变化不阻断媒体。文件 I/O 不进入设备回调或媒体泵。恢复权限或文件也不会重新授权原 Sender。

启动时先确认认证快照的 hub_id，协商响应再确认同一身份；revision 冲突后的新快照及运行中的控制快照也不能改变绑定 Hub。缺少协商身份确认的旧服务端仅可用于未绑定实验路径。拒绝不同 Hub 发生在 PCM/DTLS 发送前，不能因相同证书或相同房间名跨越 UUID 绑定。

## CLI

```sh
tools/dev target/release/neonmix-hub output add \
  --directory .local/output --credential .local/hub-lab/sender-a.json \
  --hub https://localhost:7443 --provider blackhole --name 'NeonMix — 客厅'
tools/dev target/release/neonmix-hub output show --directory .local/output
tools/dev target/release/neonmix-hub send \
  --credential .local/hub-lab/sender-a.json --hub https://localhost:7443 \
  --output-binding .local/output --seconds 60
tools/dev target/release/neonmix-hub output rename \
  --directory .local/output --expected-revision 1 --name 'NeonMix — 书房'
tools/dev target/release/neonmix-hub output disable \
  --directory .local/output --expected-revision 2
```

enable/remove 同样要求 expected-revision。enable 后必须重新明确启动 Sender；remove 不自动删除已安装第三方驱动或系统插件。Windows add 可自动选择唯一的自有 render endpoint，也可用 `--device <wasapi:stable-render-id>`；两条路径都要求驱动 INF 发布的固定 NeonMix owner 属性，拒绝普通物理端点或仅名称相同的设备。发现多个自有端点时要求明确选择，不自动切系统默认输出。

## 名称边界

display_name 在三端共享记录中稳定保存。自研 macOS HAL 的显示名称通过 `nmna` 自定义属性更新，标准 Name (`lnam`) 保持只读，CFString 类型/长度/UTF8/控制字符验证及标准 Name/自定义属性通知已接通；`cust` 描述符明确声明 CFString 和无 qualifier，使独立驱动宿主可正确封送，改名不改变 UID、时钟和媒体。`output sync-name --directory ...` 可在自研设备已加载时把保存名称应用到它；绑定 Sender 启动时也会同步。BlackHole 保留厂商原生名称，只改变 NeonMix 本地绑定标签。

Linux owner 已改为导出本地 PipeWire impl-node，名称通过 `pw_impl_node_update_properties` 在原节点上更新；`node.name`、设备身份及数据回调不重建。`output sync-name` 使用每 UID 的 abstract Unix socket，由 owner 核对 SO_PEERCRED；请求最多 256 UTF-8 bytes，队列容量 1，控制循环更新名称，音频回调不处理 socket 或文件。带 `--output-binding` 的 owner 从共享记录恢复名称并监听修改；记录首次不存在时仍可启动，支持先创建节点再绑定。

Windows INF 在 EP\0 发布 `{1A50C7B0-B766-4E50-9DF8-3F49B6583BD1},2` 标记，值为 `com.neonmix.audio.virtual-output`。`output sync-name` 校验此属性与 eRender 后，经 IMMDevice property store 写入 PKEY_Device_FriendlyName、Commit 并读回；不重建端点、驱动或 ID。该写入接口要求管理员权限，普通 Sender 只读检查端点并发送，不自动请求提升权限。[Microsoft OpenPropertyStore](https://learn.microsoft.com/en-us/windows/win32/api/mmdeviceapi/nf-mmdeviceapi-immdevice-openpropertystore)。

上述 Windows/Linux 名称路径已在 macOS 交叉类型检查并通过 Clippy，尚无对应平台运行证据。绑定记录不包含配对凭证，当前凭证实验流程仍属于 E05 的独立边界。

## macOS 证据

`tools/e06_binding_probe.py` 使用真实 BlackHole → Sender → 加密媒体 → 原生 Hub：重复创建/重开保持身份；运行中改名继续非零 PCM；冲突拒绝；disable/enable 停止旧会话、不自动恢复；删除终止 Sender、重建新输出 ID；相同 Hub 换端口保留记录；相同 TLS 证书/令牌但不同 hub_id 在媒体前拒绝。测试恢复 BlackHole 设置并删除临时凭证，结果见 [绑定证据](evidence/e06/binding.json)。

Store 并发/重建/损坏测试及 HAL 名称 ABI/时钟/音频测试在 macOS 通过；实际自研跨进程名称、绑定撤销及服务恢复通过。指定 Ubuntu 桌面原地名称、稳定逻辑身份、绑定撤销及服务重建也通过。Windows 原生运行、完整平台生命周期与分发仍为 E06 完成门槛，范围见 [E06](E06.md)。
