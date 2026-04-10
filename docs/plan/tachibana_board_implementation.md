# 立花証券 板情報（リアルタイム板）実装計画書

**作成日**: 2026-04-10
**実装完了日**: 2026-04-10
**前提**: Phase 1〜2 完了済み（認証・日足チャート・銘柄マスタ）
**目標**: EVENT I/F 経由で板情報・歩み値・現在値をリアルタイム受信し、Ladder（板表示）パネルに反映する

---

## 1. 現状と目標

### 実装済み（本計画で完了）
| 機能 | 方式 | 状態 |
|------|------|------|
| **板情報** | EVENT I/F HTTP Long-polling (`FD`コマンド) | ✅ 完了 |
| **歩み値** | EVENT I/F HTTP Long-polling (`ST`コマンド) | ✅ 完了 |
| **現在値** | EVENT I/F HTTP Long-polling (`KP`コマンド) | ✅ 完了 |

---

## 2. EVENT I/F プロトコル仕様

### 2.1 接続方式

> **実装時の重要発見 (2026-04-10):**
> WebSocket (`sUrlEventWebSocket`) は `fastwebsockets` + `hyper` のハンドシェイクで **HTTP 400** が返され接続不可。
> 原因は立花証券サーバーが `fastwebsockets` のハンドシェイクヘッダーを受け付けないため。
> **HTTP Long-polling (`sUrlEvent`)** で実装し、正常に動作を確認。

ログイン応答で取得した仮想URLを使用:
- ~~**WebSocket**: `{sUrlEventWebSocket}?{params}`~~ （400エラーのため不使用）
- **HTTP Long-polling**: `{sUrlEvent}?{params}` ← **採用**

HTTP Long-polling は公式サンプル `e_api_sample_v4r8.py` の方式と同一:
```python
p_ss = requests.session()
p_res = p_ss.get(p_url, stream=True)
for p_rec in p_res.iter_lines():
    pi_cbp(p_rec.decode('ascii'))
```

### 2.2 パラメータ形式

> **実装時の重要発見 (2026-04-10):**
> パラメータの**順序は固定**（公式サンプル `e_api_websocket_receive_tel.py` 573行目: 「先頭の項目。順番の変更は不可。」）。
> `p_rid` が先頭、`p_issue_code` が最後。順序を変えると 400 エラーになる。
> 参照: `docs/e-shiten/samples/e_api_websocket_receive_tel.py/e_api_websocket_receive_tel.py` の `func_make_websocket_url()` (573-589行目)

正しい順序:
```
p_rid=22&p_board_no=1000&p_gyou_no=1&p_mkt_code=00&p_eno=0&p_evt_cmd=ST,KP,FD&p_issue_code=7203
```

| パラメータ | 意味 | 値 | 順序 |
|-----------|------|-----|------|
| `p_rid` | リクエストID | `22`（固定値） | 1（先頭、変更不可） |
| `p_board_no` | ボード番号 | `1000`（固定値） | 2 |
| `p_gyou_no` | 行番号（銘柄スロット） | `1` | 3 |
| `p_mkt_code` | 市場コード（東証=`00`） | `00` | 4 |
| `p_eno` | イベント番号（開始位置） | `0` | 5 |
| `p_evt_cmd` | 購読コマンド | `ST,KP,FD` | 6 |
| `p_issue_code` | 銘柄コード | `7203` | 7（最後） |

### 2.3 受信データフォーマット

```
\x01  (SOH) = レコード（項目値）区切り
\x02  (STX) = カラム名:値 区切り
\x03  (ETX) = 値のサブ区切り（複数値を持つフィールド内）
```

- エンコーディング: **ASCII**（REST の Shift-JIS とは異なる）
- HTTP Long-polling では **1行 = 1イベント**（改行区切り）

### 2.4 フィールド名の命名規則

> **実装時の重要発見 (2026-04-10):**
> フィールド名は `p_{行番号}_{情報コード}` 形式。
> 公式サンプル `e_api_websocket_receive_tel.py` 752行目:
> 「`p_1_DPP` は、p:プレーン文字列_1:行番号_DPP:現在値」
> 参照: `docs/e-shiten/samples/e_api_websocket_receive_tel.py/e_api_websocket_receive_tel.py`

