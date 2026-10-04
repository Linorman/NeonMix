[English](../plugin-author-guide.md) | 日本語

# プラグイン作者ガイド

このガイドは、`tympan-aspl` の上に macOS AudioServerPlugin を書く際の
非自明な要点 — モジュールの docstring、ADR、サンプルの README に散在して
いる事柄 — を一箇所にまとめたものです。動作する出発点としては
[`examples/minimal-loopback/`](../../examples/minimal-loopback/)（最小限の
ドライバ）を、インスタンスごとの設定と方向別処理については
[`examples/gain/`](../../examples/gain/) を、インスタンスごとの本格的な
処理状態については [`examples/lowpass/`](../../examples/lowpass/) を参照して
ください。

## プロジェクトのセットアップ

ドライバは本フレームワークに依存するクレートで、`.driver` バンドル内の
ロード可能なバイナリである `cdylib` としてビルドします。`Driver` 実装を
どのホストでもユニットテストできるよう、クレートタイプには `rlib` も
残しておきます。最小限の `Cargo.toml`:

```toml
[package]
name = "my-driver"
version = "0.1.0"
edition = "2021"
rust-version = "1.80"

[lib]
crate-type = ["rlib", "cdylib"]

[dependencies]
tympan-aspl = { git = "https://github.com/penta2himajin/tympan-aspl" }

[profile.release]
panic = "abort"      # 下記「パニック戦略」を参照
lto = "thin"         # .dylib が小さくなる。コード生成上の利得はわずか
codegen-units = 1
```

フレームワーククレート自身の依存は小さなものが 1 つ
（`crossbeam-utils`）だけなので、追加しても依存ツリーは浅いままです。

## パニック戦略

`coreaudiod` は C のコードであり、Rust の `panic!()` が C ABI 境界を越えて
巻き戻る（unwind）ことは **未定義動作** です。

フレームワークの `extern "C"` エントリポイントのうち *ライフサイクル*
メソッド — `Initialize`、`StartIO`、`StopIO` — は、あなたのコードを
[`catch_unwind`] で包みます。`Driver::initialize`、`Driver::start_io`、
`Driver::stop_io` から出たパニックは捕捉され、HAL には
`OsStatus::UNSPECIFIED` として報告され、ABI を越えて巻き戻ることは
ありません。

**`Driver::process_io` は包まれません。** これはリアルタイム IO スレッド
で実行され、毎サイクルの `catch_unwind` は許容できないオーバーヘッドに
なります。したがって `process_io` でのパニックは C ABI を越えて巻き戻り
*ます*。そのため:

1. リリースプロファイルで `panic = "abort"` を設定してください。これに
   よりドライバ内のどこでパニックが起きても即座にプロセス abort となり、
   巻き戻りも ABI 越えの未定義動作もなくなり、バイナリも小さくなります。
