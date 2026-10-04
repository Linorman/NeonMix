# 原生 PIN 输入窗调查

2026-10-02：用户确认 iPad 已弹出输入窗并成功配对；同一接收器记录 ALAC 44.1 kHz、24,777 个接收/释放块、接入拒绝为零。iPhone 无输入窗问题仍待复测，不能把 iPad 的结果计为 iPhone 已恢复。

## 配对记录与测试实例生命周期

历史 `manual-desktop` 与当前 `manual-stable` fixture 使用不同 Hub UUID、不同接收密钥引用，各自保存一个不同的客户端公钥。旧脚本 `start` 先执行 `stop`，而 `stop` 删除平台 vault 条目及整个 fixture，随后重新 setup；重复运行和升级复测因此可能使源端保留的身份/配对材料与接收端脱节。历史目录虽然还留有客户端公钥，接收私钥的 Keychain 查询返回条目不存在，不能通过复制客户端列表恢复原来的接收身份。

已修改 `tools/airplay_manual_session.py`：重复 `start` 复用实例，省略名称时保留原名；`stop` 只停止自有进程并保留身份；再次 `start` 启动已有 Hub profile；只有显式 `clean` 删除 vault 条目和目录。`restart` 保留身份与配对，不再默认改名或替换已选输出。真实当前 fixture 的 stop/start 保持 Hub UUID、server profile、receiver 文件和 iPad 配对记录不变，重新开启有效 PIN 窗口；见 [生命周期证据](evidence/airplay/manual-session-persistence-macos.json)。

新增 [配对持久性探针](../apps/airplay-worker/pairing_probe.py)，同时校验 SRP 回应里的接收公钥与 pair-verify 的接收端签名，避免测试发送端只验证自己签名、漏检接收身份变化。五项 macOS 数字场景通过：同一身份/持久客户端公钥的 worker 重启后免 PIN 重连、缺失客户端记录时拒绝旧凭证并允许正确 PIN 修复、撤销优先于已知列表、换接收私钥后旧身份签名校验失败，以及 PIN 与签名绑定同一接收公钥；见 [探针结果](evidence/airplay/pairing-persistence-macos.json)。这些不控制 iPhone 原生输入窗，不足以证明其未弹窗的唯一根因。

身份保留修复后的 iPhone 实测仍无输入窗：配对窗口有效，`/info` 的 440 字节和 `/pair-pin-start` 的 97 字节全部发送成功，约 4 ms 后源端主动断开，没有 `/pair-setup-pin` 或 `/pair-verify`。见 [本次重试](evidence/airplay/iphone-pairing-retry-20261002.json)。因此尚未到已知公钥检查，不能把“服务器拒绝旧配对公钥”作为该尝试的直接原因。接收端已再次以同一身份更新 PIN 窗口，下一步为重启 iPhone 后的同目标实测，排查源端提示状态；[Apple 官方排查](https://support.apple.com/zh-cn/102587)也包含重启设备，但这不是缓存已证实失效或问题已修复的证据。

## 已确认事实

- 实机来源为 iPhone 16 Pro / iOS 27 / Spotify。初版曾完成配对并实际出声；后续复测不能据此算通过。
- 测试脚本 `start(name)` 曾被复制二进制的循环变量覆盖，实际房间名变成 `neonmix-desktop`。已改为 `binary_name` 并加入设置名称断言；新的测试名称通过系统 DNS-SD 核验。
- 配对窗为 10 分钟。旧 UI 曾继续显示过期码，现 API 过期隐藏、UI 倒计时与过期提示均已核对。
- 新传输记录来自实际 socket send 完成之后，不再把 handler 选中 200 当作发送成功。只记录连接编号、协议枚举、长度、状态和固定结果，不记录 PIN、报文、密钥、IP 或 User-Agent。
- [真实 PIN 传输记录](evidence/airplay/iphone-pin-transport-macos.json)：RTSP `/info` 完整发送 440 字节，`/pair-pin-start` 完整发送 97 字节；约 3ms 后对端关闭连接，没有 SRP 请求，也没有诊断丢失。因此没有错误 PIN proof 的证据，发送阻塞/等待 EOF 不是这次连接的表现。

## 状态声明对照

固定 UxPlay 1.73.7 源码的 PIN 模式与原 NeonMix 声明不同：

| 字段 | 原 NeonMix | 上游对照候选 |
|---|---:|---:|
| RAOP TXT `sf` | `0xC` | `0x4` |
| AirPlay TXT `flags` | `0xC` | `0x4` |
| 完整 `/info` 根 `statusFlags` | `0xC` | `0x44` |

上游 RAOP PIN 分支先写状态值，随后又被统一的 `RAOP_SF=0x4` 覆盖；对照采用实际生效值。两种 TXT 的 `pw=true` 与上游一致，不能归因于这个字段。

首轮 `qualifier=txtAirPlay` 响应只有 TXT data，完整 `/info` 的根级字段不参与该响应。候选仍保持纯音频 features、型号、PIN、SRP、签名和授权门。错误 PIN、公钥绑定、撤销、第二连接和加密音频回归通过；真实输入窗结果待确认，不能把该声明差异直接称作根因。

开发 fixture 的原地 Hub 二进制更新再次遇到平台 vault 调用阻塞，未静默改用明文。为继续测试已重建独立 fixture；所以对照的代码差异集中于状态声明，但接收 ID/公钥也随 fixture 重建变化，不能声称是严格保持身份的单变量物理实验。

## 已排除与未决

HTTP/1.1 空错误应答缺少 `Content-Length: 0`，标准 HTTP 客户端确实会等待直到超时；隔离序列化对照加 CL0 后立即完成。但当前 PIN 200 来自 RTSP，不能用该 HTTP 结果解释实机 PIN 无窗。

在取得状态位对照结果前，不同时更改 model/features/SRP、不主动关闭 PINstart 连接，也不把问题直接归因于手机系统。后续继续检查实机 UI 与配对状态，保持接收认证要求。
