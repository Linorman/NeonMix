[English](../references.md) | 日本語

# 参考資料

設計時に参照した資料一覧。

## Apple のドキュメント

- **AudioServerPlugIn.h** (CoreAudio フレームワークヘッダ)
  - macOS SDK 内の `<CoreAudio/AudioServerPlugIn.h>`
  - AudioServerPlugin の C インタフェースに関する正典
- **Creating an Audio Server Driver Plug-in**
  - <https://developer.apple.com/documentation/coreaudio/creating-an-audio-server-driver-plug-in>
- **Building an Audio Server Plug-in and Driver Extension**
  - <https://developer.apple.com/documentation/coreaudio/building-an-audio-server-plug-in-and-driver-extension>
- **Create audio drivers with DriverKit** (WWDC21 · 10190)
  - <https://developer.apple.com/videos/play/wwdc2021/10190/>
  - AudioDriverKit (別個だが隣接する技術) の概要
- **TN3169: Choosing an audio driver type**
  - AudioServerPlugin と AudioDriverKit に関するテクニカルノート

## 既存実装

### libASPL

- <https://github.com/gavv/libASPL>
- AudioServerPlugin を記述するための C++17 ライブラリ
- ライセンス: MIT
- `tympan-aspl` に最も近い類例 (言語は異なる)
- 注目点: コード生成ブリッジ層、構造化オブジェクトモデル、リクエスト
  ハンドラパターン

### BlackHole

- <https://github.com/ExistentialAudio/BlackHole>
- プロダクションの AudioServerPlugin: システム音声ルーティング用の仮想
  ループバックデバイス
- ライセンス: GPL-3.0
- 実装言語: C / Objective-C
- 参照対象として有用: バンドルパッケージング、Info.plist 構造、
  `kAudioObjectPropertyName` の扱い、フォーマットネゴシエーション

### BackgroundMusic

- <https://github.com/kyleneideck/BackgroundMusic>
- カスタム AudioServerPlugin (BGMDriver) の上に構築された macOS 用
  オーディオユーティリティ
- ライセンス: GPL-2.0
- 実装言語: C++ / Objective-C
- 参照対象として有用: 長寿命の状態、アプリ認識挙動、カスタムプロパティ
  ディスパッチ
- AudioServerPlugin の内部に関する稀に見るほど詳細なウォークスルーが
  `BGMDriver/DEVELOPING.md` にある

### Apple のサンプルコード

- **SimpleAudioDriver** ― macOS SDK に同梱された最小の
  AudioServerPlugin 例 (`Examples/AudioServerDriverExamples/`)
- 多くの実装が出発点として用いる

## 関連する Rust クレート

### 基盤

- **coreaudio-sys**: <https://crates.io/crates/coreaudio-sys>
  - CoreAudio に対する bindgen 生成の生バインディング。`tympan-aspl` が
    構築される最低レイヤ。
- **core-foundation**: <https://crates.io/crates/core-foundation>
  - CFTypeRef、CFString、CFArray などのセーフラッパー
- **core-foundation-sys**: <https://crates.io/crates/core-foundation-sys>
  - 生の CoreFoundation バインディング (推移的に使用)

### クライアント側 (tympan-aspl のスコープ外。ただし関連あり)

- **coreaudio-rs**: <https://crates.io/crates/coreaudio-rs>
  - Apple の Core Audio API に対する親しみやすい Rust インタフェース。
    オーディオデバイスを *消費* するためのもので、*実装* するためのもの
    ではない。
- **cpal**: <https://crates.io/crates/cpal>
  - クロスプラットフォームのオーディオ I/O。`coreaudio-rs` と同じ役割 ―
    クライアント側。

### リアルタイム / ロックフリー

- **crossbeam**: <https://crates.io/crates/crossbeam>
  - オーディオリアルタイムスレッドに適したロックフリーデータ構造
- **atomic-waker**: <https://crates.io/crates/atomic-waker>
  - クロススレッドのウェイク通知 (非ブロッキング)

## Audio Hardware Abstraction Layer (HAL) 関連背景

- **Core Audio Essentials**
  - <https://developer.apple.com/library/archive/documentation/MusicAudio/Conceptual/CoreAudioOverview/>
  - 古いが今なお有効な HAL アーキテクチャの概観
- **HALLab**
  - Apple の HAL デバッグツール。プラグインのプロパティを検査し、通知
    を送信し、オブジェクトの状態を観察できる。プラグイン開発時に有用。

## 規約とパッケージング

- **CFBundle Programming Guide**
  - <https://developer.apple.com/library/archive/documentation/CoreFoundation/Conceptual/CFBundles/>
  - `.driver` バンドル構造を理解するために必須の資料
- **公証 (Notarization)**
  - <https://developer.apple.com/documentation/security/notarizing_macos_software_before_distribution>
  - 現代の macOS でエンドユーザにプラグインを配布するために必須
- **AudioServerPlugin のコード署名要件**
  - プラグインは Developer ID 証明書で署名するか、ローカル用途のみであれ
    ば自己署名する必要がある。SIP がこれを強制する。
