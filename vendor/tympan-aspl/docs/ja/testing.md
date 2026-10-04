[English](../testing.md) | 日本語

# テストと CI

本ドキュメントは `tympan-aspl` のテストおよび継続的インテグレーション
戦略を記述します。GitHub ホストランナー上で自動的に検証できるもの、
手動または自己ホスト実行が必要なもの、および macOS 自体が課す制約を
扱います。

## 階層化された検証戦略

検証は深さと環境要件によって 4 つの階層に整理します。各階層は前の階層
を包含します。下位の階層は全プルリクエストで実行され、上位の階層は
スケジュール実行またはオンデマンドで実行されます。

### Tier 1: 静的検証および単体テスト

任意の macOS ホストランナーで実行可能な、標準 Rust ツールチェーンの
チェック。

| チェック | コマンド | 目的 |
|---|---|---|
| ビルド | `cargo build --all-targets` | 全クレート機能を横断したコンパイル |
| テスト | `cargo test` | HAL を必要としないロジックの単体テスト |
| Lint | `cargo clippy --all-targets -- -D warnings` | プロジェクト固有のリアルタイム安全性 lint を含む |
| フォーマット | `cargo fmt --check` | スタイルの一貫性 |
| ドキュメント | `cargo doc --no-deps --document-private-items` | ドキュメントのカバレッジと rustdoc エラー |

全プルリクエストで必須。所要時間: `macos-14` または `macos-15` ランナー
上で 5〜10 分。

### Tier 2: バンドルおよび ABI の検証

ビルドされた `.driver` バンドルが、ロード可能な AudioServerPlugin と
なるために CoreAudio が要求する構造的プロパティを満たすことを検証
します。

| チェック | ツール | 目的 |
|---|---|---|
| バンドルレイアウト | `plutil -lint Info.plist` | Info.plist の構文と必須キー |
| シンボル可視性 | `nm -gU` | エントリポイントシンボルのみが公開されている |
| アーキテクチャ範囲 | `lipo -info` | 設定どおりに universal または arm64 のみ |
| コード署名 (ad-hoc) | `codesign -v` | ビルド時に ad-hoc 署名が適用されている |
| ABI サイズ | コンパイル時 `static_assertions` | ブリッジ構造体サイズが C 定義と一致 |

ABI サイズチェックでは `static_assertions::assert_eq_size!` を
`coreaudio-sys` 生成型に対して用い、レイアウト差異を実行前に捕捉します。

ビルド可能な `.driver` 例が存在するようになり次第、全プルリクエストで
実行します。

### Tier 3: HAL ロード検証

ビルドした `.driver` バンドルを HAL プラグインディレクトリに配置し、
`coreaudiod` がそれを発見してロードを試みることを確認します。

手順:

1. `sudo cp -R target/release/example.driver /Library/Audio/Plug-Ins/HAL/`
2. `sudo launchctl kill KILL system/com.apple.audio.coreaudiod`
3. `coreaudiod` が自動再起動するまで少し待つ
4. `coreaudiod` の unified log を読み、プラグインを発見してロードを
   試みたことを確認

GitHub ホストランナーは手順 1〜4 をサポートしています。`sudo` は
パスワード不要で、`coreaudiod` は実際に HAL ディレクトリを走査して
バンドルのロードを試みます。

**ただし、プラグインを最後まで実行することはできません。** macOS 15
ではアウトオブプロセスの Core Audio Driver Service ヘルパーが
コード署名の妥当性を強制し、macOS の AMFI が ad-hoc 署名のプラグイン
バイナリを `AppleMobileFileIntegrityError -423` で拒否します — プラグ
インのコードが実行される前の段階です。GitHub ホストランナーは
Developer ID 署名を生成できないため、ホスト CI の Tier 3 は「ロードが
試みられた」ところで止まります。デバイスが列挙されること
(`system_profiler SPAudioDataType | grep <device-name>`) の確認と
IO パスの実行は、Developer ID 署名済みバンドルを必要とし、Tier 4 の
チェックとなります。

