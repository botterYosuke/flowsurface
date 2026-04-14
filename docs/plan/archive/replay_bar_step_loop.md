---
name: リプレイ バーステップループ化
description: リプレイ進行を wall-clock 連続モデルから「1バー＝1ステップ」の離散ループに統一する設計
type: project
---

# リプレイ バーステップループ化 設計プラン

**作成日**: 2026-04-13
**対象**: [src/replay/clock.rs](../../src/replay/clock.rs), [src/replay/dispatcher.rs](../../src/replay/dispatcher.rs), [src/replay/mod.rs](../../src/replay/mod.rs), [src/main.rs](../../src/main.rs)
**状態**: 実装完了 ✅ (Phase 1-4 全通過)
**前提ドキュメント**: [replay_redesign.md](replay_redesign.md), [replay_header.md](../replay_header.md)

## 目的

リプレイモードでの進行ロジックを **「1 バー = 1 ステップ」の離散ループ** に統一する。
現状の `wall_elapsed × speed` による連続的仮想時刻進行を廃止し、
リプレイは「シンプルなバー単位ループ」として再定義する。

これにより：

1. **「秒単位で `current_time` が滑らかに進む」仕様を撤回**し、バー境界でのみ時刻が更新される
2. `clock.rs` と `dispatcher.rs` のロジック量が大幅に減少する
3. D1 のための特殊 `bar_step_mode` (現状 [src/replay/clock.rs:131-154](../../src/replay/clock.rs#L131-L154)) が **デフォルトモード** になる
4. テストの境界条件（`saturating_add`, クランプ, `range.is_empty()` 判定など）が単純化する

## 撤回する旧仕様

[replay_header.md](../replay_header.md) および本セッション初回確認で合意していた以下の挙動は **撤回** する：

> ▶ ボタンをクリックすると、左上の `current time` がリアルと同じリズムで秒数が上がっていく

新仕様では `current_time` は **バー境界でのみジャンプ** する（例: 1m → `02:01:00` → `02:02:00` → `02:03:00` …）。
バー間の wall delay 中は `current_time` 表示は固定。

## 現状の問題点

### Q1. wall-clock 連続モデルが過剰

[src/replay/clock.rs:117-128](../../src/replay/clock.rs#L117-L128) で
`wall_elapsed × speed` を ms 精度で積算しているが、リプレイの目的上：

- **trade のサブ秒アニメ** は実装されていない（trade も kline と同じく一括 emit）
- **チャートに表示される最小単位** はバー（kline）であり、バー境界以外で `current_time` が動く意味がない
- 連続モデルは「1 フレーム内に 0 本／1 本／複数本」の 3 通りに分岐させるため、テスト・デバッグ負荷が高い

### Q2. `bar_step_mode` は production 未接続のテストスタブ

[src/replay/clock.rs:131-154](../../src/replay/clock.rs#L131-L154) `advance_bar_step` は存在するが、
`enable_bar_step_mode()` が `#[cfg(test)]` ゲート付きであるため
**production コードからは一切呼ばれていない**。`bar_step_mode` は常に `None` のまま動作する。

歴史的経緯:
- 30d0f2a (旧 `src/replay.rs` モノリス) で `advance_d1`/`process_d1_tick`/`is_all_d1_klines` という
  D1 スロットリング経路を production に実装した
- 258f326 の module 分割 (`src/replay/` 化) 時に **production 接続を引き継がず消滅** した
- `bar_step_mode` は同じ概念を unit test で検証する scaffold として残された残骸

**現在の production の D1 は 1m と同じ `wall_elapsed × speed` パスを通っており、
1x 速度では 1 本 = wall 24 時間待機という実質使えない挙動になっている。**

今回の変更は「二重実装を統合する」のではなく、
**テストにしかない正しい挙動（離散ステップ）を production default に昇格させる**ことが本質。
実装時は `bar_step_mode` / `advance_bar_step` / `enable_bar_step_mode` はすべて削除して構わない。

### Q3. マルチペイン timeframe 整合の暗黙性

現状は各 stream が独立に `klines_in(range)` を query し、range 内に bar boundary が含まれた stream のみ kline を受け取る。
「どの timeframe が tick を駆動するか」はコード上明示されておらず、`klines_in` の binary search 結果に依存している。
ループ化すれば「最小 timeframe = tick interval」がコードに明示される。

---

## 新設計: バーステップループ

### コア概念

```
   ┌─────────────────┐  wall_now ≥ next_step_at?
   │  StepClock      │ ──── yes ──▶  emit 1 bar (smallest TF)
   │  (state = now,  │               advance now += min_tf_ms
   │   next_step_at) │               next_step_at += step_delay
   └─────────────────┘ ──── no ───▶  no-op
```

各 frame で：

1. `wall_now >= next_step_at` を判定
2. true なら：
   - `now_ms += min_timeframe_ms`（active streams 中の最小 timeframe）
   - 全 active stream について `klines_in(prev..now)` で 0 or 1 本の kline を抽出
   - `next_step_at += step_delay_ms / speed`
3. false なら何も emit しない

### 設計原則

1. **離散時刻**: `now_ms` は **バー境界値のみを取る**。フレーム途中の中間値は持たない。
2. **単一ループ**: `bar_step_mode` 分岐を廃止し、全 timeframe で同一ロジック。
3. **smallest timeframe drives tick**: `step_size = min(active timeframes)` を単一の値として算出し、tick interval に用いる。active streams の本数・timeframe 組み合わせに関係なく同じ式で決まり、コード上の分岐は存在しない。大粒度 TF は `klines_in` の stream 独立クエリでバー境界を跨いだ tick のみヒットする。step_size は**時刻駆動の固定幅**なので、データ gap にも影響されない（後述「単一仕様で多 timeframe を吸収する」参照）。なお本設計は active TF が**互いに整除関係**にあることを前提とする（1m/3m/5m/15m/30m/1h/4h/1d はすべて 1m の倍数であり実運用で問題にならない）。
4. **speed は wall delay を圧縮**: `1x = 1000ms/step`, `2x = 500ms/step`, `5x = 200ms/step`, `10x = 100ms/step`。仮想時刻側の進行幅はバー幅で固定。speed=0 の扱いは未決（→「未決事項」参照）。
5. **trade はバーと同時 emit**: バー区間内の trade はそのバーの emit と同じ tick で一括送信（footprint アニメは引き続き犠牲）。
6. **seek はバー境界にスナップ**: `seek(target_ms)` は `step_size_ms` の倍数（`range.start` 基準）へ floor スナップする。サブバー精度の目標値は切り捨て。StepForward/StepBackward ([src/main.rs](../../src/main.rs) の ±1min 固定ロジック) もこの grid に合わせて書き換え対象。
7. **`set_step_size` 後の `now_ms` 再整列**: step_size が拡大する方向 (例: 1m → 5m) に変わった場合、現在の `now_ms` が新 step_size の境界上にない可能性がある（例: `now_ms = 00:03:00` で新 step_size = 5m）。`set_step_size` 内で `now_ms` を新 step_size 倍数へ floor 再整列すること。step_size 縮小方向 (5m → 1m) は旧 step_size が新 step_size の倍数であるため `now_ms` は既に新境界上にある。

### 確定パラメータ

| 項目 | 値 |
|---|---|
| 1 ステップで進む仮想時刻 | min_active_timeframe_ms (1m → 60_000) |
| wall delay (1x speed) | **1000 ms / bar** |
| speed multiplier | delay = 1000 / speed |
| trade ペーシング | バー単位で一括 emit |
| マルチ TF 駆動 | 最小 timeframe |

### 単一仕様で多 timeframe を吸収する

**本設計では「timeframe 混在」を特別扱いしない**。
「単一 TF のとき」と「複数 TF が同居するとき」でコードパスは分岐せず、
**同一のループ・同一のクエリロジックが任意の timeframe 組み合わせを自動的に処理する**。

これを成立させる 2 つの不変条件：

1. **step_size = min(active timeframes)** は単なる 1 つの値計算であり、`if multi_tf { ... }` のような分岐は存在しない。active streams が 1 本でも 10 本でも同じ式で算出される。
2. **`klines_in(stream, range)` は stream ごと独立に走る**。[src/replay/store.rs:63-67](../../src/replay/store.rs#L63-L67) の `range_slice` は `[start, end)` 半開区間の binary search であり、呼び出し側の「多 TF 意識」を必要としない。stream が増えれば query 回数が増えるだけ。

結果として、`dispatch_tick` のループは **1m 単独・D1 単独・1m+D1・1m+5m+D1** のいずれも区別なく処理する。
大粒度 TF はそれぞれのバー境界を跨いだ tick で `klines_in` が 1 本ヒットさせるため、追加の境界判定ロジックは存在しない。

以下のトレースは「多 TF に対応するための特別挙動」ではなく、**基本仕様がそのまま流れた結果**である。

#### トレース例: 1m + D1 同居 (1x speed, step_size = 60_000ms)

| wall 経過 | virtual now | 1m pane | D1 pane |
|---|---|---|---|
| 0s | `04-11 00:00` | — | — |
| 1s | `04-11 00:01` | +1 本 (`00:00` バー) | 0 本 |
| 2s | `04-11 00:02` | +1 本 | 0 本 |
| ... | ... | ... | ... |
| 1440s | `04-12 00:00` | +1 本 (`23:59` バー) | 0 本（D1 `04-12 00:00` は end 境界で除外）|
| 1441s | `04-12 00:01` | +1 本 (`00:00` バー) | **+1 本** (D1 `04-12 00:00` バー) |

→ 1m が毎秒更新、D1 は 24 分ごとに 1 本という自然な動き。
速く見たいときは `speed = 10x` に上げれば D1 は 2.4 分/本で実用範囲。

#### gap 耐性: データ欠損時も境界 emit は落ちない

1m データに gap があるケース（例: `04-11 23:59` の次が `04-12 00:01`, `00:00` 欠損）でも
D1 バーは正しく発火する。

| tick | range (半開区間) | 1m pane | D1 pane |
|---|---|---|---|
| `23:59:00 → 00:00:00` | `[23:59:00, 00:00:00)` | `23:59` バー ✓ | 0 本（D1 `00:00` は end 境界で除外）|
| `00:00:00 → 00:01:00` | `[00:00:00, 00:01:00)` | 0 本（1m gap）| **D1 `04-12 00:00` バー ✓** |

**原理**: step_size は**時刻駆動の固定幅**であり、データの有無に依存しない。
`klines_in` は各 stream 独立に binary search するため、1m に gap があっても
D1 stream の境界バーは別経路で必ず hit する。

#### off-by-one-tick 注記

D1 バーは「正確に `00:00:00` の wall time」ではなく、その **1 tick 後（= wall +1 秒）** に emit される。
これは range 半開区間の性質による不可避の 1 tick 遅延だが、リプレイ用途では無視できる。
（連続的な時刻進行ではないため、そもそも "exactly at midnight" という概念が離散モデルに存在しない。）

---

## 型・API 変更

### `VirtualClock` → `StepClock` へリネーム

```rust
pub struct StepClock {
    /// 現在の仮想時刻 (Unix ms)。常にバー境界値。
    now_ms: u64,
    /// 次のステップを発火する wall 時刻。Pause/Waiting 中は None。
    next_step_at: Option<Instant>,
    /// 1 ステップで進める仮想時刻幅（min active timeframe ms）。
    step_size_ms: u64,
    /// 1 ステップあたりの wall delay (1x speed 基準, ms)。
    base_step_delay_ms: u64,
    /// 再生速度倍率。実 delay = base_step_delay_ms / speed。
    speed: f32,
    status: ClockStatus,
    range: Range<u64>,
}

impl StepClock {
    pub fn new(start_ms: u64, end_ms: u64, step_size_ms: u64) -> Self;

    /// 各フレームで呼ぶ。発火タイミングなら 1 ステップ進めて emit range を返す。
    /// そうでなければ空 range を返す。
    pub fn tick(&mut self, wall_now: Instant) -> Range<u64>;

    /// active streams が変わったとき呼ぶ（最小 timeframe が変わる可能性）。
    pub fn set_step_size(&mut self, step_size_ms: u64);

    pub fn play(&mut self, wall_now: Instant);
    pub fn pause(&mut self);
    pub fn seek(&mut self, target_ms: u64);  // バー境界にスナップ
    pub fn set_speed(&mut self, speed: f32);
    pub fn set_waiting(&mut self);
    pub fn resume_from_waiting(&mut self, wall_now: Instant);
}
```

**廃止される API / フィールド**:
- `anchor_wall: Option<Instant>` （`next_step_at` に置換）
- `bar_step_mode: Option<(u64, u64)>` （デフォルト動作に統合）
- `enable_bar_step_mode()` （不要）
- `advance()` 内の `wall_elapsed × speed` 計算
- `advance_bar_step()` （`tick()` に統合）

### `dispatch_tick`

シグネチャは変えず、内部で `clock.advance()` を `clock.tick()` に置換。
`tick()` が空 range を返した場合は no-op。それ以外は現状通り
全 stream に対して `klines_in / trades_in` を query する。

```rust
pub fn dispatch_tick(
    clock: &mut StepClock,
    store: &EventStore,
    active_streams: &HashSet<StreamKind>,
    wall_now: Instant,
) -> DispatchResult {
    // Waiting / loaded チェックは現状維持
    let range = clock.tick(wall_now);
    if range.is_empty() {
        return DispatchResult::empty(clock.now_ms());
    }
    // 既存ロジックそのまま: stream ごとに klines_in / trades_in を query
}
```

### `ReplayState` 側

- `start(start_ms, end_ms)` に **min_timeframe_ms を渡す** 必要がある。
  - 呼び出し元 ([src/main.rs](../../src/main.rs) Replay::Play 処理) で active streams から最小 timeframe を計算
  - active streams が 0 の場合のフォールバック（例: 60_000ms = 1m）を定数化
- mid-replay で stream 追加/削除した際に `clock.set_step_size(...)` を呼んで再計算

---

## 実装ステップ

### Phase 1: StepClock 実装 ✅

1. ✅ [src/replay/clock.rs](../../src/replay/clock.rs) を全面書き換え
   - `VirtualClock` → `StepClock`
   - `tick()` 実装（`next_step_at` ベースの発火判定）
   - 旧 `advance` / `advance_bar_step` / `bar_step_mode` を削除
2. ✅ 単体テスト書き換え（17 テスト全通過）
   - `tick_emits_one_step_at_step_delay`
   - `set_speed_zero_pauses_clock` (speed=0 → Pause と同義)
   - `tick_advances_step_size_per_fire`
   - `seek_snaps_to_bar_boundary_floor`
   - `set_speed_2x_halves_step_delay`
   - `multiple_ticks_catchup_in_one_frame`
   - `catchup_clamps_at_range_end_and_pauses`
   - `set_step_size_floor_realigns_now_ms_on_expansion`

**Tips**: `seek(range.end)` は floor スナップしない（終端境界として有効）。
`set_speed(0.0)` は `pause()` を呼ぶ（speed 値は変更しない）。
speed 切替時 `next_step_at` は変更しない（現在のスケジュールそのままで次ステップから新 delay が適用）。

### Phase 2: dispatcher 連携 ✅

3. ✅ [src/replay/dispatcher.rs](../../src/replay/dispatcher.rs) で `clock.advance()` → `clock.tick()` 置換、`VirtualClock` → `StepClock`
4. ✅ テスト更新（6 テスト全通過）
   - `dispatch_returns_one_trade_in_half_open_range_when_store_loaded`: 旧 2 本 → 新 1 本（[0,1000) の半開区間で trade@1000 除外）
   - `dispatch_catchup_two_steps_returns_events_from_entire_range`: wall=2000ms で 2 steps catch-up → 2 本

### Phase 3: ReplayState 統合 ✅

5. ✅ [src/replay/mod.rs](../../src/replay/mod.rs) `ReplayState::start` シグネチャに `step_size_ms` 追加
   - `pub fn min_timeframe_ms(active_streams: &HashSet<StreamKind>) -> u64` を追加（Kline のみ filter、fallback = 60_000ms）
6. ✅ [src/main.rs](../../src/main.rs) `Replay::Play` で `kline_targets` から step_size を計算して `start(start_ms, end_ms, step_size_ms)` へ渡す
7. ✅ `SyncReplayBuffers` に `clock.set_step_size(min_timeframe_ms(&active_streams))` を追加

### Phase 4: E2E 検証 ✅

8. ✅ E2E テストスクリプト `C:/tmp/e2e-stepclock.sh` を作成・実行
   - **方針変更**: fixture を Live モード（`replay` フィールドなし）で起動 → 15s 待機 → Toggle + Play で streams 解決
   - 16/16 全テスト通過（2026-04-13）
   - T3: 全 5 サンプルが 60s グリッド境界 ✓
   - T4: 1x/5s wall → 5 bars (delta=300000ms) ✓
   - T5: 2x/5s wall → 10 bars (delta=600000ms) ✓
   - T6: Pause → current_time 固定 ✓
   - T7: 2nd StepForward = 60000ms, StepBackward = 60000ms ✓
   - T8: Resume → Playing + advancing ✓
   - T9: Toggle to Live ✓
9. `pane_crud_api.md` mid-replay E2E は別タスクとして継続

---

## トレードオフ・リスク

### ✓ 得るもの

- **コード量削減**: clock.rs から `wall_elapsed × speed` 計算と `advance_bar_step` 二重実装を削除（推定 -60 行）
- **テスト境界の単純化**: ms 精度のクランプ・`saturating_add` 境界が消える
- **マルチ TF が明示的**: 最小 timeframe = tick driver がコード上に出現

### ✗ 失うもの

- **`current_time` 秒進行表示**: 1m chart なら 1 秒待機 → 60s ジャンプという「カクついた」表示になる
  - → ユーザー受け入れ済み（「リアルタイムリズムで秒が上がる」を撤回）
- **サブバー精度の seek**: `seek(target_ms)` はバー境界にスナップされる
  - → リプレイ用途では問題なし

### ⚠ 注意点

- **catch-up 挙動**: ウィンドウ非アクティブ等で frame が長期間止まった後に再開した場合、`tick()` を 1 フレーム内で複数回ループさせて溜まったバーをまとめて emit する必要がある。実装漏れがあると D1 連続再生で取りこぼす（既存 `advance_bar_step` の `bars_to_advance` ロジックを移植）。**また catch-up ループ内で `range.end` 到達チェックを入れ忘れると無限ループになる**。Paused 遷移と now_ms クランプを while ループの継続条件に組み込むこと。
- **min_timeframe 計算タイミング**: ペイン追加/削除の前後で正しく `set_step_size` を呼ばないと、新ペインの最初のバーが取りこぼされる可能性
- **speed 切替時の整合**: `next_step_at` を新 delay で再計算するか、現在の wait をそのまま消化するか要決定 → **現在の wait を新 delay 比で按分** が自然

---

## 決定済み事項

- [x] **多 timeframe を分岐で扱わない**: 単一 TF も複数 TF 同居も同一のループ・同一のクエリロジックで処理する。`step_size = min(active timeframes)` と stream ごと独立な `klines_in` により、timeframe 組み合わせは仕様に内在的に吸収される（→「単一仕様で多 timeframe を吸収する」節）
- [x] **gap 耐性**: step_size は時刻駆動の固定幅とし、データ欠損時も大粒度 TF の境界 emit を落とさない
- [x] **`current_time` 秒進行表示の撤回**: バー境界でのみジャンプする離散モデルに統一
- [x] **`min_timeframe_ms` の所在**: free function `pub fn min_timeframe_ms(active_streams: &HashSet<StreamKind>) -> u64` を `src/replay/mod.rs` に配置（pub にして main.rs から呼べる）
- [x] **`min_timeframe_ms` の抽出元**: Kline stream のみ filter。Kline が 0 本 (Trades only 構成) の場合は 60_000ms (1m) を fallback
- [x] **active streams が空の状態で Play**: kline_targets が空 → step_size=60_000ms フォールバック。clock は Waiting 状態のまま即座に Playing に移行（kline_tasks が空なので `resume_from_waiting` が呼ばれる）
- [x] **speed 切替時の `next_step_at`**: 変更しない。現在スケジュールをそのまま消化し、次ステップから新 delay が適用される（シンプル＆wall_now 不要）
- [x] **speed=0 の扱い**: `set_speed(0.0)` → `pause()` を呼ぶ（speed 値は変更しない）。div-by-zero 回避
- [x] **StepClock リネーム**: `VirtualClock` → `StepClock` に全面リネーム

## 未決事項

（Phase 1-3 完了により未決事項は全解決）
