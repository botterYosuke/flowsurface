# S32: TOYOTA candlestick チャート追加テスト計画

**作成日**: 2026-04-16  
**ブランチ**: sasa/develop  
**担当**: botterYosuke  

---

## 目的

`saved-state.json` サンプル（BinanceLinear:BTCUSDT M1、Replay モード）で起動した後、TOYOTA（TachibanaSpot:7203）の D1 candlestick チャートを追加する操作を E2E テストで検証する。

---

## 期待する動作

1. **TOYOTA の 1d チャートが追加される**
   - ペイン分割（split）後にペイン数 = 2
   - 新ペインに ticker = `TachibanaSpot:7203`、timeframe = `D1` が設定される

2. **REPLAY が start 時間に戻って再開される**
   - ticker 変更後 `current_time == start_time`（`clock.seek(range.start)` が発火）
   - status = `Paused`（自動再生しない）
   - Tachibana セッションあり：streams_ready 後に Resume → `Playing`

---

## テストシナリオ（TC 一覧）

| TC | 検証内容 | Tachibana セッション要否 |
|---|---|:-:|
| TC-S32-01 | 起動後 auto-play → Playing 到達 | 不要 |
| TC-S32-02 | ペイン split → pane count = 2 | 不要 |
| TC-S32-03 | 新ペインに set-ticker TachibanaSpot:7203 → 成功 | 不要 |
| TC-S32-04 | 新ペインに set-timeframe D1 → 成功 | 不要 |
| TC-S32-05 | ticker 変更後 current_time == start_time（clock.seek）| 不要 |
| TC-S32-06 | ticker 変更後 status = Paused | 不要 |
| TC-S32-07 | 新ペインの ticker/timeframe が正しく設定されている | 不要 |
| TC-S32-08 | streams_ready = true（TOYOTA データロード完了）| **要** |
| TC-S32-09 | Resume → Playing（Replay が再開される）| **要** |
| TC-S32-10 | current_time が前進（再生が正常動作）| **要** |

---

## フィクスチャ

`saved-state.json` のサンプル内容（固定）:

```json
{
  "layout_manager": {
    "layouts": [{
      "name": "Test-M1",
      "dashboard": {
        "pane": {
          "KlineChart": {
            "layout": {"splits": [0.78], "autoscale": "FitToVisible"},
            "kind": "Candles",
            "stream_type": [{"Kline": {"ticker": "BinanceLinear:BTCUSDT", "timeframe": "M1"}}],
            "settings": {"tick_multiply": null, "visual_config": null, "selected_basis": {"Time": "M1"}},
            "indicators": ["Volume"],
            "link_group": "A"
          }
        },
        "popout": []
      }
    }],
    "active_layout": "Test-M1"
  },
  "timezone": "UTC",
  "trade_fetch_enabled": false,
  "size_in_quote_ccy": "Base",
  "replay": {
    "mode": "replay",
    "range_start": "2025-04-15 04:49",
    "range_end": "2026-04-15 06:49"
  }
}
```

**ポイント**:
- `range_start = "2025-04-15 04:49"` が clock.seek のターゲット
- auto-play 起動（pending_auto_play = true）
- Binance BTCUSDT M1 ストリームは Tachibana セッション不要

---

## 設計上の注意点

### ReloadKlineStream の挙動（§6.6）

ticker/timeframe 変更時、`kline stream あり` の場合:

```
clock.pause()          → Paused
clock.seek(range.start) → current_time = start_time
loader::load_klines()  → 新銘柄データをフェッチ
```

**自動再開なし**: ロード完了後も Paused のまま。ユーザーが手動 Resume するまで待機。

### Tachibana セッションなし時の動作

- TC-S32-05/06（current_time/Paused チェック）: セッション不要で確認可能
- TC-S32-08/09/10（streams_ready/Playing）: セッションがなければ PEND

### e2e-mock ビルドでの inject-session

`cargo build --release --features e2e-mock` ビルドなら `POST /api/test/tachibana/inject-session` で注入可能。inject が成功した場合は TC-S32-08〜10 を実行する。

---

## 実装ファイル

- テストスクリプト: `tests/e2e_scripts/s32_toyota_candlestick_add.sh`

---

## Tips / 調査で得られた知見

1. **ticker 正規化**: `pane/set-ticker` に `TachibanaSpot:7203` を渡しても、`pane/list` が返す `ticker` フィールドは `Tachibana:7203` に正規化される。TC の期待値は `include "7203"` で柔軟に判定する。

2. **auto-play タイムアウト**: Binance M1 データのプリフェッチは 60 秒を超える場合がある（ネットワーク状況依存）。`wait_playing` は 120 秒に設定。

3. **inject-session 不在時のフォールバック**: リリースビルドでは `POST /api/test/tachibana/inject-session` が HTTP 404 を返す。その場合は keyring の実セッションを確認 (`/api/auth/tachibana/status`)。

4. **clock.seek の確認方法**: `start_time` を API の `d.start_time` フィールドから取得し、`d.current_time` と比較するのが最も堅牢（ハードコードなし）。

5. **streams_ready と Resume**: ticker 変更後は `Paused` 状態になり自動再開しない（§6.6 仕様通り）。`streams_ready = true` 確認後に手動 Resume が必要。

---

## 進捗

- ✅ 計画書作成
- ✅ テストスクリプト作成 (`tests/e2e_scripts/s32_toyota_candlestick_add.sh`)
- ✅ テスト実行・結果確認（11 PASS / 0 FAIL）
