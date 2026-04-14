# StepBackward チラつきバグ修正計画

## 問題

⏮ クリック時に約1秒チャートがチラつく。  
具体的には「Start time 以前のバー（すなわち現在表示中のバー）が一瞬消える」という現象。

⏭ (StepForward) では問題なし。

---

## コード差分の概要

| 操作 | 処理フロー |
|---|---|
| StepForward | `ingest_replay_klines` のみ（リセットなし、kline を追加するだけ） |
| StepBackward | `reset_charts_for_seek` → `ingest_replay_klines`（クリア後に再注入） |

`reset_charts_for_seek` → `KlineChart::reset_for_seek()` が実行される。

---

## 仮説一覧

### H1: レンダリング中間状態（同一サイクル内の invalidate 競合）
`reset_for_seek` が `invalidate(None)` を呼び cache をクリア → その直後 `ingest_historical_klines` も `invalidate(None)` を呼ぶ。  
Iced の canvas cache clear が何らかの形で中間フレームをレンダリングさせる可能性。  
**可能性: 低**（同一 `update()` サイクル内のため通常は起きない）

### H2: 次 tick での live fetch 発行（最有力 A）
`reset_for_seek` が `request_handler = RequestHandler::default()` にリセット。  
1 秒後の pane tick → `fetch_missing_data()` が呼ばれる。  
`visible_earliest < kline_earliest` が true の場合、live kline fetch を発行。  
fetch 完了後に `insert_hist_klines` でライブデータが replay chart に混入 → 表示が崩れる可能性。  
**可能性: 中**

### H3: StepBackward 前の in-flight fetch が完了後に live data を注入
reset 前に進行中だった fetch task が、reset 後に完了して `insert_hist_klines` を呼ぶ。  
`req_id` の検証なしに klines が挿入される。  
**可能性: 中**

### H4: CenterLatest autoscale の y translation リセット
`reset_for_seek` → `invalidate(None)` → 空データ → `latest_y_midpoint` = 0.0 → `translation.y = 0.0`  
次フレームで `ingest_historical_klines` → `invalidate(None)` で正しい translation に戻る。  
ただし同一 `update()` サイクルなら問題ないはず。  
**可能性: 低**（ただし H1 と組み合わせた場合は要確認）

### H5: active_streams と pane の streams のミスマッチ
`ingest_replay_klines` が stream を照合し、マッチしないと注入しない。  
active_streams が空 or 不一致の場合、chart はクリアされたままになる。  
**可能性: 低**（StepForward でも同じ streams を使用するため）

### H6: klines_in の範囲計算ミス
`new_time` の計算や `klines_in(stream, 0..new_time + 1)` が期待する klines を返さない。  
**可能性: 低**

### H7: fetch_missing_data が is_empty チェックを通過して live fetch を発行
`reset_for_seek` で timeseries がクリアされた直後（`ingest_replay_klines` の前）に tick が割り込む。  
timeseries が空 → `is_empty` チェック通過 → 現在時刻ベースで live fetch 発行。  
**可能性: 低**（tick は別メッセージで、同一サイクル内には割り込まない）

---

## 調査ポイント（ログ追加箇所）

| 箇所 | ログ内容 |
|---|---|
| `main.rs` StepBackward | `current_time`, `new_time`, stream ごとの klines 件数 |
| `kline.rs::reset_for_seek` | 呼び出し、リセット前のデータ件数 |
| `kline.rs::ingest_historical_klines` | 注入後のデータ件数 |
| `kline.rs::fetch_missing_data` | RequestFetch 発行の有無と範囲 |
| `kline.rs::insert_hist_klines` | req_id, 挿入 klines 件数（stale fetch 検出） |

---

## 作業ログ

### 2026-04-13 調査開始
- コード分析により上記仮説を作成
- 最有力: H2（次 tick での live fetch）+ H3（stale in-flight fetch）
- ログ追加 → ビルド → 確認フェーズへ

### 2026-04-13 ログ解析・原因特定・修正完了 ✅

**実際の原因（ログで確定）：**

```
StepBackward: reset_for_seek(656 klines → 0) + ingest(8 replay klines)
→ 900ms 後 tick: visible_earliest < kline_earliest → Binance live API fetch 発行
→ 約30ms で 641 live klines 挿入（replay チャートに live data 混入）
→ 次の StepBackward: 649 klines（8 replay + 641 live）をクリア → 8 klines に縮小
→ ユーザーに「バーが一瞬消える」と見える（649→8→次tick後649...の繰り返し）
```

**修正内容（H2 + H3 に対処）：**

`src/chart/kline.rs`
- `KlineChart` に `replay_mode: bool` フィールドを追加
- `fetch_missing_data()` で `self.replay_mode` が true なら即 `None` を返す（live fetch 完全抑制）
- `set_replay_mode(bool)` メソッドを追加（true 設定時に request_handler もリセット）

`src/screen/dashboard/pane.rs`
- `rebuild_content()` で新チャートに `set_replay_mode(replay_mode)` を呼ぶ
- `reset_for_seek()` で `c.set_replay_mode(true)` を呼ぶ
- `ingest_replay_klines()` で `c.set_replay_mode(true)` を呼ぶ

**フラグの生存サイクル：**
- Live → Replay 切替: `rebuild_content(true)` → `set_replay_mode(true)`
- Seek（StepBackward 等）: `reset_for_seek()` → `set_replay_mode(true)`
- Replay → Live 切替: `rebuild_content(false)` → `set_replay_mode(false)`

---

## 修正方針

---

## 完了条件

- ⏮ クリック時にチャートがチラつかない
- ⏭ と同様に滑らかに 1 ステップ戻る
- 既存の E2E テストが全て PASS