例: `p_1_DPP\x023319` → フィールド名 `p_1_DPP`、値 `3319`

### 2.5 板情報フィールド（FD コマンドで受信）

> **実装時の重要発見 (2026-04-10):**
> 板情報のフィールド名は **`GAP`/`GAV`/`GBP`/`GBV`** であり、
> 当初想定していた `QAP`/`QAV`/`QBP`/`QBV` ではなかった。
> デモ環境（本番セッション）で TOYOTA (7203) の実データ受信により確認。

| フィールド | 意味 | 実データ例 (TOYOTA 7203) |
|-----------|------|--------------------------|
| `p_1_GAP1`〜`p_1_GAP10` | 売気配価格（最良→上位） | `3319`〜`3328` |
| `p_1_GAV1`〜`p_1_GAV10` | 売気配数量 | `10000`〜`19300` |
| `p_1_GBP1`〜`p_1_GBP10` | 買気配価格（最良→下位） | `3318`〜`3309` |
| `p_1_GBV1`〜`p_1_GBV10` | 買気配数量 | `6300`〜`4000` |
| `p_1_QAP` | 最良売気配価格（スカラー） | `3319` |
| `p_1_QBP` | 最良買気配価格（スカラー） | `3318` |
| `p_1_QAS` | 売気配状態 | `0101` |
| `p_1_QBS` | 買気配状態 | `0101` |
| `p_1_QOV` | OVER 数量 | `4218600` |
| `p_1_QUV` | UNDER 数量 | `2520200` |
| `p_1_AV` | 売気配合計数量 | `10000` |
| `p_1_BV` | 買気配合計数量 | `6300` |

### 2.6 FD フレーム全フィールド（実データ）

TOYOTA (7203) で受信した FD フレームの全69フィールド:
```
p_no, p_date, p_cmd(=FD),
p_1_AV, p_1_BV, p_1_DHF, p_1_DHP, p_1_DHP:T, p_1_DJ,
p_1_DLF, p_1_DLP, p_1_DLP:T, p_1_DOP, p_1_DOP:T,
p_1_DPG, p_1_DPP, p_1_DPP:T, p_1_DV, p_1_DYRP, p_1_DYWP,
p_1_GAP1〜GAP10, p_1_GAV1〜GAV10,
p_1_GBP1〜GBP10, p_1_GBV1〜GBV10,
p_1_LISS, p_1_PRP, p_1_QAP, p_1_QAS, p_1_QBP, p_1_QBS,
p_1_QOV, p_1_QUV, p_1_VWAP
```

### 2.7 歩み値フィールド（ST コマンドで受信）

| フィールド | 意味 |
|-----------|------|
| `p_1_DPP` | 約定価格 |
| `p_1_DV` | 約定数量 |
| `p_1_DPP:T` | 約定時刻 |
| `p_1_DYSS` | 売買区分 ("1"=売) |

### 2.8 コマンド判別

各イベント行には `p_cmd` フィールドがあり、コマンド種別を判別:
- `p_cmd=FD` → 板情報 → `fields_to_depth()` で処理
- `p_cmd=ST` → 歩み値 → `fields_to_trade()` で処理
- `p_cmd=KP` → 現在値 → 現在は未処理（将来的に mid_price 更新に活用）

> **実装時の重要発見 (2026-04-10):**
> `p_cmd` チェックなしで `fields_to_trade()` を呼ぶと、FD フレーム内の
> `p_1_DPP`（終値）と `p_1_DV`（出来高）が Trade として誤パースされ、
> 異常な値が Ladder に渡されてクラッシュ（`assertion failed: p.y.is_finite()`）する。
> `p_cmd=ST` の場合のみ Trade を生成するように修正。

---

## 3. 実装ステップ（完了報告）

### ✅ Step 1: EVENT I/F パーサー（tachibana.rs 内に追加）

**変更ファイル**: `exchange/src/adapter/tachibana.rs`

実装した関数:
- `parse_event_frame(data: &str) -> Vec<(&str, &str)>` — SOH/STX 区切りパーサー
- `fields_to_depth(fields) -> Option<DepthPayload>` — FD コマンドから板情報を抽出（`_GAP`/`_GBP` 末尾マッチ）
- `fields_to_trade(fields) -> Option<Trade>` — ST コマンドから歩み値を抽出（`_DPP`/`_DV` 末尾マッチ）
- `build_event_params(issue_code, market_code) -> String` — パラメータ構築（公式準拠の固定順序）

