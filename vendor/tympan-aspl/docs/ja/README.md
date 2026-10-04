[English](../README.md) | 日本語

# tympan-aspl ドキュメント

`tympan-aspl` の設計およびコントリビュータ向けドキュメントです。
プロジェクトの入口は[クレートルートの README](../../README.ja.md)
で、このディレクトリはより長文の資料を収めています。

## 散文ドキュメント

| ドキュメント | 内容 |
|---|---|
| [`overview.md`](overview.md) | プロジェクトの目的、スコープ、既存の AudioServerPlugin 実装との比較 |
| [`architecture.md`](architecture.md) | レイヤモデル、コアの抽象、および残る未解決の問い |
| [`plugin-author-guide.md`](plugin-author-guide.md) | ドライバの書き方: セットアップ、同一性、リアルタイム経路、パッケージング、落とし穴 |
| [`testing.md`](testing.md) | 階層化された CI 戦略、GitHub ホストランナーの制約、SIP / AMFI のコード署名に関する知見 |
| [`references.md`](references.md) | Apple のドキュメント、先行実装、関連クレート |
| [`handoff-protocol.md`](handoff-protocol.md) | 長期作業のためのセッション引き継ぎプロトコル |

## アーキテクチャ決定記録 (ADR)

[`decisions/`](../decisions/) にはプロジェクトの確定したアーキテクチャ
上の決定が収められ、[`decisions/README.md`](../decisions/README.md)
で索引化されています。ADR は英語のみです — 何が日本語にミラーされ、
何がされないかを定める国際化方針については
[ADR 0005](../decisions/0005-i18n-policy.md) を参照してください。

## 翻訳について

上記の散文ドキュメントには [`ja/`](.) 以下に日本語訳があります。
各英語ドキュメントは 1 行目のスイッチャでその翻訳へリンクしており、
各ペアにおける真実の源は英語版です。
