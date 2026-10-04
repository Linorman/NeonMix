# 桌面 UI 重构证据（2026-10-04，仅 macOS）

依据 [设计方案](../../../09_NeonMix_UI重构_设计方案.md) 与 [开发计划](../../../10_NeonMix_UI重构_开发计划.md)。所有截图来自预览模式（录制的公开状态，不启动后台、不播放音频），机器为 Apple M4 Pro。

## 目录

| 路径 | 内容 |
|---|---|
| `fixtures/` | 由 `../multi-airplay-ui-20261002/fixture.json` 派生的 7 个场景：`empty`、`member`、`full`、`playing`、`faults`、`output-lost`、`no-diagnostics` |
| `before/` | 重构前六页 × 600/1100/1600 宽截图与 `cpu.json` 基线 |
| `after/` | 重构后七页 × 三种宽度（`full` 场景）、`cpu.json`、`contrast.json` |
| `after/scenarios/` | 「现场」页 7 个场景 × 1100/600、Mixer 故障/输出丢失/无诊断/成员、Sender 成员/空、Hub 空、减少动态效果 |

## 复现

```sh
tools/dev cargo build -p neonmix-desktop --release --locked --features screenshot
tools/ui_preview_shots.sh docs/evidence/ui-rebuild-20261004/after 1100 760 docs/evidence/ui-rebuild-20261004/fixtures/full.json
```

单页单场景：`NEONMIX_SCREENSHOT_TO=out.png tools/dev target/release/neonmix-desktop --preview-page live --preview-data docs/evidence/ui-rebuild-20261004/fixtures/faults.json`。`NEONMIX_SCREENSHOT_PHASE` 固定动效时钟（脚本默认 1.25 秒），`NEONMIX_REDUCE_MOTION=1` 开启减少动态效果。

## 结果

- 检查：`cargo fmt --check`、`cargo clippy -p neonmix-desktop --all-targets -D warnings`、`cargo test -p neonmix-desktop`（27 项）通过。新增测试覆盖汇流图节点不重叠、节点 AccessKit 名称、无信号/减少动态效果时不申请连续重绘、房间动态差异、历史窗口、徽记稳定性、两种布局的方向键、横竖推子单击不跳/双击回 0 dB、600×440 侧栏不重叠。
- CPU（`after/cpu.json`）：4 路流动时「现场」1100×760 为 5.9%、1600×1000 为 6.4% 单核，空闲页面 0.96–1.26%（基线 1.03–1.23%）。30 fps 首版为 10.5%，`sample` 显示主线程大部分时间在 `CGLFlushDrawable`，因此连续动效上限改为 20 fps。
- 对比度（`after/contrast.json`）：正文、次要文字与各状态色在卡片底色上均 ≥ 4.5:1；`TEXT_3` 在悬停底色上为 4.48:1，该组合不用于说明文字。

## 未验证

预览模式没有轮询：Mixer「最近 60 秒」与诊断趋势图只在真实运行时出现，截图中显示“正在记录”或不显示。本轮未做真实 Hub + 多台 Sender/AirPlay 设备的实机运行与录屏、VoiceOver 听读、真实中文输入法组字（沿用既有 IME 单元测试）、Windows/Linux 桌面渲染与性能。
