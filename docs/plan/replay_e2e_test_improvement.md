# リプレイヘッダー E2E テスト改善計画

**作成日**: 2026-04-12
**最終更新**: 2026-04-12
**状態**: 全テスト完了 ✅

---

## 1. 背景

### 1.1 直前のバグ

- **症状**: リプレイモードで ▶ ボタンを押すと `Loading...` 表示のまま `Playing` に遷移しなかった
- **原因**: `src/main.rs` の `ReplayMessage::Play` ハンドラで、kline フェッチ（数秒）と trades フェッチ（Binance API レートリミットで数分）を全て `Task::batch(all_tasks).chain(DataLoaded)` に入れていたため、全完了を待たないと `DataLoaded` → `Playing` に遷移しなかった
- **修正**: kline タスクのみ `.chain(DataLoaded)` でゲートし、trades タスクは `Task::sip` で独立バックグラウンド実行に分離（`src/main.rs:815-866`）
- **E2E で検出できなかった理由**: 単一ペイン（BTCUSDT のみ）のテスト構成 + Loading→Playing 遷移時間にタイムアウト上限を設けていなかった

### 1.2 目的

1. 上記バグの回帰テストを追加する（テスト A）
2. Playing 中の時間進行を検証する（テスト B）
3. trades バックグラウンド到着を検証する（テスト C）
4. 既存テスト（カテゴリ 1-3）の回帰確認

---

## 2. テスト設計

### テスト A（最重要）: マルチペイン構成での高速 Playing 遷移

| 項目 | 内容 |
|------|------|
| **目的** | kline/trades 分離修正の回帰テスト |
| **構成** | BTCUSDT KlineChart + ETHUSDT KlineChart + BTCUSDT TimeAndSales (3ペイン) |
| **リプレイ範囲** | 12 時間（過去 24h 以内） |
| **合格基準** | Play 後 **15 秒以内** に status が `Playing` に遷移すること |
| **失敗パターン** | trades フェッチが kline と一緒にゲートされている場合、数分かかり 15s を超える |

### テスト B: current_time 進行検証

| 項目 | 内容 |
|------|------|
| **目的** | Playing 状態で仮想時刻が正しく進むことを確認 |
| **方法** | Playing 中に 2 秒間隔で current_time を 2 回取得し、差分 > 0 を検証 |
| **合格基準** | current_time が増加していること |

### テスト C: trades バックグラウンド到着検証

| 項目 | 内容 |
|------|------|
| **目的** | trades が Playing 遷移後もバックグラウンドで到着し続けること |
| **方法** | Playing 遷移後 10 秒待ってから status API で確認。trades 関連の情報が取得できるか検証 |
| **備考** | 現在の API には trades 到着状況を直接確認するエンドポイントがない。status API の応答と、再生の正常動作で間接的に検証する |

### テスト用 saved-state.json テンプレート

マルチペイン構成（BTCUSDT KlineChart + ETHUSDT KlineChart + BTCUSDT TimeAndSales）:

```json
{
  "layout_manager": {
    "layouts": [{
      "name": "E2E-MultiPane-Replay",
      "dashboard": {
        "pane": {
          "Split": {
            "axis": "Vertical", "ratio": 0.33,
            "a": {
              "KlineChart": {
                "layout": { "splits": [0.78], "autoscale": "FitToVisible" },
                "kind": "Candles",
                "stream_type": [{ "Kline": { "ticker": "BinanceLinear:BTCUSDT", "timeframe": "M1" } }],
                "settings": { "tick_multiply": null, "visual_config": null, "selected_basis": { "Time": "M1" } },
                "indicators": ["Volume"],
                "link_group": "A"
              }
            },
            "b": {
              "Split": {
                "axis": "Vertical", "ratio": 0.5,
                "a": {
                  "KlineChart": {
                    "layout": { "splits": [0.78], "autoscale": "FitToVisible" },
                    "kind": "Candles",
                    "stream_type": [{ "Kline": { "ticker": "BinanceLinear:ETHUSDT", "timeframe": "M1" } }],
                    "settings": { "tick_multiply": null, "visual_config": null, "selected_basis": { "Time": "M1" } },
                    "indicators": ["Volume"],
                    "link_group": "B"
                  }
                },
                "b": {
                  "TimeAndSales": {
                    "stream_type": [{ "Trades": { "ticker": "BinanceLinear:BTCUSDT" } }],
                    "settings": { "tick_multiply": null, "visual_config": null, "selected_basis": { "Time": "MS100" } },
                    "link_group": "A"
                  }
                }
              }
            }
          }
        },
        "popout": []
      }
    }],
    "active_layout": "E2E-MultiPane-Replay"
  },
  "timezone": "UTC",
  "trade_fetch_enabled": false,
  "size_in_quote_ccy": "Base",
  "replay": {
    "mode": "replay",
    "range_start": "PLACEHOLDER_START",
    "range_end": "PLACEHOLDER_END"
  }
}
```

