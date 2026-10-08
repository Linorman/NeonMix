# macOS 原生语言与菜单验收

日期：2026-10-07。工具：`tools/i18n_native_macos_probe.py`；所有运行、OS自动化、截图和产物均通过tools/dev并保存在项目目录。

最终release screenshot冻结包（SHA256 `b05318078859552336309ab67a8406ad914b4d6004d7f1d15b9be292c1da5da5`）完成54/54次实际有效语言切换，`artifacts/i18n/native-macos/result.json`为passed=true；图像和AX日志在同目录。开始与结束均校验冻结binary hash，验证期间未替换文件。先前fresh debug冻结包（SHA256 `4aeeb19f87c7b4408ab354946a646f70dcc7e1b8ed1b947fc71bda92543ca9fd`）也54/54通过，机制诊断证据另存 `artifacts/i18n/native-macos-debug/`。

本次对应上述明确冻结release二进制的语言/原生菜单机制。后续foreign操作意图等集成源码及正常正式包的source/binary对应关系由最终整体报告另列，不能把这份特定preview binary的成功泛化到随后重编的任意包。

每次以真实AX定位语言选项并用Space激活，在en与Auto(zh-CN)间切换；另验证手选简体中文。选择后检查弹出列表已关闭、焦点回到同一语言选择框、侧栏AX名称及时翻译、原生tray的打开/停止发送/退出三项更新且仅一个tray。zh-CN/en两种应用原生菜单的打开/关闭窗口/退出文案均通过；初次定位作者菜单后复用同一个原生menubar item index。

键盘与生命周期表面：Escape关闭语言列表后焦点返回；Cmd+W隐藏窗口，真实tray Show动作恢复唯一窗口且保持语言；Cmd+Q打开唯一确认框，Escape取消且进程保留。没有启动后台/音频owner，也未写入preview的私有state目录。Auto另以实际原生窗口启动检查 `[en-US]→en`、`[zh-TW,en-GB]→en`、`[fr-FR,zh-Hans-CN,en-US]→zh-CN`；这些候选来自受控preview参数，真实系统候选采集由整体适配器验收另列。

最终release字体初始化后RSS为225488 KiB，54次切换后202192 KiB，差值-23296 KiB；54次范围202192–226416 KiB，前10次均值225992 KiB，后10次均值220635.2 KiB，没有观察到持续增长。逐次采样保存在cycles数组，不能把RSS差值当作分配泄漏证明。debug对照为118944→119808 KiB（+864 KiB）。

## 修复及回归

真实native在旧release包（761acab3...cf1、4b8a05fb...ff848）中发现Space选择已改变语言，但列表仍打开。首个根因是egui0.31的ComboBox采用Memory popup manager，`ui.close_menu()`只关闭menu_state，键盘没有pointer click自动关闭。共用 `widgets::select_value` 现在显式close_popup+close_menu，并消费已处理的Space/Enter以免父选择框重新接收同一激活；没有把修复限于某页。增加真实4帧回归并模拟request_discard，检查值改变、关闭、返回焦点、Space消费和keyup不重开。

旧失败result与AX证据保留于 `artifacts/i18n/native-macos/before-keyboard-popup-fix/`。工具还纠正了两处原生定位事实：侧栏选中导航在AX中为checkbox；raw executable的应用菜单标题由Cocoa命名为neonmix-desktop，安装包CFBundleName才是NeonMix。截图等待非零菜单bounds，并固定首次找到的作者菜单，避免反复扫描系统Apple菜单。

## 边界

这是实际macOS GUI/keyboard/AX/native菜单自动化证据。没有完成VoiceOver听读、真实中文IME组字、正在播放的真实音频会话切换、Windows/Linux原生菜单或硬件验收；没有从preview无owner推断真实播放期无扰动。Rust MenuItem句柄原位更新由代码保证，AX采样另证明单tray及每次可见文案更新。代码后续集成和正式包的source/binary hash必须由最终整体报告对应。
