# order_entry.rs view メソッド分割リファクタリング

## 目的

`src/screen/dashboard/panel/order_entry.rs` の `view` メソッド（228行）を可読性・保守性向上のため private メソッドに分割する。

## 分割方針

| メソッド | 責務 | 引数 |
|---|---|---|
| `view_side_tabs` | 買い/売りボタン | `&self` |
| `view_account_row` | 口座区分・現物/信用区分ピッカー（2行を column で束ねる） | `&self` |
| `view_qty_row` | 数量入力（売り時: 全数量ボタン・保有株数表示） | `&self` |
| `view_price_row` | 価格種別ピッカー＋指値入力（指値時のみ） | `&self` |
| `view_expire_row` | 期日ピッカー | `&self` |
| `view_result_row` | 注文結果表示（受付番号 or エラー） | `&self` |
| `view_action_area` | 確認モーダル or 確認ボタン | `&self, theme: &Theme, virtual_mode: bool` |
| `view` | 組み合わせ＋パスワード行＋仮想バナー | `&self, theme: &Theme, is_replay: bool` |

**Note**: `view_action_area` は `side_color(theme)` を使うため `theme` 引数が必要。

## TDD アプローチ

### RED（テスト先行）

新メソッドを呼び出すスモークテストを先に追加 → コンパイルエラーで RED を確認。

```rust
// view_side_tabs_returns_element / view_account_row_returns_element / ...
```

### GREEN（実装）

`view` 内のロジックを対応するメソッドに切り出す。`view` 本体はこれらを組み合わせるだけにする。

### REFACTOR

既存テスト全通過を確認。`cargo clippy -- -D warnings` を実行。

## 作業チェックリスト

- [x] 計画書作成（本ファイル）
- [x] RED: スモークテスト追加（コンパイルエラー確認）
- [x] GREEN: view メソッド分割実装
- [x] 既存テスト全 PASS 確認（`cargo test`）
- [x] `cargo clippy -- -D warnings` 通過
- [x] `cargo fmt` 実行

## 設計メモ

- `view_account_row` は account と cash_margin 両ピッカーを `column![]` で束ねて 1 つの Element を返す
- `Side::Buy` / `Side::Sell` の重複条件分岐は今回は触らない（別リファクタリング対象）
- `virtual_mode` 判定（`self.is_virtual || is_replay`）は `view` 本体に残す
- ロジック変更ゼロ。純粋なメソッド抽出リファクタリング

## 進捗・知見ログ

- [x] REDテスト追加
- [x] 機能実装完了
- [x] テスト (380 tests passed) および Linter フォーマット実行完了。
- 計画通りに `view` メソッド内のレイアウト構築をプライベートメソッドに分割し、責務とコードの可読性を改善しました。