`main` へのマージごと、および毎日のスケジュールで実行します。

### Tier 4: オーディオ I/O 検証

プラグインを介した実際のオーディオデータフロー。ランナーはアプリケー
ションから利用可能な物理または仮想のオーディオハードウェアを持たない
ため、標準の GitHub ホストランナーではスコープ外です。次の方法で実施
します。

- PR レビュー中の開発者のローカルマシン
- GitHub Actions に登録された開発者の Mac (自己ホストランナー)
- 予算があれば macOS-in-cloud サービス (MacStadium、MacinCloud、AWS
  mac.metal)

## GitHub ホストの macOS ランナー

現行ランナー (2026 年 4 月時点):

| ラベル | OS | アーキテクチャ | スペック | 公開リポジトリでのコスト |
|---|---|---|---|---|
| `macos-15` / `macos-latest` | Sequoia | Apple Silicon (M1) | 3 vCPU、7 GB RAM、14 GB ディスク | 無料 |
| `macos-14` | Sonoma | Apple Silicon (M1) | 同上 | 無料 |
| `macos-13` | Ventura | Intel x86_64 | 同上 | 無料 |
| `macos-14-xlarge` / `macos-15-xlarge` | Sequoia / Sonoma | M2 Pro | 5 vCPU、14 GB RAM | 有料のみ ($0.16/分) |

公開リポジトリは全 GitHub プランで標準ランナーの無料分が無制限です。
プライベートなフォークや下流の利用者については、2026 年 1 月の値下げ
後で macOS ランナーは $0.048/分となり、月次の無料枠 2,000 Linux 換算
分はプライベートリポジトリで月およそ 200 macOS 分に相当します。

`tympan-aspl` は公開リポジトリのため、標準ランナーの利用は費用面で
制約されません。

### ランナーイメージのインベントリ

GitHub は各ランナーのイメージごとにソフトウェアインベントリを公開して
います。AudioServerPlugin 開発に関連する内容は次のとおりです。

- Xcode コマンドラインツールおよび Xcode フルインストール
- `clang`、`lipo`、`codesign`、`plutil`、`nm`、`otool`、`dyld_info`
- `system_profiler`、`launchctl`、`audio_*` 系の内省ユーティリティ

ad-hoc 署名のために追加で Apple Developer Program に加入する必要は
ありません。

## System Integrity Protection (SIP) と AMFI の考慮事項

GitHub ホストの Apple Silicon ランナーは VM 上で動作し、**SIP は無効**
の状態で提供されます — `csrutil status` は "disabled" を返し、起動
ログには `AMFI: Booted in a VM` が現れます。ただしこれは有用なレバー
ではありません。署名のない HAL プラグインを止めているコード署名の
ゲートは **AMFI** であり、`amfid` がこれを SIP とは独立に強制します。
SIP が無効でも、ランナーは ad-hoc 署名のプラグインを
`AppleMobileFileIntegrityError -423` で拒否します。

| 操作 | 許可されるか | 備考 |
|---|---|---|
| `/Library/Audio/Plug-Ins/HAL/` への `.driver` 配置 | はい | `sudo` が必要だがランナーで利用可 |
| `coreaudiod` の再起動 | はい | `launchctl kill` で実施 |
| `coreaudiod` がプラグインを発見・ロード試行 | はい | HAL ディレクトリを走査しバンドルを解析する |
| `coreaudiod` が **ad-hoc 署名**プラグインを最後までロード | **いいえ** | `amfid` が拒否 (`AppleMobileFileIntegrityError -423`)。SIP が無効でも変わらない |
| `coreaudiod` が **自己署名**プラグインを最後までロード | **いいえ** | 証明書を System キーチェーンに信頼ルートとして登録しても同様に拒否される。`amfid` はそのトラストストアを参照しない |
| `coreaudiod` が **Developer ID 署名**プラグインをロード | はい | CI では証明書を GitHub Secret 経由で渡した場合のみ可能 |
| `nvram boot-args` の書き込み (AMFI の `amfi_get_out_of_my_way` ノブ) | 書き込みは成功 | SIP が無効なので書き込み自体は通る — ただし反映には再起動が必要で、ホストランナーはジョブ途中で再起動できない |
| SIP 自体の変更 | 該当なし | ランナーでは既に無効 |

