# 仮想取引フルサイクル実装計画

作成日: 2026-04-17  
対象ブランチ: sasa/develop

---

## 目標

「注文 → 約定 → ポジション建て → キャッシュ変動 → クローズ → PnL 確定」  
フルサイクルを Rust エンジン + E2E テストで検証できる状態にする。

---

## 現状分析

### on_tick 配線は完了している

`main.rs:416` で `engine.on_tick(&ticker, &trades, clock_ms)` が呼ばれており、  
`dispatcher.rs` は EventStore から `trades_in()` を引いて `trade_events` に積んでいる。  
**Trades EventStore の統合は済んでいる。**

### 欠落している 3 つのロジック

| # | 問題 | 場所 | 影響 |
|---|------|------|------|
| 1 | **買い約定時に cash を減算しない** | `portfolio.rs: record_open()` | 約定しても cash = 1,000,000 のまま |
| 2 | **クローズ時に売却代金を cash に返還しない** | `portfolio.rs: record_close()` | PnL のみ加算 → A-1 と不整合 |
| 3 | **売り注文が既存 Long をクローズしない** | `order_book.rs: on_tick()` | 売り約定が Short として `record_open()` され `record_close()` が呼ばれない |

**cash の正しい流れ（A-0〜A-1 後）：**

| 操作 | 変動 | 残高例（初期 1,000,000 / qty 1.0） |
|------|------|--------------------------------------|
| open @90,000 | `cash -= 90,000` | 910,000 |
| close @92,000 | `cash += 92,000` | 1,002,000（PnL +2,000 込み） |

### 既存ユニットテストのギャップ

`order_book.rs` の `portfolio_pnl_after_round_trip` テストは  
`record_close()` を直接呼んで確認しているが、  
**A-0 の変更後は `record_close()` 内の cash 計算も変わるため、このテストも修正が必要。**

---

## 実装範囲

### Phase A: Rust エンジン修正（TDD）

#### A-0: `record_close()` の cash 返還ロジックを変更

**ファイル**: `src/replay/virtual_exchange/portfolio.rs`

現在は PnL のみ加算しているが、A-1 で `record_open()` が全コストを deduct するため、  
`record_close()` は **売却代金（exit_price × qty）を返還** する方式に変更する。

```rust
// Before（PnL のみ加算）
self.cash += pnl;

// After（売却代金を返還 — Long の場合）
match pos.side {
    PositionSide::Long  => self.cash += exit_price * pos.qty,   // 売却代金を返還
    PositionSide::Short => self.cash -= exit_price * pos.qty,   // 買い戻しコストを差し引く
}
```

**既存テストの修正:**  
`portfolio.rs: realized_pnl_closes_position` のアサーション  
`cash = 102,000` は A-0+A-1 後も正しく成立することを確認する。  
（open @90,000 → cash=910,000、close @92,000 → cash=1,002,000 ✅）

**追加ユニットテスト:**
- `close_short_deducts_buyback_cost` — Short open @90,000 → close @88,000 → cash が正しく変動する

#### A-1: `record_open()` で cash を操作する

**ファイル**: `src/replay/virtual_exchange/portfolio.rs`

```rust
// 現在: cash を触らない
pub fn record_open(&mut self, pos: Position) {
    self.positions.push(pos);
}

// 修正後
pub fn record_open(&mut self, pos: Position) {
    match pos.side {
        PositionSide::Long  => self.cash -= pos.entry_price * pos.qty,  // 購入コストを差し引く
        PositionSide::Short => self.cash += pos.entry_price * pos.qty,  // 売り代金を受け取る（裸ショート）
    }
    self.positions.push(pos);
}
```

**追加ユニットテスト:**
- `buy_fill_deducts_cash` — Long open 後 cash = 1,000,000 - fill_price * qty
- `short_open_credits_cash` — Short open 後 cash = 1,000,000 + fill_price * qty（裸ショート）

#### A-1.5: `VirtualPortfolio` に Long / Short 検索 API を追加

**ファイル**: `src/replay/virtual_exchange/portfolio.rs`

`portfolio.positions` は private フィールドのため、`order_book.rs` から直接検索できない。  
`on_tick()` のクローズ判定（A-2 の前提）のために Long / Short 両方の検索メソッドを追加する。

