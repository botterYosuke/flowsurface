# 📨 作業引継ぎプロンプト：market_subscriptions() リファクタリング

**作成日:** 2026-04-20  
**優先度:** 中  
**期限:** 特になし  
**責任者:** （引継ぎ先 AI）  

---

## 🎯 実装内容（概要）

flowsurface プロジェクトの `src/screen/dashboard.rs` にある `market_subscriptions()` メソッド（80行）を
責務単位に分割し、コードの重複を排除する。

**成果物:**
- `build_depth_subscriptions()` ヘルパー関数
- `build_trade_subscriptions()` ヘルパー関数
- `build_kline_subscriptions()` ヘルパー関数
- `market_subscriptions()` をリファクタリング（上記 3関数の呼び出しに変更）

**テスト:** 既存テスト全て PASS のまま（動作変更なし）

---

## 🚀 実装指示書

### 📍 対象ファイル

**メイン:** `c:\Users\sasai\Documents\flowsurface\src\screen\dashboard.rs`（行番号 1834-1914）

**テスト:** `c:\Users\sasai\Documents\flowsurface\src\screen\dashboard.rs` 内の `#[cfg(test)]` セクション

**計画書:** `c:\Users\sasai\Documents\flowsurface\docs\plan\market-subscriptions-refactor.md`

### 📌 現在の状態

```
ブランチ: sasa/develop
ステータス: 
  - 変更中: .claude/settings.json（コミット待ち）
  - 変更中: src/screen/dashboard/pane/mod.rs（未コミット）
  - 新規ファイル: src/screen/dashboard/pane/{init,update,view}.rs
  - 新規ファイル: docs/plan/pane-mod-refactor.md
最新コミット: 5774b9b docs(e2e): ポート 9876 衝突の知見を SKILL.md と CLAUDE.md に記録
```

### 🛠️ 作業の進め方（TDD アプローチ）

#### Phase 1: テスト作成（RED）

1. `cargo test` を実行して現在のテスト数を確認
   ```bash
   cargo test --lib 2>&1 | grep "test result"
   ```

2. `src/screen/dashboard.rs` の既存テストを確認
   ```bash
   grep -A 20 "mod tests" src/screen/dashboard.rs | grep -i "subscription\|market"
   ```

3. 各 `build_*` 関数のユニットテストを追加（depth/trade/kline 別）
   ```rust
   #[cfg(test)]
   mod tests {
       use super::*;
       
       #[test]
       fn build_depth_subscriptions_empty_specs() {
           let specs = StreamSpecs { depth: vec![], trade: vec![], kline: vec![] };
           let subs = Dashboard::build_depth_subscriptions(ExchangeKind::Binance, &specs);
           assert!(subs.is_empty());
       }
       
       #[test]
       fn build_depth_subscriptions_with_tickers() {
           // テストをここに追加
       }
   }
   ```

4. テストが FAIL することを確認
   ```bash
   cargo test build_depth_subscriptions 2>&1 | grep "error\|FAILED"
   ```

#### Phase 2: 実装（GREEN）

1. `build_depth_subscriptions()` を実装
   ```rust
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
           .collect()
   }
   ```

2. `build_trade_subscriptions()` を実装（MAX_TRADE_TICKERS_PER_STREAM で chunking）

3. `build_kline_subscriptions()` を実装（MAX_KLINE_STREAMS_PER_STREAM で chunking）

4. `market_subscriptions()` をリファクタリング
   ```rust
   pub fn market_subscriptions(&self) -> Subscription<exchange::Event> {
       let subs = self.streams
           .combined_used()
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
   ```

5. テストが PASS することを確認
   ```bash
   cargo test build_depth_subscriptions build_trade_subscriptions build_kline_subscriptions
   ```

#### Phase 3: 品質確認（REFACTOR）

1. コード整形
   ```bash
   cargo fmt
   ```

2. Lint チェック
   ```bash
   cargo clippy -- -D warnings
   ```

3. 全テスト実行
   ```bash
   cargo test
   ```

4. ビルド確認
   ```bash
   cargo check
   ```

5. 差分確認
   ```bash
   git diff src/screen/dashboard.rs
   ```

### 📋 計画書の更新（必須）

作業開始時・各フェーズ完了後に `docs/plan/market-subscriptions-refactor.md` を更新してください。

