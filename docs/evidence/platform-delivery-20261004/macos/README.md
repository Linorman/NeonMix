# macOS ARM64 当前快照运行包（2026-10-04）

固定 522 文件快照 canonical SHA256 为 322fda1b52872be7e880f750a02ef01fe0034895ecba6aa10741c504078de860。
本轮 workspace release 与固定 worker/probes 重新构建通过；本轮未重新跑 macOS 全部常规测试或多路矩阵。

完整 NeonMix.app、六个程序、GStreamer/编解码依赖、插件、scanner、启动脚本、源文件归档和许可记录已留存。
在含空格路径、清洁开发环境下：runtime/设备枚举、正式 600×440 UI、Cmd+K/Cmd+2、后台私有 IPC、UI退出后后台存活、独立Hub认证及CoreAudio输出、AirPlay worker ready通过。
Hub输出41,472帧，采样errors=0；vmmap实测Hub/后台加载模块无开发SDK或target路径。包内Mach-O依赖改为相对Frameworks路径，codesign --verify --deep --strict通过。
中文系统字体正常；截图目检快速操作与关闭提示完整。

第一次Hub启动在20秒窗口内没有就绪，日志为空；保留首败。仅延长启动观察至60秒的定向复验通过，未改变产品代码。原因未定位，不能用复验通过宣称间歇启动问题已根治。
打包脚本首次遇到universal Mach-O头、dylib自己的ID、系统库程序缺rpath header空间，均是交付脚本问题；修正脚本后本次包完成相对依赖与签名检查。

包为本机内部 ad-hoc 签名测试制品，未做Developer ID签名/公证。没有Apple真实来源、实听音质、视频伴音或长期稳定性结论。
将完整目录放入较短的可写真实路径，双击NeonMix.app或Start-NeonMix.command；CLI为 ./neonmix hub runtime、./neonmix audio devices。
运行资料/凭证在包目录.s，临时registry在.t。结束请用“退出后台”；分享时只使用未运行的原始ZIP。
重定位夹具、自有Hub/后台/UI及临时资料均已清理；本机既有用户房间未操作。

最终Finder/LaunchServices启动验证通过：可见窗口1个，独立后台自动创建，结束后夹具已清理（finder-launch.json）。初轮外部磁盘TCC授权与过早的AX查询均保留记录；原生启动器保持已注册的.app进程作为桌面子进程的responsible process，正常保留LaunchServices/TCC归属。启动器源码随包提供，仅更改打包入口，产品Rust制品未改。
