[English](README.md) | 日本語

# tympan-aspl

macOS AudioServerPlugin を Rust で記述するためのフレームワーク。

`tympan-aspl` は Core Audio HAL の AudioServerPlugin インタフェースに対する
Rust の抽象化を提供し、C++ や Objective-C を書くことなく、Rust アプリケーション
から仮想オーディオデバイスやオーディオドライバを macOS 上で実装できるようにします。

## ステータス

**意図した機能は実装完了。**
[`docs/ja/overview.md`](docs/ja/overview.md) の「In scope」項目はすべて実装され、
[ADR 0001](docs/decisions/0001-ci-verification-strategy.md) の Tier 4 以外の
項目はすべて CI に組み込まれています。

### フレームワーク

- Layer 1 — 低レベル AudioServerPlugin FFI ([`src/raw/`](src/raw/))：
  `extern "C"` の CFPlugIn ファクトリと IUnknown vtable
  ([`raw::entry`](src/raw/entry.rs)、
  [`raw::vtable`](src/raw/vtable.rs))、
  コンパイル時 `static_assertions` を備えた C-ABI 構造体ミラー
  ([`raw::abi`](src/raw/abi.rs))、プロパティのマーシャリング
  ([`raw::marshal`](src/raw/marshal.rs))、CoreFoundation 結合
  ([`raw::cf`](src/raw/cf.rs)、
  [`raw::clock`](src/raw/clock.rs))。
- Layer 2 — リアルタイムプリミティブ
  ([`src/realtime/`](src/realtime/))：型レベルマーカー
  [`RealtimeContext`](src/realtime/context.rs)、
  ロックフリーの SPSC リング
  ([`realtime::ring`](src/realtime/ring.rs))、AudioServerPlugin の
  ライフサイクル原子状態 [`State`](src/realtime/state.rs)、
  CFPlugIn 用のウェイトフリー
  [`Refcount`](src/realtime/refcount.rs)、別スレッドへ流す
  [`log`](src/realtime/log.rs) シンク。
- Layer 3 — オブジェクト／プロパティ／ディスパッチのサーフェス
  ([`object`](src/object.rs)、[`property`](src/property.rs)、
  [`objects`](src/objects.rs)、
  [`dispatch`](src/dispatch.rs)) と公開 API：
  [`Driver`](src/driver.rs)、[`Device`](src/device.rs)、
  [`Stream`](src/stream.rs)、[`IoBuffer`](src/io.rs)、
  CFPlugIn ファクトリシンボルをエクスポートする宣言マクロ
  [`plugin_entry!`](src/macros.rs)。
- [`bundle`](src/bundle/) — `.driver` バンドルのレイアウトと
  `Info.plist` 生成。

### サンプル

[`examples/`](examples/) 配下に 3 つのリファレンスドライバ。
いずれも `.driver` cdylib としてビルドでき、CI で
alloc-free 不変条件がピン留めされています。

- [`minimal-loopback/`](examples/minimal-loopback/) — ステートレスな
  ステレオ仮想ループバック。最小構成のドライバ。
- [`gain/`](examples/gain/) — `WriteMix` 方向のみに固定線形ゲインを
  かける例。インスタンスごとの設定値を持つテンプレート。
- [`lowpass/`](examples/lowpass/) — チャンネルごとのフィルタメモリを
  持つ 1 極ローパス。インスタンス処理状態を持ち、`start_io` で
  リセットする例。

### CI

| Tier | チェック内容 | トリガ |
|---|---|---|
| 1 | `cargo fmt --check`、`cargo clippy --workspace --all-targets --all-features -- -D warnings`、`macos-15` での `cargo build/test/doc`、`src/` 内の `static mut` grep、`cargo-deny` サプライチェーン検査 | 全 PR |
| 2 | コミット済み Info.plist と生成された Info.plist の `plutil -lint`、`nm -gU` ファクトリシンボル検証、`lipo -info` アーキ覆検査、`.driver` 組立て + ad-hoc `codesign --verify` | 全 PR |
| 3 | `coreaudiod` への HAL ロード：`.driver` を `/Library/Audio/Plug-Ins/HAL/` に配置 → `coreaudiod` を再起動 → 該当バンドルを discover してロードを *試みた* ことをログから確認。**device の enumeration はアサートしません**：macos-15 では AMFI が ad-hoc 署名を `AppleMobileFileIntegrityError -423` で拒否します。Developer ID 署名があればこの制限は外せます（ADR 0001 § Trigger for revisiting 参照） | `main` へのマージ、毎日、手動 |
| 3 ASan | `raw_lifecycle`、`realtime_safety` の in-process FFI ハーネスと `raw::*` のユニットテストを `-Zsanitizer=address` + nightly + `-Z build-std` で再実行 | 毎日、手動 |
| 3 TSan | `realtime::ring` の `cross_thread_push_pop_preserves_order`（10 万要素の並行交換）と `realtime::*` ユニットテストを `-Zsanitizer=thread` で再実行 | 毎日、手動 |

