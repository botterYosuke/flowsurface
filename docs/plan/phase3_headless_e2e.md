# Phase 3 Headless モード E2E テスト実装計画

## 目標

既存の E2E テストスクリプトを `IS_HEADLESS=true` 環境変数で  
**headless / GUI 両対応**にする。新しい独立スクリプトは作らない。

---

## アーキテクチャ

```
IS_HEADLESS=false (デフォルト)  →  GUI モード起動（従来通り）
IS_HEADLESS=true               →  --headless --ticker $E2E_TICKER --timeframe M1
```

### common_helpers.sh の拡張

| 追加要素 | 内容 |
| :--- | :--- |
| `IS_HEADLESS` 変数 | 環境変数から取得（デフォルト `false`） |
| `is_headless()` | `[ "$IS_HEADLESS" = "true" ]` を返す述語 |
| `start_app()` | `is_headless` に応じて `_start_gui_app` / `_start_headless_app` に分岐 |
| `_start_gui_app()` | 既存の GUI 起動ロジック |
| `_start_headless_app()` | `DEV_IS_DEMO=true $EXE --headless --ticker $E2E_TICKER ...` |

---

## テストスクリプトの両対応パターン

### GUI 専用ブロック

```bash
if ! is_headless; then
  # saved-state.json セットアップ
  # streams_ready 待機
  # Live ↔ Replay トグル検証
fi
```

### headless 専用ブロック

```bash
if is_headless; then
  # pane/list → 501 確認
  pend "TC-xxx" "headless は Live モードなし"
fi
```

### TC ごとの期待値分岐

```bash
if is_headless; then
  [ "$MODE" = "Replay" ] && pass "..." || fail "..."
else
  [ "$MODE" = "Live" ] && pass "..." || fail "..."
fi
```

---

## headless モードでの差分一覧（s1_basic_lifecycle.sh 例）

| TC | GUI | headless |
| :--- | :--- | :--- |
| TC-S1-01 | mode=Live | mode=Replay |
| TC-S1-02 | toggle → Replay | toggle は no-op、Replay のまま |
| TC-S1-03〜13 | 同一 | 同一 |
| TC-S1-14 | StepBackward -60000ms | PEND（未実装） |
| TC-S1-15 | Live 復帰リセット | PEND（Live なし） |
| TC-S1-H09 | なし | GET /api/pane/list → 501 |

---

## CI 統合

`.github/workflows/e2e.yml` に `IS_HEADLESS=true` で S1 を追加：

```yaml
- name: "S1 Headless lifecycle (HyperliquidLinear:BTC)"
  env:
    E2E_TICKER: HyperliquidLinear:BTC
    IS_HEADLESS: "true"
  run: bash tests/s1_basic_lifecycle.sh
```

他のテストスクリプトも同じパターンで両対応可能。

---

## 実装ステータス

- ✅ `tests/common_helpers.sh` — `IS_HEADLESS` / `is_headless()` / `_start_headless_app()` 追加
- ✅ `tests/s1_basic_lifecycle.sh` — headless/GUI 両対応
- ✅ `.github/workflows/e2e.yml` — S1 headless ステップ追加