**テスト**: 14件（パーサー4件 + 板情報4件 + 歩み値5件 + パラメータ1件）

### ✅ Step 2: HTTP Long-polling ストリーム + セッション URL 受け渡し + connect.rs スタブ解除

**変更ファイル**: `exchange/src/adapter/tachibana.rs`, `exchange/src/connect.rs`, `src/connector/auth.rs`

実装内容:
- `EVENT_HTTP_URL: RwLock<Option<String>>` — HTTP Long-polling URL の static 保持
- `EVENT_WS_URL: RwLock<Option<String>>` — WebSocket URL の static 保持（将来用）
- `set_event_http_url()` / `set_event_ws_url()` — auth.rs から呼び出し
- `connect_event_stream(ticker_info, push_freq) -> impl Stream<Item = Event>` — HTTP Long-polling ストリーム

> **設計決定（WebSocket → HTTP Long-polling）:**
> - `fastwebsockets` + `hyper` のカスタム WebSocket ハンドシェイクが立花証券サーバーで **HTTP 400** 拒否
> - 公式サンプル（Python `websockets`）は動作するが、ヘッダー構成が異なる
> - HTTP Long-polling は公式サンプル `e_api_sample_v4r8.py` の `requests.session().get(url, stream=True)` 方式
> - `reqwest` の `bytes_stream()` で同等の機能を実現
> - 将来的に `tokio-tungstenite` 等で WebSocket 対応を検討可能

### ✅ Step 3: デモ環境テスト

本番セッション（`kabuka.e-shiten.jp`）で動作確認済み:
- TOYOTA (7203): 売10本 + 買10本の板情報を正常受信
- KP（現在値）: 約5秒間隔で受信
- FD フレーム: 69フィールド、板データ含む

---

## 4. ファイル変更一覧（実装完了版）

| ファイル | 操作 | 内容 |
|---------|------|------|
| `exchange/src/adapter/tachibana.rs` | 変更 | パーサー追加、`connect_event_stream()` (HTTP Long-polling)、`EVENT_HTTP_URL`/`EVENT_WS_URL` static、テスト14件追加 |
| `exchange/src/connect.rs` | 変更 | `depth_stream` の Tachibana 分岐を `connect_event_stream` に置換 |
| `src/connector/auth.rs` | 変更 | `store_session()` で `set_event_ws_url()` + `set_event_http_url()` を呼び出す |
| `src/chart/indicator/plot.rs` | 変更 | クロスヘア描画のゼロ除算ガード追加（7.5 修正） |
| `exchange/src/lib.rs` | 変更なし | 既存の `Depth`, `Trade`, `Event` 型をそのまま使用 |

---

## 5. データフロー図（実装完了版）

```
立花証券 EVENT I/F (HTTP Long-polling)
  │
  │  GET https://{sUrlEvent}?p_rid=22&...&p_issue_code=7203
  │  (reqwest bytes_stream → 行ごとに処理)
  │
  ▼
tachibana.rs: parse_event_frame() — SOH/STX 区切りパーサー
  │
  ├─ p_cmd=FD (板情報)
  │   → fields_to_depth() — _GAP/_GAV/_GBP/_GBV 末尾マッチ
  │   → DepthPayload { asks: 10本, bids: 10本 }
  │   → LocalDepthCache::update(Snapshot) → Arc<Depth>
  │   → Event::DepthReceived
  │
  ├─ p_cmd=ST (歩み値)
  │   → fields_to_trade() — _DPP/_DV/_DYSS 末尾マッチ
  │   → Trade { price, qty, is_sell }
  │   → Event::TradesReceived
  │
  └─ p_cmd=KP (現在値)
      → 現在は未処理（約5秒間隔で受信）
  
  ▼
connect.rs → Subscription → dashboard
  │
  ├─ Ladder パネル: insert_depth(&depth) + insert_trades(&trades)
  └─ チャート: 将来的にリアルタイム足に活用
```

---

## 6. テスト結果

