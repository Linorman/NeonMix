# P4 desktop-service 结构化错误证据

日期：2026-10-07；执行主机：macOS。所有命令通过 `tools/dev`，缓存与构建目录均在项目内。

已完成：Reply 和 ProcessStatus 增加可选 fault；闭合 100 个 FaultCode；参数只允许端口；异常 fault 单独拒绝、整包降级不崩溃；真实旧/新 Serde 组合；backend 主动生成服务错误码；有效 stdout fault 优先；已知 control/AirPlay/credential/network 机器码保留；任意 stderr 转为受控类别；ProcessStatus 保留实际故障；EOF 未换行也记录；原有受控导出保持；IPC 合同已同步。没有改写原有生命周期和 Windows 管理实现的所有权/停止逻辑。

## API 与 UI 接入

`Fault { code: FaultCode, params: FaultParams }`；`FaultParams { port: Option<u16> }`。`FaultCode::ALL`、`as_str()` 提供完整枚举。`Fault::from_machine_code(&str)` 只接受机器码或受控冒号后缀，未知返回 None。Reply/ProcessStatus 的 fault 缺省或 null 为 None；存在但未知码或异常参数为 Some(GenericFailure)，避免再从 legacy code 补出不可信参数；直接解码 Fault 则拒绝。UI 接口对应 `Message::Fault{PascalCaseCode}`；仅 `Message::FaultHubPortInUse { port: u64 }` 有参数。对应消息 ID 为 `fault-` + 机器码将 `_` 改为 `-`。资源 schema/中英文已写入 faults.toml/faults.ftl。

## 已执行验证

- `tools/dev cargo test -p neonmix-desktop-service --no-fail-fast`：26个测试通过（6 unit、6 fault_contract、9 lifecycle、5 redaction），0失败；2 个已有 managed 原生探针测试因专用 fixture 要求保持 ignored；Windows-only 测试在 macOS cfg 下为 0。
- `tools/dev cargo clippy -p neonmix-desktop-service --all-targets -- -D warnings`：通过；最终100码、客户端/后台deadline错误路径更新后已再次检查；日志 `service-clippy.log`。
- `tools/dev cargo fmt -p neonmix-desktop-service`：通过。

最终命令与退出码见 `service-validation.json`；主机测试完整日志见 `service-tests.log`。已接纳请求的 runtime mutex deadline 返回 background_busy，仍允许紧急停止中断查询；未完成帧超时保持原有关闭连接语义。客户端断链/限时、启动不可用返回公开机器码，legacy消息中的端口使用已校验的公开值。

| UI 协议解码 | 后台协议编码 | 证据 | 结果 |
|---|---|---|---|
| 旧 | 旧 | OldReply/OldProcessStatus 真实 Serde round trip | 保持 legacy error/status |
| 新 | 新 | Reply/ProcessStatus 真实 Serde round trip；实际 macOS IPC+CLI fixture | fault 优先，公开参数保留 |
| 新 | 旧 | 缺 fault 的 OldReply/OldProcessStatus bytes 反序列化 | 缺省 None，已知旧机器码可映射 |
| 旧 | 新 | Reply/ProcessStatus bytes 经原字段 OldReply/OldProcessStatus 解码 | 新字段被忽略，legacy 可读 |

参数测试覆盖未知 code、未知参数/token/PIN、未知顶层 stderr、漏 port、零/负/越界/小数/字符串 port；异常整包不会 panic。秘密测试覆盖 token、PEM、PIN、路径和任意 stderr，不进入新故障主文案或受控导出。CLI 优先级测试使用真实 subprocess + Unix socket，并验证 port=9000 被保留、异常 fault 回退已知 session_changed、未知文字变 generic_failure。额外验证 network_uac_cancelled、credential_permission_denied、runtime_cleanup_incomplete 的同一语义映射。

## 验证边界

没有启动历史发布版本的完整 Desktop/background 二进制；兼容证据为真实原字段类型的序列化 fixture 加新后台 IPC fixture。双语 UI 实际呈现由主 Agent 集成验收，此报告只证明 fault 及配套双语资源存在。

Windows/Linux 原生 NamedPipe、UAC、系统密钥读取及进程退出没有在本任务实机运行；网络 helper 语义通过 fault/code fixture 验证，不能据此宣称实际网络配置成功。macOS hosted Windows 交叉检查最初默认命令因缺 `x86_64-w64-mingw32-gcc` 失败，随后复用项目内既有 MinGW headers 和 LLVM 工具重新检查；复用后的 `--all-targets` check 已通过；日志见 `service-windows-check.log`。