要点: **AMFI は、署名が Apple 発行の証明書チェーンに連ならないあらゆる
プラグインの「ロード完了」を妨げます。** macOS 15 ではアウトオブ
プロセスの Core Audio Driver Service ヘルパーがバイナリを `amfid` に
渡し、`amfid` が ad-hoc *または*自己署名の署名を
`AppleMobileFileIntegrityError -423`（"the file is adhoc signed or
signed by an unknown certificate chain"）で拒否します。`coreaudiod` は
バンドルを発見してロードを*試み*はします — これがホスト CI の Tier 3
が検証する内容です — がデバイスは列挙されません。

これは `macos-15` ランナー上で実証済みです。ad-hoc 署名、自己署名
証明書、その証明書を System キーチェーンに信頼ルートとして登録した
もの、`DisableLibraryValidation` を、単独および組み合わせて試し、
すべてのケースが `-423` で止まりました。(`security add-trusted-cert`
は `codesign --verify` と `spctl` は満たしますが、`amfid` は
`taskgated-helper: ... no eligible provisioning profiles found` を
ログに出して依然ロードを拒否します。`amfid` のトラストパスは Apple
発行の証明書であり、System キーチェーンではありません。)したがって
プラグインを最後まで実行するには Developer ID 署名済みバンドル — CI へ
は GitHub Secret 経由で渡す — が必要で、その Secret が用意されるまでは
Tier 4 のチェックです。(これはプロジェクト当初の調査の訂正です。当初
は署名のない HAL プラグインは自由にロードできると想定していましたが、
それは古いモデルでは正しく、macOS 15 の driver-service ヘルパーでは
正しくなく、SIP の無効化でも解除されません。)

## GitHub ホストランナーで検証できないこと

標準ランナー環境のハードな制限です。

- **物理スピーカーへのオーディオ出力** ― ランナーにはアプリから利用
  可能なオーディオ出力ハードウェアが存在しない
- **マイクキャプチャ** ― ランナーには入力デバイスが存在しない
- **長時間安定性** ― ジョブは 6 時間でタイムアウトする。現実的な安定
  性テストは数日〜数週間にわたる
- **公証ゲート動作** ― 公証には Apple Developer Program アカウントと
  Apple サーバへの提出が必要であり、CI でこれをコミット毎に意味のある
  形で実行することはできない
- **サードパーティのオーディオアプリとの相互作用** ― DAW、会議
  アプリ等は導入されておらず、ヘッドレスでの操作も困難
- **システム設定 UI 検証** ― システム設定 > サウンドへのデバイス表示
  確認にはログイン済みの UI セッションが必要だが、ランナーはこれを
  安定して提供しない

これらの空隙が、Tier 4 の手動 / 自己ホスト検証ステップを動機づけ
ます。

## 自己ホストの代替手段

Tier 4 検証を自動化する必要がある場合の選択肢は以下のとおりです。

### 自己ホスト GitHub Actions ランナー

開発者の Mac を GitHub Actions ランナーとして登録します。個人開発に
費用対効果が高いですが、CI 実行時にマシンが電源オンかつネットワーク
接続されている必要があります。

公開リポジトリではセルフホストランナーにプラットフォーム手数料は発生
しません。プライベートリポジトリでは 2026 年 3 月から $0.002/分の
プラットフォーム手数料がかかります。

登録手順:

1. Settings > Actions > Runners > New self-hosted runner
2. 表示されたインストールスクリプトをターゲット Mac で実行する
3. 任意で起動エージェントとして登録し、自動起動するよう設定する

### macOS-in-cloud サービス