### 6.1 ユニットテスト（14件、全パス）

| # | テスト内容 | 状態 |
|---|-----------|------|
| 1 | `parse_event_frame`: 基本パース (`p_1_DPP` 形式) | ✅ |
| 2 | `parse_event_frame`: 空データ | ✅ |
| 3 | `parse_event_frame`: STX なしレコードのスキップ | ✅ |
| 4 | `parse_event_frame`: 空カラム名のスキップ | ✅ |
| 5 | `fields_to_depth`: 売10本+買10本 (p_cmd=FD, _GAP/_GBP) | ✅ |
| 6 | `fields_to_depth`: 部分的な板情報 | ✅ |
| 7 | `fields_to_depth`: 板データなし (p_cmd=KP) → None | ✅ |
| 8 | `fields_to_depth`: `"*"` 値のスキップ | ✅ |
| 9 | `fields_to_trade`: 基本 (p_cmd=ST, 売り) | ✅ |
| 10 | `fields_to_trade`: 買い側 | ✅ |
| 11 | `fields_to_trade`: `"*"` 価格 → None | ✅ |
| 12 | `fields_to_trade`: 数量欠損 → None | ✅ |
| 13 | `fields_to_trade`: p_cmd=FD → None（Trade 生成しない） | ✅ |
| 14 | `build_event_params`: パラメータ構築 | ✅ |

### 6.2 実データテスト

| # | テスト内容 | 結果 |
|---|-----------|------|
| 1 | 本番環境 TOYOTA (7203) 板受信 | ✅ asks=10, bids=10 |
| 2 | KP（現在値）定期受信 | ✅ 約5秒間隔 |
| 3 | アプリクラッシュなし（7.4 修正後） | ✅ 安定動作 |
| 4 | インジケーター クロスヘア描画（7.5 修正後） | ✅ データ未到着時もクラッシュなし |

---

## 7. 発見された問題と解決策

### 7.1 WebSocket 400 エラー

**問題**: `fastwebsockets` + `hyper` の WebSocket ハンドシェイクが立花証券サーバーで HTTP 400 拒否。
**原因推定**: `fastwebsockets` のハンドシェイクヘッダー構成が、立花証券サーバーの期待と合致しない。Python `websockets` は動作する（ヘッダーが異なる）。
**解決**: HTTP Long-polling (`sUrlEvent`) にフォールバック。`reqwest::Client::get().send().bytes_stream()` で公式サンプルと同等のストリーミングを実現。

### 7.2 パラメータ順序の制約

**問題**: パラメータの順序を変えると 400 エラー。
**原因**: 公式サンプル（`docs/e-shiten/samples/e_api_websocket_receive_tel.py/e_api_websocket_receive_tel.py` 577行目）に「先頭の項目。順番の変更は不可。」と記載。
**解決**: 公式準拠の順序に固定。`p_rid` が先頭、`p_issue_code` が最後。

### 7.3 フィールド名の不一致（GAP vs QAP）

**問題**: 当初 `pQAP1`〜`pQAP10` と想定したが、実データは `p_1_GAP1`〜`p_1_GAP10`。
**原因**: フィールド名は `p_{行番号}_{情報コード}` 形式であり、気配フィールドのコードは `GAP`/`GAV`/`GBP`/`GBV`。公式サンプル `e_api_websocket_receive_tel.py` 752行目に形式の記載あり。
**解決**: `_GAP`/`_GAV`/`_GBP`/`_GBV` の末尾マッチに変更。

### 7.4 FD フレームでの Trade 誤生成（クラッシュ）

**問題**: FD フレーム内の `p_1_DPP`（終値 3319）と `p_1_DV`（出来高 16930900）が Trade として誤パースされ、Ladder に異常値が渡されクラッシュ（`assertion failed: p.y.is_finite()` in `lyon_path`）。
**原因**: `fields_to_trade()` が `p_cmd` を見ずに全フレームの `DPP`/`DV` を Trade として変換していた。
**解決**: `p_cmd=ST` の場合のみ Trade を生成。同様に `fields_to_depth()` も `p_cmd=FD` の場合のみ板情報を生成。

### 7.5 インジケーター クロスヘア描画でのゼロ除算パニック（2026-04-10 修正）

