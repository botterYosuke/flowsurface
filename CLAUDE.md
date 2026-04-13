# flowsurface — Claude Code Instructions

## Skills

すべての開発タスク（新機能・バグ修正・リファクタリング）を開始する前に、
必ず `Skill` ツールで `tdd` スキルを呼び出すこと。

```
Skill({ skill: "tdd" })
```

スキルの指示に従い、Red → Green → Refactor のサイクルを守ること。

## TDD 絶対ルール

以下は CLAUDE.md に直書きされた最優先ルール。スキルより優先度が高い。

1. **テストを先に書く。実装から始めない。**
2. **テストが失敗するのを確認してから実装する（`cargo test` で FAIL を目視）。**
3. **テストが通るのを確認してから次に進む（`cargo test` で PASS を目視）。**
4. **「今回だけ」という例外はない。**

テストなしで書いたコードは削除してやり直す。

## 言語・ツール

- 言語: Rust
- テスト実行: `cargo test`
- カバレッジ: `cargo tarpaulin`
- Lint: `cargo clippy`
