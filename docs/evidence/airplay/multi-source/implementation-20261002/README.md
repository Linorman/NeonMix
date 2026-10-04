# macOS 多 AirPlay 功能实现与验证

日期：2026-10-02–03。范围：07/08 方案的功能开发，macOS 开发 Alpha。所有命令经 `tools/dev` 执行，依赖、缓存、临时资料、制品和证据都在项目内；测试使用独立房间和 BlackHole，不迁移或重启正在使用的手动房间。

## 已实现

- 1–4 个稳定接收入口，各自 worker、端口、IPC、密钥和运行目录；占用确认后撤公告，释放后重新发布。发现等待独立于媒体调度；旧 goodbye 重发按服务/hostname 隔离。
- AirPlay 与原生共用四路活动/预留配额、动态 lane；暂停保留名额，超时、退出、输出时钟变化按目标清理。原生媒体准备、持久化与线程回收移出 Engine 锁。
- 控制 v2 / PCM v1，进程、连接、请求、信任代际、session/epoch 定向；grant 确认前仅有界待播。全 Hub 最多四个首次配对挑战、每入口一个，Hub 维护尝试次数；入口重试不重置窗口。
- profile v2、每来源 trim/mute/播放模式、跨入口撤销和显式重新配对；Solo 只随当前 session。新身份私有 pending 日志支持强制退出恢复，旧身份与最新撤销可离线迁移/导出。
- `/v2/airplay`、有界 command ID 去重、revision 与精确会话控制、只读成员快照；旧 v1 多路返回 `upgrade_required`。入口运行中增减、独立启停/配对、入口改名、来源别名、多行 Mixer 和按实际 stream 映射的电平已接入后台及桌面。

## 验证证据

| 检查 | 结果与边界 |
|---|---|
| macOS workspace | 常规测试、严格 Clippy、格式和 release 构建通过；显式硬件/长时 ignored 项未计入 |
| 最终制品冒烟 | [最终 2+2](../../../../../artifacts/airplay/multi-source/4c7f74de3696/result.json) 一次通过，4 次故障/恢复的存活来源欠载与迟到增量为零；全局关闭后所有入口停止且发现状态 hidden、预留与 worker 为零。Hub/worker hash 与最终制品索引一致 |
| 核心与存储 | Hub 51、identity 27、IPC 13、Ingress 7、桌面 16 项相关测试；另有实际波形四 lane 与实时无分配回归。identity 包含 create/add 各三阶段强制退出恢复 |
| 混合矩阵 | [4+0](../../../../../artifacts/airplay/multi-source/438d16ba1948/result.json)、[3+1](../../../../../artifacts/airplay/multi-source/7622a2339fe8/result.json)、[2+2](../../../../../artifacts/airplay/multi-source/9501037e8b4c/result.json)、[1+3](../../../../../artifacts/airplay/multi-source/415b32cb523b/result.json)、[0+4](../../../../../artifacts/airplay/multi-source/fce38f9a52ea/result.json) 短程加密数字源/原生 tone 通过，同一 Hub/BlackHole 输出；含占用隐藏超过 11 秒、容量拒绝、双入口竞争、旧 session 拒绝、逐路退出/故障及身份恢复 |
| 恢复压力 | [3+1 五轮](../../../../../artifacts/airplay/multi-source/c7f76540e0e3/result.json)：30 次断开/SIGKILL及同身份恢复；已记录的稳态、故障前、故障和恢复区间，存活 lane 欠载/迟到增量与全局 timed late 增量为零。未录音，不代替模拟波形或长测 |
| 管理与模式 | [跨入口管理](../../../../../artifacts/airplay/multi-source/04e0510bfe10/result.json)：运行中加减空闲入口、v1 升级拒绝、跨入口旧配对拒绝、普通允许不能恢复撤销、新 PIN 原 worker 重新配对；A synchronized 与持续 B low_latency 的 session/epoch/mapping 隔离通过 |
| 实际调音 | [Mixer 控制](../../../../../artifacts/airplay/multi-source/931d93f37e38/result.json)：−6dB 实测 RMS 约 0.500 倍，Mute/单 Solo 被排除路为零，多 Solo 与清除 Solo 正确；重连保留 trim/mute、清除 Solo，其他来源上下文保持 |
| 单路兼容 | [控制/权限/坏凭证](../legacy-regression-20261002/final-hub.json)、[PCM 1+1](../legacy-regression-20261002/final-mixer-pcm-prewarm.json)、[ALAC 暂停及 SETUP](../legacy-regression-20261002/final-mixer-alac-pause-setup.json)、[ALAC 换曲间隙](../legacy-regression-20261002/final-mixer-alac-track-gap.json) 通过，保留原时序断言，timed late 为零 |
| 桌面 | [原生截图与检查](../../../multi-airplay-ui-20261002/expanded-verification.json)：600/1600 宽，空/单/四来源、长名、输入错误和草稿保留；另保留 720/1100 的 2+2 预览。合成 fixture 不代替真实用户操作 |

各报告绑定各自 Hub/worker 哈希。矩阵、压力和增量回归分别取证；不把修改前后的时长相加，也不将旧候选结果冒充最终制品的全项验收。最终源码、制品及检查索引见 `verification.json`。

## 发现的问题与处理

1. 早期 3+1 一次 worker 恢复出现 [`media_closed`](../../../../../artifacts/airplay/multi-source/35c35636b42a/result.json)。其他来源保持；同制品重跑及后续 30 次压力均未复现。已增加终止原因、控制溢出和媒体帧超时的独立诊断，但根因仍未定位，保留异常记录。
2. 单路回归发现 `format=null`：SETUP 在 grant 前发出的格式事件被正确的会话隔离拒绝。worker 改为安装有效 grant 后重发协商格式；首次与重复 SETUP 的格式/epoch、PCM/ALAC 回归已通过，未放宽旧事件拒绝。
3. 窄窗长错误文本曾使底栏挤掉正文；改为固定单行状态栏，完整信息由 hover/可访问名称保留，600 宽截图及测试通过。
4. 原生配对完成的历史持锁落盘已迁至独立任务，邀请事务保护并发重试/取消；HTTP 取消不遗弃已提交身份。profile 信任锁忙时媒体准入及时拒绝，已授予 context 的确认有界延后，PCM 和预留计时继续。
5. PIN 去重缓存删除字段后的探针整对象比较预期已修正；缓存不保存旧 PIN，重放不会泄露过期窗口。

## 未验收范围

真实 iPhone/iPad/Mac 多设备并发、Apple 选择器实际隐藏和当前路由控制、物理端到端/音画 P95、每组合 10 分钟正式矩阵、四路 8 小时/24 小时资源长测、Windows/Linux 原生运行与发布分发均未验收。原来的 iPhone 暂停/换曲实机待确认项仍保留。MAP-G0/G2 的真实设备门槛没有因数字源通过而关闭；本次不宣称正式四路 Apple 支持或可发布。

## 使用入口

在新版 Hub 设置选择入口数量并开启接收，来源分别选择不同入口；在设备页和 Mixer 按来源操作。已有 v1 资料保留一个入口和原身份，只有显式配置多路才改变配额。普通 GET 不创建或迁移资料。构建、离线迁移与回滚导出命令见 [AirPlay 使用说明](../../../../AIRPLAY.md)。当前手动房间继续使用旧制品，本次未替换正在播放的用户会话。
