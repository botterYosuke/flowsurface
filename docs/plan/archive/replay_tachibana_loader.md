---
name: Replay Tachibana ローダー修正
description: replay/loader.rs が Tachibana 銘柄で InvalidRequest エラーになるバグの修正
type: project
---

# Replay Tachibana ローダー修正プラン

**作成日**: 2026-04-13
**対象**: [src/replay/loader.rs](../../src/replay/loader.rs)
**状態**: ✅ 実装完了 (2026-04-13)
**関連ドキュメント**: [replay_redesign.md](replay_redesign.md), [replay_bar_step_loop.md](replay_bar_step_loop.md), [replay_fixture_direct_boot.md](replay_fixture_direct_boot.md)

---

## 問題

`saved-state.json` に `replay.mode = "replay"` かつ `TachibanaSpot` 銘柄を含む構成で起動すると、
以下のエラーダイアログが表示される：

```
Replay data load failed: klines fetch error: Invalid request:
立花証券の日足データは fetch_daily_history() を使用してください
```

### 根本原因

`src/replay/loader.rs` の `fetch_all_klines()` が
`exchange::adapter::fetch_klines()` を直接呼んでいる。
この関数は [exchange/src/adapter.rs:830-834](../../exchange/src/adapter.rs#L830-L834) で
`Venue::Tachibana` に対して意図的に `InvalidRequest` エラーを返す設計になっている。

```
起動
 └─ replay.mode = "replay" + pending_auto_play = true  [replay_fixture_direct_boot.md]
     └─ 全ペイン Ready → ReplayMessage::Play 自動発火
         └─ replay::loader::load_klines(TachibanaSpot D1 stream, range)
             └─ adapter::fetch_klines()  ← Tachibana ガード → InvalidRequest ❌
```

### なぜ通常チャートでは動くのか

`src/connector/fetcher.rs` の `kline_fetch_task()` は [fetcher.rs:403-408](../../src/connector/fetcher.rs#L403-L408) で
`Venue::Tachibana` を判定し、`fetch_tachibana_daily_klines()` へ正しく分岐している。
リプレイローダーはこの分岐を持っていない。

---

## 修正方針

`fetch_all_klines()` に `Venue::Tachibana` 分岐を追加し、
`crate::connector::fetcher::fetch_tachibana_daily_klines()` へ委譲する。

### 変更箇所

**`src/replay/loader.rs`** の `fetch_all_klines()` のみ。他ファイルの変更は不要。

```rust
// 修正前
async fn fetch_all_klines(
    ticker_info: TickerInfo,
    timeframe: Timeframe,
    range: Range<u64>,
) -> Result<Vec<Kline>, String> {
    use exchange::adapter;

    adapter::fetch_klines(ticker_info, timeframe, Some((range.start, range.end)))
        .await
        .map_err(|e: AdapterError| format!("klines fetch error: {e}"))
}
```

```rust
// 修正後
async fn fetch_all_klines(
    ticker_info: TickerInfo,
    timeframe: Timeframe,
    range: Range<u64>,
) -> Result<Vec<Kline>, String> {
    use exchange::adapter::{self, Venue};

    if ticker_info.ticker.exchange.venue() == Venue::Tachibana {
        let (issue_code, _) = ticker_info.ticker.to_full_symbol_and_type();
        return crate::connector::fetcher::fetch_tachibana_daily_klines(
            &issue_code,
            Some((range.start, range.end)),
        )
        .await;
    }

    adapter::fetch_klines(ticker_info, timeframe, Some((range.start, range.end)))
        .await
        .map_err(|e: AdapterError| format!("klines fetch error: {e}"))
}
```

### なぜこの方針が正しいか

- `fetch_tachibana_daily_klines()` は既に以下をすべて実装済み：
  - `#[cfg(feature = "e2e-mock")]` モック分岐（E2E テスト対応）
  - `get_session()` によるセッションチェック
  - `daily_record_to_kline()` による Kline 変換（調整値使用）
  - `range` フィルタ（post-fetch で `retain`）
- `fetcher.rs` の `kline_fetch_task` と完全に対称な実装になる
- 他モジュール（`StepClock`, `EventStore`, `Dispatcher`）への影響なし

---

## 前提条件・制約

| 条件 | 内容 |
|------|------|
| ログインセッション | 立花証券にログイン済みであること。未ログイン時は `"セッションが存在しません。再ログインしてください。"` エラーになる（これは正しい挙動） |
| 対応 timeframe | Tachibana は D1 のみ。D1 以外の timeframe でリプレイを設定した場合の挙動は現状 undefined（別 Issue として扱う） |
| `to_full_symbol_and_type()` | `ticker_info.ticker.to_full_symbol_and_type()` が issue_code を返すこと（`fetcher.rs` で実績あり） |

---

## TDD 実装ステップ

### ✅ Phase 1: Red — 失敗テスト追加

`src/replay/loader.rs` の `#[cfg(test)]` ブロックに以下テストを追加する。
`e2e-mock` feature が有効なとき、Tachibana ストリームで `load_klines` が成功することを検証する。

```rust
#[cfg(all(test, feature = "e2e-mock"))]
mod tachibana_tests {
    use super::*;
    use exchange::adapter::{Exchange, StreamKind};
    use exchange::{Ticker, TickerInfo, Timeframe};

    fn tachibana_kline_stream() -> StreamKind {
        StreamKind::Kline {
            ticker_info: TickerInfo::new(
                Ticker::new("7203|TOYOTA", Exchange::Tachibana),
                1.0,
                0.0,
                None,
            ),
            timeframe: Timeframe::D1,
        }
    }

    /// Tachibana stream に対して load_klines が InvalidRequest を返さないことを検証。
    /// e2e-mock feature 下では mock データが返る。
    #[tokio::test]
    async fn load_klines_tachibana_uses_daily_history_not_adapter_fetch() {
        use exchange::adapter::tachibana::e2e_mock;
        use exchange::{Kline, Volume};
        use exchange::unit::MinTicksize;

        // mock データを注入
        let mock_kline = Kline::new(
            1_700_000_000_000,
            3000.0, 3100.0, 2900.0, 3050.0,
            Volume::empty_total(),
            MinTicksize::from(1.0),
        );
        e2e_mock::inject_daily_klines("7203", vec![mock_kline]);

        let stream = tachibana_kline_stream();
        let range = 0u64..u64::MAX;

        let result = load_klines(stream, range).await;

        e2e_mock::clear_mock_daily_history();

        assert!(
            result.is_ok(),
            "Tachibana load_klines は InvalidRequest を返すべきでない: {:?}",
            result.err()
        );
        let loaded = result.unwrap();
        assert_eq!(loaded.klines.len(), 1);
    }
}
```

`cargo test` → `fetch_all_klines` が `adapter::fetch_klines` を呼んでいる間は FAIL することを確認。

### ✅ Phase 2: Green — `fetch_all_klines` に分岐を追加

上記「修正後」コードを実装する。

`cargo test` → PASS することを確認。

### ✅ Phase 3: Refactor

- コードの重複・冗長な `use` 宣言がないか確認
- コメントを必要最小限に整理

**結果**: リファクタ不要。追加コードは最小限かつ `cargo clippy` クリーン（既存の警告はすべて他ファイル由来）。

## 実装メモ（知見）

- `inject_daily_klines` の第1引数は `String`（`&str` でなく）→ `"7203".to_string()` で渡す
- `clear_mock_daily_history()` という名前は存在しない。正しくは `clear_daily_klines()`
- `fetch_all_klines` 内の `use exchange::adapter::{self, Venue};` は局所 use で十分（トップレベル変更不要）
- テスト: `cargo test --features e2e-mock -p flowsurface -- tachibana_tests` → 1 passed
- フルスイート: `cargo test -p flowsurface` → 139 passed, 0 failed

---

## E2E 検証（任意）

修正後、以下の fixture で起動して自動再生が開始することを確認する：

1. `e2e-mock` feature 付きでビルド
2. `saved-state.json` に現在の Tachibana Replay 構成を設置
3. 起動 → エラーダイアログが消え、チャートにバーが描画されることを確認

---

## トレードオフ

### 得るもの
- Tachibana 銘柄での Replay 起動エラーが解消
- `fetcher.rs` と `loader.rs` で Tachibana 取得経路が対称になる

### 失うもの・残留制約
- ログイン必須という制約は変わらない（未ログイン時は別エラーになるだけ）
- D1 以外の Tachibana timeframe でのリプレイは未定義のまま（別 Issue）

### 設計への影響
- なし（`StepClock`, `EventStore`, `Dispatcher`, `auto-play` ゲートはすべて無変更）
