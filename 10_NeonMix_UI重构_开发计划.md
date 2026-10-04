# NeonMix UI 重构 · 开发计划与验收清单

日期：2026-10-04 · 依据：[09_NeonMix_UI重构_设计方案.md](09_NeonMix_UI重构_设计方案.md) · 范围：`apps/desktop`

## 1. 总体安排

七个阶段，按依赖顺序推进，每个阶段结束时都能构建、测试通过并产出对比截图。估算共约 12 个工作日。

| 阶段 | 内容 | 估算 | 依赖 |
|---|---|---|---|
| P0 | 基线、决策落地、fixture | 0.5 天 | 设计方案 §10 决策 |
| P1 | 视觉与动效基础设施 | 2 天 | P0 |
| P2 | 通道模型抽取 + 信号汇流图 + 「现场」页 | 3 天 | P1 |
| P3 | Mixer 通道条控制台 + 电平历史 | 2 天 | P1、P2 的通道模型 |
| P4 | Hub / Sender / 设备 / 诊断页新组件 | 2.5 天 | P1 |
| P5 | 外壳与微交互 | 1 天 | P1 |
| P6 | 验收、性能、文档 | 1 天 | 全部 |

P3、P4、P5 之间互不依赖，P2 完成后可以按任意顺序进行。

## 2. 代码结构

| 文件 | 动作 | 内容 |
|---|---|---|
| `src/theme.rs` | 改 | v3 token、来源类型色、`SOLO`、`DISPLAY` 字号 |
| `src/fx.rs` | 新 | 辉光纹理缓存与绘制、渐变 Mesh、斜线阴影、虚线/断口贝塞尔、LED 分段环 |
| `src/animation.rs` | 改 | 弹簧、数值过渡、`LiveClock`、`live_repaint` 帧率预算、`MotionPrefs`、截图固定相位 |
| `src/lanes.rs` | 新 | 从 `pages/mixer.rs` 抽出 `Lane` 与构建逻辑，供现场、Mixer、Sender 共用 |
| `src/flow.rs` | 新 | 汇流图模型、`layout()` 纯函数、绘制、交互、AccessKit |
| `src/history.rs` | 新 | 定长环形缓冲；电平历史带与迷你折线绘制 |
| `src/emblem.rs` | 新 | 由 `hub_id` 生成房间徽记 |
| `src/widgets.rs` | 改 | 推子核心与方向解耦；新增 `fader_vertical`、`channel_strip`、`switch`、`countdown_ring`、`capacity_slots`、`pipeline`、`radar` |
| `src/pages/live.rs` | 新 | 「现场」页：汇流图、检查器、房间动态 |
| `src/pages/*.rs` | 改 | 按设计方案 §6 更新各页 |
| `src/shell.rs` | 改 | 侧栏、顶栏、状态条、页面过渡、快速操作动效 |
| `src/main.rs` | 改 | `Page` 增加 `Live` 并设为默认；历史缓冲与动态事件状态；`--preview-page live` |
| `tools/ui_preview_shots.sh` | 改 | 页面列表加入 `live`；支持固定动效相位 |

`widgets.rs` 已有 1,534 行。新增控件放在各自文件，暂不拆分现有控件，避免和功能改动混在同一批次里。

## 3. 统一检查命令

每个阶段结束执行：

```bash
tools/dev cargo fmt -p neonmix-desktop --check
```

```bash
tools/dev cargo clippy -p neonmix-desktop --all-targets --locked -- -D warnings
```

```bash
tools/dev cargo test -p neonmix-desktop --locked
```

```bash
tools/dev cargo build -p neonmix-desktop --release --locked --features screenshot
```

```bash
tools/ui_preview_shots.sh docs/evidence/ui-rebuild-20261004/<阶段>/ 1100 760
```

截图三个尺寸：600×440（最小窗口）、1100×760（默认）、1600×1000（宽窗）。

## 4. 分阶段计划与清单

### P0 基线与准备

目标：重构前留下可对比的基线，把评审决策写进文档，准备覆盖各状态的 fixture。

