# E2E テスト: サイドバー注文ペイン開く（`/api/sidebar/open-order-pane`）

## 概要

`sidebar_order_button.md` で実装したサイドバー注文ボタン機能（`split_focused_and_init_order`）を
HTTP API 経由で E2E テストできるようにする。

**新規エンドポイント:**
- `POST /api/sidebar/open-order-pane` — フォーカスペインを Horizontal Split し、指定種別の注文ペインを新ペインに表示する

**新規 E2E スクリプト:**
- `tests/e2e_scripts/s36_sidebar_order_pane.sh`

---

## 変更ファイル一覧

| ファイル | 変更種別 | 内容 |
|---|---|---|
| `src/replay_api.rs` | 修正 | `PaneCommand::OpenOrderPane` 追加・ルート追加・パーサー追加・unit tests |
| `src/main.rs` | 修正 | `parse_content_kind` に OrderEntry/OrderList/BuyingPower 追加・`handle_pane_api` アーム追加・`pane_api_open_order_pane` 実装 |
| `tests/e2e_scripts/s36_sidebar_order_pane.sh` | 新規 | TC-A〜E の E2E テスト |
| `docs/plan/e2e_sidebar_order_pane.md` | 新規 | 本計画書 |

---

## 詳細設計

### 1. `src/replay_api.rs` — PaneCommand に OpenOrderPane 追加

```rust
/// 注文ペインを開く（POST /api/sidebar/open-order-pane）
/// kind: "OrderEntry" | "OrderList" | "BuyingPower"
OpenOrderPane { kind: String },
```

ルート登録:
```rust
("POST", "/api/sidebar/open-order-pane") => parse_open_order_pane(body),
```

パーサー:
```rust
fn parse_open_order_pane(body: &str) -> Result<ApiCommand, RouteError> {
    let kind = body_str_field(body, "kind")?;
    match kind.as_str() {
        "OrderEntry" | "OrderList" | "BuyingPower" => {}
        _ => return Err(RouteError::BadRequest),
    }
    Ok(ApiCommand::Pane(PaneCommand::OpenOrderPane { kind }))
}
```

unit tests:
- `route_post_sidebar_open_order_pane_order_entry` — 正常系
- `route_post_sidebar_open_order_pane_order_list` — 正常系
- `route_post_sidebar_open_order_pane_buying_power` — 正常系
- `route_post_sidebar_open_order_pane_invalid_kind` — 異常系 BadRequest
- `route_post_sidebar_open_order_pane_missing_kind` — 異常系 BadRequest

---

### 2. `src/main.rs` — ハンドラ追加

#### parse_content_kind に追加

```rust
"OrderEntry" | "Order Entry" => Some(ContentKind::OrderEntry),
"OrderList" | "Order List" => Some(ContentKind::OrderList),
"BuyingPower" | "Buying Power" => Some(ContentKind::BuyingPower),
```

#### handle_pane_api に追加

```rust
PaneCommand::OpenOrderPane { kind } => self.pane_api_open_order_pane(&kind),
```

#### pane_api_open_order_pane 実装

```rust
fn pane_api_open_order_pane(&mut self, kind_str: &str) -> (String, Task<Message>) {
    let kind = match Self::parse_content_kind(kind_str) {
        Some(k) => k,
        None => return (format!(r#"{{"error":"invalid kind: {kind_str}"}}"#), Task::none()),
    };
    let main_window_id = self.main_window.id;
    let task = self
        .active_dashboard_mut()
        .split_focused_and_init_order(main_window_id, kind)
        .map(move |msg| Message::Dashboard { layout_id: None, event: msg });
    let ok = serde_json::json!({ "ok": true, "action": "open-order-pane", "kind": kind_str });
    (ok.to_string(), task)
}
```

---

### 3. E2E テスト `s36_sidebar_order_pane.sh`

**フィクスチャ:** 単一ペイン BinanceLinear:BTCUSDT M1、replay モード

**検証項目:**
| TC | 操作 | 検証 |
|---|---|---|
| TC-A | `{"kind":"OrderEntry"}` POST | ペイン数 2・新ペイン type = "Order Entry" |
| TC-B | `{"kind":"OrderList"}` POST | ペイン数 3・新ペイン type = "Order List" |
| TC-C | `{"kind":"BuyingPower"}` POST | ペイン数 4・新ペイン type = "Buying Power" |
| TC-D | - | エラー通知 0 件 |
| TC-E | - | 元ペイン (PANE0) の type が "Candlestick Chart" のまま |

---

## 実装メモ

### `split_focused_and_init_order` のフォーカス挙動
- `auto_focus_single_pane` により、単一ペイン時は focus=None でも自動フォーカスされる
- 2 回目以降の split では、直前に作成されたペインが focus になる（`self.focus = Some((window, new_pane))`）
- TC-B: 2 個目の split は Order Entry ペインを分割する → 合計 3 ペイン
- TC-C: 3 個目の split は Order List ペインを分割する → 合計 4 ペイン
- TC-E では元の BTCUSDT チャートペイン（PANE0）の type が変わっていないことを確認する

### `parse_content_kind` の拡張
既存の関数は "OrderEntry"/"OrderList"/"BuyingPower" を扱っていなかった（order ペインは
サイドバーボタン経由のみ想定）。API ハンドラ内でパース用に追加する。

---

## 進捗

- ✅ `docs/plan/e2e_sidebar_order_pane.md` 作成
- ✅ `src/replay_api.rs` — PaneCommand::OpenOrderPane + route + parser + unit tests
- ✅ `src/main.rs` — parse_content_kind 拡張 + handle_pane_api + pane_api_open_order_pane
- ✅ `tests/e2e_scripts/s36_sidebar_order_pane.sh` 作成
- ✅ `cargo clippy -- -D warnings` — 警告なし
- ✅ `cargo test` — 全パス
