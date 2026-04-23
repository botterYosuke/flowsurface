## Summary

<!-- 1-3 行で何を変更したか・なぜか -->

## Test plan

<!-- どう検証したか。該当するものにチェック -->
- [ ] `cargo fmt --check` 通過
- [ ] `cargo clippy -- -D warnings` 通過
- [ ] `cargo test --lib` 全 PASS
- [ ] 既存 E2E `tests/e2e/s*.sh` `tests/e2e/s*.py` への影響確認
- [ ] 新規機能なら `tests/e2e/` に E2E テスト追加

## ADR-0001 compliance (agent 専用 Replay API)

[ADR-0001](docs/adr/0001-agent-replay-api-separation.md) により、新規 API は
`/api/agent/session/:id/*` に追加する。`/api/replay/*`（UI リモコン API）への
**新規ルート追加は禁止**（既存ルートの内部実装変更・削除は可）。

- [ ] `/api/replay/*` に新規ルートを**追加していない**
  （追加が必要な場合は `/api/agent/session/:id/*` に追加し、
  UI リモコン API 側は facade として残す）
- [ ] `VirtualExchange` のセッション状態遷移は `mark_session_*`
  （Started / Reset / Terminated）で明示的に発火している
- [ ] Agent API の state は UI リモコンハンドラから直接触っていない
  （`observe_generation()` 経由で購読）

違反した場合 `.github/workflows/adr_guard.yml` の CI で fail する。

## Related

<!-- issue 番号 / 親 PR / ADR / plan -->
