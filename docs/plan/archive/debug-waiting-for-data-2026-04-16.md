# デバッグ報告書: 起動後 "Waiting for data..." から進まない問題

**日時**: 2026-04-16  
**対象**: saved-state.json のリプレイ設定でアプリが "Waiting for data..." から進まない現象  
**ステータス**: ✅ 原因特定・修正完了

---

## 1. 症状

アプリ起動後、リプレイモードで "Waiting for data..." が表示されたまま進まない。  
ヘッダーは "Loading..." を表示し続ける。

**再現条件**: saved-state.json に以下の設定が保存されている場合

```json
"replay": {
  "mode": "replay",
  "range_start": "2025-04-15 04:49",
  "range_end":   "2026-04-15 06:49"
}
```

---

## 2. 仮説リスト (デバッグ前)

| # | 仮説 | 結果 |
|---|------|------|
| 1 | **1年分のM1データ = 527回のBinance API呼び出し** → 超低速/タイムアウト | ✅ **正解 (根本原因)** |
| 2 | Binance レートリミットにより呼び出し間に待機が発生 | ✅ 補因 |
| 3 | `DataLoadFailed` でセッションが Idle にリセットされている | ❌ 発生せず |
| 4 | `pending_auto_play` が `all_panes_ready` を待ちきれない | ❌ Play は正常に発火 |
| 5 | `parse_replay_range` 日付解析失敗 (サイレントエラー) | ❌ 正常にパース成功 |
| 6 | `dispatcher.is_loaded` range 不一致で永続的に失敗 | ❌ 発生せず |
| 7 | ネットワーク障害 / Binance API エラー | ❌ 発生せず |

---

## 3. デバッグログから得た証拠

追加したログの出力:

```
[DEBUG/main]   replay restore: mode=Replay, range='2025-04-15 04:49'..'2026-04-15 06:49',
               has_valid_range=true, pending_auto_play=true

[auto-play]    All panes ready — firing ReplayMessage::Play  ← Play は正常に発火

[DEBUG/ctrl]   Play fired: start_ms=1744692540000, end_ms=1776235740000,
               step_ms=60000, pending_count=1

[DEBUG/ctrl]   -> stream=Kline{BinanceLinear:BTCUSDT, M1}
               load_range=1744674540000..1776235740000

[DEBUG/ctrl]   pending_count=1 → Loading state

[DEBUG/loader] load_klines: tf=M1 range=1744674540000..1776235740000
               (~526020 klines, ~527 API pages)   ← ★ 根本原因

[DEBUG/adapter] fetch_klines paging: 526020 klines needed, 527 pages (tf=M1)
[DEBUG/adapter] page 1/527: 1744674540000..1744734540000 → got 1000 klines (53ms)
[DEBUG/adapter] page 2/527: 1744734540000..1744794540000 → got 1000 klines (46ms)
...
```

---

## 4. 根本原因

### 問題の構造

```
saved-state.json
  range_start = "2025-04-15 04:49"
  range_end   = "2026-04-15 06:49"   ← 今日の日付 (1年後)
```

↓ `compute_load_range` で pre-history 300本分 (5時間) を先頭に追加

```
load_range = 1744674540000 .. 1776235740000
           = 2025-04-14 23:49 .. 2026-04-15 06:49
           = 526,020 分 = 526,020 M1 klines
```

↓ `adapter.rs::fetch_klines` がページサイズ1000でループ

```
527 回のシーケンシャルな Binance API 呼び出し
```

↓ Binance レートリミット

```
PERP_LIMIT = 2400 weight/min
各1000本リクエスト = weight 5
→ 1分間の最大ページ数 = 2400 / 5 = 480 ページ
→ 527 ページ = 480 ページ目でレートリミット発動 → ~60秒待機
→ 合計所要時間: 1分30秒〜2分
```

**ユーザーは「ロード中」であることに気づかず、"Waiting for data..." が固まっていると誤認した。**

### 数値サマリ

| 項目 | 値 |
|------|-----|
| 再生範囲 | 2025-04-15 04:49 〜 2026-04-15 06:49 (約1年) |
| 必要 M1 kline 数 | 526,020 本 |
| 必要 API ページ数 | 527 ページ |
| ページあたり所要時間 | ~50ms |
| レートリミット発動タイミング | 480ページ目 (~24秒後) |
| 推定総ロード時間 | **1分30秒〜2分** |

---

## 5. 修正方針

### 採用した修正: 最大ページ数バリデーション

`ReplayUserMessage::Play` ハンドラに、推定APIページ数チェックを追加する。  
閾値を超える場合は `Toast::error` でユーザーに通知し、処理を中断する。

**閾値**: 100ページ (= ~1.7時間分の M1 データ、または D1 で~270年分)

**根拠**:
- 100ページ × 50ms = 5秒 → ユーザーが "Loading..." と認識できる範囲
- Binance レートリミット内 (100 × weight5 = 500 < 2400/min) → レートリミット待機なし
- M1 チャートで 100,000 本 ≈ 70日分 ← リプレイ用途として十分広い

### エラーメッセージ

```
Replay range too large: ~527 API pages needed for BinanceLinear:BTCUSDT M1.
Please shorten the range (max ~100 pages / ~70 days for 1m timeframe).
```

---

## 6. 修正箇所

- [src/replay/controller.rs](../src/replay/controller.rs) — `handle_user_message(Play)` に最大ページ数チェックを追加
- デバッグログをすべて削除（`[DEBUG/...]` プレフィックスのもの）

---

## 7. 再発防止策

1. **このバリデーション** — 大きすぎる範囲を早期に検出してユーザーに通知
2. 将来的には: ローディング進捗表示 (Loading... N/M pages) で UX を改善
