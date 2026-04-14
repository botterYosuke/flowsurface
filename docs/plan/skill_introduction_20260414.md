# スキル導入作業計画書

**作成日**: 2026-04-14

## 目的

flowsurface プロジェクトに 3 つのスキルを導入し、Rust テスト・ベンチマーク・ADR 記録のワークフローを強化する。

## タスク一覧

- ✅ タスク 1: `rust-testing` スキルの導入
  - ✅ `.claude/skills/rust-testing/SKILL.md` の作成
  - ✅ `.claude/rules/rust/testing.md` への proptest / mockall / cargo-llvm-cov 追記
  - ✅ `Cargo.toml` に `proptest`, `mockall`, `criterion` を追加
- ✅ タスク 2: `benchmark` スキルの Rust 向け適応・導入
  - ✅ `.claude/skills/benchmark/SKILL.md` の作成（Rust/criterion ベース）
  - ✅ `benches/flowsurface.rs` プレースホルダー作成
- ✅ タスク 3: `architecture-decision-records` スキルの導入
  - ✅ `.claude/skills/architecture-decision-records/SKILL.md` の作成
- ✅ `CLAUDE.md` のスキルガイドテーブルに 3 行追加
- ✅ `cargo check` でエラーなし確認

## 完了条件

- `.claude/skills/rust-testing/SKILL.md` が存在する
- `.claude/skills/benchmark/SKILL.md` が存在する（Rust/criterion 向け）
- `.claude/skills/architecture-decision-records/SKILL.md` が存在する
- `.claude/rules/rust/testing.md` に proptest / mockall / rstest / cargo-llvm-cov の記述が追記されている
- `CLAUDE.md` のスキルガイドテーブルが 3 行追加されている
- `Cargo.toml` に `criterion`, `proptest`, `mockall` が `[dev-dependencies]` に追加されている
- `cargo check` がエラーなしで通る