**問題**: アプリ起動後 1〜2 分でクラッシュ（`assertion failed: p.y.is_finite()` in `lyon_path`）。EVENT I/F 接続後、チャートのボリュームインジケーター上にマウスカーソルがある状態で発生。
**原因**: `src/chart/indicator/plot.rs:326` のクロスヘア描画コードで、Y 軸レンジがゼロ（`lowest == highest`）の場合にゼロ除算が発生し NaN が `Path::line` に渡されていた。
```rust
// 修正前: lowest == highest のとき (lowest - highest) = 0 → NaN
let snap_ratio = (rounded - highest) / (lowest - highest);
```
`y_extents()` がデータ未到着時に `None` を返すと、フォールバック値 `(0.0, 0.0)` が使われ、`lowest == highest == 0.0` となる。
**解決**: `span` がゼロの場合はクロスヘア描画をスキップするガードを追加。
```rust
let span = lowest - highest;
if span.abs() < f32::EPSILON {
    return;
}
let snap_ratio = (rounded - highest) / span;
```
**変更ファイル**: `src/chart/indicator/plot.rs` (317-334行目)

---

## 8. 公式サンプル参照情報

実装で参照した公式サンプルファイル（`docs/e-shiten/samples/` 配下）:

| ファイル | 参照箇所 | 内容 |
|---------|---------|------|
| `e_api_websocket_receive_tel.py/e_api_websocket_receive_tel.py` | 573-589行目 | `func_make_websocket_url()` — パラメータ順序の仕様（先頭固定・変更不可） |
| 同上 | 752行目 | フィールド名形式 `p_{行番号}_{情報コード}` の解説 |
| 同上 | 754行目 | WebSocket 接続: `websockets.connect(pi_url, ping_interval=None)` |
| 同上 | 745-749行目 | 区切り子仕様: `^A`(SOH), `^B`(STX), `^C`(ETX) |
| `e_api_sample_v4r8.py/` | 412-415行目 | HTTP Long-polling: `requests.session().get(url, stream=True).iter_lines()` |
| 同上 | 460-468行目 | `proc_print_event_if_data()` — SOH/STX パーサーのリファレンス実装 |
| 同上 | 509行目 | パラメータ文字列の例: `p_rid=22&p_board_no=1000&...` |

---

## 9. 制約・前提条件

1. **HTTP Long-polling 方式**: WebSocket (fastwebsockets) は 400 エラーのため不使用。将来的に `tokio-tungstenite` で WebSocket 対応を検討。
2. **東証立会時間のみ**: 9:00-11:30, 12:30-15:30 JST。時間外は板データ更新なし。KP のみ定期受信。
3. **板は10本板**: 最良気配から10本（売10+買10）。
4. **単一銘柄**: 現在は1ストリームにつき1銘柄。複数銘柄の同時板表示は `p_gyou_no` を複数指定して拡張可能。
5. **電話認証が前提**: セッション開始前にユーザーが電話認証を完了していること。

---

## 10. 未対応・将来課題

| # | 課題 | 優先度 |
|---|------|--------|
| 1 | WebSocket 接続対応（`tokio-tungstenite` 等） | 低 |
| 2 | KP（現在値）の Ladder 反映 | 中 |
| 3 | 複数銘柄の同時板表示（`p_gyou_no` 複数指定） | 低 |
| 4 | セッション切れ → 自動再ログイン → ストリーム再生成 | 中 |
| 5 | OVER/UNDER 数量（`p_1_QOV`/`p_1_QUV`）の表示 | 低 |
| 6 | VWAP（`p_1_VWAP`）の表示 | 低 |

---

## 11. 関連ドキュメント

- [✅tachibana_review_report.md](✅tachibana_review_report.md) — EVENT I/F プロトコル詳細（セクション3）
- [✅tachibana_migration_plan.md](✅tachibana_migration_plan.md) — 全体移行プラン
- [✅tachibana_session_restore.md](✅tachibana_session_restore.md) — セッション永続化設計
- `docs/e-shiten/samples/e_api_websocket_receive_tel.py/` — EVENT I/F WebSocket 公式サンプル
- `docs/e-shiten/samples/e_api_sample_v4r8.py/` — EVENT I/F HTTP Long-polling 公式サンプル
