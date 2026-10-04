# 本地凭证迁移完成（2026-10-02，macOS）

按用户授权，已迁移两套在用资料并完成入口切换。当前 AirPlay 房间从 `.local/airplay/manual-stable-b3de8677` 复制到 `.local/airplay/manual-file-b3de8677`；`manual-session.json` 指向新目录，复制并启动当前 release Hub/worker/background/desktop/audio，Hub 与界面运行、AirPlay 接收就绪、输出帧推进。默认桌面资料复制验证后放回 `.local/desktop`，旧目录完整保留为 `.local/desktop-native-20261002`；默认后台正常读取新版资料，Hub/Sender 保持未自动启动。

两套 Hub UUID、证书、管理员 token/TLS 引用及设备身份一致；发布时 Authority 字节完全一致。当前房间 receiver 原有每个字段保持，2 台来源、空阻止集合、允许播放和 low_latency 模式未改变。两个 Hub 的 HTTPS `/v1/me` 管理员认证通过。新文件共 5 个秘密条目，目录 0700、文件 0600，日志与报告未包含实际秘密值。旧原库条目未删除。

实际迁移暴露并修正两处兼容问题：手动会话根 bin 和接收端 GStreamer registry 是运行资产，精确排除，不复制进持久资料；来源公钥应原样保存 worker 编码（实际 Base64），存储层不能误当 64 字符十六进制摘要。新增编码保留/限额/控制字符回归，18 项 identity 测试、16 项迁移测试、严格 Clippy 与 release 通过。修复后 native helper 真实读取与 TLS/管理员/Ed25519 校验成功，发布无半成品。先前原库等待和格式失败记录保留为历史诊断，当前结果见 result.json。

较早的 `.local/airplay/manual-desktop-fa03d33a` 三个原库条目均已不存在，无法保留身份迁移；该旧测试资料保持原状，没有新建身份或替换其配对记录。当前迁移没有证明实际 iPhone/iPad 重新播放，需用户复测；接收就绪、原密钥与信任集合保持已验证。Windows/Linux 未执行。

当前资料位置：

- AirPlay：`.local/airplay/manual-file-b3de8677`（当前后台、Hub、worker、桌面界面）。
- 默认桌面：`.local/desktop`（文件 v2，后台待机）。
- 原 AirPlay：`.local/airplay/manual-stable-b3de8677`（保留；原实例不运行）。
- 原桌面：`.local/desktop-native-20261002`（保留；旧 v1 state_path 原指向 `.local/desktop/hub/state.json`，不能直接以备份路径运行旧版）。
- 原手动入口：`.local/airplay/manual-session-native-20261002.json`（备份）。

回退须停止新实例并审查新状态；新版已发生启动和接收设置写入，不能把旧快照直接覆盖新目录。桌面若另行回退，还必须恢复原目录位置，否则旧绝对 state_path 会访问新版 state。新旧同身份不得同时运行。此任务保留源与旧库，不执行清理旧账户。

本轮自有 staging 与诊断临时程序已清理。源码、编译产物、缓存、资料和脱敏证据全部在项目内。原失败记录中“未完成”是当时事实，以本记录的最终状态为准。
