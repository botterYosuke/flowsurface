# market_subscriptions() リファクタリング 作業依頼

**作成日:** 2026-04-20  
**優先度:** 中  
**推定工数:** 2-3時間  
**関連ファイル:** [src/screen/dashboard.rs:1834-1914](src/screen/dashboard.rs#L1834-L1914)

---

## 🎯 ゴール

[src/screen/dashboard.rs:1834](src/screen/dashboard.rs#L1834) の `pub fn market_subscriptions()` 関数（80行）を責務単位に分割し、
コードの再利用性と可読性を向上させる。動作・外部 API・テストは変更しない。

### 現状の問題

```rust
pub fn market_subscriptions(&self) -> Subscription<exchange::Event> {
    let unique_streams = self
        .streams
        .combined_used()
        .flat_map(|(exchange, specs)| {
            let mut subs = vec![];

            if !specs.depth.is_empty() {
                let depth_subs = specs
                    .depth
                    .iter()
                    .map(|(ticker, aggr, push_freq)| { /* ... */ })
                    .collect::<Vec<_>>();
                if !depth_subs.is_empty() {
                    subs.push(Subscription::batch(depth_subs));
                }
            }

            // ← ここから trade ブロック（同じパターン）
            if !specs.trade.is_empty() { /* ... */ }

            // ← ここから kline ブロック（同じパターン）
            if !specs.kline.is_empty() { /* ... */ }

            subs
        })
        .collect::<Vec<Subscription<exchange::Event>>>();

    Subscription::batch(unique_streams)
}
```

**重複パターン:**
- depth/trade/kline の 3ブロックで、`if !specs.XXX.is_empty() { ... subs.push(...) }` が同じ構造
- 各ブロック内で `Subscription::batch()` で複数サブスクリプションを束ねる処理が同一
- 各ブロックの `.chunks()` / `.map()` パターンが類似（trade は `MAX_TRADE_TICKERS_PER_STREAM`、kline は `MAX_KLINE_STREAMS_PER_STREAM`）

---

## 📋 分割方針

### 新しい関数設計

```rust
// dashboard.rs のメイン関数（変わらない）
pub fn market_subscriptions(&self) -> Subscription<exchange::Event> {
    let subs = self.streams.combined_used()
        .flat_map(|(exchange, specs)| {
            let mut subs = vec![];
            subs.extend(Self::build_depth_subscriptions(exchange, &specs));
            subs.extend(Self::build_trade_subscriptions(exchange, &specs));
            subs.extend(Self::build_kline_subscriptions(exchange, &specs));
            subs
        })
        .collect::<Vec<_>>();
    Subscription::batch(subs)
}

// プライベートヘルパー関数群
fn build_depth_subscriptions(
    exchange: ExchangeKind,
    specs: &StreamSpecs,
) -> Vec<Subscription<exchange::Event>> {
    if specs.depth.is_empty() {
        return vec![];
    }
    specs
        .depth
        .iter()
        .map(|(ticker, aggr, push_freq)| {
            let tick_mltp = match aggr {
                StreamTicksize::Client => None,
                StreamTicksize::ServerSide(tick_mltp) => Some(*tick_mltp),
            };
            let config = StreamConfig::new(
                *ticker,
                ticker.exchange(),
                tick_mltp,
                *push_freq,
            );
            Subscription::run_with(config, exchange::connect::depth_stream)
        })
        .collect::<Vec<_>>()
        .into_iter()
        .collect::<Vec<_>>()
        .chunks(...)  // ← depth は chunking しない（1つずつ）
        .map(|chunk| Subscription::batch(chunk.to_vec()))
        .collect()
}

fn build_trade_subscriptions(
    exchange: ExchangeKind,
    specs: &StreamSpecs,
) -> Vec<Subscription<exchange::Event>> {
    if specs.trade.is_empty() {
        return vec![];
    }
    specs
        .trade
        .chunks(MAX_TRADE_TICKERS_PER_STREAM)  // ← trade は chunking あり
        .map(|tickers| {
            let config = StreamConfig::new(
                tickers.to_vec(),
                exchange,
                None,
                PushFrequency::ServerDefault,
            );
            Subscription::run_with(config, exchange::connect::trade_stream)
        })
        .collect::<Vec<_>>()
        .into_iter()
        .collect::<Vec<_>>()
        .chunks(...)  // ← さらに batch?
        .map(|chunk| Subscription::batch(chunk.to_vec()))
        .collect()
}

fn build_kline_subscriptions(
    exchange: ExchangeKind,
    specs: &StreamSpecs,
) -> Vec<Subscription<exchange::Event>> {
    // kline も同様（MAX_KLINE_STREAMS_PER_STREAM で chunking）
}
```

### 実装上の留意点

1. **関数は `fn` にしない（`Self::` で呼び出すため静的メソッド or プライベート関数）**
   - `impl Dashboard` ブロック内で定義
   - または `pub(crate)` フリー関数として定義
   
2. **戻り値型は統一すること**
   - `Vec<Subscription<exchange::Event>>` で返して、メイン関数で `Subscription::batch()` に統一

3. **各関数の職責**
   - `build_depth_subscriptions`: depth stream の Subscription ベクターを組み立て
   - `build_trade_subscriptions`: trade stream の Subscription ベクターを組み立て（chunking あり）
   - `build_kline_subscriptions`: kline stream の Subscription ベクターを組み立て（chunking あり）

4. **既存テストの確認**
   - [src/screen/dashboard.rs:1946](src/screen/dashboard.rs#L1946) 付近の `#[cfg(test)]` mod で `market_subscriptions()` をテストしているはず
   - リファクタリング後も全テストが PASS すること

---

## 🧪 TDD アプローチ（必須）

`.claude/skills/tdd-workflow/SKILL.md` に従って実装してください。

### Red → Green → Refactor サイクル

1. **Red Phase（テスト作成）**
   - 現在の `market_subscriptions()` が返す Subscription の構造を保証するテストを最初に確認
   - 必要に応じて新規テストを追加（各 `build_*` ヘルパーのユニットテスト）
   - テストが FAIL することを確認

2. **Green Phase（実装）**
   - `build_depth_subscriptions()`, `build_trade_subscriptions()`, `build_kline_subscriptions()` を実装
   - `market_subscriptions()` をリファクタリング
   - テストが PASS することを確認

3. **Refactor Phase**
   - コードの重複をさらに削減できないか検討
   - 関数名・変数名が意図を明確に表しているか確認
   - `cargo clippy`, `cargo fmt` を実行

### テスト実行コマンド

```bash
# テスト実行
cargo test

# テスト + 出力表示（デバッグに便利）
cargo test -- --nocapture

# 特定のテストのみ
cargo test market_subscriptions
```

### カバレッジ目標

- 新規ヘルパー関数それぞれで 80% 以上のカバレッジ
- 既存の `market_subscriptions()` テストは PASS したまま

---

## 📝 計画書の管理（必須）

### 作業開始時

`docs/plan/market-subscriptions-refactor.md` の下記セクションを更新してください：

```markdown
## 実装進捗

- [ ] Step 0: cargo test 全通過を確認（目標: 通過）
- [ ] Step 1: 既存テストの確認・理解（test_market_subscriptions など）
- [ ] Step 2: build_depth_subscriptions テスト作成（RED）
- [ ] Step 3: build_depth_subscriptions 実装（GREEN）
- [ ] Step 4: build_trade_subscriptions テスト作成（RED）
- [ ] Step 5: build_trade_subscriptions 実装（GREEN）
- [ ] Step 6: build_kline_subscriptions テスト作成（RED）
- [ ] Step 7: build_kline_subscriptions 実装（GREEN）
- [ ] Step 8: market_subscriptions() をリファクタリング（委譲に変更）
- [ ] Step 9: cargo check / cargo test / cargo clippy / cargo fmt 全通過
- [ ] Step 10: コードレビュー（可読性・重複排除）

## 新たな知見・設計思想

ここに実装中に気付いたことを記録してください。例：
- depth stream は 1 つずつ Subscription にする必要がある（なぜか）
- trade/kline は複数をまとめて Subscription にできる（なぜか）
- StreamConfig::new() の第3引数は tick_mltp だが、trade/kline では常に None な理由
```

### 完了時

完了した Step に ✅ を付けてください。

---

## 📚 コンテキスト・背景情報

### flowsurface について

- **言語**: Rust（TypeScript/JavaScript は使わない）
- **GUI フレームワーク**: iced 0.14.x
- **用途**: 暗号資産マーケットのチャートプラットフォーム
- **ビルド**: `cargo build`, `cargo test`

### src/screen/dashboard.rs の役割

Dashboard パネルのメイン状態管理。複数のペイン（チャート・オーダーリスト等）を管理し、
WebSocket ストリーム（depth, trade, kline）のサブスクリプション設定を行う。

### StreamSpecs 構造体

```rust
pub struct StreamSpecs {
    pub depth: Vec<(Ticker, StreamTicksize, PushFrequency)>,
    pub trade: Vec<Ticker>,
    pub kline: Vec<KlineStream>,
}
```

- `depth`: ティッカーごとの depth stream 設定（ticksize, push frequency を個別指定可）
- `trade`: トレードティッカーのリスト（複数まとめて 1 Subscription に可）
- `kline`: K線ストリーム（timeframe + ticker）のリスト

### 定数（src/connector/connect.rs から import）

```rust
pub const MAX_TRADE_TICKERS_PER_STREAM: usize = 100;
pub const MAX_KLINE_STREAMS_PER_STREAM: usize = 50;
```

- trade ティッカーは 100 個ずつまとめて 1 ストリームへ
- kline ストリームは 50 個ずつまとめて 1 ストリームへ

---

## 🔍 確認事項

実装を始める前に下記を確認してください：

- [ ] 現在のテストで `market_subscriptions()` がテストされているか
  ```bash
  grep -n "market_subscriptions" src/screen/dashboard.rs
  ```
- [ ] StreamSpecs, StreamConfig, PushFrequency などの型定義を理解した
- [ ] depth/trade/kline の違いを理解した（chunking 有無など）
- [ ] `Subscription::batch()`, `Subscription::run_with()` の役割を理解した

---

## ✅ 完了条件

- [x] `docs/plan/market-subscriptions-refactor.md` が実装進捗で更新されている
- [ ] `cargo build` が通る
- [ ] `cargo test` が全テスト PASS（目標: 360+ テスト）
- [ ] `cargo clippy -- -D warnings` が通る
- [ ] `cargo fmt` 適用済み
- [ ] `market_subscriptions()` の責務が 3つに分割されている
- [ ] 各 `build_*` ヘルパー関数が 50 行程度の適切なサイズ
- [ ] リファクタリング前後でテスト結果が同じ（動作変更なし）

---

## 🛠️ 開発環境コマンド

```bash
# ビルド確認
cargo check

# コンパイル
cargo build

# テスト実行
cargo test

# ユニットテストのみ
cargo test --lib

# 特定モジュールのテストのみ
cargo test screen::dashboard

# Lint（警告をエラーとして扱う）
cargo clippy -- -D warnings

# フォーマット（実行必須 — CI がチェック）
cargo fmt

# フォーマット確認（修正なし）
cargo fmt --check
```

---

## 📖 参考資料・ルール

### コーディングルール

実装は下記ルールに従ってください：

- **[.claude/rules/rust/coding-style.md](.claude/rules/rust/coding-style.md)**: フォーマット・命名・エラーハンドリング
- **[.claude/rules/rust/patterns.md](.claude/rules/rust/patterns.md)**: デザインパターン（Repository, Newtype, Builder など）
- **[.claude/rules/rust/testing.md](.claude/rules/rust/testing.md)**: テスト構成・カバレッジ 80%+

### 関連コミット（参考）

最近の同様のリファクタリングを参考にしてください：

```
49164b5 refactor(pane): pane.rs（2804行）を4ファイルに分割し品質改善
d963574 refactor(dashboard): update() 494行を責務単位の小関数群に分割
8da00b1 refactor(app): main.rs（3453行）を src/app/ 以下12ファイルに分割
```

---

## 💡 ヒント

1. **test-first アプローチ**: まず既存の `market_subscriptions()` テストを理解し、新規ヘルパーのテストを書く
2. **extract 関数**: IDE の "Extract Function" を使ってリファクタリング効率を上げる
3. **diff を小さく**: 1 関数 = 1 commit のイメージで進める（レビュー・デバッグが容易）
4. **cargo test の頻度**: 実装の合間に何度も実行（リグレッション早期発見）

---

## 🎬 次のステップ

1. このドキュメントを読んで確認事項をチェック
2. 実装フェーズを開始（TDD: Red → Green → Refactor）
3. 進捗を **Step ごとに** `docs/plan/market-subscriptions-refactor.md` に記録
4. 完了後、PR を作成（コミットメッセージは `refactor(dashboard): market_subscriptions() を 3つのヘルパーに分割`）

