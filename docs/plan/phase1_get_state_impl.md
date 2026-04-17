# GET /api/replay/state 実装計画（Phase 1）

## 実装方針と設計の背景

`GET /api/replay/state` は現在骨格のみ（`not_implemented: true`）。
`ReplaySession::Active { clock, store, active_streams }` からデータを取り出し、
現在時刻の直前 N 件の Klines と Trades を JSON で返す。

### 設計上の決定

| 決定事項 | 選択値 | 理由 |
| :--- | :--- | :--- |
| 返す件数 | klines: 直前50件、trades: 直前50件 | Phase 1 は定数。将来クエリパラメータ化可 |
| 時間ウィンドウ | klines: 無制限（時刻降順で最大50件）、trades: `current_time - 300_000ms..current_time` | klines は bar 数ベース、trades は時間ウィンドウが自然 |
| Klines クエリ方法 | `store.klines_in(stream, 0..current_time+1)` で全取得し末尾 50 件を slice | `klines_in` は binary search で O(log n)、その後の slice は O(1) |
| Trades クエリ方法 | `store.trades_in(stream, (current_time - WINDOW)..current_time+1)` | 5分ウィンドウで直近 trades を取得 |
| Trade stream の特定 | `active_streams` の kline stream から `StreamKind::Trades { ticker_info }` を導出 | active_streams は kline のみ保持するため、ticker_info を流用してトレードストリームキーを生成 |
| Serialize 対応 | `KlineStateItem` / `TradeStateItem` 中間 struct を `main.rs` に定義 | `exchange::Kline` / `Trade` は Serialize 未実装 |
| stream 文字列 | Kline: `"{ticker}:{timeframe}"` (例 `"BinanceLinear:BTCUSDT:1m"`)、Trades: `"{ticker}:Trades"` | 既存の Display impl を活用 |
| アクセス経路 | `ReplayController::get_api_state(limit)` を新設 | controller の "内部直接アクセス禁止" 設計を維持 |
| Idle 時のレスポンス | 400 エラー（既存エンドポイントと同じ挙動） | 依頼仕様通り |

### データフロー

```
GET /api/replay/state
  → main.rs ハンドラ
  → self.replay.get_api_state(50)   // ReplayController の新メソッド
  → ReplaySession::Active { clock, store, active_streams }
  → active_streams の各 kline stream でクエリ
      → store.klines_in(kline_stream, 0..=now) → 末尾 50 件
      → store.trades_in(trades_stream, (now-300s)..now) → 最大 50 件
  → KlineStateItem / TradeStateItem に変換
  → JSON シリアライズ
```

## 作業リスト

- [x] 計画書作成
- [x] `ReplayController::get_api_state(limit)` の実装
  - controller.rs に `ApiStateData` 構造体と `get_api_state` メソッドを追加
- [x] `KlineStateItem` / `TradeStateItem` の定義（main.rs）
- [x] `GetState` ハンドラの完成（main.rs）
- [x] ユニットテスト（controller.rs）
  - `get_api_state_returns_none_when_idle`
  - `get_api_state_returns_none_when_loading`
  - `get_api_state_returns_current_time_when_active_empty_store`
  - `get_api_state_returns_klines_and_trades_from_active_store`
  - `get_api_state_limits_klines_to_n_most_recent`
  - `get_api_state_stream_label_format`
- [x] `cargo test` 通過確認（299 passed）
- [x] `cargo clippy -- -D warnings` 通過確認
- [x] `cargo fmt` 適用
- [x] E2E テスト作成
  - `tests/s43_get_state_endpoint.sh`（新規: TC-A〜K/L）
  - `tests/s34_virtual_order_basic.sh` TC-L を実データ検証に拡張
  - `tests/run_all_binance.sh` に s43 を追加

## 注意点・知見

- `active_streams` は kline streams のみ保持（Trades stream は含まない）。
  Trades を取得するには kline stream の `ticker_info` から
  `StreamKind::Trades { ticker_info }` を構築してストアをクエリする。
- `exchange::Kline` に `Serialize` は実装されていない。中間 DTO が必須。
- `Price::to_f64()` で f64 変換可能。`Qty::to_f32_lossy()` で f32 変換可能。
- `Timeframe` は `Display` を実装済み（`format!("{tf}")` → "1m", "5m" など）。
- `Volume::total()` → `Qty`。`Qty::to_f32_lossy()` で浮動小数点に変換。
- `EventStore` のメソッドは全て `&[T]` を返す（cloneが必要）。

## 完了の定義

- [x] `GET /api/replay/state` が OHLCV（Klines）と Trades の実データを返す
- [x] リプレイセッションが Idle のとき 400 エラーを返す
- [x] ユニットテストが通る（`cargo test`）
- [x] `cargo clippy -- -D warnings` がエラーなし
- [x] この計画書が作成・更新されている
