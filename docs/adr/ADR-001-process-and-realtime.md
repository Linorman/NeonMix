# ADR-001 / ADR-003：进程、实时边界与时间模型（E00/E01）

状态：工程基线已实施，三端完整验收待补。依据设计 §2、§5、§8.4。

E02–E04 已在独立 Sender/Hub、媒体与控制模块扩展；最新实时/时钟决策见 [ADR-003/004](ADR-003-media-mixer-control.md)。下述 E00/E01 范围保留为历史基线。

Cargo workspace 将音频核心、原生I/O、平台策略、音频CLI和egui诊断程序拆开。UI不持有音频流，通过启动同目录音频程序读取只读设备JSON。其他手动启动的播放进程不依赖UI生命周期。该历史通道仅属于 E00/E01。E07 已接入独立用户后台、受限 typed IPC 与权限检查，见 [ADR-012](ADR-012-desktop-background.md)。

采用32块SPSC队列和generation清理，避免生产者覆盖消费者内存，也避免过期恢复补播。回调与转换在预分配内存上运行；对象销毁在控制线程。详见 `docs/AUDIO-CONTRACT.md` 和分配计数测试。

内部48kHz stereo，保留原生采集时间线，输出按设备消费节奏转换。rubato 0.16.2 SincFixedIn使用固定chunk预分配API，当前不实现E03漂移控制；不能把名义采样率转换当作双时钟稳定性验收。

设备标识固定绑定，设备丢失终止探针并返回错误；不会自动切默认输出。重新打开构造新的时钟实例。原始设备帧位置通过CPAL最小扩展统一暴露，保留平台时钟含义；timestamp-derived presentation estimate单独命名，不以此代替原始设备时钟。