- [x] 用当前代码截取六页 × 三尺寸基线截图，存入 `docs/evidence/ui-rebuild-20261004/before/`
- [x] 测量基线 CPU（预览模式，30 秒进程 CPU 时间），记录到 `before/cpu.json`：各页约 1.0–1.2% 单核（M4 Pro），现有界面空闲时也以 5 Hz 重绘
- [x] 评审决策：设计方案 §10 五项全部按建议执行（2026-10-04）。`DESIGN.md` / `UX-CONTRACT.md` 的条文在 P6 按最终实现一次性修订，避免文档先于代码
- [x] 由 `docs/evidence/multi-airplay-ui-20261002/fixture.json` 派生场景 fixture，存入 `docs/evidence/ui-rebuild-20261004/fixtures/`：
  - [x] `empty.json` 空状态（无房间）
  - [x] `member.json` 仅 Sender（成员身份，看到远端房间）
  - [x] `full.json` 4 路满载（2 原生 + 2 AirPlay，含 Solo、静音、降级、限幅）
  - [x] `playing.json` 4 路全部可闻
  - [x] `faults.json` 缓冲、网络中断、AirPlay 静音
  - [x] `output-lost.json` 输出丢失
  - [x] `no-diagnostics.json` 有快照无诊断
- [x] CPU 目标：无音频时 ≤ 基线 + 0.5%；4 路流动时 1100×760 ≤ 8%、1600×1000 ≤ 10%（单核百分比）

### P1 视觉与动效基础

目标：新 token 和基础设施就位，现有页面整体换肤但布局不变，所有现有测试照常通过。

- [x] `theme.rs` 替换为 v3 token；新增 `SRC_NATIVE`、`SRC_AIRPLAY`、`SRC_HUB`、`SRC_OUTPUT`、`SOLO`、`BG_DEEP`、`EDGE_LIGHT`、`DISPLAY`
- [x] 对比度校验，记录到 `after/contrast.json`（`TEXT_3` 在悬停底色上 4.48:1，不用于说明文字）
- [x] `fx.rs`：卡片渐变（纹理着色）、`glow`（egui 模糊矩形代替纹理）、斜线阴影、贝塞尔采样与虚线、LED 分段环、电平分区色
- [x] `animation.rs`：`ease_to` 缓动（代替弹簧）、`live_phase` 只在活动时推进、`live_repaint` 帧率预算（实测后定为 20 fps / 失焦 10 fps）、`once` 一次性动效、减少动态效果开关、`NEONMIX_SCREENSHOT_PHASE`
- [x] 换肤：主按钮/危险按钮光晕、胶囊描边、卡片表面、面板、Solo 改用 `SOLO` 色、推子旋钮
- [ ] 按钮按下内容下沉 1px（egui 按钮文字位置不可直接偏移，未做）
- [x] 测试：无活动数据时不申请连续重绘；减少动态效果时也不申请（`animation` 与 `live_graph_moves_only_while_signal_flows`）
- [x] 截图对比：布局与基线一致，只有外观变化

### P2 信号汇流图与「现场」页

- [x] `Lane` 移到 `lanes.rs`，增加会话状态与“本机”标记，Mixer 测试全部通过
- [x] `flow.rs` 模型、`layout()` 纯函数（左右三栏 / 窄窗一行紧凑卡）与不重叠测试
- [x] 来源节点（类型色电平环、“你”角标、Solo 光晕）、房间核心（28 段环、徽记、限幅弧）、输出节点
- [ ] 来源电平环上的峰值保持刻度（未做，电平环只画 RMS）
- [x] 连线七种状态 + 核心→输出的静音/丢失；光点每线 8 个
- [x] `emblem.rs` 与稳定性测试
- [x] 悬停高亮路径与事实浮层；单击选中；单击核心打开 Mixer（由双击改为单击）；离线胶囊跳转设备管理
- [x] 键盘：Tab 可聚焦节点，↑/↓ 选通道，M/S 作用于选中通道
- [x] AccessKit 节点名称与测试
- [x] 一次性动效：新来源接入
- [ ] 一次性动效：节点离开、Solo 聚光过渡（未做；Solo 为静态光晕与描边）
- [x] `pages/live.rs`：汇流图 + 检查器 + 房间动态；空状态入口
- [x] 房间动态差异与测试；切换身份清空
- [x] `Page::Live` 默认、⌘1–⌘6、快速操作跳转、`--preview-page live`、截图脚本

### P3 Mixer 通道条控制台

- [x] 推子横竖共用核心；原单击/双击测试对两种方向通过
- [x] 通道条、总控条（徽记、刻度电平、限幅衰减表、预设、总静音、读数）
- [x] 静音斜线、Solo 黄色帽、不可闻 68%
- [x] 详情抽屉；布局切换（通道数 × 138 + 188）；不新增滚动容器
- [x] 方向键随布局切换与测试；提示行随布局切换
- [x] `history.rs` 60 秒窗口与测试；电平历史带；底部长说明移入悬停提示
- [x] 截图：满载、故障、输出丢失、无诊断、成员、窄窗行式

### P4 各页新组件

