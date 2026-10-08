# Windows 页面切换闪烁与 AirPlay 开启失败

调查于 2026-10-08 开始，2026-10-09 完成收尾。机器为 `administrator@192.168.100.186`，所有远端源码、缓存、编译和测试文件均在 `E:\Desktop\NeonMix-test` 内；原 `D:\NeonMix` 安装未覆盖，真实用户资料未修改。没有使用子 Agent。

## 根因与修改

- AirPlay：后台把私有资料路径 canonicalize 为 `\\?\E:\...`。MinGW `std::filesystem::parent_path()` 将扩展路径前缀当成 UNC 根，祖先遍历错误地尝试打开设备命名空间，worker 报 `identity_permission_denied`，经 Hub 映射为 `failure_stage=identity_permission`。同一个公开 RFC 8032 密钥普通路径成功、扩展路径失败；修复后两者均成功，见 `path-repro.json`。现在明确解析 Win32 盘符与共享根，保留扩展路径，并继续逐级固定祖先、拒绝 reparse point、核对当前用户私有 ACL、从同一文件句柄读取密钥。
- 界面：安装版桌面程序哈希与当时最新测试构建一致。可见窗口中导航触发正文整页从 60% 透明度和下移 6px 入场。新回归在原代码上检测到正文文字 alpha 为 154，要求为 255；见 `navigation-before.log`。现移除整个页面的透明度与位移动画，局部控件反馈保留。源码级回归同时检查文字实际网格 alpha 和位置。
- 测试夹具：`icacls` 的本地代码页输出不再强行以 UTF-8 解码；仅使用其退出码，不改变权限设置行为。

## 验证范围

- macOS 和 Windows 原生各 90 项 Desktop 测试通过；Windows/MSVC release build、两端严格 Clippy、格式、746 条双语资源与 UI AST、premium 静态审计通过。
- Windows 27 项身份检查、29 组实际 worker 启动场景通过，包含六个合法密钥的扩展路径、中文/空格路径、错误格式、ADS 拒绝、同句柄替换保护、EOF 停止；`identity-fixed.json`。UNC 为根解析单元覆盖，没有创建 SMB 共享或声称真实远端文件读取通过。
- 包内真实后台→Hub→worker，在实体 UA2 输出上执行 1、2、4 个入口各三轮启动/关闭；每次所有目标入口 ready、published、无错误，关闭后运行密钥为零；`package-lifecycle.json`。该报告的 desktop 哈希属于加强测试断言前的构建；最终桌面与完整包哈希以 `package.json` 为准。参与九轮生命周期验证的 Hub/worker 与最终包相同。
- Windows Session 1 实际窗口：旧版刷新设备和开启 AirPlay 的房间标题区域像素稳定，未观察到整窗黑白闪屏；总静音产生预期的局部状态变化。导航前后逐帧指标分别见 `navigation-before-frames.json`、`navigation-after-frames.json`，修复后房间文字区域最大像素差为 0。`navigation-after.png` 是修复版导航首帧；`airplay-ready.png` 是修复 worker 后的真实 UI。
- 最终完整包使用真实鼠标点击再验证：`result-start.json` 确认后台运行，`result-airplay.json` 确认 1 个入口 ready，`result-mute.json` 确认权威总静音从 true 变为 false。导航、AirPlay、总静音采样分别为 30/75/75 帧，焦点保持；正文文字区域最大像素差均为 0。最终截图见 `final-navigation-first.png`、`final-airplay.png`、`final-mixer.png`；截图采样结束后才读取权威结果，因此提交后的下一次 UI 轮询可能晚于末帧。
- 第一次自动化误选了 OpenGL 的辅助窗口；另有一轮采样被其他窗口遮挡，均排除在上述视觉结论外，没有把屏幕背景变化算作产品渲染结果。最终包重新生成时遇到测试 UI 持有文件，关闭该测试实例后成功重打包。

没有执行真实 iPhone 播放、安装升级、长时间音频或所有显示驱动组合验收。本次证明入口能开启并发布，以及已定位的页面导航明暗/位置变化已消除；不把这些结果写成完整发布验收。

## 交付

Windows 安装包位于：

`E:\Desktop\NeonMix-test\windows-fix-20261008\NeonMix-0.2.0-windows-x64-fixed-20261008-setup.exe`

完整展开目录为同级 `NeonMix-fixed`。安装包未自动运行，原安装目录保持。可复制到本机项目 `artifacts/windows-fix-20261008/` 的同名安装包；SHA256 和包内逐文件哈希在 `package.json`，源文件哈希已与本地工作区核对。

收尾已退出本轮测试 UI/后台，移除一次性桌面测试任务；无关文件未删除。合成资料、逐帧临时截图、旧安装测试副本和脚本在保留上述证据后清理。最终安装包为 22,571,822 字节，SHA256：`ef13cc4ea2b81b1cb4610318d08cfecc9d310c0c570377273c225fe585da0a88`。
