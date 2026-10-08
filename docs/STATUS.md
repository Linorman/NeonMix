# NeonMix 实现状态

## Windows 页面闪烁与 AirPlay 开启失败实机修复（2026-10-09）

在指定 Windows 机器上复现 AirPlay `identity_permission`：普通密钥路径正常，后台实际传入的 `\\?\` 扩展路径被 MinGW 父目录解析误判。worker 现明确处理盘符/UNC 根，保留逐级目录与同句柄私有权限校验。真实后台→Hub→worker 的 1/2/4 入口各三轮启停全部 ready/published，关闭后无运行密钥遗留。页面导航的整页 60% 淡入和 6px 位移已移除，原代码文字 alpha 154 的回归复现后，新代码保持 255 和固定位置。

两端 Desktop 各 90 项、Windows 原生 release/严格 Clippy、格式/i18n/UI 审计通过；Windows 27 项身份检查和 29 组 worker 启动通过。修复安装包已生成并校验，保存在指定测试目录，未覆盖原安装。真实 Apple 播放、安装升级和长测不在本次结论内。详见 [证据与交付](evidence/windows-ui-airplay-fix-20261008/README.md)。

## 周期刷新导致控件闪烁的追加修复（2026-10-08）

用户反馈 v0.2 中控件仍反复启用/禁用。旧代码已通过两项注入回归复现：正常请求超过 4 秒使所有页面共用控制变灰；一次短暂快照读取失败立即清空共享可用状态。另修复辅助诊断错误清除成功控制快照的问题。轮询改为最后读取权威控制快照，在途只延长输入接收、保持实际混音派发的 4 秒新鲜度门槛；短暂失败保留原显示，失败后的重试不能反复重新启用，权限失败和停止立即禁用。旧 runtime 的辅助数据不混入新快照。

另在旧代码复现 Mixer/800ms：AirPlay 电平期限 750ms 同时触发按钮禁用和整行淡出。现把电平未知与控制权限/静音状态分开，读数仍显示未知，不使按钮或整行随采样期限闪烁。

macOS Desktop 89 项、严格 Clippy、746 条双语与 UI AST 检查通过；新增回归检查全页控件、排队后恢复派发、持续失败、权限丢失、停止及跨 runtime 数据。Windows 原生验证与此次安装包更新记录在本轮构建工件中；这些确定性回归不代替用户机器上相同触发条件的最终可见桌面复测。

## 共享启动与界面布局修复（2026-10-08）

Windows AirPlay worker 启动补齐 `CREATE_NO_WINDOW`，消除控制台程序默认创建窗口的路径；后台和 Hub 自身的标志不会由子进程继承。桌面普通 worker 忙时保留一个冻结参数的后续操作，队列满明确反馈；共享启动时 Hub 开关、现场和快速操作可立即停止，并取消尚未发送的启动。修复长房间名挤出顶栏操作、英文身份框撑宽侧栏、底栏还原按钮裁切与长说明越界，保留完整 tooltip/AX。

Desktop 84 项、Hub AirPlay 14 项测试通过，严格 Clippy、格式、746 条双语资源与 UI AST 检查通过。Windows Hub/Desktop 全 targets 交叉类型检查通过；macOS release 的中英文原生预览覆盖 600×440 / 1100×760，布局自动回归含 200% 缩放。此次未执行 Windows 实际桌面/焦点复测，也未重打 Windows 安装包，不能将代码修复和交叉检查等同于该平台闪烁已实机验收。开发日志和截图位于 `artifacts/ui-fix-20261008/`。

## UI 多语言开发交付（2026-10-07）

已完成默认Auto、简体中文和English；首期七页/AirPlay、动态语义反馈/事件、tooltip/AX、原生菜单、偏好兼容与英文回退，724条消息/20双语模块严格检查通过。当前desktop/service/i18n联合113测试、严格Clippy与格式通过；macOS固定release54次实际语言/菜单/焦点/隐藏恢复回归通过，Windows固定源原生功能测试/构建通过，Linux仅离线交叉检查。

完整macOS开发DMG的嵌入资源、签名/依赖、非ASCII搬移与离线检查通过；仅ad-hoc，未notarized、未安装。包与源码快照有独立哈希，后续共享操作意图改动及最新工作区验证不冒称为同一包。真实IME、各端读屏、LinuxGUI、Windows实际菜单和中文用户目录升级、真实音频中20次切换、完整三端安装/升级/发布仍未验收。见 [实施结果](UI-I18N-IMPLEMENTATION-20261007.md)、[checklist](UI-I18N-DEVELOPMENT-PLAN-20261007.md)与[汇总证据](evidence/ui-i18n-20261007/summary.json)。

## 稳定性计划实施（2026-10-08，开发中）

P07软件范围已完成：Native/AP版本域与真实epoch重启、幂等回执、满队列终态gate/FIFO/lane重绑、desired/applied callback进度和Desktop行内/底栏反馈通过。最新DSP目标在queue满时仍记录，callback确认前不显示已应用；两秒stalled、未知/跨runtime读数、旧字段被覆盖和房间切换均正确处理。实际Native Stop/未知Revoke、AP三种终态与另一来源保持、retirement lease及实时零分配回归通过；BlackHole实际callback desired=applied=3，733条双语与68项Desktop检查及600×440/1100预览通过。三端完整媒体、真实Apple/读屏/长期和最终包验收仍由P05/P06/P08/P10/P11分项。

P07 AirPlay命令v3已分开持久cfg与事件条件，旧v2保留原CAS；profile/journal版本支持旧资料导入，同一未知候选同步不重复递增。业务state明确确认配置与即时限制，遥测不推进业务游标，未版本化discovery变化会在读取/新CAS前对账；确认框保留原条件。28项API、Desktop66、identity32及真实HTTPS的初始化兼容/重放/重启身份保留/旧epoch拒绝通过。完整门禁随后绑定；P07 FIFO/应用进度已在上述最终软件阶段完成。

P07 Native控制v2已分开config_revision/event_sequence，旧revision保持原事件CAS，v2条件混用/缺失明确拒绝；Desktop/Sender接入绑定命令，订阅强绑定原epoch。8项新域回归、原有集成回归及严格Clippy通过；固定release在BlackHole上完成真实Hub进程GET→重启→TLS/WSS、旧Start拒绝和实际watch两次重取/订阅，无旧会话。AirPlay版本域已由新阶段补齐，desired/applied/FIFO联合验收已由上述最终软件阶段补齐。

P07 pending期间输出/会话健康已即时发布；Prepared只保留配置候选及原对象基线，commit合并到最新runtime，abort不还原旧健康。真实Snapshot事件副本、保存barrier/GET handler、会话结束时保留trim/mute而不复活旧流/Solo、事件历史淘汰及u64提交序号保留回归通过。该早期阶段Hub77项、control21项及严格Clippy通过；Native版本域/WSS重启已由上述新阶段补齐，AirPlay版本域已补齐，desired/applied/FIFO联合验收已由上述最终软件阶段补齐。

P06 Native HTTP和配对已按publication处理未知结果，删除字节相同即成功；原候选/session/device/预备SDK冻结重试，durable前不授予，published revoke立即拒绝认证/关闭gate且DSP队列满不阻止。受Admin/epoch约束的原事务恢复及完成回执查询接入，Hub76项回归通过。AirPlay配置与worker信任发布未知已保留原candidate/request，确认前不授予，published revoke/disable独立关闭gate；配置部分准备复用UUID，Admin/epoch恢复不重启停止中的owner。三端原生/掉电语义、资料迁移与最终制品联合验收仍待完成；R02和P06整体尚未关闭。

P06 UUID管理条件已贯穿输出绑定Store、五个CLI管理动作、IPC/Desktop、Local Forget、保存回执native sync与Sender启动同步。替换后复用revision仍拒绝A的旧UUID；native effect全程持管理锁。Store、CLI parser/实际dispatch和真实IPC保存/重建/名称同步交错通过；绑定资料schema保持，缺UUID旧客户端明确拒绝。真实平台setter/权限和最终包仍需分项验证；R02整体未关闭。

P06 已完成发布阶段原语与双文件设置日志：Prepared保旧、CommitDecided向前恢复，43个真实写入进程中断/重启恢复点、身份/ACL/偏好保持及重试不增revision通过。Hub产品配置读取及后台启动/HubSettings真实IPC均接入恢复，错误固定为结果待确认/需要恢复，日志排除在工件外。HTTP/AirPlay授予与限制边界及B13 UUID CAS已分别完成本地代码回归；资料迁移及三端原生持久边界仍未完成，P06主项保留未勾选。证据见本轮开发记录。

P05 已接入独立 lifecycle endpoint、Stop operation、冻结实例/停止代次和 Ready；20 秒远端变更、普通连接配额满、客户端断开、迟到 Start/Ready 和 UI 未确认结果回归通过。macOS/Linux 新增同目录 guardian，在后台被强杀后仍持有媒体组，5 秒升级并在未 reap leader 的 PID 保留下确认全树退出；Windows 延续现有 Job。macOS 真后台/合成媒体树验证约 5.07 秒退出、假运行密钥清理、另一实例与持久资料保留；满日志消费者回收也通过。Linux 为交叉类型检查，非原生运行；真实产品媒体、三端、可见 UI 与完整包仍分项，不关闭 P05 主门槛。见 [P05 证据](evidence/review-stability-20261007/README.md)。

P04 已完成阶段取消和 session 协议 gain：精确 generation/connection/request 关联、未 grant 取消、独立 cleanup、取消记录/迟到 grant 隔离及 Hub 关闭确认期限。Worker 五阶段 barrier、Hub 实际资源 barrier、六项签名发送端/加密 UDP/PCM IPC、既有七项 SETUP 边界、PCM/ALAC/AAC decoder seam 均通过；两路 macOS 数字恢复完成四次故障/恢复，另一来源连续性通过。新 owner 初始 gain=1；合法重复 stream SETUP 与 FLUSH/恢复保留音量。能力 `session_control_version=1` 在 startup/ready 匹配，该批 PCM v1 不变，后续 P03 已升级为显式协商的 v2/120-byte。详见 [P04 证据](evidence/review-stability-20261007/README.md)。其他平台和真实 Apple 未验收，P03/P07 FIFO 授权隔离仍未闭环。

按 [开发计划](REVIEW-IMPROVEMENT-DESIGN-PLAN-20261007.md) 开始实施：P00 已冻结源码/未提交差异并建立 B01–B20/R01–R03 记录；P01 已修复共享 Mixer 空态键盘、完整 RPATH 解析/错误处理、独立 Mach-O 验证及原生 HTTP Replay。P02 已接入 Desktop 字段调度、完整 context/target 绑定、确认后字段 Undo、Unknown 原请求对账和 AirPlay patch/全量兼容；身份/runtime 在 IPC 与两种 API 上校验，冻结凭证元数据防止路径替换误写。9 项新增交错、原生 IPC 转发及现有状态回归通过；本地门禁 243 项 Rust 测试通过、4 项按原条件忽略，7 项打包回归、严格 Clippy 和 release 构建通过。600×440 八页 macOS 预览已归档，真实 IME/VoiceOver 与跨平台另验。P03 已由控制面明确 timed/native 模式并分离 DSP/计量，修复旧 timed 队列头造成新原生流持续无声、持续断流漏计，以及中途输出重开未先应用配置。真实组件 release 回归及实时零分配通过；一秒断流精确计 48000 内部帧，有效静音 PCM 仍计 Running。

P03 现已完成 generation/独立授权原子、Ingress 显式源坐标、分段预取及 FIR 隔离；真实组件波形/变分块/拒包与零分配通过，PCM/ALAC/AAC SRC 及 RTP 回绕通过。完整 worker/门禁结果见证据。Hub 全撤销事务接线、生命周期、存储恢复、事件游标、诊断 freshness、本地边界和发布联合验收仍在后续批次。新增 PR/main 轻量门禁定义，未执行 GitHub PR/required checks 验证。macOS 的组件结果不关闭 Windows/Ubuntu、真实 Apple、实体声卡和最终包门槛；当前桌面 i18n 工作区的扩大验收结果另记。记录和已完成的 checklist 见 [本轮证据](evidence/review-stability-20261007/README.md)。

## Windows AirPlay 起播调查（2026-10-07）

已用原配对资料私有副本复现 iOS27/Spotify“只响一声”：配对/ALAC格式正确，208接收块中184块过期，最终pcm_output_queue；声卡输出errors0。Windows MediaPipe的PIPE_NOWAIT+5ms轮询实际将写端限制到约64包/秒，低于所需125包/秒。改用单个overlapped写与完成事件，保留250ms预算及队列上限；600包原生对照从9.35秒降至21.4ms，吞吐/超时/停止/对端关闭回归通过；同身份iOS27/Spotify实听用户确认正常，76个active样本约41.1秒、4712块接收，late和worker/output错误0，正常TEARDOWN。起播85块malformed计数及停播前后75943 Mixer欠载保留，不算全程零异常。另修复缺省spf、整数缩窄、UDP异常包三项缺陷，两端七项边界及协议回归通过。新身份PIN-start日志不代表原配对失败。原安装未覆盖；完整修复、潜在问题与证据见 [调查报告](WINDOWS-AIRPLAY-INVESTIGATION-20261007.md)。

## 桌面 UI 重构「Neon Console」（2026-10-04，仅 macOS 预览）

按 [设计方案](../09_NeonMix_UI重构_设计方案.md) 与 [开发计划](../10_NeonMix_UI重构_开发计划.md) 重构桌面界面，突出“多源汇入一个房间”：新增默认首页「现场」，以信号汇流图显示来源 → 房间核心 → 实体输出，线型对应会话/静音/Solo/断线状态，光点亮度对应该路真实 RMS；Mixer 宽窗改为竖向通道条控制台并加最近 60 秒电平历史；Hub 增加房间徽记、共享开关与一次性波纹、邀请倒计时环、共享输入容量槽；Sender 以五段发送链路管线兼做步骤并加发现雷达；设备改为卡片网格；诊断磁贴加 2 分钟趋势。视觉改为 v3 深色霓虹 token，来源类型色（原生青、AirPlay 紫）与交互色、状态色分开。动效原则改为“只有真实信号变化时才动”：连续动效上限 20 fps，无信号、隐藏或开启减少动态效果时不连续重绘。desktop 27 项测试、严格 Clippy、格式与截图构建通过；4 路流动时 CPU 5.9%（1100×760）/6.4%（1600×1000）单核，空闲页与基线相当。导航改为七页（⌘1–⌘6），Mixer 控制台布局下方向键为 ←/→ 选通道、↑/↓ 调音量。仅预览模式与单元测试验证；真实多设备运行、VoiceOver、真实输入法与 Windows/Linux 未验收。见 [证据](evidence/ui-rebuild-20261004/README.md)。

## Windows UA2 输出兼容修复（2026-10-04）

在 administrator@192.168.100.186 原生复现 UA2 默认384kHz被项目格式限制拒绝，旧Hub虽启动但output=null。WASAPI render现自动协商设备支持的48kHz应用流，保持系统384kHz格式；已有默认、显式rate/period和capture限制保持。相关8项library测试、Clippy与audio/Hub/background/desktop新release通过；实际后台TestTone、HubStart、停止重启及20秒原生接收通过，10活动样本输出/媒体/Mixer质量计数零，原Hub身份/证书/设备ID保持。原D盘安装未覆盖、系统设置未改；按用户要求临时运行副本和脚本已清理，仅保留兼容代码与证据。详见 [UA2兼容修复](WINDOWS-UA2-COMPAT-20261004.md)。

## 安装包后台启动与退出修复（2026-10-04）

Windows 以内置 Administrator 或提升权限运行时，启动器用默认安全属性创建的 `%LOCALAPPDATA%\NeonMix` 属主为 `BUILTIN\Administrators`，后台按“目录必须属于当前用户 SID”拒绝启动，界面因此列不出设备。启动器现以当前用户 SID 为属主、私有 DACL 建目录，并把此前提升运行留下的 Administrators 属主目录改回当前用户；其他属主仍拒绝。Windows x64 节点用安装副本复验：属主修复后后台启动，`devices` 返回 9 个端点，`shutdown` 后后台退出。后台不可达时“退出后台”现直接关闭界面，不再留下隐藏窗口进程。macOS“开始共享”报“后台操作失败”系旧手测 Hub 占用 7443；端口占用现明确提示，`HubStart` 等待 Hub 启动或退出后再返回结果。

## Windows 起播跳帧修复与 macOS 验证（2026-10-04）

针对最新 Windows 四路恢复跳5帧，在 macOS 确定性复现提前缓存首包被小幅回调相位跳变跳过的问题；共享 Mixer 新增等待采样网格，原条件跳帧由5降为0，真实晚到和大时钟跳变仍计跳帧。Mac workspace239项通过、5项忽略，正式成员格式/严格Clippy/release及加速两分钟相位通过；五组合40次恢复/125质量窗零质量增量，四路25,145次查询零错误，PCM/ALAC暂停与同步模式、双原生60秒数字读回通过。3+1/2+2稳态前既有717/794欠载及采集discontinuities1均保留，不能写成全程全计数为零。仅Mac新执行，Windows原生四路、历史worker队列失败与可见GUI未关闭，Ubuntu旧失败保持；旧包未覆盖、临时夹具已清理。原因、精确补丁及证据见 [本轮修复报告](WINDOWS-ONSET-FIXES-20261004.md)。

## macOS / Windows 完整重测（2026-10-04，01:27:57 快照）

追加完整测试使用同一523文件快照，与上一交付轮仅文档不同，产品源码未改。Mac236项常规测试、Windows221项通过，各5项忽略；正式成员格式、严格Clippy、release、worker/身份/配对/codec及单路/后台新执行通过。Mac五组合40次恢复/125窗全部质量增量零，四路27,179次查询零错误；双原生60秒数字读回及正式GUI11流程通过。Windows四路30秒稳态通过，但第2个disconnect/source2恢复窗口跳帧5，仅观察2/24，质量失败未复跑；另四组合52窗/16次恢复通过。新Mac→Windows60秒29活动样本所有质量计数零，正常user_stopped并清理。WindowsSession0仅加载检查，console交互未验收；Ubuntu沿用上一轮未通过，不据Mac新通过消除旧间歇问题。三端旧包保持，两端自有夹具已清理，详情及原始首败见 [完整重测](PLATFORM-FULL-RETEST-20261004-012757.md)。

## Ubuntu 重新测试与三平台运行包（2026-10-04）

固定 522 文件快照在 Ubuntu/Windows 新执行的常规测试分别 226/221 项通过，各5项忽略；Ubuntu正式成员格式、严格Clippy、release、worker单路/暂停/后台和正式GUI通过。Ubuntu多路质量仍失败：四AirPlay稳态跳帧4086、3+1恢复跳帧20、2+2欠载62，四原生唯一诊断复验欠载4320。最终包短测欠载48；Mac→最终Ubuntu包计划60秒，55.369秒媒体会话先消失，Sender报Connection refused，活动期PCM丢弃93、Mixer欠载15360，lost/overflowPLC为零；Hub仍存活，原因未唯一定位。

Ubuntu ARM64、Windows x64和macOS ARM64新可运行包均已留存，包含运行库/插件/启动入口/源码与许可；含空格重定位、依赖模块和全部文件哈希校验完成。Windows集成AirPlay就绪及单路回归通过，完整多路未重跑、Session0可见GUI仍未验收；macOS新release/worker、正式包GUI/后台/Hub/AirPlay和Finder启动通过，没有沿用旧Mac常规测试充数。三端自有进程和秘密夹具已清理，旧包保留，测试未改产品逻辑；Ubuntu可选迁移工具release因磁盘不足未交付。下载、启动、精确范围、失败/包验证与清理见 [完整报告](PLATFORM-DELIVERY-20261004.md)。

## 四路 AirPlay 起始负载门槛复测（2026-10-03）

按用户最新澄清，80%仅作启动前整机CPU门槛，运行中测试自身高占用正常、只记录。本轮起始10个连续样本平均27.266%/峰值36.318%，同Hub/worker二进制、4来源+4读者×5ms、30秒稳态与完整24恢复/73质量窗口通过，29,331次查询零错误；聚合跳帧、存活欠载和ingress late均无增量。整段CPU平均64.753%/峰值77.75%。未修改产品代码或降低负载，原四个用户进程保持、自有夹具已清理。旧四路两次失败仍保留，不把本轮成功当作CPU根因证明或间歇问题已修复。详见[本轮复测](FOUR-AIRPLAY-CPU-RETEST-20261003.md)。

## 串行复测问题修复与 macOS 验证（2026-10-03）

确定性复现并修复原生 jitter 容量与期限混用：正常10ms发包、35ms输出停顿时旧配置产生2次overflow PLC；改为40ms期限与独立32包认证在途上限后同条件零丢包、PCM连续。恢复期探针补充质量硬断言，旧Ubuntu四个恢复失败窗口/1188帧准确被拒绝；新增逐lane跳帧、Hub调度和夹具发包节奏诊断。Mac workspace236项通过、5项忽略，格式/严格Clippy/release通过；3+1/2+2/1+3/0+4、PCM/ALAC暂停及同步模式、双原生60秒数字读回通过。四AirPlay原强度首测稳态等待超时；唯一诊断复验稳态零增量，但第三worker恢复出现media_frame_timeout，仅完成6/24恢复，其他三路和输出时钟保持。四路仍未通过，旧间歇断流和Windows decoder队列根因仍未完全定位；Ubuntu/Windows未重测，不宣称三端修复完成。原始失败、具体修复、哈希与清理见[本轮修复报告](PLATFORM-SERIAL-FIXES-20261003.md)。

## Ubuntu / macOS 新轮严格串行复测（2026-10-03，14:30 快照）

依用户要求重新执行同一519文件快照，Ubuntu完整清理后才开始Mac本机检查，时序审计确认没有重叠。新执行222/232项常规测试分别通过，各5项忽略；正式成员格式、严格Clippy、release、worker及单路/后台通过。两端五组合稳态均通过；Mac40次恢复/125观察窗质量增量零，Ubuntu28次恢复存活通道保持但恢复期聚合跳帧123/226/51/788。Mac→Ubuntu60秒仍400次overflow PLC、12,960帧Mixer欠载，质量失败。两端正式窄窗内容600×440已核实。此前同源四路失败尚未解释，本轮没有产品代码修正，不将新通过当作已消除间歇问题。Windows仅引用旧轮记录，未计入新轮重新测试。新日志、SHA256、原始计数审计、截图及清理见 [严格串行重测报告](PLATFORM-SERIAL-RETEST-20261003-1430.md)。

## 最新复测问题修复与 macOS 验证（2026-10-03）

确认并修复低延迟AirPlay没有扣除设备输出积压的问题：以Ubuntu报告的106.6ms积压/1024帧回调在macOS确定性重现3秒跳过19,092帧，修复后跳帧、欠载和稳态波形缺口归零。定时通道缺PCM时漏记欠载也已修正，多路探针新增稳态质量硬断言。另细分worker队列失败阶段，修复跨平台快捷键提示、窄窗快速操作裁切及macOS测试程序GStreamer rpath。

macOS workspace 232项通过、5项忽略；Clippy/格式/release/worker构建通过。五种四路组合共40次恢复、125个观察窗无跳帧/存活通道欠载/ingress late增量；4路+4读者完成28,909次查询、零错误。PCM/ALAC暂停及重复SETUP、同步播放通过；原生双Sender60秒数字读回288万帧、无静音、丢包补偿、PCM丢弃或Mixer欠载。正常/最小窗口的真实快捷键与截图验证通过。Ubuntu原生跨机overflow PLC和Windows偶发decoder队列首败仍未定位，本轮未重测两端，不宣称已修复全部三端质量问题。原因、前后证据、命令和清理见[修复报告](PLATFORM-RETEST-FIXES-20261003.md)。

## 更新后同快照三端复测（2026-10-03，02:23）

共同 516 文件快照在 macOS/Ubuntu/Windows 原生测试分别 229/219/214 项通过、各 5 项忽略，release、严格 Clippy 与成员格式均通过。Ubuntu PKCS#8 v2 身份启动和 Windows PEM/Applink、私有 NamedPipe 阻断已原生消失。用户取消强制 CPU 上限后，Mac 原强度四 AirPlay+四读者完整完成 24 次恢复、32,171 次成功查询；总 CPU 平均 66.17%、峰值 89.52%，五组合最终共 40 次恢复，观察窗无定时丢帧/欠载增量。Ubuntu 多路功能和 36 次恢复通过，但四路30秒稳态 timed late 增 999,658，单路 PCM/ALAC 定时断言失败；Mac→Ubuntu 60秒仍398 overflow PLC、1 PCM丢弃、16,320欠载，CPU未满。Windows五组合最终28次恢复通过，3+1 decoder_queue 首败及LAN2PLC/480欠载首败未定位、复验通过；GUI仍Session0未验收。Mac正式AX交互和Ubuntu正式Ctrl导航/后台通过。历史CPU保护中止、最新无强制门槛复验、逐平台哈希与清理均保留，见 [新实验报告](PLATFORM-RETEST-20261003-022352.md)。本轮未改生产源码或旧手测包。

## 平台问题修复与 macOS 复验（2026-10-03）

已修复 AirPlay SETUP 把短暂 profile/Engine 锁竞争当拒绝、旧 OpenSSL 对 ring PKCS#8 v2 身份读取失败后的 NULL 崩溃、Windows worker 缺少 NamedPipe/跨 CRT PEM 输入，以及原生大块 PCM 交接溢出放大。平台 unsafe lint 与 IPC 单独 Windows feature 也已修正。

macOS workspace 228 项通过、5 项忽略；最后诊断分类改动后 Hub 54 项通过，严格 Clippy、格式、workspace release 与 worker 构建通过。最终制品五种四路数字组合通过，共40次定向故障/恢复；四 AirPlay 含4个并发状态读者、31,819次成功查询，原版相同竞争场景 SETUP403。调音偏好重连、撤销/重新配对、PCM/ALAC重复SETUP及暂停恢复通过；17项身份启动与OpenSSL3.0.9兼容验证通过；独立后台12场景及脱敏复验通过。

这些结果只推进 macOS 数字路径。Windows/Linux 未原生重测；Ubuntu jitter 驱逐与原始 `media_transport` 仍未完全定位，不能把队列/诊断修复当作三端质量验收。原因、制品哈希、旧版失败及复验见 [修复报告](PLATFORM-FIXES-20261003.md)。

## 三端共同快照复测（2026-10-03）

本地 macOS、Ubuntu 192.168.100.112 与 Windows 192.168.100.186 按共同 512 文件快照独立原生构建并最终校验无源码差异；常规测试分别 225/215/210 项通过、各 5 项忽略，release 均通过。macOS 严格 Clippy 通过，Linux/Windows 在既有 unsafe 安全注释处失败。新版本 Mac→Windows 60 秒采样丢包/PCM 丢弃/欠载/输出错误均零；Mac→Ubuntu 接通但活动样本最大 lost 445、PCM drop 6、累计欠载 53,760。macOS 单路及 2 AirPlay+2 原生通过，4 AirPlay 两次恢复失败、Mixer 重连 SETUP403；Ubuntu 新 v2 worker 复现 SIGSEGV，GDB 指向 Ed25519 公钥读取空指针链，具体 PEM 导入失败原因未定位；Windows 新 v2 worker 仍有 NamedPipe/TCP 不匹配与 OpenSSL Applink 两项启动阻断。macOS 正式 GUI 主要交互通过，Windows 可见 GUI 仍受 Session 0 限制；首败、复验、背景异常与清理均保留。完整版本、制品哈希与证据见 [三端实验报告](PLATFORM-TEST-20261003.md)。本轮没有修改产品源码或覆盖旧手测包。

## 多 AirPlay 功能 Alpha（2026-10-02–03，仅 macOS 验证）

已实现 1–4 个独立接收入口、四路 AirPlay/原生共享容量、动态 lane、每来源 Mixer/播放偏好、占用撤公告与释放恢复、定向控制 v2、profile v2 和断点恢复、旧身份迁移/回滚导出，以及后台/桌面多来源管理。原生启动与配对持久化在 Engine 锁外执行；成员读取不创建资料，旧 v1 多路调用返回升级要求。当前手动房间与实际用户资料未切换。

macOS workspace 常规测试、严格 Clippy、release 构建和原生 UI 预览通过；五种四路数字混合组合、30 次故障恢复、跨入口撤销/新 PIN 重新配对、不同播放模式、实际增益/Mute/多 Solo 及单路 PCM/ALAC 回归有独立证据。曾发现 grant 前格式事件丢失，已修复并回归。早期 3+1 一次恢复 `media_closed` 未复现，根因仍未定位，失败记录保留。

这是功能开发 Alpha，不是四台 Apple 实机或发布验收。真实 Apple 并发与列表隐藏、物理音画指标、每组合正式长时矩阵、8/24 小时长测及 Windows/Linux 运行均未完成；原单路实机待确认项继续保留。制品哈希、分阶段证据和范围见 [实现与验证记录](evidence/airplay/multi-source/implementation-20261002/README.md)。


## Ubuntu 新快照复测与两端手测包（2026-10-02）

Ubuntu 新 511 文件快照 workspace release 与 182 项串行测试通过（5 项忽略），修正了一项测试断言失败时持锁析构导致的挂起，生产行为未变。Ubuntu ARM64 和上一轮已验证的 Windows x64 制品均已打包，包含运行库、插件、启动入口及源码/许可记录；含空格路径、干净开发环境下的重定位启动与后台/Hub 输出验证通过，包已留在各自机器并回传本地。Ubuntu 包 60 秒 Mac 跨机仍 network_degraded（末次 lost 500、PCM sink dropped 7、Mixer underrun 54,240），新 AirPlay 启用返回 503/后台权限错误，未达到 worker 启动，不能沿用旧 SIGSEGV 结论。Windows 包保留既有 NamedPipe/TCP 阻断与 Session 0 GUI 限制。下载、启动路径、SHA256、完整结果和清理记录见 [复测与运行包报告](PLATFORM-RETEST-PACKAGES-20261002.md)。

## Ubuntu / Windows 原生与 macOS 协同测试（2026-10-02）

按两台指定机器的同一 503 文件源码快照测试，并只同步本轮明确修正。Ubuntu ARM64 workspace release、170 项测试、后台/凭证与独立 AirPlay 协议/解码通过；Hub 集成 worker 仍 SIGSEGV，原生 Mac→Ubuntu 两次 30 秒发送有丢包和 Mixer 欠载。Windows 原生分组测试及桌面/Hub/worker 构建通过，修正 WASAPI 固定 256 帧周期后 Mac→Windows 输出接通；10 秒检查点 playing、非零电平且丢包/欠载/输出错误为零。Windows AirPlay worker 拒绝 Hub 的 NamedPipe 地址，接收未就绪；Session 0 未完成可见 GUI 交互。两端均不能计作 AirPlay 接收或完整桌面验收通过。修正、源码边界、详细证据及清理见 [平台测试报告](PLATFORM-TEST-20261002.md)；不覆盖期间其他并行功能修改。

## AirPlay 自动换曲无声修复（2026-10-02，macOS，实机待确认）

复现了曲间空档保持 RTP/PTS 映射、不发送 FLUSH 时，定时 Mixer 游标继续前进而 ingress PCM 位置只累计解码帧，下一曲被持续跳过的问题。定时 lane 耗尽后按下一块 PTS 重新获取播放位置并淡入，保留未来等待与过期采样丢弃。旧版本回归失败，修复后五次加密 ALAC 空档均恢复非零输出，无媒体 epoch 重建及定时跳帧；每次正确丢弃一个过期解码残留块。61 项相关常规测试、额外两分钟加速音质测试、严格 Clippy、格式检查和 Hub release 构建通过；同步模式、暂停恢复、96 包预缓冲回归通过。当前手动实例已替换 Hub 并就绪，身份与配对记录保持一致。真实 iPhone/Spotify 自动换曲仍待实听确认，见 [修复与证据](evidence/airplay/track-transition-20261002/README.md)。

## 本地凭证迁移完成（2026-10-02，macOS）

当前 AirPlay 房间与默认桌面资料已迁移并切换为文件凭证，共 5 个秘密条目；两个 Hub 的身份、证书、管理员及引用保持，HTTPS 认证通过。当前房间保留 2 台来源和 low_latency，Hub/worker/后台/桌面运行、接收就绪、输出帧持续推进；默认 `.local/desktop` 已是 v2，后台待机不自动发送/启动 Hub。旧 AirPlay 目录、桌面备份 `.local/desktop-native-20261002` 与原钥匙串条目保留。

修正实际来源公钥是 Base64 编码而非十六进制的共享校验错误，并精确排除手动 bin/GStreamer registry；18 项 identity、16 项迁移测试、Clippy 与 release 通过。历史测试房间原库条目缺失，保留不重建；实际 Apple 来源恢复播放及 Windows/Linux 尚未验证。先前原库阻塞由系统授权后完成读取，未采用身份重置。位置、认证证据与回退限制见 [本地迁移记录](evidence/credential-storage/local-migration-20261002/README.md)。

## 文件凭证实施（2026-10-02，仅 macOS 验证）

正式 Hub/Sender v2、receiver v1 和桌面后台使用 profile 相邻 `.credentials` 明文 JSON、私有权限与稳定 OS 锁；typed 元数据、旧格式迁移提示、相对 state_path、管理员保护、pending/forget 恢复和 AirPlay 密钥保持已接入。默认产品移除 keyring；独立 `apps/credential-migrate` 支持 offline copy、身份/撤销/绑定保持、可终止原库 helper、私有 staging 与排他发布。新增 ADR-014、文件/依赖探针和归档排除规则。

macOS 完整 workspace fmt/Clippy/tests/release，以及最后针对性 57 项测试通过；迁移 18 项普通测试、Clippy/fmt/release 通过。纯文件 9 类、E05 16 类、AirPlay Hub 6 类、E07 后台 12 类探针通过，自有 fixture 清理完成。修复并发 spawn 继承锁描述符产生的短暂 busy，10 轮并发回归通过。Windows/Linux、跨账户 owner、真实 native fixture 迁移、完整 GUI/Mixer 和实际 Apple 来源升级回归未执行；现有资料与手动会话未切换，没有读取真实凭证库。历史 Keychain/Windows 记录保持原样，属于旧方案证据。见 [本轮交付与边界](evidence/credential-storage/implementation-20261002/README.md) 和 [升级说明](CREDENTIAL-STORAGE.md)。


## iPhone 暂停恢复候选修订（2026-10-02，仅 macOS，实机待确认）

审阅固定 UxPlay 1.73.7/NTP UDP 音频方案与上游 OS 27 记录，数字源复现了恢复后沿用旧低延迟提前量导致持续迟到，以及重复 SETUP 返回零音频端口。候选实现按 RTP/PTS 锚点断点申请新媒体 epoch，重复 SETUP 重建 reader/端口和解码器；保留来源身份与控制连接，不按无媒体超时断开，也未放宽认证或声明 buffered/PTP 支持。

53 项相关测试、严格 Clippy、worker 原有协议/配对拒绝检查、PCM/ALAC/AAC 解码、最终数字 ALAC 的短暂停、连续五次重新 SETUP、FLUSH、音画同步和 96 包预缓冲通过；无迟到包及定时跳帧。此为合成源经真实 macOS Hub/worker/CoreAudio 的证据，尚未确认用户 iPhone 上的实际症状消失。当前实例候选启动仍停在 `SecKeychainFindGenericPassword` 读取原有身份，三个钥匙串条目均存在，等待系统访问完成；未据查询超时重建身份。[调查、候选方案和证据](evidence/airplay/pause-resume-20261002/README.md)。


## AirPlay 延迟与房间设备（2026-10-02，仅 macOS）

按用户选择默认低延迟音乐播放：每个 epoch 在 Hub ingress 固定提前来源 PTS，首包保留 120ms 接收后余量；维持连续 sinc SRC，flush/重连重建映射。可选音画同步保持来源时间，模式持久化；切换后立即关闭的保存竞态已修复。设备页合并当前/最近 AirPlay 来源、搜索与计数，活动来源在首屏；沿用管理员和独立 revision，不授予房间 principal。补齐当前 Hub Unix IPC 与 worker 的 macOS 接缝。

相关 49 项常规测试、核心 39 项音频/时间/质量测试、严格 Clippy（含 screenshot）、release 构建、worker 协议/PCM-ALAC-AAC 解码回归与两种窗口截图通过。真实 CoreAudio 上的合成加密 ALAC 对照：音画同步待播约 1,938ms；低延迟 30 秒稳定段约 118ms、96 包预缓冲约 136ms，无迟到包及定时跳帧，1+1 混音和故障隔离通过。软件估计不代表手机端到端或原生 AirPlay 对照验收；未运行 Windows/Linux。已按用户要求替换当前手动测试实例的 Hub/worker/UI/后台副本，保留原房间身份与 1 台来源的配对记录；低延迟接收就绪、GUI/Hub 运行和输出帧持续推进已核对，见 [替换证据](evidence/airplay/latency-devices-20261002/replacement.json)。

## E07 桌面界面重新设计 2.0（2026-10-02，仅 macOS）

桌面 UI 重新设计为以房间为中心的控制台：每页一个主视图（Hub 设置是可原地编辑的房间与共享开关；Sender 用一句话说明声音去向和下一步；Mixer 以总控为首、每路一行；诊断以四个健康磁贴概括），未完成的设置用步骤条引导，细节放进按数据决定默认展开的折叠面板。新增侧栏线框图标与 ⌘1–⌘5、跨页顶部栏（房间状态、总输出电平、总静音）、⌘K 快速操作（按当前身份权限生成）、混音「还原」（⌘Z，按新 revision 发送反向操作）、通道键盘控制（↑↓/←→/M/S）、相对拖动推子（单击不跳音量、双击回 0 dB、聚焦或 Option 才响应滚轮、AccessKit slider）、成员头像概览、设备状态筛选与未保存更改条。AirPlay 职责拆分为 Hub 设置（接收、配对码、播放方式）、Mixer（作为一路调音）与设备管理（作为成员），保留另一会话加入的播放方式、配对窗口剩余时间与时序诊断。代码拆为 `shell.rs`、`palette.rs`、`icons.rs` 与 `pages/` 六个模块；见 [DESIGN.md](../DESIGN.md)。

新增测试发现并修复两处实际缺陷：推子在值变化时于 `data_mut` 内读取 `input`，嵌套两把上下文锁导致界面卡死；输入法组字时回车落到搜索框使其失焦，提交的候选字丢失。另修复样式按钮在 `horizontal_wrapped` 中不换行导致窄窗溢出。desktop crate 格式、`-D warnings` Clippy（含/不含 screenshot feature）与 12 项测试通过（新增推子单击不跳变与双击归零、组字回车不执行、还原按新 revision 发送）；五页 × 600/1100/1600 宽、首次启动、快速操作、还原、确认框与按钮进度截图见 [证据](evidence/e07-ui-redesign-20261001/)。原生 AX 探针依赖的控件名称保留，但本会话仍无辅助功能授权且 7443 被 AirPlay 手动测试占用，探针未运行。

## E07 桌面界面重构（2026-10-01，仅 macOS）

桌面 UI 改为左侧固定导航 + 页头 + 单一正文滚动区 + 单行状态条，Hub/Sender/诊断宽窗两列、Mixer 通道自动分列。原布局在 600×440 下正文只剩约 160px，Mixer 电平位于首屏之外；现在两种窗口尺寸下总控电平、推子与通道都在首屏。停止发送、隐藏窗口、退出后台常驻侧栏。新增半粗 CJK 标题字面、带峰值保持的分区电平表、0 dB 刻痕推子、Mute/Solo 锁定控件和仅在状态变化时运行的动效；见 [DESIGN.md](../DESIGN.md)。

修复的界面缺陷：Mixer 队列估计用 stream id 当数组下标导致恒为「未取得」，现按 `lane_stream_ids` 映射；增益预设在每个推子上重复出现两行；增益提交后滑块弹回旧值，现保持草稿至权威快照确认或命令失败；危险按钮悬停时被覆盖色块遮住文字；卡片悬停高亮按「指针在卡片下方任意位置」误判；空状态与状态点的呼吸动画让窗口持续 60fps 重绘；换页标题首帧透明；确认框默认焦点落到危险按钮而非「取消」；未就绪时「创建一次性邀请」可点但静默无效；诊断与绑定信息显示带引号的原始 JSON；macOS 上 PingFang 固定路径不存在（实际回退冬青黑体），且原路径取第 0 号字面为港版字形。另外 AirPlay 卡在接收时显示该通道电平；采集峰值是会话最大值，Sender 页改为数值显示而非实时电平。

验证：desktop crate 格式、`-D warnings` Clippy（含/不含 screenshot feature）与 7 项测试通过；五页 × 1100×760/600×440 的录制状态、派生「接收/发送中/Mute/Solo/断开/撤销」状态与首次启动空状态均实际渲染截图核对，证据与派生数据见 [e07-ui-redesign-20261001](evidence/e07-ui-redesign-20261001/)，可用 `tools/ui_preview_shots.sh` 复现。**原生 AX 端到端探针本轮未能运行**：当前会话的 osascript 未获辅助功能授权（-25211），且端口 7443 被另一 AirPlay 手动测试实例占用；按钮、字段与下拉框的可访问名称保持原值，待在已授权终端重跑 `tools/e07_native_ui_probe.py` 与 `tools/macos_sender_ui_probe.py`。Windows/Linux 未运行。

**第二轮（2026-10-02）交互闪烁与尺寸**：交互时整页闪烁的根因是正文外层在每次操作往返期间整体禁用，egui 会把整页按禁用透明度重绘；另外写入后 `fresh` 清空让控件变灰到下一次轮询，页头「状态刷新中」胶囊闪现并推动身份控件，轮询失败会清空诊断让电平和卡片消失再出现，换页淡入首帧全透明，状态条每次点击闪两次。现在写入期间控件保持可用（视觉门槛改用最近权威快照时间，逻辑门槛仍是新鲜 revision），期间的新输入只保留最后一次意图并在下一份快照后按新 revision 发送；轮询失败保留上次读数；换页从 55% 不透明度淡入；状态条不再闪底色。新增回归测试覆盖「写入中控件不禁用、第二次写入排队后按新 revision 发送」，旧实现下该测试失败。

尺寸与层级：按钮、文本框、下拉框统一 32px，紧凑按钮 28px 只用于卡片标题行；页头身份控件降为 28px/13px；按钮文字不再在按钮内折行（原「撤销当前 AirPlay 配对」被折成两行）；卡片标题行顶端对齐，胶囊不再下沉；正文与页头最大宽度 1180px；Mixer 列数不超过通道数。新增总音量 20px 主读数、通道/总控/发送卡状态条、分段增益预设、Mute/Solo 颜色过渡、按钮内进度环、复制确认、邀请有效期细条；诊断页 AirPlay 通道按名称显示，采集峰值（会话最大值）改为数值。desktop crate 格式、`-D warnings` Clippy（含/不含 screenshot feature）与 9 项测试通过，五页 × 600/1100/1600 宽及忙碌、确认框、空状态截图见 [证据](evidence/e07-ui-redesign-20261001/)。原生 AX 探针仍因本会话无辅助功能授权、7443 端口被另一 AirPlay 手动测试占用而未运行。

## Windows x64 GitHub Actions 验证（2026-10-01）

当前工作区快照已在 GitHub Actions 原生 Windows x64/MSVC 上通过 workspace 格式、Clippy、131 项自动测试与 release 构建（5 项默认忽略，其中平台凭证测试另行显式通过）。Credential Manager、NamedPipe、健康空设备枚举、GStreamer 必需组件、双路/丢包重放/错误身份拒绝及 44.1/48/96 kHz 软件模拟通过；五秒媒体探针 Mixer 欠载为 0。WaveRT 驱动使用 VS 2022 x64 MSBuild 和项目内锁定 WDK，通过编译、链接、INF/Inf2Cat；SYS/INF/CAT/PDB 已归档，下载后哈希和 NX/ASLR/CFG 核对通过。修复托盘调用 macOS 专属 API 的 Windows 编译阻断，以及 WDK 工具/打包配置；回滚测试改为只测失败提交后的清理，保留五秒阈值，并补齐失败后继续的测试覆盖。

**GUI 未通过**：托管 runner 不满足 egui 的 OpenGL 2.0 要求。音频设备枚举为 0，实体声卡/虚拟输出长测、内核驱动加载和真实 GUI 交互未验证；本轮不能解释或替代下文 Windows ARM64 虚拟机的持续采集 Xrun 验收。完整版本、结果与证据见 [Windows Actions 报告](WINDOWS-ACTIONS-20261001.md)。


## AirPlay Speaker 专项实现（2026-10-01）

2026-10-02 配对修复：用户确认当前 iPad 已显示 PIN 并成功配对，接收/释放 24,777 块、接入拒绝为零。测试脚本重复启动曾删除接收身份；现 start/stop/restart 保留 Hub UUID、平台接收密钥和配对记录，仅显式 clean 清理。当前 iPad fixture 真实 stop/start 的身份及配对文件一致；五项接收签名/公钥绑定、持久重连与失配/撤销数字验证通过。iPhone 输入窗与当前音质仍待对应实测，详见 [配对调查](AIRPLAY-PAIRING-INVESTIGATION.md)。

**追加实听：iPhone 16 Pro / iOS 27 / Spotify 已由用户确认实际可听。初版有爆豆声与明显损耗，已定位定时游标硬跳/重复采样并改为连续高质量 SRC。数字质量与修订 release 1+1/故障隔离通过，尚待同源复听与视频伴音验收。**

已增加固定 UxPlay 1.73.7 的独立纯音频 worker、固定 LE 媒体/有界控制 IPC、4 秒/2 MiB 定时 ingress、按声卡呈现锚点的 Mixer 通道、Hub 单来源及 1+1 配额、PIN/公钥准入/撤销、独立后台与桌面接收控制。worker 不注册发现、不打开声卡、不编译 mirror/HLS/video receiver；Hub 在音频就绪后统一广播。密钥经平台 vault，运行私钥文件和开发产物均在项目内。

macOS 常规 workspace 最新 **141 项测试通过**；另新增 idle 输出 epoch 变化保留 PIN 窗口、active epoch 撤销回归。接收协议正确/错误 PIN、真实 20-byte SRP proof、签名与加密 RTP/NTP/PCM 数字链路、解析边界、视频请求拒绝及 ASAN 通过；Hub/CoreAudio 的启停、权限、旧 revision 和 worker SIGKILL 不拖停原生 Sender 功能通过。桌面原生窗口预览和 release 构建通过，已有桌面未用组件等警告保留。

真实 Mac 27.0 / 26A5416b 系统音频选择器已发现目标，并有正确 PIN、签名、FairPlay 和音频 SETUP/RECORD 的通过记录。首轮实际 PCM 全拒定位为 C JSON 库截断大 u64 session ID，现限定控制 ID 到 `1..2^53-1` 并拒绝越界。后续源端 PIN 输入窗自动化不稳定，加密数字源已通过修复后的实际 Mixer/CoreAudio 定时播放、播放中加入原生流的 1+1 混音和 worker 故障隔离；统计改为非阻塞快照发布，避免原生媒体构建持锁拖住 AirPlay，队列没有增大。**真实 Mac 源端的最终播放、视频伴音与同步目标尚未通过**，没有宣布专项完成。自定义型号的 ANNOUNCE 失败与 `AppleTV3,2` 纯音频兼容 profile 的握手推进分列历史证据；型号不开放视频能力。

入口、字段、源码/许可与实际边界见 [AirPlay 使用与验证](AIRPLAY.md)、[行为合约](AIRPLAY-CONTRACT.md)、[IPC 合约](AIRPLAY-IPC-CONTRACT.md)、[ADR-013](adr/ADR-013-airplay-audio-worker.md)及 [AirPlay 证据](evidence/airplay/)。本轮接收端只在 macOS 测试；Windows/Linux worker、iPhone 重连、iPad 完整播放/音质与视频伴音、真实音画差、长测与正式分发仍待对应阶段，单平台原型不代表三端完成。

## macOS E00–E07 问题修复（2026-10-01）

上轮发现页返回值、最小窗口导航、输入框辅助功能名称和严格 Clippy 问题已修复并本机复验。后台发现统一返回完成快照；公共字段/密码框/滑块关联标签，导航、状态、反馈与动作分行。当前 workspace 格式、Clippy `-D warnings`、140 项测试和无警告 release 构建通过；9 组原生 GUI、8 组 Sender/设备管理及五页两种窗口尺寸通过。测试工具按可访问名称和原生键盘操作验证，修正屏幕外 AX 查询及同名 GUI 进程定位。未运行的新制品长测、完整 VoiceOver/真实 IME、跨机/休眠/热插拔和分发边界保持原口径。详见 [修复记录](E00-E07-MACOS-FIXES-20261001.md)与[复验证据](evidence/macos-e00-e07-fixes-20261001/)。


## Windows ARM64 底层补验（2026-10-01）

已通过 SSH 连接 `192.168.100.113` 的 Windows 11 ARM64 / Parallels 节点。项目内固定 Rust 1.95.0 与 LLVM-MinGW 构建、执行真实 ARM64 PE；这不替代 x64 MSVC 或内核驱动验收。最新底层快照 **61 项原生测试、Clippy 与音频/后台 release 构建通过**，Windows NamedPipe 的启动、双 owner 拒绝、关闭重开及 macOS 6 项后台生命周期回归通过。首次私有目录默认归 Administrators 导致启动失败，现对新目录和锁文件原子设置当前用户 SID；已有异用户目录仍拒绝。另修正持锁测试读取方式，保持 Windows 强制锁语义。

WASAPI 首包/恢复包现按每次 Start 跟踪，不再通过设备位置为零推断；后续 Xrun 检测保留。Windows 回调启用并持有 MMCSS Pro Audio 注册，退出恢复。重启前 25 项短时场景通过：44.1/48/96 kHz 输出到 48 kHz mix 的非零回读、时钟换算、暂停恢复及不支持周期的明确拒绝。**五分钟持续采集仍遇原生 Xrun，稳定性未通过**，没有增大网络队列或屏蔽故障。Windows Credential Manager 在已登录桌面会话中读写/删除通过，SSH 登录上下文返回 1312。

节点自动更新重启后扬声器 ID 改变，未登录时 loopback 全零。用户登录后非零回读恢复，但持续采集仍约 12 秒出现 Xrun。原生直读两分钟通过；加入项目采集桥接的对照记录到设备位置缺口 960 帧（20 ms），标志 `0x1`。增加默认采集环形缓冲、指定宿主内置扬声器以及关闭自适应 hypervisor 均未解决；实验代码和环境设置全部恢复。尚不能称 Windows 底层全部正常，需要进一步原生采集/参考硬件对照。依用户最新要求不验收界面；GStreamer/Hub 媒体与 x64 MSVC/驱动加载仍未在该节点通过。源码哈希、制品架构、失败和通过记录见 [底层补验摘要](evidence/windows-arm64/summary.json) 与 [原生测试](evidence/windows-arm64/latest-backend-tests.log)。

## E07 桌面产品与独立后台（2026-10-01）

已实现 egui/eframe 五页产品：Hub 设置、发现/配对与 Sender 输出绑定、真实电平 Mixer、设备授权管理、分段诊断与严格白名单脱敏导出。正式配置复用 E04/E05/E06 的稳定身份、平台 vault、revision 权限与输出绑定；陌生/过期状态不允许写入，管理员重新允许不自动发送。

独立 `neonmix-background` 管理音频进程，关闭窗口/Cmd+W 仅隐藏，托盘恢复；Cmd+Q/退出后台须确认并结束本实例音频。macOS 实际 GUI SIGKILL 后 Hub PID 与输出帧继续，重开读取权威状态；9 组真实窗口流程和 12 组双路（真实 BlackHole 采集 + 独立 Sender）功能通过。普通刷新不锁表单；急停可中断只读查询，快速开始→停止按顺序最终停止。原生 AX 暴露的无效焦点崩溃已修复并加入回归；首次测试音失败及其未确认根因保留历史证据。

macOS **110 项 workspace 测试**、格式、workspace Clippy `-D warnings` 与 release 构建通过。新增 Mixer 50 ms Peak/RMS 与 limiter 原子遥测保持实时零分配。所有开发缓存/产物位于项目，测试自建凭证与临时目录已清理。源码与 IPC 包含 Windows/Linux 平台实现；本轮不运行外端。三端实机托盘/驱动/桌面、完整 VoiceOver/真实中文输入法矩阵、LAN 与长测、安装签名分发仍待对应阶段验收，不等于三端 E07/Beta/v1 已全部通过。

使用、口径、证据与验收边界见 [E07](E07.md)、[IPC 合约](DESKTOP-IPC-CONTRACT.md)、[ADR-012](adr/ADR-012-desktop-background.md) 与 [本轮证据](evidence/e07/)。

## Ubuntu ARM64 E02–E04 补验（2026-09-30）

**最新：21:47 修复后五分钟跨机验证与完整控制回归通过，已标记本次验收成功。** 欠载、丢包/PLC、PCM 缺口、队列丢弃、回调超预算及采集过期帧均为零。修复 Linux 实际调度/状态读回、CPAL realtime 与 PipeWire 实时客户端配置；恢复系统 VM 的 1,024 帧默认周期，Hub/重开/Sender 采集不再固定请求低于平台范围的周期。RTP/抖动缓冲/Mixer 水位未增加。Ubuntu 24 项、macOS 28 项针对性测试及格式/Clippy/release 通过；[成功证据](evidence/ubuntu-e02-e04/fixed-300s-summary.json)与[根因/复现条件](UBUNTU-ARM64-E02-E04.md)已归档。以下是修复前历史记录，不代表最新结论。

用户要求的五分钟复测已于 20:56 启动并跑满 300 秒，仍失败：欠载 158,400 帧、一路媒体丢包 1,496 包及 PCM 缺口 51 次，两路交接队列均有丢弃。未修改实现、未并行编译，不标记成功；[复测摘要](evidence/ubuntu-e02-e04/retest-300s-20260930.json)和[清理记录](evidence/ubuntu-e02-e04/retest-300s-cleanup.json)已归档。

用户追加授权的 Ubuntu 节点已完成 core/I/O/control/media/Hub 的 62 项常规测试、3 项 release 漂移注入、格式/Clippy 与 ARM64 release 构建。实际使用 GStreamer 1.24.2，修复其缺少 `dropped` 属性导致的 panic；真实队列序号还用于在编码前丢帧时关闭上下文，避免依赖下游是否保留 `DISCONT`。Mac 多网卡下的 UDP 回复现绑定相应 HTTPS 的实际本地地址，保留 IPv6 scope。

中间修订的 Mac Hub/Ubuntu Sender 短时加密、混音/控制与重启保持有通过证据；最终制品纯 macOS 30 秒控制回归通过，最终 LAN 两方向复验仍有频率偏差。Ubuntu Hub 可认证并输出，但五分钟试验出现 Sender 等待约 143 ms 后停止、采集回调超预算及读回偏差，**Ubuntu 的持续稳定性未通过，不能判 E02–E04 三端全部完成**。独立 60 秒 epoll 也存在约 16 ms 抖动，但未证明 143 ms 的根因。原 macOS 一小时验收仍有效于其原制品，本次修订没有冒用该长期结果。完整结果、所选报告及实验边界见 [Ubuntu E02–E04](UBUNTU-ARM64-E02-E04.md)。

## E05 发现与配对（2026-09-30）

共享 `neonmix-identity` 和 Sender/Hub 已接入 DNS-SD 注册/浏览/解析、同名独立身份、多网卡地址、地址更新与离线事件。首次信任采用单一高熵一次性邀请，证书与稳定 Hub UUID 固定验证；邀请有期限/取消/单次使用和精确重试，配对成员经 E04 持久化事务登记，角色不由客户端指定。

正式 `setup/pair` 的长期令牌与 Hub 私钥使用平台凭证库，JSON 只保存公开身份与引用；macOS Keychain 的实际读写/删除通过。Windows Credential Manager 和 Linux Secret Service 已接入共享库，本轮遵照用户要求没有执行另外两端测试。实验 `init` 明文资料保留为明确标记的兼容入口。

macOS 93 项常规测试、E05/Hub Clippy、格式和 release 构建通过。14 组真实场景验证了自动发现与配对、异钥/异 UUID 拒绝、超时取消与重复请求、媒体/WSS 撤销、地址与 IPv6 恢复及正常/异常离线。临时资料和测试 Keychain 条目已清理。所选证据和完整边界见 [E05](E05.md) 与 [ADR-011](adr/ADR-011-discovery-and-pairing.md)；桌面流程已由 E07 接入；三端实体互通及发现性能统计仍属后续验收。

E05 追加审查已补齐 Windows 私有文件 ACL/长路径/ADS 边界、IPv6 scope 与监听族匹配、原子替换恢复以及正式默认 LAN 入口。macOS 97 项 workspace 测试、最新 30 项针对性测试和 17 组真实场景通过。Windows/Linux 身份与控制核心（包括平台后端）在 macOS 上的交叉类型检查及 Clippy 通过，没有运行外端程序。完整 Hub 媒体 SDK 链接和平台运行仍待对应环境验证。详见 [E05 完整性检查](E05-AUDIT.md)。

## E06 虚拟输出开发中（2026-09-30）

macOS 已加入显式 BlackHole 提供者：`--virtual-output --virtual-output-provider blackhole` 按精确 UID 读取，Ctrl+C/SIGTERM 正常停止会话；设备或时间线故障明确停止，手动重启建立新媒体上下文。真实普通 afplay 系统选路、44.1→48 kHz 转换、双应用混音、设备音量/Mute、无源静音、停止后设备继续可用及改率后的新会话均通过，系统设置已恢复。详情与所选证据见 [E06](E06.md)。

自研 Rust HAL 已补齐标准主音量/Mute 控件、多客户端生命周期、完整可调用接口表及参数验证；纠正上游 I/O 操作码和结构布局，本机 SDK 对照通过，数据与时钟回调经过 1000 周期零分配审计。项目内 arm64 bundle 完整 ad-hoc 签名和 strict 校验通过，已按用户授权安装系统副本并由 Core Audio 隔离宿主实际发布设备；普通应用、数字桥接、跨进程名称、绑定撤销及服务恢复已通过。早期注册失败保留为历史记录。

Linux 已改为独立本地虚拟输出 owner：Sender 断网/停止不删除系统节点，每 UID 独占标记和私有文件锁防止重复，core/proxy 移除及服务错误触发有界退避重建。用户追加授权的 Ubuntu 24.04.3 ARM64 桌面已通过系统输出可见、普通应用转换/双应用混音、原生增益/Mute/停源静音、名称绑定及加密发送、PipeWire/WirePlumber 重启和节点移除后的数字重开。修复异步 export readiness 与会话管理器重启恢复；正常用户 manager 与 NoNewPrivileges 基线、root/跨 UID 拒绝、release/Clippy 和 7 项针对性测试通过。临时进程、节点和状态目录已清理，桌面服务和原默认输出正常。完整范围和证据见 [Ubuntu E06](UBUNTU-ARM64-E06.md)。

Windows 已新增单 render WaveRT 驱动源码，来自锁定的微软简化 SysVAD/MS-PL 样例。去掉采集、测试音和 PCM 落盘，声明软件音量，补 DMA/通知上限及先停定时器再释放的生命周期；十个 C++ 单元在 macOS 以项目内 WDK/SDK 10.0.26100.6584 编译并链接成未签名 x64 PE native 映像，NX/ASLR/CFG 检查通过。这不代表 MSVC、系统加载、音频或 HVCI 已通过。[驱动合约](../drivers/windows/wavert/DRIVER-CONTRACT.md)说明边界。

共享输出绑定已实现：hub_id/output_id、精确设备身份和显示名稳定落盘；revision 并发冲突、本地授权代际、独立线程撤销守卫已接入 Sender。真实 BlackHole 重开/改名/换地址、禁用再启用、删除重建、相同证书不同 Hub UUID 拒绝探针通过。自研 HAL 可写 Name 的离线 ABI 测试通过；第三方 BlackHole 名称不修改。详见 [绑定合约](OUTPUT-BINDING-CONTRACT.md)。

本轮补齐 HAL 并发回调/时钟一致性及异常客户端退出；实际 bundle 的 Apple CFPlugIn 宿主加载通过。Linux 原地名称/绑定恢复、Windows 自有端点标记与管理员名称同步源码已接通，macOS 交叉检查通过。用户已授权并完成系统安装副本，签名、权限与项目文件哈希一致；补齐标准属性并激活后，Core Audio 已真实发布自研设备，系统默认选路/afplay、双应用、音量/Mute、无源与加密发送通过。跨进程标准 Name 写入被拒绝；nmna 自定义 CFString 名称修订的 SDK 宿主与实际跨进程名称/绑定撤销复验通过；实际服务重启、旧 I/O 故障退出及同 UID 数字桥接恢复也通过。实际睡眠/唤醒与剩余发布门槛仍待验收。三端 E06 仍未完成：平台运行/生命周期与发布门槛尚未齐全。细节见 [E06](E06.md)、[ADR-009](adr/ADR-009-virtual-outputs.md) 和 [ADR-010](adr/ADR-010-windows-render-driver.md)。

更新：2026-09-30。**E02/E03/E04 的 macOS 功能回归与完整 30 分钟双路验证通过。** 本轮先抓到两路 Sender 在 1 ms 等待中停留约 123 ms；仅换等待方式仍失败。实际调度读回确认 CLI 的 QoS 成功标记不能证明优先级生效。已改为 socket/appsink 事件唤醒及普通分时优先级 47，并验证线程进入与恢复；没有采用 Mach 硬实时期限策略，也未继续增加 70 ms 目标水位。

BlackHole 读回 86,400,000 帧、静音帧为 0；欠载、丢包/PLC、PCM 缺口、队列丢弃和回调超预算均为 0，Sender 最大额外等待约 1.15 ms。macOS workspace 格式/Clippy、74 项常规测试、控制/11 组故障和真实采集隔离回归通过。详见[稳定性修复与证据](E02-E04-STABILITY-20260930.md)。历史失败仍保留在[故障复核](E02-E04-REVIEW-20260930.md)。完整计划的跨机、模拟端延迟与 8/24 小时矩阵尚未验收；本轮没有执行 Windows/Linux 测试，不等于三端 G2 或 v1 完成。

续轮更新：补齐错误乱序 RTP 的时钟隔离与实际 Opus 10 ms 包时长验证；raw UDP 固定窗口节流经约 4.95 万包真实注入验证，B 路无新增欠载。E04 revision 耗尽不再造成未标版本的修改，慢订阅的错误/Close 发送也有期限。当前 82 项常规测试、3 项 release DSP 注入、12 组故障与 IPv6 功能通过；原制品 8 小时双路测试因 E03 淡入审计受控停止，未记为通过。已修正缓冲静音提前消耗 fade 的问题，低/高增益与总控切换均采用固定 240 帧渐变；修订制品的 84 项常规测试、3 项 DSP 注入、原生控制与 12 组故障复验通过；原测试参数保持不变连续运行超过一小时；用户明确按一小时验收，本轮零欠载/丢包/PLC/PCM 缺口/丢弃通过，现已受控收尾。原 8 小时目标未跑满，不计正式长测通过。逐项证据与未完成条件见[需求核对](E02-E04-REQUIREMENTS.md)。

## E02–E04 新增实现

| 范围 | 当前能力 |
|---|---|
| E02 原生媒体 | gstreamer-rs、Opus 48k stereo/10ms、规范 RTP 时钟、UDP 单播、DTLS 指纹和密钥 ready 绑定、SRTP/SRTCP 重放保护、旧上下文隔离、PLC 与受保护反馈 |
| E02 有界与反馈 | 包长/流数/握手/活性/期限限额，有界非阻塞收发、迟到/丢包、水位反馈、192→64 kbit/s 降级及内容暂停/恢复 |
| E03 时钟与混音 | 预分配 16 槽位、独立 SPSC/FIFO/sinc、长期漂移与水位滤波、比率/变化限幅、断点重置、增益/Mute/多 Solo/总控/limiter 与渐变 |
| E03 原生恢复 | 输出按设备回调取帧，控制线程回收 Mixer，清旧积压、原 UID 重开、输出 epoch 重置；不切系统默认设备 |
| E04 权威控制 | TLS 1.3 HTTPS/WSS，服务端 Principal 权限、快照/增量事件、revision 冲突、幂等、注册/撤销/禁止/重新允许、原子持久化与单 owner 状态锁；客户端快照/事件复制和断线续订 |
| 运行入口 | `neonmix-hub init/serve/send/snapshot/watch/control/diagnostics/runtime/probe`；44.1/48/96k 和 Mono 采集转换，IPv4/IPv6 实际 TLS 地址绑定 |

macOS 原生媒体 SDK 为项目内提取的 GStreamer 1.28.7 official universal archive（API 基线 1.24），完整 SHA-256 与 GLib/Opus/libsrtp/OpenSSL 版本锁在 `native-dependencies.toml`。没有执行系统安装器，E01 CLI 保持不加载媒体库。

## E02–E04 前轮功能与漂移证据

macOS workspace fmt、Clippy `-D warnings`、**63 项常规测试**与 release 通过，另有两项 release 漂移注入通过。真实 native security 测试覆盖 RTP 序号/时间戳回绕、乱序/迟到、坏包/包长、SRTP/SRTCP 重放、新协商密钥隔离和上下文期限；错误证书的 PCM 帧数为 0。丢包注入的最新实际 PLC 样本数见[检查记录](evidence/e02-e04/media-loss-final-20260930.log)，源时间线连续且 Mixer 欠载为 0。

四组 ±100/±500 ppm 各 600 秒的加速时钟注入，通过实际 sinc Mixer 路径，欠载/断点为 0，队列有界；437 Hz 测试音频率与 RMS 在断言范围内。动态比率、Mute、epoch 重置和槽位复用路径测得零分配、零重分配、零释放。

BlackHole 实际数字读回和 HTTPS/WSS 控制通过：单/双路、输入 Mute、多个 Solo、总控静音、权限拒绝、冲突、幂等、管理员断开、重新允许不自动发声、撤销与重启保持。30 秒双路基线的原生输出错误、回调超预算、PCM 丢帧和 Mixer 欠载均为 0。这是本机功能基线，不冒充跨机模拟端延迟或长期硬件验收。

补充 macOS 真实故障探针通过：UDP 丢包反馈驱动实际编码码率/DTX 下降和恢复；慢控制不阻塞真实采集及媒体发包；快照/WSS 断线续订与 Hub 重启后新事件、真实落盘失败原子性、并发版本冲突、A 路故障时 B 路数字输出，以及同 UID 44.1 kHz 重开。历史 2880 帧目标水位的加速模拟在 ±1000 ppm 各 300 秒无欠载，±2000 ppm 显示有界降级；这不代表真实链路长期通过。[完整核对](E02-E04-AUDIT.md)列出证据与剩余项。

IPv6 loopback 的同一整套 HTTPS/WSS、双路 UDP 与 BlackHole 控制验证通过。实际 BlackHole 44.1/48/96 kHz 采集分别转为 48k Opus/SRTP 并由明确指定的 Built-in Speakers 输出；三档认证、非零 PCM、规范 RTP 时钟、原生回调和无错误检查通过。见 [采集证据](evidence/e02-e04/macos-capture-sender.json)。

补充的[最终功能故障探针](evidence/e02-e04/completion-final-20260930.json)通过 11 个场景。B 的 stdout 被显式填满后，容量 2 的诊断队列实际丢弃 47 条统计，媒体/PCM 继续且正常阶段无欠载；解堵后诊断恢复。拥塞恢复使用两秒滤波水位，诊断保留原始水位和实际受保护反馈。输出故障与单路故障分开保存前后基线，A 路断网时只允许 A 路欠载，B 路欠载增量必须为 0。

完整操作见 [E02–E04](E02-E04.md)，字段与时间/队列图见 [合约](MEDIA-CONTROL-CONTRACT.md)，选取的本轮证据在 [evidence/e02-e04](evidence/e02-e04/)。后续边界为 E05 的三端实体/性能验收、E07 三端桌面体验补验、E09 三端分发，以及开发计划中的跨机实体与长期稳定性矩阵。

## E00/E01 已实现与历史基线

| 范围 | 实现 |
|---|---|
| E00 单仓基础 | Cargo workspace、固定Rust 1.95.0/Cargo.lock、独立平台crate、Git仓库、项目内缓存/临时目录/产物入口 |
| E00 应用与自动化 | 独立音频CLI、egui诊断程序、三端CI定义、Python独立锁环境、构建/依赖/源码SHA256清单和符号归档 |
| E01 统一音频 | 固定容量音频块、源位置与epoch/断点、sine/impulse/silence、Mute推进、44.1/48/96kHz转换及Mono映射 |
| E01 原生I/O | WASAPI render endpoint loopback / Core Audio虚拟设备input side / native PipeWire sink monitor，三端实体输出 |
| E01 设备与时钟 | 稳定ID、格式和周期查询、原生回调、设备变化/故障、原始设备帧位置、48k换算、重开/回退重置 |
| E01 实时与诊断 | 预分配SPSC、溢出代际清理/过期丢弃、暂停恢复首块保留、静音和无数据分开、错误码与回调统计 |

CPAL 0.18.2采用仓库内扩展（native I/O revision 3），公开WASAPI IAudioClock、HAL sampleTime、PipeWire ticks，并修复PipeWire节点移除、空设备列表和SPA输入chunk处理。补丁、原版SHA256和Apache-2.0许可证在 `patches/`、`vendor/`；不是声称原版CPAL已经提供所有能力。原始坐标、归零/换算后的设备位置与播放延迟推算值分开记录。

## 已完成验证

- `cargo fmt --all -- --check`、workspace Clippy `-D warnings`、**30项测试**、workspace release build、离线探针全部通过；[检查记录](evidence/checks.json)。
- Windows x64 GNU目标和Linux x64目标的**完整workspace交叉检查**通过，包含音频程序与Rust UI；[Windows记录](evidence/windows-workspace-check.log)、[Linux记录](evidence/linux-workspace-check.log)。这是条件编译/类型检查，不是MSVC原生链接或Linux运行证据。
- Linux新增原生运行证据：Ubuntu 24.04 x86_64／PipeWire 1.0.5／WirePlumber 0.4.17，在项目内QEMU TCG环境通过23项核心/I/O测试和9组数字运行场景，包括44.1/48kHz、Mono下混、系统Mute、暂停恢复、设备移除和健康空列表；[运行证据](evidence/linux-runtime.json)。实际ELF已链接运行；不作为实体声卡验收。
- macOS arm64／本机SDK 27.0：实体输出44.1kHz、127帧与48kHz、511帧真实回调通过，原始sampleTime有效、递增，错误数为0；[44.1k记录](evidence/macos-output-44100.jsonl)、[48k记录](evidence/macos-output-48000.jsonl)。短测不能代替长期稳定性或模拟端延迟验收。
- 新接入External Headphones：44.1/48/96kHz、127/256/257/511/1024帧的输出回调与时钟换算已有实测，静音/恢复的样本计数正常；用户确认左右声道正确且无杂音。实体拔插时原流报设备不可用并退出，同ID重连输出正常；运行中外部改率会使原流明确失败。首次44.1k启动失败已保留，修复后的5次切换均通过。另在同进程连续关闭/重开3次，验证epoch递增、位置归零及旧快照隔离。详见 [耳机测试](HEADPHONE-TEST.md)。
- macOS已有Itour虚拟设备：静音采集、暂停、恢复、无数据统计与epoch切换通过，恢复首块位置为0且带RESUMED标记；[采集记录](evidence/macos-capture.jsonl)。
- macOS egui界面成功启动并查看截图；中文字体、真实设备列表可见。4种状态有headless绘制测试，设计/静态审计无错误；完整辅助功能和另外两端GUI实测待补。
- steady-state采集、队列与sinc输出分配计数为0，未发生重分配或释放；此结论仅覆盖项目测试路径，不扩展为第三方内部或系统硬实时保证。
- Core Audio服务重启恢复通过：原采集/输出均以设备不可用报错退出；同UID重开后非零数字桥接恢复，无错误或回调超时。[恢复记录](evidence/macos-blackhole/service-recovery/result.json)。
- BlackHole新增300秒数字通路基线：48k/256帧、14400256输入帧，错误/回调超时/drop/stale均为0，时钟连续；期间终止诊断UI未中断音频进程。[稳定性记录](evidence/macos-blackhole/soak-300s/result.json)。这是5分钟基线，不是长期稳定性承诺。

## 验收补充与剩余事项

| 项目 | 当前结果与下一步 |
|---|---|
| macOS数字虚拟桥接闭环 | BlackHole 2ch 0.7.1已读回非零测试音，44.1/48/96kHz桥接通过；本轮细项见[BlackHole验收](BLACKHOLE-TEST.md)。此前Itour全零记录缺少授权前提检查，不能据此认定驱动不兼容 |
| Windows / Linux实体音频 | Windows目前只有交叉检查；Linux已通过虚拟机数字路径，但缺少实体声卡/USB和实际输出测量。仍需对应专用机器，虚拟机不替代实体设备验收 |
| macOS USB/服务重启/休眠 | BlackHole已补44.1k输入；耳机接口实体拔插、运行中服务重启故障报告及同UID重开桥接均通过。休眠唤醒仍需受控测试；USB属于额外设备覆盖 |
| 原生CI | 三端workflow已写好，仓库未配置远程地址，尚未运行GitHub Actions；macOS arm64硬门槛和原生报告归档均在配置中 |
| 依赖/驱动成品 | GStreamer属E02集成；Windows驱动/macOS插件属E06，签名安装属E09；未交付伪驱动或关闭系统安全的安装流程 |

首次debug小周期探针曾收到一次原生错误；本轮在关闭虚拟机后立即进行的release短周期测试也报告过一次Xrun。没有足够证据确定根因，失败记录保留。其后空闲条件下的6次静音短测（44.1/48kHz、127/128/257/511/960帧）均通过，不能据此宣称长期稳定。故障事件现带完整统计和时钟快照，便于后续定位。

新增一致的输出时钟快照接口：原始帧位置与同次观测时间戳、采样率、实际回调帧数一起发布，独立于预测播放时间；并发读取不会组合两次回调的数据。零分配测试也覆盖这一路径。

## 本轮修复与实验边界

- BlackHole首次全零的明确阻断是宿主应用的麦克风权限被拒绝；授权后同一二进制三档桥接通过。macOS后端现加入原生权限预检查，权限不足时明确失败，不用全零回调表示有效采集。
- 频率诊断将连续50ms低于门限的样本作为信号分段，避免把停源静音计入两次测试音的频率跨度；新增437Hz→静音→659Hz回归测试。

- Core Audio采样率通知现在读取实际值，只有与流配置不匹配或查询失败时才报告失效；监听注册后再次核对，覆盖配置到监听之间的变化。首次44.1k失败的精确时序无法从旧日志恢复，因此不把后续短测成功当作根因已经完全证实。Windows/Linux已有运行证据属于revision 2；revision 3只修改macOS监听与CLI版本标识。

- 32块采集队列可装下完整8192帧回调，仍独立执行100ms过期清理；不会因单个合法大回调的尾块使整批数据失效。
- 流主动析构不再计作设备丢失；实际移除所选PipeWire节点仍会失败，且不会切换到其他设备。
- PipeWire枚举排除应用流；健康空列表与服务失败分开。SPA chunk检查范围/对齐、支持环绕及EMPTY/CORRUPTED标记，流失效后不再调用数据回调。
- epoch统一分配，重开不会复用暂停/断点重置用过的编号；耗尽时明确停止，不能回绕。
- 桥接探针检查测试音频率、RMS、原始设备时钟和回调预算，不能仅凭非零数据判通过；Mono测试分别检查左声道的1/2幅度和反相抵消。
- 并发重型编译时，TCG曾发生超过周期的回调，造成一轮44100Hz探针失败。失败日志保留在artifacts/linux-vm，空闲条件下重测通过。新增callback_over_budget；它与原生Xrun分开，不能把errors=0等同于无欠载。

## 使用与维护

启动与测试命令见 [README](../README.md)，完整验收步骤见 [ACCEPTANCE](ACCEPTANCE.md)，音频字段/容量/错误码见 [AUDIO-CONTRACT](AUDIO-CONTRACT.md)。

`.local/`保存可复用下载与工具缓存，`target/`保存编译及中间产物，`artifacts/`保存运行日志、UI截图、二进制与符号包，全部位于项目内并被Git忽略；已选取的长期证据进入`docs/evidence/`。逐项完成审计见 [E00-E01-AUDIT](E00-E01-AUDIT.md)。当前没有创建提交或远程仓库。Linux实验环境与复现入口见 [LINUX-LAB](LINUX-LAB.md)。

## Ubuntu ARM64 节点补验（2026-09-30）

用户追加的Ubuntu 24.04.3 ARM64/Parallels节点已完成E00/E01原生验证：release CLI与GUI构建、30项自动测试、格式/Clippy/离线模拟、9组PipeWire场景均通过；仿真HDA输出和Wayland GUI启动通过。测试绑定到固定工作区快照，不覆盖期间继续修改的E02–E04内容，也不等同于物理声卡或Linux x86_64验收。详情见[Ubuntu ARM64 E00/E01验证](UBUNTU-ARM64-E00-E01.md)。


## Windows AirPlay 可靠性补充（2026-10-07）

已实施 Unicode 同句柄密钥读取、后台→Hub/Sender→worker 协作式停止、Windows Job 音频树兜底，以及每用户安装的独立提升防火墙 helper 和按安装实例维护入口。原身份与配对格式保持；旧组件能力不足时明确拒绝。macOS/Windows 身份、50 次 owner 恢复、Windows Job 异常回收及规则双实例维护通过；macOS 1–4 路活动停止、相同资料/端口重开及 EOF 清理通过，正常约 0.67–0.97s、forced=false、无运行 PEM 遗留。Windows 完整未签名候选包及包内身份/7 项加密 PCM 回归已构建验证。Windows 双窗口维护测试通过：目标实例正常退出，其他目录实例持续运行。

Public 防火墙开启、非 ASCII Windows 账户、真实 iPhone 30 分钟、跨用户 UAC、策略/双栈与活动升级卸载仍待联合验收，不记作发布放行；额外 mix 质量回归中的 media_frame_timeout / timed_late_frames 失败保留，不能合并为 pass。工作区含其他并行改动，构建以源码/组件哈希快照为准。详见 [可靠性改进](WINDOWS-AIRPLAY-RELIABILITY-IMPLEMENTATION-20261007.md)。

## 稳定性P08诊断状态（2026-10-08）

独立控制/诊断/AirPlay/采集时钟、必要字段和可用性校验、近期增量与累计错误、有效静音/无新采样、共享Mixer Starved、Sender认证目标锁存已接入。顶栏/总控不再绘制过期缓存；电平通过binding/stream/output代次和当前session校验，未观察或旧代为未知。源码/测试/原生preview与四份release哈希见[本轮证据](evidence/review-stability-20261007/README.md)。软件门禁、release实时零分配及macOS600×440中文/宽窗英文通过；真实IME/VoiceOver、Windows/UbuntuGUI及三端媒体/长测/最终包仍未验收，不作为发布放行。

## 稳定性P09与本地候选（2026-10-08）

桌面Sender显式UntilStopped；CLI期限/终止原因、实际peer+接口的采集前防反馈、Linux UID owner版本/后台代次/binding UUID握手已完成。macOS真实TLS限时/停止及loopback/LAN/IPv6同端点拒绝，Linux真实节点/双后台/正常退出隔离通过；Linux后台场景远端Sender是夹具，未测其媒体质量。最终macOS标准18项、workspace 433项及release实时回归通过。macOS中文/空格候选包独立Mach-O、重定位payload启动和包内双路媒体通过，ad-hoc DMG保存在项目artifacts，未安装或发布。用户已明确暂缓真实Apple、Windows/Ubuntu可见桌面与8/24小时长测，相关checklist继续未勾选；CI和软件组合的最终结果见[实施证据](evidence/review-stability-20261007/README.md)。

三端基础CI已在[草稿PR #1](https://github.com/Linorman/NeonMix/pull/1)实际通过，main required checks按macos-15/ubuntu-24.04/windows-2025稳定名称设置；Windows补齐大计量夹具堆缓冲和生命周期连接有界收尾，25次NamedPipe代次重开通过。最终候选重新构建并通过重定位payload/包内媒体，制品与源码绑定见本轮证据。Actions归档配额已满，optional归档失败单独保留，检查日志已取回项目；实机/GUI/长测/安装升级条件依旧未放行。
