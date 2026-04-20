# `market_subscriptions()` リファクタリング計画書

## 背景と設計思想

`src/screen/dashboard.rs` の `market_subscriptions()` 関数（旧 L1834–L1914）には、
**depth / trade / kline** の3種類のサブスクリプション構築ブロックが重複していた。
各ブロックのパターンは「空なら skip → StreamConfig 生成 → Subscription::batch に詰める」と同一。

### 分割方針

各ストリーム種別の責務を `build_*_subs` 小関数に切り出した。
返り値を `Option<Subscription<exchange::Event>>` にすることで呼び出し側が `.flatten()` で
空ケースを自然にスキップできる。

`depth` だけ `chunks()` を使わない（1ティッカー = 1 WebSocket接続）ため、関数の形は統一しつつ
実装は個別に持つ。`depth` は `ticker.exchange()` を使用するため exchange 引数は不要 → `_exchange` で明示。

### exchange/ クレートは変更しない

`StreamSpecs`・`UniqueStreams`・定数はすべて読み取り専用で利用した。

---

## 作業項目

- ✅ TDDステップ1: テストを先に書く（現行コードで Green になること確認）
- ✅ TDDステップ2: `cargo test` でテストが Green になることを確認（371 PASS）
- ✅ TDDステップ3: 小関数へのリファクタリングを実施
- ✅ TDDステップ4: リファクタリング後も `cargo test` が全 PASS （371 PASS）
- ✅ `cargo clippy` が警告ゼロ
- ✅ `cargo fmt` でフォーマット適用
- ✅ `src/screen/dashboard/pane/view.rs:594` の既存コンパイルエラー（`exchange::PriceStep` private）も修正

---

## テスト戦略

`Subscription<exchange::Event>` は `iced` の型であり内部状態の比較ができない。
そのため **抽出後の `build_*_subs` 関数の振る舞いをユニットテストする**戦略を採った。

### 追加したテストケース（10件）

| テスト名 | 検証内容 |
|---------|---------|
| `build_depth_subs_returns_none_when_specs_depth_is_empty` | depth 空 → None |
| `build_depth_subs_returns_some_when_specs_depth_is_non_empty` | depth 非空 → Some |
| `build_trade_subs_returns_none_when_specs_trade_is_empty` | trade 空 → None |
| `build_trade_subs_returns_some_when_specs_trade_is_non_empty` | trade 非空 → Some |
| `build_trade_subs_returns_some_when_trade_exceeds_max_per_stream` | trade 101件（MAX超） → Some |
| `build_kline_subs_returns_none_when_specs_kline_is_empty` | kline 空 → None |
| `build_kline_subs_returns_some_when_specs_kline_is_non_empty` | kline 非空 → Some |
| `build_kline_subs_returns_some_when_kline_exceeds_max_per_stream` | kline 101件（MAX超） → Some |
| `market_subscriptions_returns_batch_for_empty_dashboard` | ストリームなしでパニックしない |
| `depth_subs_use_ticker_exchange_not_argument_exchange` | exchange 引数と ticker.exchange() の非対称性を文書化 |

---

## 実装後の構造

```rust
// モジュールレベルのプライベート関数（pub 不要）
fn build_depth_subs(_exchange: Exchange, specs: &StreamSpecs) -> Option<Subscription<exchange::Event>>
fn build_trade_subs(exchange: Exchange, specs: &StreamSpecs) -> Option<Subscription<exchange::Event>>
fn build_kline_subs(exchange: Exchange, specs: &StreamSpecs) -> Option<Subscription<exchange::Event>>

pub fn market_subscriptions(&self) -> Subscription<exchange::Event> {
    let subs = self.streams.combined_used()
        .flat_map(|(exchange, specs)| {
            [
                build_depth_subs(exchange, specs),
                build_trade_subs(exchange, specs),
                build_kline_subs(exchange, specs),
            ]
            .into_iter()
            .flatten()
        })
        .collect::<Vec<_>>();
    Subscription::batch(subs)
}
```

---

## Tips（後続作業者へ）

- `depth` は 1ティッカー = 1ストリームのため `chunks()` を使わずティッカーごとに個別 Subscription を生成する
- `trade` / `kline` は複数ティッカーを1ストリームに束ねるため `chunks(MAX_*)` が必要
- `build_depth_subs` の第1引数は `_exchange`（アンダースコアプレフィックス）。depth は `ticker.exchange()` から exchange を取得するため引数は不要だが、シグネチャ統一のために保持している
- `build_*_subs` は `pub` 不要（モジュール内プライベート。テストは `use super::*` でアクセス）
- `exchange::PriceStep` は `exchange::unit::PriceStep` と書かないとコンパイルエラー（`view.rs` の修正も実施済み）
- PowerShell では `cargo` コマンドが成功していても exit code 1 を返すことがある（stderr への書き込みが原因）。`Finished` と表示されていれば実際には成功

---

## 新たな知見

- `Subscription::batch()` は `Vec` を受け取り、空の Vec の場合には `Subscription::none()` 相当になる
- `StreamConfig::new()` の `exchange` 引数について:
  - depth は `ticker.exchange()` （TickerInfo から取得）
  - trade / kline は引数の `exchange: Exchange` を使う
  - この非対称性は既存の実装通りに踏襲する
- Rust の `Option<T>` を返す設計と `.into_iter().flatten()` の組み合わせで、空チェックのネストなしに「空スキップ」を表現できる
