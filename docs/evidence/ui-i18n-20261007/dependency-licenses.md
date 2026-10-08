# UI i18n 依赖许可（2026-10-07）

来源：`tools/dev cargo metadata --format-version 1 --locked --filter-platform aarch64-apple-darwin`。表格覆盖 neonmix-i18n 的实际依赖闭包（含构建/检查器依赖）及 sys-locale；精确版本来自 Cargo.lock，manifest来自项目内 `.local/cargo/registry/src`。

| crate | 精确版本 | manifest license |
|---|---|---|
| displaydoc | 0.2.7 | MIT OR Apache-2.0 |
| equivalent | 1.0.2 | Apache-2.0 OR MIT |
| fluent-bundle | 0.16.0 | Apache-2.0 OR MIT |
| fluent-langneg | 0.13.1 | Apache-2.0 OR MIT |
| fluent-syntax | 0.12.0 | Apache-2.0 OR MIT |
| hashbrown | 0.17.1 | MIT OR Apache-2.0 |
| indexmap | 2.14.2 | Apache-2.0 OR MIT |
| intl-memoizer | 0.5.3 | Apache-2.0 OR MIT |
| intl_pluralrules | 7.0.2 | Apache-2.0/MIT |
| itoa | 1.0.18 | MIT OR Apache-2.0 |
| language-tags | 0.3.2 | MIT/Apache-2.0 |
| memchr | 2.8.3 | Unlicense OR MIT |
| proc-macro2 | 1.0.107 | MIT OR Apache-2.0 |
| quote | 1.0.47 | MIT OR Apache-2.0 |
| rustc-hash | 2.1.3 | Apache-2.0 OR MIT |
| self_cell | 1.3.0 | Apache-2.0 OR GPL-2.0-only |
| serde | 1.0.229 | MIT OR Apache-2.0 |
| serde_core | 1.0.229 | MIT OR Apache-2.0 |
| serde_derive | 1.0.229 | MIT OR Apache-2.0 |
| serde_json | 1.0.151 | MIT OR Apache-2.0 |
| serde_spanned | 0.6.9 | MIT OR Apache-2.0 |
| smallvec | 1.16.2 | MIT OR Apache-2.0 |
| stable_deref_trait | 1.2.1 | MIT OR Apache-2.0 |
| syn | 2.0.119 | MIT OR Apache-2.0 |
| syn | 3.0.6 | MIT OR Apache-2.0 |
| synstructure | 0.14.0 | MIT |
| sys-locale | 0.3.2 | MIT OR Apache-2.0 |
| thiserror | 2.0.21 | MIT OR Apache-2.0 |
| thiserror-impl | 2.0.21 | MIT OR Apache-2.0 |
| tinystr | 0.8.4 | Unicode-3.0 |
| toml | 0.8.23 | MIT OR Apache-2.0 |
| toml_datetime | 0.6.11 | MIT OR Apache-2.0 |
| toml_edit | 0.22.27 | MIT OR Apache-2.0 |
| toml_write | 0.1.2 | MIT OR Apache-2.0 |
| type-map | 0.5.1 | MIT/Apache-2.0 |
| unic-langid | 0.9.6 | MIT OR Apache-2.0 |
| unic-langid-impl | 0.9.6 | MIT OR Apache-2.0 |
| unicode-ident | 1.0.26 | (MIT OR Apache-2.0) AND Unicode-3.0 |
| winnow | 0.7.15 | MIT |
| yoke | 0.8.3 | Unicode-3.0 |
| yoke-derive | 0.8.3 | Unicode-3.0 |
| zerofrom | 0.1.8 | Unicode-3.0 |
| zerofrom-derive | 0.1.8 | Unicode-3.0 |
| zerovec | 0.11.8 | Unicode-3.0 |
| zerovec-derive | 0.11.6 | Unicode-3.0 |
| zmij | 1.0.23 | MIT |

`self_cell` 提供 `Apache-2.0 OR GPL-2.0-only` 双许可，本工程可沿 Apache-2.0 许可路径使用。其余声明为 MIT、Apache-2.0 或双许可；分发时需随正式包保留所选许可要求的通知。没有新增专有外部字体；本阶段继续只读复用系统 CJK 字体。

Windows/Linux仍需各自目标构建及平台验收；上述平台过滤不代替目标端原生测试。
