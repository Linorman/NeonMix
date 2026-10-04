[English](../architecture.md) | 日本語

# アーキテクチャ

本ドキュメントは計画中のアーキテクチャを記述したものです。実装はまだ
開始していません。設計レビューを経て詳細は変わる可能性があります。

## モジュール構成

```
tympan-aspl/
├── src/
│   ├── lib.rs            # 再エクスポート。公開 API 表面
│   ├── driver.rs         # Driver トレイト、プラグインのエントリポイント
│   ├── device.rs         # Device の抽象化
│   ├── stream.rs         # Stream の抽象化
│   ├── property.rs       # プロパティレジストリとディスパッチ
│   ├── io.rs             # IOProc 登録、高レベルラッパー
│   ├── raw/              # 低レベル: vtable、C ABI ブリッジ、FFI
│   │   ├── mod.rs
│   │   ├── vtable.rs     # CFPlugIn IUnknown vtable の構築
│   │   ├── selectors.rs  # プロパティセレクタ定数
│   │   └── host.rs       # ホストインタフェース (coreaudiod 側)
│   ├── realtime/         # リアルタイムセーフなプリミティブ
│   │   ├── mod.rs
│   │   ├── context.rs    # RealtimeContext マーカ型
│   │   ├── ring.rs       # ロックフリー SPSC リングバッファ
│   │   └── state.rs      # アトミック状態機械のヘルパー
│   └── bundle/           # .driver バンドルのパッケージングヘルパー
│       ├── mod.rs
│       └── plist.rs      # Info.plist の生成
├── examples/
│   └── minimal-loopback/  # 最小の仮想ループバックデバイス
└── tests/
    └── ...               # 可能な範囲での結合テスト
```

## レイヤモデル

モジュール境界によって分離された、概念上の 3 レイヤです。

### レイヤ 1: `raw` ― unsafe な FFI

- `unsafe extern "C"` 宣言の唯一の所有者
- `coreaudio-sys` および `core-foundation-sys` の唯一の利用者
- CFPlugIn IUnknown の vtable を Rust の関数ポインタにマップする
- 生の型エイリアス (例: `RawObjectID = AudioObjectID`) を提供する

`tympan-aspl` の利用者がこのモジュールに触れる必要はありません。フレーム
ワーク内部で使うため、または高レベル抽象を回避する必要のある上級利用者の
ために存在します。

### レイヤ 2: `realtime` ― ゼロアロケーションのプリミティブ

- アロケータを使用しない
- `std::sync::Mutex` も `std::collections::HashMap` も使用しない
- ロックフリー SPSC / MPMC リングバッファ (`crossbeam-utils` ベース)
- ドライバライフサイクルのためのアトミック状態機械
- 次のような `RealtimeContext` ゼロサイズマーカ:
  - `IOProc` から呼び出して安全な関数の引数として要求される
  - フレームワーク外部からは構築できない
  - リアルタイム安全性のコンパイル時の証拠として機能する

このレイヤの不変条件: `IOProc` から到達可能な任意の関数は
`&RealtimeContext` を受け取り、ヒープ操作を含まないこと。

### レイヤ 3: 公開 API ― 安全かつイディオマティック

- `Driver`、`Device`、`Stream` トレイト
- ビルダーパターンのコンストラクタ
- 可逆性のある失敗には Result 型を使用
- プラグイン状態へのライフタイム束縛参照

このレイヤが、利用者の 95% が触れる層となります。

## コアの抽象

### `Driver`

利用者が実装する最上位のトレイトです。AudioServerPlugIn オブジェクトに
対応します。

```text
trait Driver {
    fn devices(&self) -> &[Device];
    fn create_device(&mut self, spec: DeviceSpec) -> Result<DeviceId>;
    fn on_property_changed(&mut self, address: PropertyAddress);
    // …
}
```

フレームワークはプラグインのエントリポイントをマクロとして提供します。

```text
tympan_aspl::plugin_entry!(MyDriver);
```

これは `coreaudiod` から呼び出される `#[no_mangle] extern "C" fn` のプラ
グインファクトリへと展開されます。