**テンプレート:**
```markdown
## 実装進捗

- [x] Step 0: cargo test 全通過を確認 → 362 PASS
- [x] Step 1: 既存テストの確認・理解 → test_market_subscriptions なし、test_* で確認
- [ ] Step 2: build_depth_subscriptions テスト作成（RED）
- [ ] Step 3: build_depth_subscriptions 実装（GREEN）
...

## 新たな知見・設計思想

**洞察1: depth stream は個別ハンドリング**
- depth は ticksize・push frequency が ティッカーごとに異なるため、1つずつ Subscription に
- trade/kline は複数まとめて 1 Subscription に可能（uniform な設定）

**洞察2: StreamConfig の用途別設定**
- depth: Ticker, exchange, tick_mltp, push_freq を個別指定
- trade: Vec<Ticker>, exchange, None, ServerDefault
- kline: Vec<KlineStream>, exchange, None, ServerDefault

**決定事項:**
- ヘルパー関数は `fn` (プライベート) として `impl Dashboard` ブロック内に定義
- 各関数は `Vec<Subscription<exchange::Event>>` を戻す（メイン関数で batch へ統一）
```

---

## 🔧 セットアップ（初回のみ）

プロジェクトの前提条件を確認してください：

```bash
# プロジェクトディレクトリへ移動
cd c:\Users\sasai\Documents\flowsurface

# git ブランチ確認（sasa/develop に在ること）
git branch

# 最新コミットを確認
git log -1 --oneline

# 既存テストが全て PASS することを確認
cargo test 2>&1 | tail -20
```

---

## 📚 プロジェクト背景

### flowsurface について

| 項目 | 内容 |
|------|------|
| 言語 | Rust（100% — TypeScript/JavaScript なし） |
| GUI | iced 0.14.x フレームワーク |
| 用途 | 暗号資産マーケットのチャートプラットフォーム |
| ビルド | cargo（Rust 標準） |
| テスト | #[test], #[tokio::test], rstest, mockall |
| 形式 | Workspace（複数クレート） |

### 現在のリファクタリング進捗

最近のリファクタリング例（参考）：

| コミット | 内容 |
|---------|------|
| 49164b5 | pane.rs（2804行）を 4ファイルに分割 |
| d963574 | update()（494行）を責務別に分割 |
| 8da00b1 | main.rs（3453行）を src/app/ へ分割 |

**パターン:** 大きなファイル → 責務別に分割 → 各ファイル / 関数で 200-300行 以下を目指す

### src/screen/dashboard.rs の役割

Dashboard パネルのメイン状態管理。

**責務:**
- 複数ペイン（チャート、オーダーリスト等）の状態管理
- WebSocket ストリーム（depth, trade, kline）のサブスクリプション組み立て
- リプレイモード・リアルタイムモードの切り替え
- ペイン間のインタラクション処理

**主要構造体:**
- `Dashboard`: メイン状態（ペイン、ストリーム、リアルタイム/リプレイモード）
- `StreamSpecs`: depth/trade/kline の設定リスト

---

## 🔍 理解すべき概念

### StreamSpecs 構造体

```rust
pub struct StreamSpecs {
    pub depth: Vec<(Ticker, StreamTicksize, PushFrequency)>,
    pub trade: Vec<Ticker>,
    pub kline: Vec<KlineStream>,
}
```

**depth:**
- `Ticker`: 通貨ペア（BTCUSDT など）
- `StreamTicksize`: クライアント側 vs サーバー側 aggregation
- `PushFrequency`: push 頻度（高頻度 / 低頻度）

**trade:**
- `Vec<Ticker>`: トレード対象のティッカーリスト
- 複数 ticker をまとめて 1 stream へ

**kline:**
- `Vec<KlineStream>`: K線ストリーム（Ticker + Timeframe）
- 複数を chunks でグループ化

### Subscription/StreamConfig

```rust
// Subscription 作成の流れ
let config = StreamConfig::new(ticker, exchange, tick_mltp, push_freq);
let subscription = Subscription::run_with(config, exchange::connect::depth_stream);

// 複数 subscription をまとめる
Subscription::batch(vec![sub1, sub2, sub3])
```

### 定数（ファイル: src/connector/connect.rs）

```rust
pub const MAX_TRADE_TICKERS_PER_STREAM: usize = 100;
pub const MAX_KLINE_STREAMS_PER_STREAM: usize = 50;
```

- `trade`: 100 個ずつ chunking して 1 stream へ
- `kline`: 50 個ずつ chunking して 1 stream へ

---

## ✅ チェックリスト

### 開始前

- [ ] プロジェクトをクローンした / 既存リポジトリで開く
- [ ] `cd flowsurface && cargo test` で全テスト PASS を確認
- [ ] git ブランチが `sasa/develop` であることを確認
- [ ] CLAUDE.md / MEMORY.md を読んだ
- [ ] `docs/plan/market-subscriptions-refactor.md` を読んだ

### 実装中