ホスト型 CI ではプラグインを *完了* まで動かせません：macOS 15 の
`amfid` が ad-hoc 署名された HAL バンドルをコード実行前に弾きますし、
ランナーには物理オーディオハードウェアが露出していません。
したがって device-enumeration と audio-data-path の検証は
[`docs/testing.md`](docs/testing.md) の Tier 4（リリース前の手動
／セルフホストのチェックリスト）に置いてあります。ただし
in-process の `raw_lifecycle` ハーネスは実際の
`AudioServerPlugInDriverInterface` vtable を end-to-end で叩いて
くれるので、FFI 層自体は機械的に実行されます — `coreaudiod`
自身に渡さないだけです。

このフレームワークは実用可能です：`cdylib` クレートに
`impl Driver for MyDriver` と
`tympan_aspl::plugin_entry!(MyDriver)` を書けば、Developer ID 署名後に
`coreaudiod` へロードできる `.driver` バンドルが得られます。
最小レシピは
[`examples/minimal-loopback/`](examples/minimal-loopback/) を、
実践レシピ（同一性、パッケージング、リアルタイム経路のデバッグ、
落とし穴）は
[`docs/ja/plugin-author-guide.md`](docs/ja/plugin-author-guide.md) を
参照してください。

### 今後の作業

完全な Tier 4 検証への道筋は
[ADR 0001 § Trigger for revisiting](docs/decisions/0001-ci-verification-strategy.md) に
記載しています。次のステップは Developer ID 署名鍵を GitHub Secret
として Tier 3 ワークフローに接続することで、これにより
device-enumeration を Tier 4 からホスト CI 側に戻せます
（コード変更不要）。audio-data-path I/O 検証はランナーに物理オーディオが
無いため Tier 4 のままで、セルフホストランナーかローカルマシンが必要です。

タグプッシュで `cargo publish --dry-run` を走らせる `release.yml`
ワークフローは別途整備予定です。

## 名前について

*Tympan* は蛾の鼓膜器官 (tympanal organ) を指します。これはメイガ科やヤガ科
などの蛾の腹部にある膜ベースの超音波センサで、コウモリの反響定位を検知する
ために進化しました。この名称はライブラリの役割を反映しています ― OS の
オーディオエンジンとユーザー空間の Rust コードとの間に介在する、薄い膜です。

## ドライバ作者向けクイックスタート

このフレームワークに依存する新しい `cdylib` クレートを作り、
[`Driver`](src/driver.rs) を実装して
[`plugin_entry!`](src/macros.rs) を呼び出します。

```toml
# Cargo.toml
[package]
name = "my-driver"
version = "0.1.0"
edition = "2021"

[lib]
crate-type = ["rlib", "cdylib"]

[dependencies]
tympan-aspl = "0.1"
```

```rust
// src/lib.rs
use tympan_aspl::{
    plugin_entry, DeviceSpec, Driver, IoBuffer, RealtimeContext,
    StreamFormat, StreamSpec,
};

pub struct MyDriver;

impl Driver for MyDriver {
    const NAME: &'static str = "My Driver";
    const MANUFACTURER: &'static str = "your name";
    const VERSION: &'static str = "0.1.0";

    fn new() -> Self {
        Self
    }

    fn device(&self) -> DeviceSpec {
        let format = StreamFormat::float32(48_000.0, 2);
        DeviceSpec::new("com.example.mydriver", "My Driver", Self::MANUFACTURER)
            .with_sample_rate(48_000.0)
            .with_input(StreamSpec::input(format))
            .with_output(StreamSpec::output(format))
    }

    fn process_io(&mut self, _rt: &RealtimeContext, buffer: &mut IoBuffer<'_>) {
        // alloc-free / lock-free の経路。
        let n = buffer.output.len().min(buffer.input.len());
        buffer.output[..n].copy_from_slice(&buffer.input[..n]);
        buffer.output[n..].fill(0.0);
    }
}

plugin_entry!(MyDriver);
```

