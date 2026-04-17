# リプレイ機能

過去のチャートデータを再生し、当時の動きを体験できる機能。REPLAY モード中は仮想注文も出せる。

## LIVE ↔ REPLAY 切替

- ヘッダーバーの **`[LIVE]`** または **`[REPLAY]`** ボタンをクリック
- キーボード: **F5**
- HTTP API: `POST /api/replay/toggle`

LIVE → REPLAY に切り替えると、WebSocket が切断されてリアルタイム更新が止まる。REPLAY → LIVE に戻ると再接続される。

---

## 再生の開始

1. **REPLAY モードに切り替える**（上記）

2. **期間を入力する**

   ヘッダーバーの入力フィールドに開始・終了を入力:
   ```
   2026-04-01 09:00  ~  2026-04-01 15:00
   ```
   フォーマット: `YYYY-MM-DD HH:MM`（UTC として解釈）

3. **▶ ボタンを押す**

   - データをサーバーから取得開始（`Loading...` 表示）
   - 取得完了後、自動で再生スタート

---

## 再生中の操作

| 操作 | 説明 |
|------|------|
| **⏸ 一時停止** | 現在時刻で停止する |
| **▶ 再開** | 停止中から再生を再開 |
| **⏭ 次バー** | 1 本分だけ進む（Paused 中のみ）|
| **◀ 前バー** | 1 本分だけ戻す |
| **⏮ リセット** | 開始位置に戻して停止 |
| **1x / 2x ボタン** | 再生速度を変える（1x → 2x → 5x → 10x → 1x） |

### 再生速度

| 速度 | 1 バーあたりの時間（wall time） |
|------|-------------------------------|
| 1x | 100ms |
| 2x | 50ms |
| 5x | 20ms |
| 10x | 10ms |

速度を変えても再生位置はリセットされない。

### 再生中のペイン操作

リプレイ再生中でも以下の操作が可能:

- **ペインの追加・削除**（追加したペインはそのタイムフレームのデータを自動で取得して追いつく）
- **タイムフレームの変更**（変更すると開始位置にリセットされる）
- **銘柄の変更**（変更すると開始位置にリセットされる）

リプレイ中に変更できない操作:

- **ペインの drag/resize**（レイアウト変更は不可）

---

## 取引所・データ種別ごとの対応状況

| 取引所 | ローソク足 | 歩み値（Trades） |
|--------|-----------|-----------------|
| Binance Spot / Linear / Inverse | ✅ 全 tf | ✅ |
| Bybit | ✅ 全 tf | — |
| Hyperliquid | ✅ 全 tf | — |
| OKX | ✅ 全 tf | — |
| MEXC | ✅ 全 tf | — |
| 立花証券 | ✅ 日足のみ | — |

**板情報（Depth）はすべての取引所でリプレイ不可**（過去スナップショット API 非提供）。ヒートマップ・DOM・Ladder ペインにはリプレイ中に「Replay: Depth unavailable」が表示される。

---

## 仮想取引（REPLAY モード中）

REPLAY モード中は、証券 API を使わない仮想注文が出せる。

### 注文の出し方

1. サイドバーの **🖊（鉛筆）** ボタン → **Order Entry** を選択してペインを開く
2. 売買区分・数量・価格を入力
3. **「仮想注文確認」** ボタンを押す（パスワード入力は不要）
4. 確認モーダルで発注

REPLAY モード中は注文パネルに **「⏪ REPLAYモード中 — 注文は無効です」** バナーが表示され、パスワード欄が非表示になる。

### 約定ルール

| 注文種別 | 約定タイミング |
|---------|--------------|
| 成行 | 次の tick（バー）で、その tick の最初の価格で約定 |
| 指値買い | `約定価格 ≤ 指値` のトレードが届いた tick で約定 |
| 指値売り | `約定価格 ≥ 指値` のトレードが届いた tick で約定 |

**注意**: 現時点では StepForward 時にローソク足の終値から合成したトレードで約定判定を行う。リアルな tick データによる判定は Phase 2 予定。

### 約定通知

約定すると Toast 通知が表示される:
```
[仮想] 約定: BTCUSDT Long 0.1 @ 92500.00
```

### ポートフォリオの確認

HTTP API で確認できる:

```bash
curl http://127.0.0.1:9876/api/replay/portfolio
```

```json
{
  "cash": 985250.0,
  "unrealized_pnl": 230.5,
  "realized_pnl": 1200.0,
  "total_equity": 986680.5,
  "open_positions": [ ... ],
  "closed_positions": [ ... ]
}
```

- 初期資金: 1,000,000 固定
- seek（リセット・巻き戻し）するとポートフォリオも初期化される

### HTTP API で仮想注文を出す

```bash
# 成行買い
curl -X POST http://127.0.0.1:9876/api/replay/order \
  -H "Content-Type: application/json" \
  -d '{"ticker":"BTCUSDT","side":"buy","qty":0.1,"order_type":"market"}'

# 指値売り
curl -X POST http://127.0.0.1:9876/api/replay/order \
  -H "Content-Type: application/json" \
  -d '{"ticker":"BTCUSDT","side":"sell","qty":0.1,"order_type":{"limit":93000.0}}'
```

---

## 起動時の自動再生

`POST /api/app/save` または通常終了で保存した状態に REPLAY 構成が含まれている場合、次回起動時に自動で同じ区間を再生開始する。

手動で無効にするには: LIVE モードに切り替えてから状態を保存する。

---

## HTTP API によるリプレイ制御

外部スクリプトやツールからリプレイを操作できる。

```bash
# 現在の状態を確認
curl http://127.0.0.1:9876/api/replay/status

# 再生開始
curl -X POST http://127.0.0.1:9876/api/replay/play \
  -H "Content-Type: application/json" \
  -d '{"start":"2026-04-01 09:00","end":"2026-04-01 15:00"}'

# 一時停止 / 再開
curl -X POST http://127.0.0.1:9876/api/replay/pause
curl -X POST http://127.0.0.1:9876/api/replay/resume

# 1 バー進む / 戻る
curl -X POST http://127.0.0.1:9876/api/replay/step-forward
curl -X POST http://127.0.0.1:9876/api/replay/step-backward

# 速度循環
curl -X POST http://127.0.0.1:9876/api/replay/speed
```

全エンドポイントは [開発者仕様書 (GitHub)](https://github.com/flowsurface-rs/flowsurface/blob/main/docs/spec/replay_header.md#11-http-制御-api) を参照。
