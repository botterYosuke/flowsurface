# スキル導入・カスタマイズ計画書

## 概要

ECC（everything-claude-code）から4つのスキルを flowsurface（Rust デスクトップアプリ）向けにカスタマイズして導入する。
加えて `CLAUDE.md` を新規作成する。

## 進捗

- ✅ 計画書作成
- ✅ ソーススキルの内容確認
- ✅ `verification-loop` — Rust コマンドでカスタマイズ済み
- ✅ `strategic-compact` — Node.js フック参照を除去済み
- ✅ `agent-introspection-debugging` — Rust 環境チェックを追加済み
- ✅ `coding-standards` — Rust 規約で全面書き直し済み
- ✅ `CLAUDE.md` 新規作成済み

## 設計思想・背景

### なぜカスタマイズが必要か
- ECC のスキルは Node.js/TypeScript/React を前提としており、Rust プロジェクトには適用不能なコマンドが多数含まれる
- flowsurface はフル Rust プロジェクトのため、`npm`, `npx`, `tsc`, `pnpm` 等のコマンドは存在しない
- 環境固有の設定（HTTP API ポート 9876、Windows パス等）を反映する必要がある

### verification-loop のカスタマイズ方針
- `npm run build` → `cargo build`
- `npx tsc --noEmit` → `cargo check`（型チェック相当）
- `npm run lint` → `cargo clippy -- -D warnings`
- `npm run test` → `cargo test`
- Phase 2.5 として `cargo fmt --check` を追加（Rust ではフォーマットが重要）
- Overall 判定条件: ビルドエラー・clippy 警告（-D warnings）・テスト失敗がゼロのとき READY

### strategic-compact のカスタマイズ方針
- `suggest-compact.js` は Node.js スクリプトのため動作不可 → Hook Setup セクションを削除
- 代わりに「手動で `/compact` を実行する」旨を明記
- フェーズ遷移テーブルを Rust 開発フロー（cargo check/test）に合わせて調整

### agent-introspection-debugging のカスタマイズ方針
- 主要構造は言語非依存のためほぼそのまま
- Phase 2 の環境チェックに Rust 固有コマンドを追補
  - `cargo check` でビルド状態確認
  - `curl http://localhost:9876/health` で HTTP API 状態確認

### coding-standards のカスタマイズ方針
- TypeScript/React/Next.js/JSDoc のセクションをすべて削除
- Rust 固有の規約（命名規則、所有権、エラーハンドリング、テスト）で全面書き直し
- KISS/DRY/YAGNI は言語非依存のためそのまま適用

## Tips（他の作業者向け）

- `cargo clippy -- -D warnings` は警告をエラーとして扱う。CI と同じ厳しさで検証できる
- `cargo fmt --check` は修正しない（--check フラグ）。差分があれば非ゼロ終了する
- HTTP API（ポート 9876）は `--features e2e-mock` でビルドした場合のみ起動する
- Windows 環境のため、パスの区切り文字はスラッシュ `/` を使用する（bash 環境）
- 既存の `e2e-testing` と `tdd-workflow` スキルには触れない