```rust
/// 指定 ticker の open Long ポジションの order_id を返す（最古優先 = FIFO）
pub fn oldest_open_long_order_id(&self, ticker: &str) -> Option<&str> {
    self.positions
        .iter()
        .filter(|p| {
            p.ticker == ticker
                && p.side == PositionSide::Long
                && p.exit_price.is_none()
        })
        .min_by_key(|p| p.entry_time_ms)
        .map(|p| p.order_id.as_str())
}

/// 指定 ticker の open Short ポジションの order_id を返す（最古優先 = FIFO）
pub fn oldest_open_short_order_id(&self, ticker: &str) -> Option<&str> {
    self.positions
        .iter()
        .filter(|p| {
            p.ticker == ticker
                && p.side == PositionSide::Short
                && p.exit_price.is_none()
        })
        .min_by_key(|p| p.entry_time_ms)
        .map(|p| p.order_id.as_str())
}
```

**追加ユニットテスト（Long）:**
- `oldest_open_long_returns_none_when_empty` — ポジションなし → None
- `oldest_open_long_returns_oldest` — 複数 Long → entry_time_ms が最小のものを返す
- `oldest_open_long_ignores_closed` — close 済みは返さない
- `oldest_open_long_ignores_short` — Short ポジションは返さない

**追加ユニットテスト（Short）:**
- `oldest_open_short_returns_none_when_empty` — ポジションなし → None
- `oldest_open_short_returns_oldest` — 複数 Short → entry_time_ms が最小のものを返す
- `oldest_open_short_ignores_closed` — close 済みは返さない
- `oldest_open_short_ignores_long` — Long ポジションは返さない

#### A-2: `on_tick()` に対称クローズロジックを追加

**ファイル**: `src/replay/virtual_exchange/order_book.rs`

売り（Short）注文は既存 Long をクローズし、買い（Long）注文は既存 Short をクローズする。  
対応するポジションがなければ新規で開く（裸ショート / 裸ロング）。

```
fill_price 確定
  ↓
order.side == Short?
  → Long が存在する: record_close(oldest_long_id, fill_price, now_ms)
  → Long がない:     record_open(Position { side: Short, ... })   ← 裸ショート

order.side == Long?
  → Short が存在する: record_close(oldest_short_id, fill_price, now_ms)
  → Short がない:     record_open(Position { side: Long, ... })   ← 通常ロング
```

**追加ユニットテスト（Long → Short クローズ）:**
- `sell_fill_closes_existing_long` — 買い約定後に売り約定 → open_positions=0, closed_positions=1
- `sell_fill_without_long_opens_short` — Long なしで売り注文 → Short ポジション open
- `round_trip_realized_pnl_positive` — buy @90,000 → sell @92,000 → realized_pnl = +2,000
- `round_trip_realized_pnl_negative` — buy @90,000 → sell @88,000 → realized_pnl = -2,000
- `cash_round_trip_profit` — buy @90,000 qty 1.0 → sell @92,000 → cash = 1,002,000
- `cash_round_trip_loss` — buy @90,000 qty 1.0 → sell @88,000 → cash = 998,000

**追加ユニットテスト（Short → Long クローズ）:**
- `buy_fill_closes_existing_short` — 売り約定後に買い約定 → open_positions=0, closed_positions=1
- `buy_fill_without_short_opens_long` — Short なしで買い注文 → Long ポジション open
- `short_round_trip_profit` — sell @92,000 → buy @90,000 → realized_pnl = +2,000
- `short_round_trip_loss` — sell @90,000 → buy @92,000 → realized_pnl = -2,000
- `cash_short_round_trip_profit` — short @92,000 qty 1.0 → buy @90,000 → cash = 1,002,000
- `cash_short_round_trip_loss` — short @90,000 qty 1.0 → buy @92,000 → cash = 998,000

**追加ユニットテスト（指値注文の cash フロー）:**
- `limit_buy_fills_and_deducts_cash` — 指値買い @91,000 が約定 → cash = initial - 91,000 × qty
- `limit_sell_closes_existing_long` — 指値売りが約定 → Long をクローズ、PnL 確定

#### A-3: `#[allow(dead_code)]` の除去

`portfolio.rs: record_close()` の `#[allow(dead_code)]` を削除する。  
（A-2 で `on_tick()` から呼ばれるようになるため不要になる）

---

### Phase B: E2E テスト S40

**ファイル**: `tests/e2e_scripts/s40_virtual_order_fill_cycle.sh`

**シナリオ**: Playing → Pause → 成行買い → step-forward で約定 → portfolio 確認 →  
成行売り → step-forward で約定 → PnL 確認

