# Windows AirPlay 可靠性自动验证

本目录关联本次源码、补丁和未签名候选包。实施与未验收项见 [改进记录](../../WINDOWS-AIRPLAY-RELIABILITY-IMPLEMENTATION-20261007.md)。

- `mac-stop-*.json`：1–4 路活动停止、原资料/端口重开和 EOF 清理；正常退出、无强制回收、无运行 PEM。
- `windows-identity.json` / `windows-package-identity.json`：ACP936，含当前代码页不可表示的非 BMP 路径及同句柄文件替换拒绝；身份和签名保持。
- `windows-network-helper.json`：规则模板读回、双实例隔离、端口及修复/删除；profile 开关均保持原关闭状态，不能计作防火墙开启的播放验收。
- `windows-job-owner.json`：异常 owner 结束后，持有的两个子孙进程对象均退出。
- `windows-package.json`：完整安装包、各组件和源码哈希；测试镜像无 Git，commit=null。
- `windows-package-pcm.json`：包内匹配的 DLL/plugin 组合，7 项加密 UDP→PCM/SETUP 边界回归。
- `windows-package-smoke.json`：runtime、无 owner 不启动后台、目标 UI 正常退出且另一目录 UI 持续运行。
- `mac-quality-*-failed.json`：独立质量失败，未合并为 pass；shutdown 的 passed 只对应资源收尾范围。

真实 iPhone、Public 防火墙开启、非 ASCII Windows 账户、跨用户 UAC、组织策略、双栈和活动安装升级卸载仍未执行。没有替换原 D 盘安装和原配对资料；合成来源不代替真实 Apple 设备。