| サービス | 提供モデル | おおよその費用 | 適した用途 |
|---|---|---|---|
| MacStadium | 専有 Mac mini、月額 | $80〜200/月 | 永続的な状態、フル管理 |
| MacinCloud | 共有・専有、時間制 | $1〜3/時 | アドホックな検証 |
| AWS EC2 mac.metal | ベアメタル、24 時間単位 | 約 $25/日 | 短期集中のキャンペーン |
| Xcode Cloud | Xcode プロジェクト向けの Apple CI | Developer Program 加入で利用可 | iOS / macOS アプリの CI。ライブラリ向きではない |

これらのサービスは、Tier 4 検証をパイプラインの一部として自動実行
する必要があり、ローカルの開発者マシンでは不十分な場合 (例: リリース
検証) に適しています。

## ワークフローファイル

`.github/workflows/` の構成（`tier1`〜`tier3` は配置済み、`release.yml`
は今後の予定）:

```
.github/workflows/
├── tier1.yml           # 全 PR で cargo build/test/clippy/fmt/doc
├── tier2.yml           # 全 PR でバンドルおよび ABI 検証
├── tier3.yml           # main へのマージおよび毎日で HAL ロード検証
├── tier3-asan.yml      # インプロセスのハーネスを AddressSanitizer 下で毎日実行
└── release.yml         # タグ付きリリース公開 (cargo publish dry-run) — 予定
```

Tier 4 はワークフローセットから意図的に除外しています。手動または
標準パイプライン外の自己ホストランナーで実施します。

## CI におけるリアルタイム安全性の強制

Clippy が提供する lint に加え、フレームワークはプロジェクト固有の
lint 群を定義し、リアルタイムコード経路にリアルタイム非対応のパターン
が現れた場合に CI を失敗させます。

- `IOProc` から到達可能な関数内のアロケーション呼び出し
- リアルタイムモジュール内での `std::sync::Mutex` 使用
- リアルタイムコード内のブロッキングシステムコール (ファイル I/O、
  ネットワーク等)

これらの lint は次の手段で強制します。

- リアルタイムモジュールが許可する依存を制限するカスタム
  `cargo clippy` 設定 (`clippy.toml`)
- リアルタイム非対応な推移的依存の誤った導入を防ぐ `cargo-deny`
  ルール
- モジュールレベル属性でのコンパイル時 `#[deny(...)]` 指定

実装詳細は最初のリアルタイムモジュールが入り次第追加します。

## 実装ステータス

Tier 1・2・3 が配線済みです。

- `tier1.yml` — 全プルリクエストで build、test、`clippy`、`fmt`、
  `doc`、`cargo deny`、および `static mut` 不在の grep。
- `tier2.yml` — 全プルリクエストで、コミット済みおよび生成された
  Info.plist の `plutil -lint`、サンプル `.driver` cdylib のビルド、
  `nm` によるファクトリシンボル確認、`lipo -info`、`.driver` バンドル
  組み立て + ad-hoc `codesign`。
- `tier3.yml` — `coreaudiod` プラグインロード検証。`.driver` を
  インストールし、`coreaudiod` を再起動し、`coreaudiod` の unified log
  からプラグインを発見してロードを*試みた*ことを確認します。デバイス
  列挙までは表明しません。ホストランナーでは AMFI が ad-hoc 署名の
  バイナリを拒否するためです（上記 SIP セクション参照）。ADR 0001 に
  従い、`main` へのマージ・毎日のスケジュール・手動ディスパッチで
  実行し、プルリクエストでは実行しません。
- `tier3-asan.yml` — インプロセスのハーネス（`raw_lifecycle`、
  `realtime_safety`）と `raw` モジュールのユニットテストを
  `-Zsanitizer=address` 下で再実行し、手書き FFI 層での
  use-after-free・double-free・範囲外アクセスを検出します。毎日および
  手動ディスパッチで実行。`tier3.yml` と同じく非ブロッキングです。

Tier 4 はリリース前の手動チェックリストのままで、AMFI のコード署名
制約がホスト CI から押し出すチェック — Developer ID 署名済み
`.driver` の完全ロード、`system_profiler` によるデバイス列挙の確認、
IO パスの実行 — も担います。
