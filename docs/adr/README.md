# Architecture Decision Records

本ディレクトリは flowsurface のアーキテクチャ上の意思決定を ADR (Architecture Decision Record) として記録する。

## インデックス

| ADR | Title | Status | Date |
|-----|-------|--------|------|
| [0001](0001-agent-replay-api-separation.md) | Agent 専用 Replay API を UI リモコン API から分離する | proposed | 2026-04-22 |

## ライフサイクル

```
proposed → accepted → [deprecated | superseded by ADR-NNNN]
```

- **proposed**: PR レビュー中の下書き。実装着手前。
- **accepted**: PR マージ時に `proposed → accepted` に更新する。実装が本 ADR に従う拘束力を持つ。
- **deprecated**: 決定が無効化された（機能削除など）。理由を追記。
- **superseded**: 別 ADR に置き換えられた。常に置き換え先 ADR 番号をリンクする。

## 新規 ADR の作成

1. 本ディレクトリの `template.md` をコピーし `NNNN-decision-title.md` にリネーム
2. 既存 ADR の最大番号 + 1 を採番
3. 本書のインデックステーブルに 1 行追加
4. PR で提出 → マージ時に Status を `accepted` に更新

ADR スキルの詳細は `.claude/skills/architecture-decision-records/SKILL.md` を参照。
