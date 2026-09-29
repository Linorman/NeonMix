# Core Audio 虚拟设备（E06）

E01通过Core Audio UID读取已存在虚拟设备的input side。本目录尚无AudioServerPlugIn实现，也不会把系统tap当作虚拟输出。E06需实现Rust ABI、设备时钟、受控有界桥接、属性/音量、加载、卸载与签名公证；单独验证Apple Silicon与目标SDK。
