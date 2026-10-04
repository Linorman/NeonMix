[English](../overview.md) | 日本語

# 概要

## 目的

`tympan-aspl` は、macOS の **AudioServerPlugin** ― `coreaudiod` 内部で
動作し、システムからはオーディオデバイスとして見えるユーザー空間のオーディオ
ドライバ ― を実装するための Rust フレームワークです。

このライブラリの目的は、Rust アプリケーションから次のようなことを可能にする
ことです。

- 仮想オーディオデバイス (入力・出力) の実装
- カスタムオーディオルーティングや処理ドライバの実装
- ノイズ抑圧、音声エフェクト、オーディオブリッジングプラグインの構築

… これらを、これまで必要とされてきた C++ や Objective-C のコードを書くこと
なく実現します。

## 存在意義

AudioServerPlugin API は C ヘッダ (`<CoreAudio/AudioServerPlugIn.h>`) で
ドキュメント化されています。Apple 公式の Rust バインディングやフレームワーク
は存在しません。既存の AudioServerPlugin 開発手段は次のとおりです。

| アプローチ | 言語 | 成熟度 | トレードオフ |
|---|---|---|---|
| Apple のサンプルコード (`SimpleAudioDriver`) | C++ | Apple 公式 | リファレンスのみ。フレームワークではない |
| **libASPL** (gavv/libASPL) | C++17 | プロダクション品質 | C++ 限定。Rust への道筋はない |
| **BlackHole** | C / Objective-C | プロダクション品質 | ループバック専用。フレームワークではない |
| **BackgroundMusic** | C++ / Objective-C | プロダクション品質 | 特定アプリケーション向け |
| Rust からの手書き FFI | Rust + unsafe FFI | 公開実装なし | 各利用者が車輪を再発明する |

このフレームワークは Rust に存在する空隙を埋めるものです。libASPL と同じ
ニッチ ― ただし Rust 利用者向け ― を占めることを意図しています。アプリ
ケーション固有のドライバを構築するための、汎用的で再利用可能な基盤となる
ことを目指します。

## スコープ

### スコープ内

- AudioServerPlugin のエントリポイントと CFPlugIn vtable の接続
- オブジェクト階層: Driver, Device, Stream, Box, Plug-In
- プロパティレジストリとディスパッチ (CoreAudio プロパティプロトコル)
- I/O サイクル: `StartIO`, `StopIO`, `BeginIOOperation`, `EndIOOperation`,
  `WillDoIOOperation`、リアルタイム read/write コールバック
- リアルタイムセーフなプリミティブ (ロックフリーリングバッファ、アトミック
  状態機械)
- `.driver` パッケージングのためのバンドルレイアウトと Info.plist 生成
  ヘルパー
- 例: 最小限の仮想ループバックデバイス

### スコープ外

- 信号処理アルゴリズム (DSP、ML、コーデック) ― これらは `tympan-aspl`
  に依存する利用側クレートに属します
- GUI / 環境設定ペイン
- アプリケーションコード
- iOS や DriverKit (別の関心事。将来的に姉妹クレートとなる可能性はあり)
- macOS 以外のオーディオバックエンド (Linux ALSA/PipeWire、Windows
  WASAPI/WDM)

## 名前について

*Tympan* は鼓膜器官 (tympanal organ) ― メイガ科をはじめとする蛾の腹部に
ある膜ベースの聴覚器官を指します。この器官はコウモリの反響定位への防御として
進化しました。超音波を捉え、付随する弦音受容器を介して振動を神経信号に変換
します。

類比は次のとおりです。

- 鼓膜器官は外界と蛾の神経系の間に位置し、ある物理ドメイン (空気圧) を
  別のドメイン (神経インパルス) に変換します。
- `tympan-aspl` は macOS オーディオエンジンとユーザー空間 Rust コードの
  間に位置し、あるプログラミングドメイン (C ABI、vtable、リアルタイム
  コールバック) を別のドメイン (安全な Rust 型、所有権、ライフタイム) に
  変換します。

二語目の `aspl` は、Apple のサンプルコードや既存ライブラリ (libASPL) で
慣習的に用いられている AudioServerPlugin の略称です。

## ステータス

**実装済み。** フレームワーククレートは `src/` 全体に実装されています
— `raw` FFI 層（CFPlugIn ファクトリ、vtable、エントリポイント、
マーシャリング）、安全なオブジェクト／プロパティ／ディスパッチ層、
`realtime` プリミティブ、`bundle` Info.plist ヘルパー — に加えて
`minimal-loopback` のリファレンス例があります。
[`testing.md`](testing.md) の CI tier（静的検証、バンドル／ABI 検証、
`coreaudiod` HAL ロード検証）が変更ごとに実行されます。

- API 設計は [`architecture.md`](architecture.md) にドキュメント化されて
  います
- 参考資料は [`references.md`](references.md) にまとめています
- 検証戦略は [`testing.md`](testing.md) および
  [`decisions/0001-ci-verification-strategy.md`](decisions/0001-ci-verification-strategy.md)
  にあります

## ターゲット利用者

- 仮想デバイスやカスタムドライバを必要とする macOS オーディオアプリケー
  ションを構築する Rust 開発者
- リアルタイム制約に習熟したオーディオプラグイン作者
- macOS のデバイス層と統合する必要があるオーディオ処理パイプラインを
  プロトタイプする研究者

次の用途には適しません。

- アプリケーションレベルのオーディオ再生 (`cpal`、`rodio`、`coreaudio-rs`
  を使用してください)
- DAW プラグインフォーマット (AU、VST3、AAX) ― これらはまったく別の API
  です

## 既存実装との比較

### coreaudio-rs との比較

`coreaudio-rs` は **Core Audio HAL クライアント API** ― 既存デバイスからの
読み書き、プロパティ問い合わせなど ― に対する Rust バインディングを提供
します。HAL が他のクライアントに提供する新しいデバイスを *実装* すること
はできません。`tympan-aspl` はその補集合 ― *ドライバ* 側の実装 ― に
あたります。

完全なオーディオアプリケーションでは両者を併用することがあります。仮想
デバイスを露出するために `tympan-aspl` を用い、利用者向けアプリで
`coreaudio-rs` (もしくは `cpal`) を使ってそのデバイスにレンダリングする、
といった具合です。

### libASPL との比較

`libASPL` (C++17) は AudioServerPlugin 開発のリファレンスフレームワーク
であり、よく設計されており実戦投入もされています。`tympan-aspl` は同等の
能力を Rust 利用者に提供することを目指しますが、以下の違いがあります。

- リソース寿命に Rust の所有権モデルを用いる (C++ の `shared_ptr` や
  CFRetain/CFRelease による手動参照カウントとは対照的)
- リアルタイム経路の保証を型レベルで強制する (`RealtimeContext` マーカ
  によって、I/O コールバックでの誤ったヒープ確保を防止する)
- Rust の慣習に従った API 名 (`snake_case`、可逆性のある失敗には
  `Result` 型)
- libASPL との C ABI 互換性は意図しない

### 手書き FFI との比較

`coreaudio-sys` (生の bindgen 生成バインディングを持つ) を経由して、Rust
開発者は誰でも AudioServerPlugin を直接呼び出すことができます。その結果、
ドライバごとに数百行の `unsafe` FFI が必要になります。`tympan-aspl` は
このボイラープレートを集約し、安全なデフォルトを提供します。
