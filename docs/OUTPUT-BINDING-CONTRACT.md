# E06 本地输出绑定合约

实现：`crates/output-binding`，非实时线程持久化；`neonmix-hub output` 管理，`send --output-binding <directory>` 使用。2026-09-30。

每个状态目录最多有一个输出。`binding.json` 保存 schema/revision、稳定 `output_id`、经过 TLS/服务端权限校验取得的 `hub_id`、提供者、精确原生 `device_id`、可编辑 `display_name`、enabled 和 authorization_epoch。地址/IP 不进入身份，房间名和显示名变化不生成设备/输出新身份。重复 add 同一 Hub/设备返回原记录；换 Hub 或设备须先明确 remove，再 add 新输出 ID。

管理写入取得进程间文件锁，使用 `(expected_output_id, expected_revision)` 比较防止覆盖并发修改或已替换对象；私有目录/文件在 Unix 为 0700/0600，拒绝符号链接，JSON 限制为 16 KiB。新记录先在同目录临时文件写入并 sync，原子 rename 后同步目录。失败或冲突不发布半份 JSON，不把损坏记录视为“未绑定”并覆盖。

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
  --directory .local/output --expected-output-id '<show 返回的 output_id>' --expected-revision 1 --name 'NeonMix — 书房'
tools/dev target/release/neonmix-hub output disable \
  --directory .local/output --expected-output-id '<同一 output_id>' --expected-revision 2
```

enable/remove 同样要求 expected-output-id 与 expected-revision。enable 后必须重新明确启动 Sender；remove 不自动删除已安装第三方驱动或系统插件。Windows add 可自动选择唯一的自有 render endpoint，也可用 `--device <wasapi:stable-render-id>`；两条路径都要求驱动 INF 发布的固定 NeonMix owner 属性，拒绝普通物理端点或仅名称相同的设备。发现多个自有端点时要求明确选择，不自动切系统默认输出。

## 名称边界

display_name 在三端共享记录中稳定保存。自研 macOS HAL 的显示名称通过 `nmna` 自定义属性更新，标准 Name (`lnam`) 保持只读，CFString 类型/长度/UTF8/控制字符验证及标准 Name/自定义属性通知已接通；`cust` 描述符明确声明 CFString 和无 qualifier，使独立驱动宿主可正确封送，改名不改变 UID、时钟和媒体。`output sync-name --directory ... --expected-output-id <UUID> --expected-revision <revision>` 可在自研设备已加载时把保存名称应用到它；绑定 Sender 启动时也会同步。BlackHole 保留厂商原生名称，只改变 NeonMix 本地绑定标签。

Linux owner 已改为导出本地 PipeWire impl-node，名称通过 `pw_impl_node_update_properties` 在原节点上更新；`node.name`、设备身份及数据回调不重建。`output sync-name` 使用每 UID 的 abstract Unix socket，由 owner 核对 SO_PEERCRED；请求最多 256 UTF-8 bytes，队列容量 1，控制循环更新名称，音频回调不处理 socket 或文件。带 `--output-binding` 的 owner 从共享记录恢复名称并监听修改；手工无绑定的实验owner仍支持先创建节点；传入--output-binding时必须已有有效记录，固定该binding UUID，不自动采用重建对象。

Windows INF 在 EP\0 发布 `{1A50C7B0-B766-4E50-9DF8-3F49B6583BD1},2` 标记，值为 `com.neonmix.audio.virtual-output`。`output sync-name` 校验此属性与 eRender 后，经 IMMDevice property store 写入 PKEY_Device_FriendlyName、Commit 并读回；不重建端点、驱动或 ID。该写入接口要求管理员权限，普通 Sender 只读检查端点并发送，不自动请求提升权限。[Microsoft OpenPropertyStore](https://learn.microsoft.com/en-us/windows/win32/api/mmdeviceapi/nf-mmdeviceapi-immdevice-openpropertystore)。

上述 Windows/Linux 名称路径已在 macOS 交叉类型检查并通过 Clippy，尚无对应平台运行证据。绑定记录不包含配对凭证，当前凭证实验流程仍属于 E05 的独立边界。

## macOS 证据

`tools/e06_binding_probe.py` 使用真实 BlackHole → Sender → 加密媒体 → 原生 Hub：重复创建/重开保持身份；运行中改名继续非零 PCM；冲突拒绝；disable/enable 停止旧会话、不自动恢复；删除终止 Sender、重建新输出 ID；相同 Hub 换端口保留记录；相同 TLS 证书/令牌但不同 hub_id 在媒体前拒绝。测试恢复 BlackHole 设置并删除临时凭证，结果见 [绑定证据](evidence/e06/binding.json)。

Store 并发/重建/损坏测试及 HAL 名称 ABI/时钟/音频测试在 macOS 通过；实际自研跨进程名称、绑定撤销及服务恢复通过。指定 Ubuntu 桌面原地名称、稳定逻辑身份、绑定撤销及服务重建也通过。Windows 原生运行、完整平台生命周期与分发仍为 E06 完成门槛，范围见 [E06](E06.md)。


## P06 管理对象 CAS（2026-10-08）

rename/enable/disable/remove/sync-name 必须携带读取时的 output UUID 和 revision；缺少 UUID 的旧 CLI/IPC 管理客户端明确拒绝，要求刷新/升级，服务端不补入当前 UUID。A 删除后在相同目录创建 B，即便两者 revision=1，A 的旧请求仍返回 `output_object_replaced`，不会修改 B。绑定 schema/version与身份文件格式保持原值。

原生名称同步在 Store 管理锁下校验同一对条件，锁一直保持到平台 setter 返回；验证后不重新 load 当前对象。Desktop 保存后的自动 sync 使用保存回执中的 UUID/revision。Sender 启动名称同步使用启动时捕获的同一 snapshot，在当前对象已变化时失败，不用旧名称改写新绑定。正在发送的授权 watcher 继续以 output_id/authorization_epoch 等原条件工作。

自动/真实 API 接线、CLI 必填参数、实际本地 IPC 回归覆盖替换冲突、冻结字段和“保存 A 后同步期间 B 已重建”的交错；软件回调可证明陈旧请求没有调用 native setter。平台 setter 和真实设备的名称/权限/生命周期仍按每平台独立验收。


## Linux owner实例边界（P09）

每UID固定节点的内核abstract socket维持排他持有。独立后台创建owner时传入本次instance_generation；私有握手先以SO_PEERCRED检查当前UID，再读取version=1、instance_generation、output_id与ready。请求是NUL加有界JSON，最多1024字节，与既有UTF-8名称更新区分；失败/旧协议不当作就绪。不同后台代次或binding UUID返回ResourceOwnedByOtherInstance；没有创建时进程句柄时不收养现存owner。第二实例失败或退出不会清理首个实例的节点。原owner退出释放内核socket，新实例可按原持久binding重新创建。

后台Sender也传递原owner代次；CLI --output-binding在原生命名或采集前检查相同共享握手与binding UUID。手工CLI没有后台代次时可显式消费同一binding的手工owner，但不取得或管理其生命周期；不同binding/未知协议仍拒绝。无binding的--virtual-output继续是显式实验来源，不宣称持有跨state_dir共享owner。名称不作为所有权标识。握手仅在非实时控制线程执行，不进入音频callback或LAN API。