### `Device`

ドライバが公開する個々のオーディオデバイス。1 つ以上のストリーム (入力 /
出力) を持ちます。

```text
struct Device {
    uid: Uid,
    streams: Vec<Stream>,
    // …
}
```

プロパティディスパッチはフレームワークが処理します。カスタムプロパティは
`Property` トレイトを介して登録できます。

### `Stream`

デバイス上のオーディオ流路の片方向。入力ストリームではサンプルを供給し、
出力ストリームでは受け取ります。

I/O エントリポイントは利用者の `process_io` メソッドで、リアルタイム
スレッド上で `RealtimeContext` とともに呼び出されます。

```text
fn process_io(
    &mut self,
    rt: &RealtimeContext,
    timestamp: AudioTimeStamp,
    input: &[Sample],
    output: &mut [Sample],
);
```

### `RealtimeContext`

型レベルでの中心的な保証です。インスタンスはフレームワークの I/O ハーネス
から利用者コードに参照渡しされます。フィールドを持たず、利用者コードから
構築する手段もありません。

ヒープ確保、ロック、システムサービス呼び出しを行う関数は
`&RealtimeContext` を引数に取りません。これにより、リアルタイム非対応の
コードが `process_io` から呼び出された場合にコンパイルエラーとなります。

## 横断的関心事

### CoreFoundation との相互運用

AudioServerPlugin API はプロパティ値として `CFStringRef`、`CFArrayRef` 等
を用います。`tympan-aspl` は `core-foundation` (生の `core-foundation-sys`
ではなく) を介して、これらを安全な Rust 型でラップします。

- `CFString` ⇔ `&str` / `String` の変換
- ライフタイム束縛された `&CFArray<T>` ビュー
- Rust の drop セマンティクスによる自動 `CFRetain` / `CFRelease`

### プロパティプロトコル

CoreAudio のプロパティディスパッチは AudioServerPlugin ボイラープレートの
大部分を占めます。フレームワークは標準プロパティ (サンプルレート、フォー
マット、レイテンシ、音量、ミュート、名前など) を自動で処理します。カスタ
ムプロパティは次のように追加できます。

```text
impl Property for MyCustomProperty {
    const SELECTOR: PropertySelector = /* … */;
    type Value = f64;
    fn read(&self, ctx: &PropertyContext) -> Self::Value { /* … */ }
    fn write(&mut self, ctx: &PropertyContext, val: Self::Value) -> Result<()> { /* … */ }
}
```

### ロギング

リアルタイムコードは `tracing` や `log` 経由でログを出力できません (どち
らもアロケーションを行います)。`realtime` モジュールは、`IOProc` から
診断イベントを取り込むためのロックフリーなログキューを提供します。別途
非リアルタイムスレッドがキューをドレインし、エントリを標準の `log`
クレートへと転送します。

## 未解決の問い

実装に持ち越された問いの大半は、その後決着し `docs/decisions/` の
ADR に記録されました。コードの暫定的な答えが、いまや文書化された
答えです。

- [x] `Driver` をオブジェクト安全なトレイトにすべきか、それとも汎用
  パラメータにすべきか? — 汎用とし、FFI 境界で一度だけ
  `dyn AnyDriver` へ型消去する。
  [ADR 0002](../decisions/0002-driver-trait-type-erasure.md) を参照。
- [x] `RealtimeContext` の区別はどの程度の粒度で行うべきか? — 単一
  マーカとする。
  [ADR 0003](../decisions/0003-single-realtime-context.md) を参照。
- [ ] サポートする macOS の最低バージョンは? AudioServerPlugin は
  macOS 10.10 で大きく成熟し、12.0 でさらに成熟しました。未解決 —
  CI は `macos-15` で動作しており、下限はまだ確約されていません。
- [x] フレームワークは AudioDriverKit (macOS 11 以降の DriverKit
  ベースドライバ) とどう相互作用するか? — スコープ外とする。
  [ADR 0004](../decisions/0004-audiodriverkit-out-of-scope.md) を参照。
