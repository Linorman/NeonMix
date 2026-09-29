# E00 / E01 完成审计

依据：开发计划§4.1、§4.2与设计方案§2、§3、§5。按用户最新指示，当前执行范围收敛到macOS；其他平台保留历史证据，暂不作为本轮阻断。审计区分实现、自动检查、原生运行与专用机器验收；未证实的要求仍然未完成。

| 要求 | 当前权威证据 | 结论 |
|---|---|---|
| 单仓分层、核心不依赖平台/UI类型 | Cargo.toml、crates/audio-core、adapters、apps、drivers、tools、docs | 已实现；驱动目录只有范围说明，不是驱动实现 |
| 固定Rust、Cargo.lock及使用中的Rust依赖 | rust-toolchain.toml、Cargo.lock、native-dependencies.toml、vendor/cpal-provenance.json | 已锁定并本机构建 |
| 三端实际用户态适配，不能用空实现冒充 | Wasapi/CoreAudio/PipeWire的条件编译路径、vendored原生I/O、三目标检查 | 已实现；Windows只有交叉检查，不能视作运行通过 |
| 独立Python工具环境 | tools/pyproject.toml、uv.lock；产品二进制不调用Python | 已实现；核心工具仅标准库，Meson是可选linux-lab组 |
| 版本、日志、自动入口、符号和产物可追溯 | tools/check.py、collect_build.py、原生/交叉构建清单与哈希 | macOS产物/符号已验证；Linux ELF已链接运行；Windows原生产物/PDB尚缺 |
| 三端CI构建、格式、静态检查、逻辑与模拟测试；macOS arm64 | .github/workflows/ci.yml、本机检查记录 | workflow已实现；无remote，三端远端CI未执行。协议实现/测试随E02接入，当前不把空协议测试记作通过 |
| SDK/WDK、原生组件与独立构建路径 | native-dependencies.toml、docs/adr、Linux依赖锁 | 使用中的macOS/Linux版本有记录；Windows SDK基线尚未原生验证。WDK/插件与GStreamer实际构建分别留在E06/E02，未宣称已分发 |
| 统一测试信号与音频块字段 | signal.rs、block.rs、capture.rs、metadata/period tests | 已实现，保留源位置、采样率/布局、帧数、epoch、断点；支持静音/脉冲/正弦及单侧/反相 |
| Windows虚拟render端点采集 | adapters/windows、WASAPI loopback路径 | 代码已实现；未在兼容虚拟端点实机验证，不以实体回录替代 |
| Linux PipeWire Sink读取 | linux_runtime_probe.py、docs/evidence/linux-runtime.json | 已有真实Linux数字运行证据，含非零测试音；不是实体声卡验收 |
| macOS虚拟设备桥接 | Core Audio输入侧、BlackHole 2ch 0.7.1原生探针 | 44.1/48/96kHz非零数字桥接与4组生命周期通过；旧Itour全零记录缺少宿主权限前提，不能证明不兼容 |
| 暂停、静音、无活动、设备变化、44.1/48k及格式不支持 | 逻辑测试、Linux原生脚本与JSONL、Mac部分探针 | Linux数字路径/共享逻辑覆盖；macOS增加BlackHole三档采集、暂停恢复、系统Mute与停源重启证据；休眠/运行中服务重启和其他平台专用设备矩阵仍待补 |
| 无数据/静音区分、有界队列、无历史补播 | stats/capture/queue tests，真实暂停/系统Mute记录 | 已实现；0帧回调不当作收到数据；32块容量与100ms过期策略独立 |
| 枚举、稳定ID、回调输出、格式/周期、位置与故障 | NativeBackend、平台策略、原生时钟扩展、Linux移除/空列表、Mac输出 | 代码与部分原生证据具备；实体Windows/Linux设备尚缺 |
| 任意合理块大小、非48k转换、声道映射 | 1～8192帧逻辑测试、Mac变周期、Linux44.1/48k与Mono左右反相探针 | 本机/虚拟机所测范围通过；不承诺所有设备接受任意period |
| 输出重开重置时钟映射 | OutputClock、全局epoch分配、原始时钟与并发一致性测试 | 逻辑已验证；macOS耳机实体拔插、同ID新进程重开及同进程3次重开均通过，epoch为1/2/3且每次流相对位置从0开始；Windows/Linux实体对照待补 |
| 完整时钟观测可供后续映射 | OutputPosition、position_channel、RunningOutput.latest_position、CLI clock对象 | 已实现：原始帧坐标和原生时钟锚点一致读取，预测播放时间另列；读取不阻塞回调 |
| 真音频/设备生命周期在对应专用机器验收 | ACCEPTANCE.md矩阵 | 尚未完成；虚拟机、类型检查、短测均不替代此项 |

## 当前结论

基础框架和E00/E01用户态工程实现已具备可审查代码与复现入口，且运行实验已修复多个真实后端问题。但不能宣布完整目标或三端E01验收完成。

macOS已新增300秒数字稳定性基线和UI进程终止隔离证据，均通过。运行中音频服务重启已通过故障报告、同UID重新枚举和非零桥接恢复验证。剩余验收包括休眠恢复，以及计划要求的干净环境/原生CI运行；300秒测试不替代更长时间稳定性覆盖。首次管理员授权超时的记录保留；用户就绪后的复测已成功。若继续承诺macOS 14.6最低版本，还需对应系统验证。本机耳机实体输出与BlackHole虚拟采集已形成非零音频证据；BlackHole只作测试设备，不是NeonMix E06自研驱动或分发承诺。

## 权限前提纠正

BlackHole首次全零时，原生权限查询和TCC日志确认宿主应用拒绝麦克风访问；授权后同一二进制三档桥接成功。此前Itour失败不能在未核对授权的情况下归因于驱动。新增macOS采集权限预检查会在权限不足时明确失败。详细结果与原始证据见[BlackHole验收](BLACKHOLE-TEST.md)。