2. `process_io` で決して `panic!` しないこと。`assert!`、`unwrap`、
   デバッグビルドでの整数オーバーフロー、範囲外のスライス添字 — これらは
   すべてパニックします。明示的な境界チェック（どのサンプルも
   `buffer.output.len().min(buffer.input.len())` でクランプしています）と、
   クラッシュの代わりに診断を記録する
   [`LogSink`](#診断イベント-logsink) パターンを使ってください。
3. `Driver::initialize` / `Driver::start_io` の失敗は第一級です。
   `Err(OsStatus::...)` を返せば、フレームワークはパニックせずに HAL へ
   報告します。

[`catch_unwind`]: https://doc.rust-lang.org/std/panic/fn.catch_unwind.html

## ドライバの同一性: デバイス UID とファクトリ UUID

2 つの識別子がドライバの安定 ABI の一部です。リリース後にどちらかを
変更すると、静かに何かが壊れます。

- **デバイス UID** — `DeviceSpec::new` の第 1 引数であり、バンドルの
  `CFBundleIdentifier` でもあります。macOS はデバイスごとのユーザー設定
  （選択されたサンプルレート、音量、既定デバイスかどうか）をこの文字列を
  キーに保持します。変更すると、そのデバイスの保存済み設定はすべて
  孤立します。逆 DNS 形式（例: `com.example.MyDriver`）を使ってください。
- **ファクトリ UUID** — `Info.plist` の `CFPlugInFactories` 辞書のキーで
  あり、`CFPlugInTypes` 配列内の値でもあります。ドライバ固有でなければ
  なりません。`uuidgen` で一度だけ新しい UUID を生成し、決して変更しない
  でください。サンプルでは `BundleConfig` に渡す `FACTORY_UUID` 定数として
  公開しています。

`Driver` トレイトの `NAME` / `MANUFACTURER` / `VERSION` 関連定数は
リリース間で自由に変更できます — これらは表示用文字列であって同一性では
ありません。

## シンボルの可視性

フレームワークの [`plugin_entry!`](../../src/macros.rs) マクロは
`#[no_mangle] pub unsafe extern "C" fn TympanAsplDriverFactory` を生成
します（名前はマクロの第 2 引数で上書き可能）。これが `coreaudiod` が
解決する唯一のシンボルです — バンドル `Info.plist` の
`CFPlugInFactories` 辞書で名指しされるので、マクロの引数と `BundleConfig`
のファクトリ関数引数は一致していなければなりません。

Rust の `cdylib` は `#[no_mangle] extern "C"` 項目はエクスポートしますが、
通常の Rust の公開項目はエクスポートしないので、`.dylib` から他に漏れ出る
ものはありません。Tier 2 CI はファクトリシンボルが存在しマングルされて
いないことを表明します:

```sh
nm -gU target/release/libmy_driver.dylib | grep -E ' _TympanAsplDriverFactory$'
```

`plugin_entry!` はクレートルートで一度だけ呼び出してください — `#[no_mangle]`
シンボルを生成するので、リンクツリー内で一意でなければなりません。

## `.driver` バンドル

`coreaudiod` が走査するロード可能な成果物は `.driver` バンドル — コミット
された `Info.plist` とビルドした `cdylib` を、CFBundle レイアウトに
収めたものです:

```text
MyDriver.driver/
└── Contents/
    ├── Info.plist
    └── MacOS/
        └── MyDriver        ← ビルドした cdylib をリネームしたもの
```

`Info.plist` を手書きしないでください。`BundleConfig` から
`bundle::plist::generate(&config)` で生成し、結果をクレートの隣に
コミットします。コミットしたファイルとジェネレータがずれないよう、
ユニットテストで固定します — `gain` と `lowpass` のサンプルはどちらも
これを持っています:

```rust
#[test]
fn committed_info_plist_matches_the_generator() {
    assert_eq!(
        tympan_aspl::bundle::plist::generate(&MyDriver::bundle_config()),
        include_str!("../Info.plist"),
    );
}
```

## リアルタイム経路の規律

`Driver::process_io` は `coreaudiod` のリアルタイム IO スレッドで実行
されます。そこから到達可能なコードは次をしてはいけません:

1. **アロケーション。** すべてのバッファを `Driver::start_io`（そこでの
   アロケーションは許可されます）または `Driver::new` で事前確保して
   ください。`lowpass` のサンプルはフィルタメモリを固定長配列として
   保持し、`start_io` でリセットします。
2. **`std::sync::Mutex` の取得。** アトミック、フレームワークの
   [ロックフリー SPSC リング](../../src/realtime/ring.rs)、または
   ドライバが排他的に所有するインスタンスごとの状態（`process_io` は
   `&mut self` を取ります）を使ってください。
3. **ブロッキングシステムコール** — `println!` も、ファイル I/O も、
   `std::process` も不可。診断には
   [`LogSink`](#診断イベント-logsink) を使ってください。
4. **スレッドの spawn / join。** `initialize` と `start_io` ではそれが
   可能です。`process_io` では不可です。

`process_io` への `&RealtimeContext` 引数はこれのコンパイル時の証拠です。
アロケートやブロックを行うフレームワーク関数は `&RealtimeContext` を
受け取らないので、`process_io` からそれを呼ぶと型エラーになります。
*すべて* を捕捉できるわけではない（素の `Vec::new()` は依然コンパイル
できます）ので、CI も機械的にこの不変条件を強制します。Tier 1 は
`tests/realtime_safety.rs` と `tests/raw_lifecycle.rs` を実行し、どちらも
IO 経路を `assert_no_alloc` グローバルアロケータガードの中で駆動し、
あらゆるアロケーションで abort します。このパターンを自分のクレートでも
真似て、保証を広げてください — テスト用の足場は約 40 行です。

## 方向別処理

ループバックデバイス — 入力ストリームと出力ストリームの両方を持つもの —
は `process_io` が **2 つ** の IO 操作で呼ばれます。`WriteMix`（クライ
アントからデバイスに入ってくるオーディオ）と `ReadInput`（クライアントへ
出ていくオーディオ）です。無条件に適用される変換は、ループバックを
またいで *2 回* 適用されてしまいます。

`IoBuffer::operation` で分岐し、処理をちょうど 1 回だけ適用してください。
サンプルが従う慣習は `WriteMix` で処理する — 「デバイスは入ってくる
オーディオを変換する」 — こと、そして `ReadInput` では素通しすることです:

```rust
fn process_io(&mut self, _rt: &RealtimeContext, buffer: &mut IoBuffer<'_>) {
    let n = buffer.output.len().min(buffer.input.len());
    match buffer.operation {
        IoOperation::WRITE_MIX => { /* あなたの DSP、input → output */ }
        _ => buffer.output[..n].copy_from_slice(&buffer.input[..n]),
    }
    buffer.output[n..].fill(0.0);
}
```

完全なパターンは [`examples/gain/`](../../examples/gain/) と
[`examples/lowpass/`](../../examples/lowpass/) を参照してください。

## 診断イベント: `LogSink`

`process_io` から診断 — 「IO サイクルが想定より大きい」「値をクランプ
した」「予期しない操作」 — を出したいときは、
[`LogSink`](../../src/realtime/log.rs) にプッシュします。`LogSink::log`
は単一のロックフリー try-push です。2 つのアトミック操作と `T` サイズの
memcpy だけで、アロケーションもシステムコールもありません。小さな `Copy`
な enum が標準的なイベント形です:

```rust
#[derive(Debug, Clone, Copy)]
enum MyLogEvent {
    CycleClamped { requested: u32 },
}
```

シンクは `Driver::initialize` で構築し（そこでアロケートし、オフスレッド
のドレイナをスポーンします）、ドライバオブジェクトが解放されるときに
drop させます。ドレイナはあなたの `drain_one` クロージャをリアルタイム
スレッドの外で実行します。そのクロージャがパニックしてもドレイナスレッド
が終わるだけで `process_io` は動き続けます — 失敗するログシンクが
オーディオをクラッシュさせることはありません。

## HAL へのインストール

`coreaudiod` は `/Library/Audio/Plug-Ins/HAL/` を `.driver` バンドルに
ついて走査します。ビルド後:

```sh
cargo build --release -p my-driver

BUNDLE=MyDriver.driver
mkdir -p "$BUNDLE/Contents/MacOS"
cp Info.plist "$BUNDLE/Contents/Info.plist"
cp target/release/libmy_driver.dylib "$BUNDLE/Contents/MacOS/MyDriver"
codesign --force --sign - "$BUNDLE"

sudo cp -R "$BUNDLE" /Library/Audio/Plug-Ins/HAL/
sudo killall coreaudiod
```

**macOS 15 ではコード署名は省略できません。** ad-hoc 署名（`--sign -`）は
`coreaudiod` がバンドルを *発見してロードを試みる* には十分ですが、
アウトオブプロセスの Core Audio Driver Service ヘルパーが、デバイスが
列挙される前に AMFI 層でそれを拒否します
（`AppleMobileFileIntegrityError -423`）。デバイスが実際に *システム設定 ▸
サウンド* に現れるには **Developer ID Application** 署名が必要です。
制約の全容と CI への影響については
[`docs/testing.md`](testing.md) の「System Integrity Protection と AMFI の
考慮事項」を参照してください。

## よくある落とし穴

| 落とし穴 | 症状 | 対処 |
|---|---|---|
| `crate-type = ["cdylib"]` の欠落 | `lib*.dylib` が生成されず、バンドルに入れるものがない | `[lib] crate-type = ["rlib", "cdylib"]` を追加 |
| `Info.plist` のファクトリ UUID と `BundleConfig` のファクトリ名の不一致 | `coreaudiod` はバンドルを見つけるがインスタンス化できない | plist を同じ `BundleConfig` から生成し、`committed_info_plist_matches_the_generator` テストで固定する |
| `process_io` で変換を無条件に適用 | ループバックをまたいで効果が 2 回適用される（例: ゲイン²） | `IoBuffer::operation` で分岐し、`WriteMix` でのみ処理する |
| `process_io` での `println!` / `dbg!` / `unwrap` | `assert_no_alloc` テストが abort、または未定義動作・オーディオのグリッチ | `LogSink` を使う。`unwrap` を明示的なハンドリングに置き換える |
| `panic = "abort"` なしで `process_io` から到達可能な `panic!` | 未定義動作 — C ABI をまたぐ巻き戻り | `panic = "abort"` を設定。IO 経路で決してパニックしない |
| `process_io` でのアロケーション | `assert_no_alloc` ハーネスが abort | `start_io` または `new` で事前確保し、構造体に保持する |
| リリース間でのデバイス UID の変更 | 保存済みのデバイスごと設定が孤立する | UID は一度決めたら変えない。違いを示したいなら `NAME` を変える |
| `process_io` でのスレッド spawn | オーディオのドロップアウト | `initialize` でスポーンする（例: `LogSink::new` 経由）。`process_io` からは決してしない |

## さらに読むには

- [`docs/overview.md`](overview.md) — プロジェクトのスコープと目標。
- [`docs/architecture.md`](architecture.md) — 内部モジュール構成と
  レイヤモデル。
- [`docs/testing.md`](testing.md) — 階層化された CI 戦略と macOS の
  コード署名制約。
- [`docs/decisions/`](../decisions/) — フレームワークの重要な決定を扱う
  ADR 群。
- [`examples/`](../../examples/) 以下のサンプルクレート — ここで述べた
  パターンの完全な動作ドライバ。
