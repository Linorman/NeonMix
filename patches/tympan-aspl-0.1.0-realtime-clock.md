# tympan-aspl 0.1.0 / NeonMix 本地修订 4

来源为 crates.io `tympan-aspl 0.1.0`，许可证 MIT OR Apache-2.0。原始 `.crate` SHA256 为 `2c12d37253045f85e23b4456b6b580504a4f83f2d59998d4fb23458db7d18863`。源码和两份许可证保留在 `vendor/tympan-aspl`，Cargo patch 固定使用它；完整差异见 `patches/tympan-aspl-0.1.0.patch`。

本地修改：

- 按 Apple SDK 修正 ProcessInput/ProcessOutput/ProcessMix/WriteMix 操作码，补齐 AudioServerPlugInIOCycleInfo 的全部字段和 SMPTETime 的子帧字段/计数器类型；SDK 27.0 独立对照全部一致。
- 补上静态设备不支持的 CreateDevice/DestroyDevice/配置修改回调，返回标准错误，避免空函数指针；校验 I/O 设备、流、长度及空指针。
- 标准输出主音量/Mute 控件及 legacy device aliases，属性通知、标量/dB 转换、finite/range/size 校验；参数通过原子状态进入 WriteMix，ReadInput 不重复应用。新增 IoBuffer.render_gain 只用于这一路径。
- 多客户端 StartIO/StopIO 仅在首个/最后一个客户端切换驱动和时钟，客户端数量限制为 128。
- GetZeroTimeStamp 移除 DeviceState mutex，创建对象时缓存 mach timebase；回调只读取单调时钟和原子设备时钟。
- 更新上游测试中的常量、结构布局和静态回调断言；279 项测试通过、1 项文档示例忽略。

NeonMix 自身的逐帧时间标记环形缓冲位于 `drivers/macos/hal`，不以这个依赖原始环形缓冲的取模数据作为读端输出。1000 周期的实际 C 接口表数据/时钟回调分配审计为零分配、零重分配、零释放。以上是本机源码/离线证据，未证明 Core Audio 已加载插件。

E06 绑定补充：标准设备 Name 可写，控制线程持有可编辑名称，CFString 类型/长度/UTF16/UTF8 与控制字符校验后复制并发送属性通知；改名不重建对象、UID 或时钟。记录通过独立共享绑定 Store 持久化，插件本体不读用户文件。

2026-09-30 并发与生命周期补充：`Driver` 要求 `Send + Sync`，生命周期及数据回调改为共享引用，删除 `DriverInstance` 的 `UnsafeCell` 与依赖 HAL 串行的可变别名。非实时生命周期 mutex 串行启动/停止；实时入口至多八次原子 CAS，停止先关闭入口、在控制线程等待已有回调结束，再调用 stop hook。音频线程不等 mutex。时钟最多三次版本快照读取，不能将旧 anchor 与新 seed 拼接。

`RemoveDeviceClient` 验证对象与客户端记录，并回收该客户端尚未 StopIO 的计数；只有最后一个退出才停止设备。首次启动/最终停止在释放框架锁后通知标准 DeviceIsRunning 属性。NeonMix 的样本与时间标记以 SeqCst 发布，立体声样本打包为单个 AtomicU64；并发环绕不产生跨帧左右声道。实际项目 bundle 在 Apple CFPlugIn 宿主中加载并通过 SDK vtable、标准控件、属性通知、退出与重启验证；这个宿主不等同于 coreaudiod 系统加载。

HAL 标准插件 UID→Device 翻译与 ResourceBundle 属性已补齐，qualifier 的 CFString 类型、指针大小和输出缓冲长度均有校验；未知 UID 返回 Unknown，不依赖显示名。Apple SDK/bundle 宿主额外验证了这一路径。

补齐标准可选性/hidden/clock-domain/related-device/stream-active 属性及 56-byte AudioStreamRangedDescription，SDK bundle 宿主独立验证实际物理格式范围。`trace-properties` 为默认关闭的开发 feature，仅初始化/控制属性输出至多 96 条 syslog，不调用于数据或时钟回调。

系统绑定复验纠正了离线标准 Name 写入的边界：Core Audio 客户端标准 Name 只读，实际使用 nmna 自定义 CFString setter，cust 描述符为三个 UInt32（selector/data-type/qualifier-type），写后通知标准 Name 与自定义属性。SDK bundle 宿主、实际名称读回/UID保持/绑定撤销及服务重启后的同 UID 数字重开均通过。签名脚本可显式使用已有正式身份，默认仍为开发 ad-hoc；正式签名/公证不因此通过。