- [x] Hub：房间徽记、共享开关（可访问名称为“开始共享/停止共享”，兼容原生 UI 探针）、开始共享波纹、邀请倒计时环、输入容量槽
- [ ] Hub：邀请被使用后的对勾、到期后“重新生成”按钮、AirPlay 入口槽位卡（未做；后台不回报邀请已被使用，入口仍为原列表样式）
- [x] Sender：五段发送链路管线兼步骤、本机通道电平驱动光点、发现雷达、窄窗纵向
- [x] 设备：卡片网格（列最小 300px）、来源类型色帽、已撤销斜线；搜索/筛选/计数不变
- [ ] 设备：筛选芯片选中底色滑动（未做）
- [x] 诊断：2 分钟 1 Hz 指标历史与趋势折线、异常磁贴光晕。历史只在 UI 内存中，脱敏导出由后台从 Hub 诊断生成，不经过这些数据，因此未加导出断言

### P5 外壳与微交互

- [x] 品牌标记随总输出 RMS 起伏，无声静止；导航指示条缓动 + 辉光
- [x] 底部降噪：停止发送改描边；隐藏 / 退出所有宽度并排（窄侧栏紧凑按钮）
- [x] 顶栏房间胶囊加徽记
- [ ] 身份卡（头像 + 名称 + 角色）（未做，保留原下拉框与角色胶囊）
- [x] 页面切换淡入 + 6px 上移；还原 8 秒倒计时细条；快速操作淡入落位
- [ ] 快速操作选中高亮滑动（未做）
- [x] 正文氛围光随状态 200ms 过渡
- [x] 关于页“减少动态效果”，保存到状态目录 `ui-preferences.json`（仅该布尔值）
- [x] 测试：600×440 侧栏不重叠（发现并修复 2px 重叠）

### P6 验收与文档

- [x] fmt、严格 Clippy、27 项测试、截图构建通过
- [x] 七页 × 三尺寸（`full`）+ 22 张场景截图，与 `before/` 并列归档
- [x] CPU 复测满足目标，写入 `after/cpu.json`
- [x] 静止验证：以单元测试检查无信号时的重绘请求（代替计数输出）；空闲页面 CPU 与基线相当
- [x] 减少动态效果截图与 CPU（1.23%）
- [ ] macOS 真机运行：真实 Hub + 多台 Sender/AirPlay 录屏（未做）
- [ ] VoiceOver 听读（未做；AccessKit 树断言已加）
- [ ] 真实中文输入法组字（未做；沿用既有 IME 单元测试，字段与快速操作逻辑未改）
- [x] 更新 `DESIGN.md`、`UX-CONTRACT.md`、`docs/STATUS.md`；README 无截图引用，不需改
- [x] 清理临时文件

## 5. 风险与对策

| 风险 | 对策 |
|---|---|
| egui 每帧重新细分整页，30 fps 连续重绘可能明显增加 CPU（已发生：10.5%，降到 20 fps 后 5.9%） | 光点数量上限；辉光用纹理不用多层描边；只在有活动信号时重绘；P0 基线 + P6 复测作为硬门槛，超标则把帧率降到 20 fps 或改为只有汇流图可见时重绘 |
| 诊断轮询间隔为 250ms / 1s，光点亮度会阶梯变化 | 复用现有电平弹道平滑；光点亮度取弹道值而非原始值 |
| 新增页面改变 ⌘ 编号，影响现有测试与用户习惯 | 测试统一引用 `Page::ALL`；快速操作仍可按名称跳转；在关于页更新说明 |
| 自绘画布的无障碍容易遗漏 | 每个可交互图形元素都必须调用 `widget_info`，P2 / P3 加 AccessKit 树断言 |
| 动效与“诚实展示”冲突 | 设计方案 §7.2 规则写入 `DESIGN.md`；代码中每个数据驱动动效注明绑定的量；评审时逐项核对 |
| `Lane` 抽取引入行为回归 | P2 第一项单独完成并跑全量 Mixer 测试后再开始绘制工作 |
| 霓虹色在浅色系统主题或投屏下刺眼 | 本次只做深色主题；辉光强度集中在 `fx.rs` 一个常量，必要时统一调低 |

## 6. 完成判据

1. 「现场」页在第一屏用汇流图表达“多源 → 房间 → 输出”，九种连线状态都有 fixture 截图证据。
2. Mixer 宽窗为通道条控制台，窄窗回退行式，推子交互与还原行为与重构前一致。
3. Hub、Sender、设备、诊断四页的新组件全部上线，原有功能测试全部通过。
4. 无音频时界面不连续重绘；有音频时 CPU 满足 P0 写定的目标；减少动态效果开关有效。
5. 600×440、1100×760、1600×1000 三个尺寸无裁切、无重叠。
6. `DESIGN.md`、`UX-CONTRACT.md` 与实现一致。
