# ADR-008 / ADR-007：语言和构建边界（初稿）

Rust 1.95.0负责所有当前产品代码；Python仅运行外部CLI、分析JSON和整理构建证据，独立tools/pyproject.toml + uv.lock，当前无第三方Python依赖。所有下载、缓存、临时文件、构建结果在项目`.local/`、`target/`、`artifacts/`。

无新增C/C++源码。CPAL、PipeWire和系统SDK的原生依赖不等于本项目维护C++主应用。Windows SysVAD/WDK驱动和macOS AudioServerPlugIn尚未开始，drivers目录只有边界说明，不放可编译假实现。

Rust依赖固定关键crate精确版本并提交Cargo.lock；工具链、平台支持基线和SDK清单见native-dependencies.toml。本地交叉检查Ubuntu头文件的版本/下载地址/SHA256见docs/evidence/linux-cross-dependencies.json。CI镜像标签不是不可变系统快照，实际SDK/包清单每次归档，不把镜像标签当依赖锁。

GStreamer尚未用于E01。E02接入时必须锁定三端完整runtime、插件与许可证，E09安装包必须带有相应组件并验证独立部署；不因Cargo构建成功而宣称GStreamer部署完成。

release保留debug=2/不strip，归档dSYM、PDB或.debug，连同源文件哈希、编译器信息、Cargo元数据、二进制SHA256。当前不是签名安装包，不宣称完成驱动/插件发布。