> **重要**: `step-forward` は **Paused 状態でのみ 1 bar 前進** する。  
> Playing 中に呼ぶと range 末尾まで一気にシークして停止してしまう（`controller.rs:415-422`）。  
> そのため Playing 到達後すぐに Pause し、以降は step-forward でティックを手動制御する。

| TC | 操作 | 期待値 | 検証内容 |
|----|------|--------|---------|
| A | REPLAY Playing 到達 | status=Playing | 前提確認 |
| B | `POST /api/replay/pause` | status=Paused | step-forward を有効にする |
| C | 成行買い 1.0 BTC @ market | HTTP 200, order_id 返却 | 注文受付確認 |
| D | step-forward ループ（最大 10 回、trades が来るまで） | open_positions.length >= 1 | A-2: on_tick 経由で約定・ポジション建て |
| D-check | ループ後チェック | open_positions >= 1 でなければ FAIL | 5 回リトライ失敗を検出 |
| E | `portfolio.cash < 1000000` | cash = 1,000,000 - fill_price × 1.0 | A-1: record_open() の cash 減算 |
| F | 成行売り 1.0 BTC @ market | HTTP 200, order_id 返却 | 注文受付確認 |
| G | step-forward ループ（最大 10 回、trades が来るまで） | open_positions.length == 0 | A-2: 売り約定で Long をクローズ |
| G-check | ループ後チェック | open_positions == 0 でなければ FAIL | クローズ失敗を検出 |
| H | `closed_positions.length == 1` | closed_positions に 1 件移動 | A-2: record_close() 呼び出し確認 |
| I | `realized_pnl != 0` | PnL が確定している | A-0: realized_pnl の計算確認 |
| J | `abs(cash - 1000000) ≈ abs(realized_pnl)` | `cash = initial_cash + realized_pnl` | A-0: 売却代金返還の正確な検証 |
| K | `total_equity = cash + unrealized_pnl` | スキーマ整合性 | PortfolioSnapshot の計算整合 |

**フィクスチャ:** BinanceLinear:BTCUSDT M1, replay auto-play (UTC[-3h, -1h])

> **注意**: `step-forward` が trades を含むかはリプレイデータに依存する。  
> trades が空の step が続く可能性があるため、TC-D / TC-G は最大 10 回リトライする。  
> 10 回試みて条件を満たさない場合はテスト FAIL とする。

```bash
# TC-D: step-forward を最大 10 回試みて open_positions が 1 になるまで待つ
for i in $(seq 1 10); do
  api_post /api/replay/step-forward > /dev/null
  sleep 0.3
  PORTFOLIO=$(curl -s "$API_BASE/api/replay/portfolio")
  OPEN=$(echo "$PORTFOLIO" | jq '.open_positions | length')
  [ "$OPEN" -ge 1 ] && break
done
# D-check: ループ後にも条件を満たしていなければ FAIL
[ "$OPEN" -ge 1 ] \
  && pass "TC-D: step-forward で約定 → open_positions=$OPEN" \
  || fail "TC-D" "10 回 step-forward しても open_positions が増えない (=$OPEN)"

# TC-J: cash = initial_cash + realized_pnl の検証（A-0: 売却代金返還）
CASH=$(echo "$PORTFOLIO_AFTER_SELL" | jq '.cash')
REALIZED=$(echo "$PORTFOLIO_AFTER_SELL" | jq '.realized_pnl')
# |cash - 1000000 - realized_pnl| < 1.0 （浮動小数点許容）
DIFF=$(echo "$CASH - 1000000 - $REALIZED" | bc)
DIFF_ABS=$(echo "${DIFF#-}")  # 絶対値
[ "$(echo "$DIFF_ABS < 1.0" | bc)" = "1" ] \
  && pass "TC-J: cash = initial_cash + realized_pnl (cash=$CASH, realized=$REALIZED)" \
  || fail "TC-J" "cash ($CASH) ≠ 1000000 + realized_pnl ($REALIZED), diff=$DIFF"
```

---

### Phase C: E2E 追加テスト

#### S41: 指値注文ラウンドトリップ（`s41_limit_order_round_trip.sh`）

**目的**: 指値買い → 指値売り の cash フロー・クローズロジックを E2E で確認する。  
指値価格のトリック: BTC 価格帯（数万〜数十万 USD）を利用し、  
「必ず約定する指値」と「絶対に約定しない指値」を使って両方のパスを検証する。

