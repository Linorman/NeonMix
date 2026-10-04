# Windows 虚拟 Render Endpoint（E06）

已新增 `wavert/Source`，以微软简化 SysVAD 的 Simple Audio Sample 为基础收敛一个 48 kHz stereo PCM16 WaveRT render endpoint。稳定硬件 ID 为 `ROOT\NeonMixAudio`，服务为 `NeonMixAudio`。采集端、测试音、PCM 文件写入和虚假硬件音量节点已移除；Windows 软件端点音量及现有 Rust WASAPI loopback 负责用户态桥接。

源码使用 MS-PL，原始版权/许可证、仓库 commit 和文件哈希保留。SDK/WDK 10.0.26100.6584 包均在项目 `.local/windows-driver-sdk`，SHA256 锁见 `wavert/sdk-lock.json`。macOS 的 Clang 交叉构建已产生未签名 x64 PE native 映像。2026-10-01 在 GitHub Actions 的 Windows Server 2022 / VS 2022 x64 MSBuild 上通过原生编译、链接、INF 和 Inf2Cat 检查，归档 `.sys`、`.inf`、未签名 `.cat` 和 PDB；下载后哈希与 NX/ASLR/CFG 标志已核对。这仍不是 Windows 加载或音频实测。

```sh
# macOS：只读系统编译器，所有 SDK/产物留在项目。
tools/dev python3 tools/prepare_windows_driver.py
tools/dev env -u DYLD_LIBRARY_PATH python3 tools/windows_driver_header_check.py
```

原生 Windows 构建入口：`tools/dev.ps1 powershell -File tools/build_windows_driver.ps1`。锁定的 WDK 要求 VS 2022 的 **x64 MSBuild**，构建入口会显式定位它，并选择项目内 x64 WDK 工具。它只构建和归档，输出在 `target/windows-driver` 与 `artifacts/windows-driver`，不安装驱动、不创建证书或修改安全设置。具体接口、上限与门槛见 [驱动合约](wavert/DRIVER-CONTRACT.md) 和 [ADR-010](../../docs/adr/ADR-010-windows-render-driver.md)。

驱动经适用签名并明确安装后，用户态入口仍为 `neonmix-hub send --capture <wasapi:stable-endpoint-id>`。端点的系统可选性、loopback 音量作用点、无活动应用、休眠/禁用/重启、Verifier/HLK/HVCI 及幂等安装升级仍未验收，不能据此称为 E06-W 成品。
