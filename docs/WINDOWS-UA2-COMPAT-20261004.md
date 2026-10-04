# Windows UA2 输出兼容修复（2026-10-04）

已在 `administrator@192.168.100.186` 复现并修复 UA2 作为房间扬声器无法正常共享的问题。UA2 系统 mix 为 384kHz/f32/stereo；旧版直接用它创建应用流，因项目只接受 44.1/48/96kHz，在打开 WASAPI 输出前失败。现有后台仍显示 `hub_started`，但 `output=null`，所以房间虽然监听 7443，却没有实际输出。

修复只修改产品兼容代码，不要求更改 UA2 的系统格式。所有新源码、依赖缓存入口、临时资料、target、运行文件和日志均位于 `E:\Desktop\NeonMix-test\ua2-20261004-1` 或同测试根内既有依赖缓存。原 `D:\NeonMix` 安装二进制未覆盖，系统默认音频路由、设备格式、驱动和权限策略未修改。按用户最后指示，验证时的临时运行副本和测试脚本已清理，不交付修复脚本或运行副本。

## 原生复现

旧安装对所选 UA2 不指定 rate 播放退出 1：`unsupported audio format: sample rate 384000; supported: 44100, 48000, 96000`。同一旧程序明确指定 `--rate 48000` 后退出 0、实际 client stream 48kHz、period 480 帧、输出错误和 callback over budget 为零。这证明可使用现有 WASAPI shared-mode 转换，不需要修改系统设备格式。

通过原后台 NamedPipe 只读状态查询确认同一个 UA2 的 Hub 输出为空。为验证同一 `HubStart` 流程，通过应用自己的 `HubStop` 请求停止这个无输出、占用 7443 的旧房间，未直接删除原安装、配置或配对资料。[旧运行复现](../artifacts/windows-ua2-fix-20261004/remote-evidence/runtime-before.json)、[原后台状态](../artifacts/windows-ua2-fix-20261004/remote-evidence/status-before.json)。

## 产品改动

[audio-io](../crates/audio-io/src/lib.rs) 在 WASAPI render 未显式指定采样率、且默认格式不受支持时，从设备的同声道、同采样类型支持配置选择 48kHz client，其次 96/44.1kHz。既有支持范围内的默认格式、显式 rate 请求和 period 约束保持原规则。capture/loopback 不启用此回退，没有扩大核心音频格式范围或假定采集侧也能转换 384kHz。

[后台错误分类](../crates/desktop-service/src/daemon.rs) 对 `unsupported audio format` 返回明确的采样率/声道/缓冲设置提示，保留脱敏，不再把此类命令错误统一显示为“后台操作失败”。Hub 本身已有的“输出不可用时保留服务、随后重试”行为未改变；该改动不能被描述为修复了所有 Hub 初始输出错误的诊断。

三组协商回归覆盖384k默认→48k client、capture不回退、已有默认保持、显式率严格、无匹配格式拒绝、声道/采样类型和period约束；另有一项后台格式错误脱敏回归。

## Windows 新执行验证

| 检查 | 结果 |
|---|---|
| 两个相关 crate 的 library tests | 8 passed / 0 failed |
| 两个相关 crate 的 Clippy `--all-targets -- -D warnings` | 退出0；vendor CPAL 的既有 `frames_to_duration` dead-code warning 保留 |
| audio / Hub / background / desktop release | 新独立 target 构建通过 |
| UA2 默认输出，不指定 rate | 退出0，实际48kHz/f32/stereo，输出错误与回调超预算0 |
| 后台真实 `TestTone` | 通过，48kHz/480帧 |
| 后台真实 `HubStart` | 通过，`output` 非空，48kHz/480帧，同原Hub身份/证书/设备ID |
| 原生来源20秒接收 | 发送退出0；10个活动样本输出非零且帧数推进 |
| `HubStop → HubStart` | 再次成功打开 UA2 |
| 系统 mix 前后 | 均384kHz |

20秒来源的10个活动样本中，输出 errors/callback over budget、Mixer欠载、receiver lost/overflow PLC/PCM sink drop/timing gap/late/queue drop 均为零。[构建与测试](../artifacts/windows-ua2-fix-20261004/remote-evidence/build-checks.json)、[运行验证](../artifacts/windows-ua2-fix-20261004/remote-evidence/runtime-after.json)、[连续活动样本](../artifacts/windows-ua2-fix-20261004/remote-evidence/native-samples.json)。

测试通过真实后台 IPC 验证与“开始共享”按钮相同的请求及实际 WASAPI 输出，不计作 console Session1 的鼠标/键盘 GUI 验收或模拟听感验证。macOS 同步通过相关 library tests、Clippy 与格式检查；没有把旧全平台成绩计入本轮。

## 证据与清理

原失败保留。测试脚本曾遇到 PowerShell 参数展开、正常编译stderr被当作异常、命令行长度、路径转义和默认编码问题，均在测试侧修正，没有为通过而放宽产品/质量断言。Windows 构建及测试退出码按实际子进程记录。最初 PowerShell 读取的房间名称元数据存在编码乱码；真实产品和 IPC 使用 UTF-8，原资料按字节复制，Hub身份、证书和输出ID验证未依赖该乱码字段。

本轮源码清单411文件最终逐文件一致。[清单](../artifacts/windows-ua2-fix-20261004/source-manifest.json)、[远端末检](../artifacts/windows-ua2-fix-20261004/remote-evidence/source-verification-final.json)。原安装EXE哈希前后匹配；测试来源及凭证、测试进程和临时运行副本均已清理，原配置/身份/配对文件没有删除。[清理记录](../artifacts/windows-ua2-fix-20261004/remote-evidence/cleanup.json)。仅保留源代码兼容改动、构建结果和非秘密测试证据；没有另行安装或替换用户现有程序。
