# flowsurface — CLAUDE.md

## プロジェクト概要

flowsurface は Rust 製デスクトップアプリで、暗号資産マーケットのチャートプラットフォームです。
`iced` GUI フレームワークを使用し、リアルタイムデータの表示・リプレイ機能・仮想約定エンジンによる模擬取引を提供します。

主要モジュール：
- `src/replay/` — リプレイ制御・仮想約定エンジン（`virtual_exchange/`）
- `src/connector/` — WebSocket ストリーム・注文送信（`order.rs`）・認証（`auth.rs`）
- `src/replay_api.rs` — E2E テスト用 HTTP API（ポート 9876）
- `src/screen/dashboard/panel/` — 注文入力・注文一覧・買付余力パネル

## 技術スタック

- **言語**: Rust（TypeScript / JavaScript は一切使用していない）
- **GUI**: `iced` 0.14.x
- **ビルド**: `cargo`
- **主要クレート**: `iced`, `tokio`, `thiserror`, `serde`
- **プラットフォーム**: Windows 11

## 開発コマンド一覧

```bash
# ビルド
cargo build

# テスト
cargo test

# Lint（警告をエラーとして扱う）
cargo clippy -- -D warnings

# フォーマット（コミット前に必ず実行すること — format.yml CI がチェックする）
cargo fmt

# フォーマットチェック（修正なし）
cargo fmt --check

# コンパイル確認（高速）
cargo check

# E2E テスト（アプリ起動後、別ターミナルで実行）
uv run tests/s1_basic_lifecycle.py   # 例：個別スクリプト（Python 版）
bash tests/s2_persistence.sh        # 例：bash スクリプト
# HTTP API はポート 9876 で受け付け（replay_api.rs）
```

## スキルの使い方ガイド

| スキル | いつ使うか | 呼び出し方 |
| :--- | :--- | :--- |
| `verification-loop` | PR 作成前、機能実装後、リファクタリング後 | `/verification-loop` |
| `strategic-compact` | 長いセッション（フェーズ切り替え時）、デバッグ完了後 | `/compact` を手動実行 |
| `agent-introspection-debugging` | エージェントがループ・失敗を繰り返すとき | 明示的に呼び出し |
| `coding-standards` | コードレビュー、新モジュール追加時、規約確認時 | `/coding-standards` |
| `tdd-workflow` | 新機能実装、バグ修正、リファクタリング | `/tdd-workflow` |
| `e2e-testing` | E2E テスト作成・実行（HTTP API ポート 9876 経由） | `/e2e-testing` |
| `rust-testing` | Rust テスト作成、property-based testing、カバレッジ計測 | `/rust-testing` |
| `benchmark` | PR 前後のパフォーマンス計測、criterion ベンチマーク作成 | `/benchmark` |
| `architecture-decision-records` | 重要な技術的意思決定（フレームワーク・設計パターン・ライブラリ選定）を記録 | `/architecture-decision-records` または `ADR this` |

## コーディングルール

実装は `.claude/rules/rust/` のルールに従ってください：

- **[coding-style.md](.claude/rules/rust/coding-style.md)**: フォーマット・命名・借用・エラーハンドリング
- **[patterns.md](.claude/rules/rust/patterns.md)**: Repository/Service/Newtype/Builder/State Machine パターン
- **[security.md](.claude/rules/rust/security.md)**: シークレット管理・unsafe の扱い・依存関係監査
- **[testing.md](.claude/rules/rust/testing.md)**: テスト構成・カバレッジ 80%+ 目標

## 作業依頼の必須フォーマット

計画書を `docs/plan` に作成し、進捗があり次第更新してください。完了項目には ✅ を付けてください。
TDD アプローチ：`.claude/skills/tdd-workflow/SKILL.md` で実装してください。
