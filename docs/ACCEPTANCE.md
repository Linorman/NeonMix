# E00 / E01 验收记录与复现步骤

## 自动化

运行 `tools/dev python3 tools/check.py`。必须全部退出0：格式、Clippy无警告、全部workspace测试、release构建、离线模拟。测试包含格式错误、任意块大小、44.1/48/96kHz、音调/RMS、单声道映射、静音/无数据、过期/溢出、epoch切换、原始设备时钟独立推进/回退、回调核心零分配、CLI错误及UI状态。

Windows/macOS/Linux CI须在对应runner上分别运行；本地cross-check仅证明类型及条件编译，不能替代链接、运行或安装。

## 专用机器操作矩阵

每次保留 `doctor.py`、二进制哈希、设备ID/接口、系统版本、格式/period、JSONL、测量方法与结果。没有设备/权限/已验证驱动时记录blocked，不记pass。

| 测试 | 操作 | 成功条件 |
|---|---|---|
| 枚举与稳定ID | 两次启动devices；设备重插后再读 | 同一端点ID可复用；不依赖名称或序号 |
| 输出44.1/48k | release play，支持范围内分别设置rate | 真实回调，原生位置递增且rate换算正确；无错误 |
| 设备周期 | 支持范围内测试127、256、441、511、1024或对应合理周期 | actual与回调period可解释；无固定480假设 |
| 单声道映射 | 指定mono设备，使用已知L/R源 | 下混(L+R)/2，与规范一致 |
| 数字虚拟桥接 | probe.py向虚拟render端写测试音并从指定read端读取 | frames>silent_frames、频率/幅度对照；仅零样本不算通过 |
| 44.1/48k采集 | 虚拟端分别选择两种格式 | 块头保留源rate/位置；不支持明确失败 |
| 无活动应用 | 停止源应用但保留Sender采集 | 无忙轮询；no-data和silence准确；不积压补播 |
| 系统静音 | 对虚拟端点/应用执行Mute和恢复 | 测量实际PCM，记录增益位置；不是CLI --mute-at的替代测试 |
| 暂停/恢复 | capture --pause-at 1 --resume-at 2 | 新epoch/RESUMED标记；源位置重置，旧代数据清除 |
| 设备/格式变化 | watch同时拔插/改rate；重开同一ID | changed/removed或原生错误；不切未知设备；时钟重新建立 |
| 输出失效 | 运行play时移除指定USB设备 | 明确失败非零退出；不会改路由 |
| 服务/休眠 | 音频服务重启与系统休眠 | 断点/故障可见；重开新epoch；不播放历史PCM |
| UI隔离 | 独立CLI持续play时关闭诊断UI | CLI回调持续，退出UI不改变音频 |

正式虚拟端点安装、离线时钟、持久身份、签名、升级卸载等另属E06/E09。本表不意味着这些能力已经交付。

## 本次本机证据

按用户最新范围，本轮聚焦macOS。BlackHole 2ch 0.7.1的44.1/48/96kHz非零桥接、暂停恢复、设备系统Mute、停源重启已通过，见[BlackHole验收](BLACKHOLE-TEST.md)。虚拟输入也必须先取得宿主应用/终端的Microphone权限；此前Itour全零记录缺少这一前提，不能单独证明驱动不兼容。

见 `docs/STATUS.md` 和 `docs/evidence/`。开发机器是macOS arm64，不具备Windows/Linux音频实机；Windows目前只有交叉检查；Linux新增了项目内Ubuntu x86_64虚拟机数字运行证据，但实体声卡仍未验收。原生CI尚待配置远程仓库后执行。

已有Itour虚拟设备只作为对照：输入采集、静音及暂停/恢复成功，但同UID写入后无法读回测试音。记录该结果为未通过桥接闭环，不推断是NeonMix或第三方驱动的已确认缺陷。

## 耳机输出与受控拔插

本机External Headphones测试见[耳机记录](HEADPHONE-TEST.md)。可用 `tools/dev python3 tools/hotplug_probe.py --device coreaudio:BuiltInHeadphoneOutputDevice --seconds 180` 复现静音拔插检查：等待READY提示，再拔下、等待至少3秒并插回。脚本保存设备事件、原流故障和同ID重开的JSONL；不会自动改为其他输出。重开为新进程，不把不同进程中的epoch数值比较当作进程内代际验证。

同进程重开另用 `apps/audio/tests/output_reopen.rs` 的显式硬件测试验证，命令和本机结果见[耳机记录](HEADPHONE-TEST.md#同进程重开测试入口)。普通workspace测试默认忽略此项；只有设置稳定设备ID并指定`--ignored`才打开输出。

Core Audio运行中服务重启也已完成：旧采集/输出均明确失败退出，同UID重开后恢复437Hz数字桥接。复现入口为 `tools/dev python3 tools/macos_service_recovery_probe.py --device coreaudio:BlackHole2ch_UID --restart-coreaudio`，需要操作人员在系统窗口完成管理员验证；见[成功证据](evidence/macos-blackhole/service-recovery/result.json)。