| TC | 操作 | 期待値 | 検証内容 |
|----|------|--------|---------|
| A | REPLAY Playing 到達 → Pause | status=Paused | 前提確認 |
| B | 指値買い @999,999,999（必ず約定） | HTTP 200 | 注文受付確認 |
| C | step-forward ループ（最大 10 回） | open_positions.length >= 1 | 指値買いが on_tick で約定 |
| D | `cash < 1,000,000` | cash 減算確認 | A-1: 指値 fill でも cash deduct される |
| E | 指値売り @1（必ず約定） | HTTP 200 | 注文受付確認 |
| F | step-forward ループ（最大 10 回） | open_positions.length == 0 | 指値売りが Long をクローズ |
| G | `closed_positions.length == 1` | closed_positions に移動 | A-2: 指値クローズパス |
| H | `cash = initial_cash + realized_pnl` | A-0: 指値 close の cash 返還 | 売却代金返還の正確な検証 |
| I | 指値買い @1（絶対に約定しない） | HTTP 200 | 注文受付確認 |
| J | step-forward × 3 後 portfolio | open_positions 変化なし | 指値未達では約定しない |
| K | `GET /api/replay/orders` で pending 1件 | orders.length == 1 | 指値が pending のまま残る |

> 指値価格のトリック根拠（`order_book.rs`の約定ロジック）:  
> - Long 指値: `trade_price ≤ limit` → @999,999,999 なら任意の BTCUSDT 価格で約定  
> - Short 指値: `trade_price ≥ limit` → @1 なら任意の BTCUSDT 価格で約定  
> - Long 指値: `trade_price ≤ limit` → @1 なら現実的な価格では絶対不成立

---

#### S42: 裸ショートフルサイクル（`s42_naked_short_cycle.sh`）

**目的**: 成行売り（Long ポジションなし）→ Short open → 成行買いで Short クローズ の  
フルサイクルを E2E で確認する。A-2 の対称クローズロジック（buy closes Short）の検証。

| TC | 操作 | 期待値 | 検証内容 |
|----|------|--------|---------|
| A | REPLAY Playing 到達 → Pause | status=Paused | 前提確認 |
| B | `open_positions.length == 0` を確認 | Long ポジションなし | 裸ショートの前提 |
| C | 成行売り 1.0 BTC（Long なし） | HTTP 200 | 裸ショート注文受付 |
| D | step-forward ループ（最大 10 回） | open_positions.length == 1 | Short open |
| D-check | ループ後チェック | open_positions >= 1 でなければ FAIL | タイムアウト検出 |
| E | `open_positions[0].side == "Short"` | Short として open | A-2: 裸ショートが Short ポジションになる |
| F | `cash > 1,000,000` | cash += fill_price × 1.0 | A-1: Short open で cash が増加 |
| G | 成行買い 1.0 BTC（Short クローズ） | HTTP 200 | 注文受付確認 |
| H | step-forward ループ（最大 10 回） | open_positions.length == 0 | A-2 拡張: 買い約定で Short をクローズ |
| H-check | ループ後チェック | open_positions == 0 でなければ FAIL | タイムアウト検出 |
| I | `closed_positions.length == 1` | closed_positions に移動 | A-2: Short の record_close() 呼び出し |
| J | `realized_pnl != 0` | PnL 確定 | A-0: Short close の PnL 計算 |
| K | `cash = initial_cash + realized_pnl` | A-0: Short close の cash 処理 | 買い戻しコスト差し引きの正確な検証 |

---

## 作業順序と TDD サイクル

```
 1. ✅ A-0 テスト先行: record_close() の cash 返還を確認するテスト（壊れることを確認）
 2. ✅ A-0 実装: record_close() の cash 処理を売却代金方式に変更
 3. ✅ A-1 テスト先行: record_open() の cash deduction を確認するテスト（壊れることを確認）
 4. ✅ A-1 実装: record_open() に cash 操作を追加
 5. ✅ A-1.5 テスト先行: oldest_open_long_order_id() のテスト（RED）
 6. ✅ A-1.5 実装: VirtualPortfolio に oldest_open_long_order_id() を追加
 7. ✅ A-2 テスト先行: on_tick() 経由のクローズ（Long → Short sell でクローズ）のテスト（RED）
 8. ✅ A-2 実装: on_tick() に Short 約定→既存 Long クローズロジックを追加
 9. ✅ A-3: #[allow(dead_code)] 削除
10. ✅ cargo test — 全ユニットテスト PASS (289 passed)
11. ✅ cargo clippy -- -D warnings — PASS
12. ✅ B: S40 スクリプト作成
13. [ ] B: S40 実行・PASS 確認
14. [ ] C: S41 スクリプト作成
15. [ ] C: S41 実行・PASS 確認
16. [ ] C: S42 スクリプト作成（対称クローズ: buy closes Short — Phase 2）
17. [ ] e2e_order_panels_replay.md の PEND 項目を更新
```