- [ ] RED: テスト作成で FAIL を確認
- [ ] GREEN: テスト PASS を確認
- [ ] REFACTOR: clippy / fmt / check が全て通ることを確認
- [ ] 計画書に進捗を記録（Step ごと）

### 完了前

- [ ] `cargo test` で全テスト PASS（目標: 360+）
- [ ] `cargo clippy -- -D warnings` で警告なし
- [ ] `cargo fmt` を実行済み
- [ ] `cargo check` で compile error なし
- [ ] コミット前に `git diff` で意図しない変更がないか確認

### 完了後

- [ ] `docs/plan/market-subscriptions-refactor.md` の完了条件が全て ✅
- [ ] `git log` で新規コミットが作成されたことを確認
- [ ] コミットメッセージが明確（例: `refactor(dashboard): market_subscriptions() を 3つのヘルパーに分割`）

---

## 🎓 学習リソース（プロジェクト内）

### コーディングルール

必読（実装前に確認）:

- `c:\Users\sasai\Documents\flowsurface\.claude\rules\rust\coding-style.md` — フォーマット・命名・エラーハンドリング
- `c:\Users\sasai\Documents\flowsurface\.claude\rules\rust\testing.md` — テスト構成・カバレッジ 80%+

### 参考コミット

実装パターンの参考：

```bash
# 同様の分割リファクタリング
git show 49164b5  # pane.rs 分割
git show d963574  # update() 分割
git show 8da00b1  # main.rs 分割
```

### スキルの使用

実装中に質問がある場合：

- `/coding-standards` — Rust コーディング規約確認
- `/rust-testing` — テスト書き方のヒント
- `/simplify` — コード品質レビュー

---

## 🛠️ よく使うコマンド

```bash
# ビルド確認（最速）
cargo check

# コンパイル
cargo build --release

# テスト実行（全テスト）
cargo test

# テスト + 出力表示（デバッグに便利）
cargo test -- --nocapture

# 特定テストのみ
cargo test build_depth

# ユニットテストのみ
cargo test --lib

# Lint（警告 = エラー）
cargo clippy -- -D warnings

# 自動フォーマット
cargo fmt

# フォーマット確認（修正なし）
cargo fmt --check

# git 差分確認
git diff src/screen/dashboard.rs
```

---

## 📞 トラブルシューティング

### コンパイルエラー: "cannot borrow `self` as mutable"

**原因:** `&mut self` が複数回必要

**対策:** メソッドを分割するか、スコープを調整

### テスト FAIL: "assertion failed: subs.is_empty()"

**原因:** テスト期待値が実装と異なる

**対策:** テストの期待値を実装に合わせるか、実装ロジックを確認

### cargo clippy 警告: "unnecessary closure"

**原因:** 不要な closure が使われている

**対策:** 警告を確認し、シンプルな式に変更（警告を詳しく読むこと）

### git conflict

**回避方法:** `sasa/develop` ブランチで作業（main とは conflict なし）

---

## 🎬 実行フロー（概要）

```
1. プロジェクト setup → cargo test 確認
2. 既存テスト理解 → market_subscriptions() 関連テストを確認
3. Phase 1: RED → depth/trade/kline テスト作成 → FAIL 確認
4. Phase 2: GREEN → 3つのヘルパー実装 → PASS 確認
5. Phase 3: REFACTOR → fmt/clippy/check → All PASS
6. 計画書更新 → Step 完了マーク（✅）
7. コミット作成 → "refactor(dashboard): market_subscriptions() を 3つのヘルパーに分割"
8. git log 確認 → 新規コミット表示
```

---

## 📬 完了報告

実装完了時、以下を報告してください：

1. **計画書の最終状態** (`docs/plan/market-subscriptions-refactor.md`)
   - 全 Step に ✅ が付いているか
   - 新たな知見セクションが充実しているか

2. **テスト結果**
   ```bash
   cargo test 2>&1 | grep "test result"
   ```

3. **コンパイル・Lint 状態**
   ```bash
   cargo check && cargo clippy -- -D warnings && cargo fmt --check && echo "✅ All PASS"
   ```

4. **最新コミット**
   ```bash
   git log -1 --oneline
   ```

5. **変更ファイル**
   ```bash
   git diff --name-only HEAD~1
   ```

---

## 🤝 コミュニケーション

実装中の質問・課題：

1. **停止状態** → 計画書の "新たな知見" セクションに記録し、別の AI に続きを依頼
2. **テスト FAIL** → テスト条件を計画書に記録し、原因調査
3. **設計判断** → CLAUDE.md の patterns.md / coding-style.md を参照し、記録

---

**このドキュメントが明確で実行可能な指示書として機能しますように。🚀**