`cargo build --release` でビルドし、コミット済みの `Info.plist` と
ビルド済み cdylib を `Contents/MacOS/` に置いた `.driver` バンドルを
組立てて `/Library/Audio/Plug-Ins/HAL/` 配下にインストールします。
SIP を切ったローカル開発機ならディスクに置くだけなら ad-hoc 署名で
事足りますが、本番の macOS 15 では `coreaudiod` ヘルパーが
Developer ID 署名を要求するので、ロード完了まで進めるには Developer ID
署名が必要です。パッケージング手順は
[`docs/ja/plugin-author-guide.md`](docs/ja/plugin-author-guide.md) に、
完全なクレート形態は
[`examples/minimal-loopback/`](examples/minimal-loopback/) に
あります。

## 開発

このプロジェクトの CI 階層（上記）は
[`.github/workflows/tier1.yml`](.github/workflows/tier1.yml)、
[`tier2.yml`](.github/workflows/tier2.yml)、
[`tier3.yml`](.github/workflows/tier3.yml)、
[`tier3-asan.yml`](.github/workflows/tier3-asan.yml)、
[`tier3-tsan.yml`](.github/workflows/tier3-tsan.yml) に分割されており、
階層化された検証戦略の根拠は
[ADR 0001](docs/decisions/0001-ci-verification-strategy.md) に
記録されています。

ローカルでも `git push` 前に同じ fmt と clippy のチェックを走らせるには、
リポジトリの pre-push フックをオプトインします。

```sh
git config core.hooksPath .githooks
```

フック本体は [`.githooks/pre-push`](.githooks/pre-push) です。push する
範囲に `*.rs` / `Cargo.toml` / `Cargo.lock` の変更が含まれていなければ
no-op なので、ドキュメントのみの push が遅くなることはありません。
単発で迂回するには `git push --no-verify` を使ってください。

## ライセンス

以下のいずれかの下でライセンスされています。

- Apache License, Version 2.0 ([LICENSE-APACHE](LICENSE-APACHE) もしくは
  <http://www.apache.org/licenses/LICENSE-2.0>)
- MIT license ([LICENSE-MIT](LICENSE-MIT) もしくは
  <http://opensource.org/licenses/MIT>)

利用者の選択により、いずれを選んでも構いません。

### コントリビューション

明示的に別段の表明がない限り、Apache-2.0 ライセンスで定義される、本著作物への
組込みのために意図的に提出されたあらゆるコントリビューションは、追加の条項や
条件を伴うことなく、上記のとおりデュアルライセンスとなります。

## ドキュメント

| ドキュメント | 内容 |
|---|---|
| [`docs/ja/overview.md`](docs/ja/overview.md) | プロジェクトの目的、スコープ、既存実装との比較 |
| [`docs/ja/architecture.md`](docs/ja/architecture.md) | API 設計とモジュール構成 |
| [`docs/ja/plugin-author-guide.md`](docs/ja/plugin-author-guide.md) | ドライバの書き方: セットアップ、同一性、リアルタイム経路、パッケージング、落とし穴 |
| [`docs/ja/testing.md`](docs/ja/testing.md) | 階層化されたテストおよび CI 戦略 |
| [`docs/ja/references.md`](docs/ja/references.md) | Apple のドキュメント、先行実装、関連クレート |
| [`docs/decisions/`](docs/decisions/) | アーキテクチャ意思決定の記録 (ADR) |
| [`docs/ja/handoff-protocol.md`](docs/ja/handoff-protocol.md) | 長期作業のためのセッション引き継ぎプロトコル |

## サンプル

| サンプル | 説明 |
|---|---|
| [`examples/minimal-loopback/`](examples/minimal-loopback/) | ステートレスなステレオ仮想ループバック。最小構成のフレームワーク利用例。CI で alloc-free をピン留め。 |
| [`examples/gain/`](examples/gain/) | `WriteMix` 方向に固定線形ゲインをかける例。インスタンス設定と `IoOperation` 分岐を示し、ループバック往復で 1 回だけゲインが適用されるようになっている。 |
| [`examples/lowpass/`](examples/lowpass/) | チャンネルごとのフィルタメモリを持つ 1 極ローパス。インスタンス処理状態、`start_io` でのリセット、加減算積算によるリアルタイム再帰式を示す。 |