## 実装メモ（2026-04-17）

### 設計上の判断

- **A-0 と A-1 は不可分**: `record_close` で売却代金を返還する方式に変更すると、  
  `record_open` で購入コストを差し引かないと既存テストが壊れる。両者をセットで実装。

- **borrow 回避パターン**: `on_tick()` 内で Short 約定時に `oldest_open_long_order_id()`  
  の戻り値（`Option<&str>`）を `.map(str::to_string)` で clone してから `record_close()` を呼ぶ。  
  `self.portfolio` の immutable/mutable borrow 競合を回避するため。

- **既存テストへの影響**: `realized_pnl_closes_position` (initial=100,000) は  
  A-0+A-1 後も成立: open @90,000 → cash=10,000、close @92,000 → cash=102,000。

- **対称クローズ（buy closes Short）は Phase 2**: 計画書には S42 として記載されているが、  
  現行実装は「sell closes Long」のみ。「buy closes Short」の実装は  
  `oldest_open_long_order_id` と対称の `oldest_open_short_order_id` が必要。

- **S40 の retry 回数**: 計画書の当初案は最大 5 回。最新仕様は最大 10 回だが、  
  S40 スクリプトは当初案の 5 回で作成した（十分なはず）。

- **Trades EventStore 統合未完のための迂回策（2026-04-17 追記）**:  
  `controller.rs:517` で `ingest_loaded(... trades: vec![])` と固定されており、  
  Playing 中も含めて `store.trades_in()` は常に空を返す。  
  S35 の「Trades EventStore 未統合のため約定なし」コメントが正確な現状。  
  **方針 A として `synthetic_trades_at_current_time()` を実装**: StepForward 後に  
  kline.close から合成 Trade を生成して `engine.on_tick()` に渡す。  
  **合わせて StepForward 時の engine.reset() を廃止**: 従来は `time_before != time_after` で  
  無条件リセットしていたが、StepForward では `is_step_forward` フラグで除外することで  
  pending 注文を維持した状態で約定させられるようになった。  
  変更ファイル: `src/replay/controller.rs`（新メソッド追加）、`src/main.rs`（条件分岐修正）。

---

## 影響範囲

| ファイル | 変更種別 |
|---------|---------|
| `src/replay/virtual_exchange/portfolio.rs` | `record_open()` cash 操作、`record_close()` cash 処理変更、`oldest_open_long/short_order_id()` 追加 |
| `src/replay/virtual_exchange/order_book.rs` | `on_tick()` に対称クローズロジック追加（sell→Long close, buy→Short close） |
| `tests/e2e_scripts/s40_virtual_order_fill_cycle.sh` | 新規作成（成行ラウンドトリップ） |
| `tests/e2e_scripts/s41_limit_order_round_trip.sh` | 新規作成（指値ラウンドトリップ） |
| `tests/e2e_scripts/s42_naked_short_cycle.sh` | 新規作成（裸ショートフルサイクル） |
| `docs/plan/e2e_order_panels_replay.md` | PEND 項目を更新 |

---

## 既知制限（本計画のスコープ外）

| 項目 | 理由 |
|------|------|
| 損失シナリオの E2E 決定論的テスト | replay 価格が上昇か下降かを制御できないため E2E では非決定論的。利益・損失の両方は unit テスト（`round_trip_realized_pnl_negative` 等）でカバー。S40/S41/S42 の TC-K は符号を問わず `cash = initial + realized_pnl` を検証するため数式は確認できる |
| 部分クローズ（qty が一致しない売り） | Phase 2 以降で対応 |
| 複数銘柄同時ポジションの unrealized_pnl | `portfolio.snapshot(current_price)` は単一価格前提 |
| StepBackward 後のエンジンリセット | `docs/order_windows.md §未実装` — 別 issue |
| 手数料・スリッページモデル | Phase 3 以降で対応 |