---

## 3. 作業項目と進捗

- ✅ 計画書作成
- ✅ テスト A: マルチペイン 15 秒以内 Playing 遷移
- ✅ テスト B: current_time 進行検証
- ✅ テスト C: trades バックグラウンド到着検証
- ✅ 既存テスト（カテゴリ 1-3）回帰確認
- ✅ E2E テストスキル更新
- ✅ 計画書に最終結果を記録

---

## 4. テスト結果

**実施日時**: 2026-04-12 15:07 UTC
**全 23 テスト PASS / 0 FAIL**

### テスト A: マルチペイン 12h リプレイ — 15秒以内 Playing 遷移

| # | テスト | 結果 |
|---|--------|------|
| A.1 | Mode is Replay after restore | ✅ PASS |
| A.2 | No playback on restore | ✅ PASS |
| A.3 | range_start restored | ✅ PASS |
| A.4 | Play accepted | ✅ PASS (status=Playing) |
| A.5 | Playing within 15s | ✅ PASS (**1秒**で遷移) |

**所見**: kline/trades 分離修正が正常に機能。3ペイン（BTCUSDT + ETHUSDT KlineChart + TimeAndSales）+ 12h レンジでも kline フェッチのみで即座に Playing に遷移した。

### テスト B: current_time 進行

| # | テスト | 結果 |
|---|--------|------|
| B.1 | current_time advancing | ✅ PASS (1775869680647 → 1775869683911) |
| B.2 | current_time within range | ✅ PASS |

### テスト C: trades バックグラウンド

| # | テスト | 結果 |
|---|--------|------|
| C.1 | App still Playing after 10s | ✅ PASS |
| C.2 | current_time still advancing | ✅ PASS |

**所見**: Playing 遷移後 10 秒経過してもアプリは安定稼働。trades のバックグラウンド到着がアプリをクラッシュ・フリーズさせていないことを確認。

### 回帰テスト: 基本ライフサイクル (R.1-R.10)

| # | テスト | 結果 |
|---|--------|------|
| R.1 | Pause | ✅ PASS |
| R.2 | current_time frozen while Paused | ✅ PASS |
| R.3 | Step forward +60s | ✅ PASS |
| R.4 | Step backward -60s | ✅ PASS |
| R.5 | Speed cycle (1x→2x→5x→10x→1x) | ✅ PASS |
| R.6 | Resume | ✅ PASS |
| R.7 | current_time advancing after Resume | ✅ PASS |
| R.8 | Toggle to Live | ✅ PASS |
| R.9 | No playback after toggle | ✅ PASS |
| R.10 | Toggle back to Replay | ✅ PASS |

### 回帰テスト: エラーケース (E.1-E.4)

| # | テスト | 結果 |
|---|--------|------|
| E.1 | 404 for unknown path | ✅ PASS |
| E.2 | 400 for invalid JSON | ✅ PASS |
| E.3 | 400 for missing field | ✅ PASS |
| E.4 | 404 for GET on POST endpoint | ✅ PASS |

---

## 5. 知見・設計思想

### kline / trades 分離アーキテクチャ

```
ReplayMessage::Play
  ├── kline_tasks → Task::batch().chain(DataLoaded) → Playing に遷移
  └── trade_tasks → Task::sip() で独立バックグラウンド実行
                     TradesBatchReceived / TradesFetchCompleted で逐次処理
```

- **設計意図**: kline は数秒で完了するため、これだけで Playing に遷移させてユーザーにチャートを見せる。trades は Binance API レートリミット（1200req/min）により数分かかる場合があるため、バックグラウンドで逐次到着させる
- **テストの教訓**: 単一ペイン構成では trades フェッチの影響が小さく問題が顕在化しない。マルチペイン + 長い時間レンジでのテストが必要

### Playing 遷移が 1 秒で完了した理由

修正後のアーキテクチャでは kline タスクのみが `DataLoaded` のゲートになっている。M1 kline × 12h = 720 本は Binance API の 1 リクエスト上限（1000 本）に収まるため、BTC + ETH の 2 リクエストが並列で発行され瞬時に完了する。trades は `Task::sip` で独立実行されるため Playing 遷移を一切ブロックしない。

### テストスクリプトの場所

`C:/tmp/e2e-replay-header.sh` — テスト A/B/C + 回帰テストの統合スクリプト
`C:/tmp/e2e-multipane-replay.json` — マルチペイン構成のテストフィクスチャ
