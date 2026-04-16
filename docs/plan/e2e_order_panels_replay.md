# E2E テスト計画: replay モード注文パネル 3種

作成日: 2026-04-16  
対象ブランチ: sasa/develop

---

## 目的

replay モードで

- **Order Entry** (`OrderEntry`)
- **Order List** (`OrderList`)
- **Buying Power** (`BuyingPower`)

の 3パネルを E2E テストする。

---

## 既存テストとのカバレッジ整理

| スクリプト | カバー内容 |
|-----------|-----------|
| **s34** | 注文 API 基本（place / errors / live guard） |
| **s35** | portfolio 初期スキーマ・Paused 中約定なし・Live↔Replay 遷移リセット |
| **s36** | 3パネル開閉（`open-order-pane` 経由、ペイン数 + type 確認のみ） |

**未カバーのシナリオ（本計画の対象）：**

| # | シナリオ | 理由 |
|---|---------|------|
| 1 | 3パネル同時起動 + 注文フロー統合 | s36 はパネル開閉のみ、s34/s35 はパネルなしで API を直接叩く |
| 2 | `GET /api/replay/orders` で pending 注文一覧取得 | エンドポイント未実装・未テスト |
| 3 | BuyingPower × `portfolio.cash` の初期整合性 | s35 は cash 値のみ確認、BuyingPower パネルとの紐付けなし |
| 4 | Playing 中に複数注文 → Pause → portfolio 変化なし連続シーケンス | s35 TC-H は 2件のみ・パネルなし |

---

## 必要な実装（テスト前提）

### GET /api/replay/orders エンドポイント（新規追加）

`OrderList` パネルの E2E テストには pending 注文の一覧を取得する API が必要。

**追加箇所：**

1. `src/replay_api.rs` — `VirtualExchangeCommand` に `GetOrders` バリアントを追加  
2. `src/replay_api.rs` — ルーティングに `("GET", "/api/replay/orders")` を追加  
3. `src/replay/virtual_exchange/` — `VirtualExchangeEngine::get_orders()` を実装  
4. `src/main.rs` — `VirtualExchangeCommand::GetOrders` の dispatch を追加

**レスポンス形式（暫定）：**

```json
{
  "orders": [
    {
      "order_id": "uuid",
      "ticker": "BTCUSDT",
      "side": "buy",
      "qty": 0.1,
      "order_type": "market",
      "limit_price": null,
      "status": "pending"
    }
  ]
}
```

---

## 新規テストスクリプト

### S37: 3パネル統合テスト（`s37_order_panels_integrated.sh`）

**概要：** OrderEntry / OrderList / BuyingPower の 3パネルを同時起動した状態で  
注文 API を叩き、アプリがクラッシュせず portfolio が正しく変化することを確認する。

**フィクスチャ：** BinanceLinear:BTCUSDT M1、replay auto-play

| TC | 操作 | 期待値 | s34/s35/s36 との差分 |
|----|------|--------|---------------------|
| A | 3パネルを順に open | pane count=4、エラー通知 0 件 | s36 と同操作だが **replay Playing 中** に行う |
| B | Playing 中に成行買い × 3件 place | 各 HTTP 200、order_id 返却 | s34 は 2件・パネルなし |
| C | Pause → portfolio 取得 | cash=1000000（Paused 中は約定なし） | s35 TC-H の確認、パネルあり環境で再検証 |
| D | Pause 中に指値買い × 2件・指値売り × 2件 place | 各 HTTP 200、status="pending" | s34 は各 1件のみ |
| E | portfolio.open_positions の長さ | 0（Paused 中は約定しない） | s35 TC-I の再確認、パネルあり環境 |
| F | 4ペインに対してエラー通知 0 件 | error count=0 | s36 TC-D のシナリオを注文後に再検証 |

---

### S38: 注文一覧 API テスト（`s38_order_list_api.sh`）

**前提:** `GET /api/replay/orders` エンドポイントの実装が完了していること。

**概要：** `GET /api/replay/orders` で pending 注文が正しく返ることを確認する。

| TC | 操作 | 期待値 |
|----|------|--------|
| A | LIVE 中に GET /api/replay/orders | HTTP 400（live guard） |
| B | REPLAY Paused で注文 0件のとき GET | HTTP 200、`orders=[]` |
| C | 成行買い 1件 place 後 GET | `orders` length=1、order_id 一致 |
| D | さらに指値買い 1件 place 後 GET | `orders` length=2 |
| E | 各注文のフィールド検証 | ticker/side/qty/order_type/status がすべて存在 |
| F | Live→Replay 再遷移（reset）後 GET | `orders=[]`（エンジンリセット） |

---

### S39: BuyingPower × portfolio 整合性テスト（`s39_buying_power_portfolio.sh`）

**概要：** BuyingPower パネルが開いている状態で `portfolio.cash` と整合することを確認。  
現時点では HTTP API で cash を検証する（パネルの描画内容は直接取得不可）。

| TC | 操作 | 期待値 |
|----|------|--------|
| A | BuyingPower パネルを開く | pane count=2、type="Buying Power" |
| B | Paused で portfolio.cash 確認 | cash=1000000 |
| C | Paused で成行買い × 3件 place | HTTP 200 |
| D | Paused のまま portfolio.cash 確認 | 変化なし（約定していないため） |
| E | Live→Replay 遷移後 portfolio.cash 確認 | 1000000 にリセット |
| F | BuyingPower パネルのエラー通知なし | error count=0 |

> **補足（既知制限）：** Trades EventStore 未統合のため、Playing 中でも on_tick に trade が来ず  
> 市場注文は約定しない（`docs/order_windows.md §未実装`）。  
> 約定後の cash 減少テスト（TC-G 相当）は EventStore 統合後に追加予定。

---

## 実装・作業順序

- ✅ `GET /api/replay/orders` エンドポイント実装（s38 の前提）
  - `src/replay_api.rs` に `GetOrders` ルート追加
  - `VirtualExchangeEngine::get_orders()` 実装（`VirtualOrderBook::pending_orders()` 経由）
  - `src/main.rs` の dispatch 追加
  - `replay_api.rs` 内ユニットテスト追加（`route_get_replay_orders`, `route_post_replay_orders_not_found`）
  - `virtual_exchange/mod.rs` 内ユニットテスト追加（初期空・place 後・複数・reset 後）
- ✅ **S37** スクリプト作成（`tests/e2e_scripts/s37_order_panels_integrated.sh`）
- [ ] **S37** 実行・パス確認
- [ ] **S38** スクリプト作成・実行（エンドポイント実装後）
- ✅ **S39** スクリプト作成（`tests/e2e_scripts/s39_buying_power_portfolio.sh`）
- [ ] **S39** 実行・パス確認
- [ ] 全スクリプト CI 統合（既存 e2e スイートに追加）

---

## 備考・未実装事項

| 項目 | 状況 |
|------|------|
| StepBackward 後のエンジンリセット | 未実装（s35 TC-J で PEND 済み） |
| Playing 中の約定（Trades EventStore 統合） | 未実装（`docs/order_windows.md §未実装`） |
| OrderEntry UI フォーム入力の E2E | HTTP API では UI 内部状態を取得できないため現在検証不可 |
