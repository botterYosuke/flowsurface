# リプレイ統一ステップ + リプレイ中ペイン操作許容 計画書

**作成日**: 2026-04-12
**最終更新**: 2026-04-12（Phase 5 レビュー修正完了、全 140 テスト green）
**対象**: `src/replay.rs`, `src/main.rs`, `src/screen/dashboard.rs`, `src/screen/dashboard/pane.rs`, `src/chart/kline.rs`
**前提ドキュメント**: [docs/replay_header.md](../replay_header.md), [docs/plan/tachibana_replay.md](tachibana_replay.md)
**状態**: Phase 1〜5 コード実装完了（実機 E2E 検証は未実施）

## 進捗サマリ

| Phase | 状態 | commit |
|:-:|---|---|
| 0 (事前調査) | ✅ 完了 | - |
| 1 (Tick 統合 + 既存テスト移行) | ✅ 完了 | 未 commit |
| 1b (StepForward/Backward 離散化) | ✅ 完了 | 未 commit（Phase 1 と同 commit） |
| 3-0 (バックフィル関数抽出) | ✅ 完了 | 未 commit |
| 3-1+ (mid-replay 操作許容) | ✅ 完了 | 未 commit |
| 4 (新規テスト + UI tooltip) | ✅ 完了 | 未 commit |
| 5 (コードレビュー修正) | ✅ 完了 | 未 commit |
| E2E 実機検証 | ⬜ 未実施 | - |

### Phase 1 + 1b 実装メモ（2026-04-12）

**実装の判断**:
- Phase 1b を Phase 1 から分離する計画だったが、`ReplayMessage::StepForward` / `StepBackward` も `is_all_d1_klines()` を参照していたため、Phase 1 で削除する時点で **Phase 1b の内容（両 Step の離散化）を同 commit に含めざるを得なかった**。計画では「別 commit に分離して revert しやすく」としていたが、コンパイルエラー回避のため不可能。→ revert 時は Phase 1 全体を戻す前提に変更
- `fire_status()` は `Option<FireStatus>` を返す設計にした。`None` は「kline chart が 1 つも存在しないダッシュボード」を表し、main.rs Tick ハンドラで **linear advance 経路にフォールバック**する（heatmap-only リプレイの既存挙動を温存するため）
- `process_tick()` は `Ready` 経路で drain を「毎 Tick」実行し、ジャンプ時は「drain → advance → drain」の 2 段構えにした（§2.1 case 1 / 穴 A）。`drain_until` は cursor ベースで冪等なので二重実行しても問題ない
- `FireStatus::Terminal` 時は `virtual_elapsed_ms = 0.0` にクリアしてから `Paused`。`Pending` 時は **加算も減算もせず据え置き**（§2.1 case 2、ジャンプ遅延による乱れを防ぐ）

**新規追加 API**:
- `ReplayKlineBuffer::is_ready()` — `klines.is_empty()` の否定
- `KlineChart::replay_buffer_ready()` — 上記を Option で包んだ版
- `pane::State::replay_kline_chart_ready() -> Option<bool>` — 3 状態（非kline=None / kline未ready=Some(false) / kline+ready=Some(true)）
- `Dashboard::fire_status(current, main_window) -> Option<replay::FireStatus>`
- `replay::FireStatus { Ready(u64), Pending, Terminal }` enum
- `replay::TickResult { current_time, trades_collected }` struct
- `replay::process_tick(pb, virtual_elapsed_ms, elapsed_ms, fire_status) -> TickResult`
- `replay::COARSE_CUTOFF_MS` / `replay::COARSE_BAR_MS` 定数

**削除した API**:
- `replay::advance_d1()` / `replay::process_d1_tick()` / `replay::D1TickResult`
- `Dashboard::is_all_d1_klines()` / `all_kline_streams_are_d1()`（純粋関数 + テスト 6 件）
- `ReplayState::d1_virtual_elapsed_ms`（→`virtual_elapsed_ms` にリネーム）
- Tick ハンドラの `is_d1` 分岐（main.rs:285-387）
- StepForward / StepBackward ハンドラの `is_d1` 分岐（main.rs:931-1052）

**新規テスト (replay.rs)**:
- `process_tick_m1_jumps_after_60s_virtual_elapsed` — M1 実時間連動
- `process_tick_m1_throttles_until_accumulated_threshold` — M1 累積で初めてジャンプ
- `process_tick_d1_jumps_after_1s_virtual_elapsed` — D1 粗補正
- `process_tick_h1_boundary_uses_coarse_threshold` — H1 境界（`>=` で粗補正側）
- `process_tick_h4_uses_coarse_threshold` — H4 粗補正
- `process_tick_d1_speed_scales_jump_rate` — speed スケーリング
- `process_tick_terminal_pauses_and_clears_virtual_elapsed` — Terminal 遷移
- `process_tick_pending_holds_virtual_elapsed_and_status` — Pending 据え置き
- `process_tick_drains_trades_when_jumped` — ジャンプ時 drain
- `process_tick_drains_every_tick_even_without_jump` — 穴 A: 未達 Tick でも drain
- `process_tick_skips_drain_when_all_buffers_empty` — Tachibana ケース
- `replay_buffer_is_ready_*` (3 件) — ReplayKlineBuffer::is_ready 述語

**Phase 3 への申し送り**:
- `fire_status()` の `None` フォールバック（kline chart 無し）は linear advance 経路に落とす設計。Phase 3 で mid-replay 追加後もこのフォールバックを維持することで、heatmap-only → kline 追加の過渡期を安全に通過できる
- `process_tick` は内部で `drain_all_trade_buffers` を呼んでおり、この関数は Phase 3 で `pending_trade_streams` によるスキップロジックを追加する予定（§2.3.1.1 穴 B）
- Tick ハンドラの既存挙動は Phase 3 commit で linear-fallback 経路も同様にフィルタを追加するか、リファクタで process_tick 経路に一本化するか検討が必要

### Phase 3-0 / 3-1+ / 4 実装メモ（2026-04-12）

**Phase 3-0 (バックフィル関数抽出)**:
- `build_kline_backfill_task(pane_id, stream, start_ms, end_ms, layout_id) -> Task<Message>` を `main.rs` に自由関数として追加
- `build_trades_backfill_task(stream, start_ms, end_ms) -> Task<Message>` を同じく追加
- Play ハンドラのインライン kline/trades fetch ロジックを上記関数呼び出しに置換（挙動は不変）
- 計画では「`Dashboard::build_kline_backfill_task`」としていたが、これらは Task を直接生成する関数であり Dashboard 状態に依存しないため、`main.rs` の自由関数の方が適切と判断

**Phase 3-1+ (mid-replay 操作許容)**:
- **`ReplayMessage::SyncReplayBuffers`** 新規追加。`refresh_streams()` を改造する代わりに、`Message::Dashboard` ハンドラの末尾で常に `Task::done(SyncReplayBuffers)` を chain する実装にした
  - 理由: `refresh_streams()` は `Task<Message>` を返す設計で、dashboard 内部で `Message::Replay(...)` を生成できない（dashboard 側には top-level `Message` 型が無い）。Event 経由の通知は既に他用途で使われており、`Option<Event>` を変更すると影響範囲が大きい。Message::Dashboard ハンドラ末尾での chain は冪等で単一のタッチポイントで済む
  - 冪等性: `SyncReplayBuffers` ハンドラは replay 未有効時は即リターン。新規 stream が無ければバックフィル task も生成しない
- **`PlaybackState::pending_trade_streams: HashSet<StreamKind>`** 追加（§2.3.1.1 穴 B）
  - `drain_all_trade_buffers` は pending set に含まれる stream をスキップする
  - `TradesFetchCompleted` ハンドラで `advance_cursor_to(current_time)` を呼び、pending から削除
- **`TradeBuffer::advance_cursor_to(target_time) -> usize`** を §2.3.1 の擬似コード通りに実装（単調増加ガード含む）
- **`pane::State::enable_replay_mode_if_needed() -> bool`** で冪等な replay モード切替を実現（既に replay 中なら no-op）
- **`Dashboard::collect_new_replay_klines()`** は「replay_kline_buffer が None の kline chart」を探して enable_replay_mode を呼び、kline stream の (pane_id, stream) リストを返す
- **`Dashboard::is_all_d1_klines()` と `replay::process_d1_tick()` の跡地はすべて削除済み**
- timeframe / ticker 変更時の挙動: `set_basis()` → `Effect::RequestFetch` → `refresh_streams()` → `Message::Dashboard` ハンドラ末尾で `SyncReplayBuffers` が発火する経路で処理される
- ペイン削除時の orphan trade stream 掃除も `SyncReplayBuffers` ハンドラで差分計算により実施

**Phase 4 (新規テスト + UI tooltip)**:
- 追加したテスト（replay.rs）:
  - `advance_cursor_to_*` × 5 件（§2.3.1 ケース網羅）
  - `process_tick_skips_drain_for_pending_trade_streams`（穴 B 検証）
  - `pending_trade_stream_cleared_on_advance_cursor_to_simulation`（§2.3.1.1 全体フロー模擬）
- **UI tooltip**: リプレイヘッダの speed ボタンに `iced::widget::tooltip` をラップし、「H1 以下: 実時間連動 / H4 以上: 1 バー/sec × speed」を表示（§4-7）
- 最終テスト件数: **129 件 all green**（`cargo test --bin flowsurface -- --test-threads=1` で確認）

### Phase 5 実装メモ（2026-04-12、レビュー駆動の修正）

Phase 1〜4 完了後のコードレビューで発見された 4 件のバグ・設計不整合を TDD で修正。全て RED → GREEN の順で検証。

#### ✅ Fix 1 🔴: `KlineChart::set_basis()` が `replay_kline_buffer` を再初期化しない

**問題**: リプレイ中に timeframe を M1→M5 に変更すると、旧 M1 の klines が `replay_kline_buffer` に残ったまま新 M5 の空 `data_source` に注入され、チャートが崩壊する。さらに `collect_new_replay_klines()` は `replay_kline_buffer.is_none()` を要求するため、新バックフィルも発火しない → ペインが完全に「死ぬ」。

**根本原因**: [src/chart/kline.rs:655](../../src/chart/kline.rs#L655) の `set_basis()` は `data_source` を空に作り直すが、リプレイ専用バッファ `replay_kline_buffer` には一切触れていなかった。

**修正**: `set_basis()` の末尾で、`replay_kline_buffer` が `Some` の場合は `klines.clear()` + `cursor = 0`。`None`（非リプレイ）の場合は副作用なし（`set_basis_does_not_enable_replay_mode_when_disabled` テストで担保）。

**設計思想**: リプレイモード自体（`Some/None`）は維持し、**「中身だけ空にする」**。これにより `replay_buffer_ready() == false` になり、`fire_status()` から除外されて Pending 扱いされる。次 Tick 以降で `SyncReplayBuffers` が発火し、`collect_new_replay_klines` が空バッファを検出 → バックフィル発火、という既存経路に綺麗に合流する。

**テスト** (src/chart/kline.rs):
- `set_basis_resets_replay_kline_buffer_when_in_replay_mode` — RED→GREEN 確認済み
- `set_basis_does_not_enable_replay_mode_when_disabled` — 非リプレイ chart に副作用を出さない

**Tip（後続作業者向け）**: この fix は単体テスト可能だが、`KlineChart::new` の全引数を揃える test helper (`build_test_kline_chart`) を追加した。今後 KlineChart レベルのテストを書くときはこれを再利用できる。`TickerInfo::new(Ticker::new("BTCUSDT", Exchange::BinanceSpot), 1.0, 1.0, None)` と `PriceStep { units: 1 }` は有効。

---

#### ✅ Fix 2 🔴: `TradesBatchReceived` が `or_insert_with` で orphan buffer を自己復活させる

**問題**: mid-replay でペイン削除 → `SyncReplayBuffers` が orphan trade stream を削除 → しかし進行中の `fetch_trades_batched` タスクは `_handle` が捨てられているため abort されない → 次のバッチ到着時に `or_insert_with` で **削除したはずのバッファが復活** → 次の `SyncReplayBuffers` で再度 orphan 検出 → 無限 flap。メモリリークには至らないが、削除直後の数秒〜数分は `pending_trade_streams` / `trade_buffers` の状態が乱れる。

**根本原因**: [src/main.rs:1048-1057](../../src/main.rs#L1048) の `or_insert_with` が「未登録 stream を黙って作る」挙動を持っていた。ペイン削除経路でタスクをキャンセルする設計が不在のため、受信側でガードするしかない。

**修正方針（案 b を採用）**: 1 行修正で orphan buffer の自己復活を止める。`fetch_trades_batched` のキャンセル経路を後付けするより、受信側で「登録済み stream のみ accept」するほうが変更範囲が小さく冪等。

**実装**:
1. `PlaybackState::ingest_trades_batch(stream, batch) -> bool` を新設。`trade_buffers.get_mut()` が `Some` のときのみ追記し、戻り値で accept/drop を返す。
2. `TradesBatchReceived` ハンドラを `pb.ingest_trades_batch(stream, batch)` の 1 行に置換。戻り値は現状無視（`let _ = ...`）、将来デバッグログが欲しければここで拾う。

**テスト** (src/replay.rs):
- `ingest_trades_batch_accepts_for_registered_stream` — 正常系
- `ingest_trades_batch_drops_batch_for_orphan_stream` — orphan 拒否、`trade_buffers` に再出現しないことを確認
- `ingest_trades_batch_preserves_cursor_on_accepted_append` — cursor への副作用がないこと

**Tip**: この fix は **fetch タスクの abort 問題そのものを解決していない**。残存タスクは自然に完了するまでバッチを送り続けるが、全て drop される。CPU / ネットワーク負荷が気になるならフォローアップで `PlaybackState::trade_fetch_handles: HashMap<StreamKind, AbortHandle>` を導入して orphan 時に `abort()` を呼ぶ（案 a）。

---

#### ✅ Fix 3 🟡: Tooltip 文言と境界値の不整合

**問題**: 速度ボタンのツールチップに「H1 以下: 実時間連動 / H4 以上: 1 バー/sec × speed」と書かれていたが、実装の境界は `COARSE_CUTOFF_MS = 3_600_000ms` で **>= H1 が粗補正側**になる。つまり H1 は tooltip 上 fine、実装上 coarse という矛盾。加えて H2 / H3 の扱いが不明瞭。

**修正**:
- Tooltip 文言を「M30 以下: 実時間連動 × speed / H1 以上: 1 バー/秒 × speed」に変更。H1 を境界の上側として明示。
- 該当箇所（[src/main.rs:1402-1415](../../src/main.rs#L1402)）にコメントで `replay::COARSE_CUTOFF_MS = 3_600_000ms` を記載し、将来値を変えるときに文言も見直すべき旨を明記。

**テスト** (src/replay.rs):
- `process_tick_m30_uses_fine_threshold` — M30 (1_800_000ms) は fine 側、1 秒では threshold に達せずジャンプしない
- `coarse_cutoff_boundary_matches_h1_in_ms` — `COARSE_CUTOFF_MS == 3_600_000` の固定化（この値が動いたら tooltip も見直しが必要という invariant を残す）

**Tip**: threshold 境界を「timeframe 名で区切る」のは本質的に不正確（境界は時間値で決まる）。将来 M45 や M50 を追加した場合、ツールチップの文言を「1 時間未満 / 1 時間以上」に変えるほうが長持ちする。

---

#### ✅ Fix 4 🟡: `SyncReplayBuffers` が ticker 選択経路でトリガーされない

**問題（review で想定していた抜けより深刻）**: `Message::Sidebar` → `Action::TickerSelected` → `init_focused_pane()` / `switch_tickers_in_group()` → **`Task::none()` を返すケースがある**（heatmap-only のようにフォローアップ fetch タスクが無い場合）。すると `task.map(|| Message::Dashboard)` から 1 件もメッセージが流れず、`Message::Dashboard` ハンドラ末尾の `SyncReplayBuffers` chain が発火しない → 新しい trade stream が登録されず、バックフィルも走らず、ペインは永遠に空のまま。

**発見経路**: Phase 3 実装時は「`refresh_streams()` を呼ぶ全入口は `Message::Dashboard` を通る」と仮定していたが、[src/screen/dashboard.rs:739-777](../../src/screen/dashboard.rs#L739) の `init_focused_pane()` は kline stream がある場合だけ fetch task を返し、無ければ `Task::none()` を返す設計だった。`set_content_and_streams()` は内部で `streams` を変更しているのに、その副作用を伴うメッセージは発信しない。

**根本原因**: "設定変更系" と "データ取得系" がどちらも同じ Task 経路に乗っているため、「設定が変わったがデータ取得は不要」というケースが沈黙する。

**修正方針**: `init_focused_pane()` 側を直すのは影響範囲が大きいので、**呼び出し側（`Message::Sidebar` ハンドラ）で常に `SyncReplayBuffers` を chain する**。Replay 中でなければ handler 側で no-op になるので非リプレイ時のコストはほぼゼロ。

**加えて（リファクタ）**: `SyncReplayBuffers` ハンドラ内の「新規 / orphan trade stream の差分計算」を `PlaybackState::diff_trade_streams(current: &[StreamKind]) -> TradeStreamDiff` として純粋関数に抽出。テスト容易性を確保し、Fix 4 の回帰テストの入口を提供する。

**実装**:
1. `PlaybackState::diff_trade_streams(current) -> TradeStreamDiff` 追加
2. `TradeStreamDiff { new_streams, orphan_streams }` 構造体（`Default` derive）
3. `SyncReplayBuffers` ハンドラの差分計算ブロックを `diff_trade_streams()` 呼び出しに置換（挙動は不変）
4. `Message::Sidebar` → `TickerSelected` の return path に `.chain(Task::done(Message::Replay(SyncReplayBuffers)))` を追加

**テスト** (src/replay.rs):
- `diff_trade_streams_detects_new_streams`
- `diff_trade_streams_detects_orphan_streams`
- `diff_trade_streams_empty_when_no_change`
- `diff_trade_streams_both_new_and_orphan`

**Tip（後続作業者向け）**: 他にも同じパターンで `Message::Dashboard` を経由しない経路が無いか確認する価値あり。grep 候補:
- `Message::Sidebar` の action 経路全般
- `Message::Hotkey` / `Message::Shortcut` 系
- `Layout::LoadLayout` / `LayoutSelected` 経路（リプレイ中の layout 切替は本計画スコープ外だが）

特に「ペインの streams 集合を mutate するが、後続 fetch タスクを発火しない可能性のある関数」をリストアップすると見通しが良い。`set_content_and_streams` の caller を追うのが最短。

---

#### Phase 5 最終テスト件数

**140 件 all green**（`cargo test --bin flowsurface --quiet`, 2026-04-12 時点）。
- Phase 4 終了時点から +11 件（Fix 1: +2, Fix 2: +3, Fix 3: +2, Fix 4: +4）

#### Phase 5 残課題

- **🟢 [低] clippy 警告 6 件**: `collapsible_if` × 3、`unnecessary_map_or` × 1、その他 2 件。いずれも本計画での新規コードに起因しない既存の警告。Fix 対象外。
- **🟡 [中] フォールバック経路の `pending_trade_streams` 未対応**: [src/main.rs:389-410](../../src/main.rs#L389) の linear advance 経路は `drain_all_trade_buffers` を使わず旧 `drain_until` 直呼びのため、heatmap-only リプレイ中にペインを追加しても pending ガードが効かない。heatmap-only + mid-replay 追加は限定ユースケースなので未対応で残した。将来、linear fallback を `process_tick` 経路に統合する際に一緒に解消するのが筋。
- **🟡 [中] fetch タスクのキャンセル経路不在**: Fix 2 の Tip に記載した通り、残存 fetch タスクは abort されず自然完了まで稼働する。`PlaybackState::trade_fetch_handles: HashMap<StreamKind, AbortHandle>` を追加すれば解消できるが、影響範囲が中規模になるため見送り。

#### Phase 5 で増えた設計上の不変条件（invariants）

1. **`set_basis()` は replay モードを維持する**: `Some/None` 状態を変えず、中身だけ空にする。これを破ると mid-replay の timeframe 変更が壊れる。
2. **`trade_buffers` への挿入は `ingest_trades_batch` 経由のみ**: 他経路で `or_insert_with` 等を使うと Fix 2 の orphan 自己復活ループが再発する。
3. **`SyncReplayBuffers` は `Message::Dashboard` 末尾の chain に加えて明示的な chain も必要**: `Task::none()` を返す Sidebar 経路があるため。新しく「streams を mutate するが fetch task を返さない関数」を追加する場合は、その呼び出し側でも SyncReplayBuffers を chain する必要がある。
4. **`COARSE_CUTOFF_MS` は tooltip 文言とセット**: この定数を変更するときは必ずツールチップも見直す。`coarse_cutoff_boundary_matches_h1_in_ms` テストがトリップワイヤー。

---

**残存課題 / 申し送り**:
- **E2E 実機検証**は未実施。以下を実機で確認する必要がある:
  - §6.2 #1 単独 M1 Replay → Play（実時間連動、旧挙動と同等）
  - §6.2 #2 Tachibana D1 Replay → Play（1 D1/sec、Phase 3 互換）
  - §6.2 #3 混在 M1+D1 Replay → Play
  - §6.2 #4 M1 Replay 中 StepForward（Phase 1b 挙動変化点）
  - §6.2 #5 リプレイ中 SplitPane → ticker 選択 → バックフィル → 同期
  - §6.2 #6 リプレイ中 timeframe 変更（M1 → M5）
  - §6.2 #7 リプレイ中 ClosePane → orphan stream 削除
  - §6.2 #8 リプレイ中 SplitPane → 即 Pause
  - §6.2 #10 バックフィル失敗時の Toast 表示
- **clippy 警告**: 既存コードに多数あるが、本 Phase で追加した分（pane.rs:434 の collapsible_if）は修正済み。残りは事前警告で本計画スコープ外
- **keyring テストの parallel 失敗**: 既存問題。`cargo test -- --test-threads=1` で解消。本変更とは無関係

**Phase 3 で採用しなかった設計**:
- `Dashboard::sync_replay_buffers()` を `refresh_streams()` 内で呼ぶ案 → `Task<Message>` の型境界の都合で断念
- `Effect::RequestFetch` 処理末尾での明示 `sync_replay_buffers()` 呼び出し → `Message::Dashboard` ハンドラ末尾 chain に統合
- 旧計画の 2 箇所集約（§2.6.5）は「1 箇所集約（Message::Dashboard 末尾）」に変更。`refresh_streams()` のあらゆる呼び出し経路は Message::Dashboard を経由するので、これでカバーできる

---

## 0. この計画の立ち位置

[docs/plan/tachibana_replay.md](tachibana_replay.md) Phase 3 で導入した「`is_all_d1_klines()` による Tick ハンドラの 2 分岐」は、`StreamKind` 集合の状態に依存する all-or-nothing 判定であり、**リプレイ中にペイン構成や timeframe が変わった瞬間に意図しない経路に切り替わる脆さ**を抱えている。さらに「リプレイ中のペイン操作無効化」（[docs/replay_header.md](../replay_header.md) §3 Phase 3）はユーザにとっての制約が大きく、解除要望が強い。

本計画は **2 つの構造変更を 1 つの方向に束ねる**：

1. **Tick ハンドラを「全 kline ストリームの最小 timeframe バー 1 本」を最小ステップとする単一経路に統合する**（`is_all_d1_klines()` 分岐を撤廃）
2. **リプレイ中のペイン追加・削除・timeframe 変更を許容する**

両者は独立した変更だが、(1) の「ライブ buffer 集合を毎 Tick 読む」設計が (2) の前提条件になるため、まとめて 1 計画として進める。

### ゴール

- `Message::Tick` 内の D1 / 非 D1 分岐を消し、`replay::process_tick()` 1 関数に統合
- リプレイ中の `SplitPane`、ペイン削除、timeframe 変更、ticker 変更を許容
- 上記操作が発生した場合、新規ストリームに対して `start_time → current_time` のバックフィルを非同期で実行し、完了後に既存と同じ経路で再生に合流

### 非ゴール

- `speed` ラベルや UI スライダの全面再設計（Replay 側のツールチップ追記のみ）
- Trades の細粒度補間（1 Tick あたり 1 バー分の trades 一括 drain で済ませる）
- リプレイ中の Layout 切替・Dashboard 切替（別計画）
- Tachibana のマルチ timeframe 対応（Tachibana は D1 固定のまま）

---

## 1. 現状の整理

### 1.1 Tick ハンドラ（[src/main.rs:285-387](../../src/main.rs#L285-L387)）

```text
elapsed_ms 計算
↓
is_d1 = active_dashboard().is_all_d1_klines(main_window_id)
↓
if is_d1:
    next_kline_time = active_dashboard().replay_next_kline_time(current, ...)
    process_d1_tick(pb, &mut d1_virtual_elapsed_ms, elapsed_ms, next_kline_time)
else:
    pb.advance_time(elapsed_ms) で連続前進
    各 trade_buffer から drain_until(current_time)
    end_time 到達で Paused
```

**問題点**:
- D1 と非 D1 で完全に別経路。`is_all_d1_klines()` の判定がペイン構成に依存し、ペイン追加で切替が起きる
- 「混在 D1（M1+D1）」のとき非 D1 経路に落ちる → D1 ペインは 24 時間/本でほぼ停止

### 1.2 ペイン操作のリプレイ中ガード

| 場所 | 現状 | 撤去後 |
|------|------|------|
| [src/screen/dashboard.rs:632-636](../../src/screen/dashboard.rs#L632-L636) | `is_replay` で `on_drag`/`on_resize` を無効化 | **維持**（ペイン位置移動だけ無効化） |
| `Dashboard::view()` の `is_replay` 引数 | `pane_grid` 全般を制御 | **維持**（Drop/Resize ガードのみ） |
| `pane::State::view()` の `is_replay` | Heatmap 等で「Depth unavailable」を表示 | **維持** |
| 新規追加ペインの content 構築 | リプレイ中はバッファ無効 | **要修正**: `replay_kline_buffer` を有効化して fetch 経路へ流す |

つまり「ガード」を全撤廃するのではなく、**Drop/Resize と Heatmap オーバーレイは維持し、ストリーム構成変更だけを許容する**。

### 1.3 Play ハンドラのバックフィル経路（[src/main.rs:786-915](../../src/main.rs#L786-L915)）

- `prepare_replay()` で全ペインの content をクリア（`enable_replay_mode()` を込みで）
- 各 kline ターゲットに対して `kline_fetch_task()` を発行（`fetch_start = start_ms - 450 * tf`）
- Binance trades は `fetch_trades_batched()` でストリーム取得 → `TradesBatchReceived` で `TradeBuffer` に追記
- kline 全完了後に `DataLoaded` メッセージで `pb.status = pb.resume_status` に遷移

**この経路を「リプレイ中の単一ペイン追加」にも再利用できるよう汎用化する**のが鍵。

### 1.4 既存テスト

- `all_kline_streams_are_d1` 純粋関数: 6 ケース（[dashboard.rs:1453-1488](../../src/screen/dashboard.rs#L1453-L1488)）
- `advance_d1` / `process_d1_tick`: 8 ケース（[replay.rs テスト群]）
- `ReplayKlineBuffer::next_time_after` / `prev_time_before`: 数ケース（[chart/kline.rs](../../src/chart/kline.rs)）

統合後はこれらを **`advance_min_bar` ベースのテスト**に置き換える。

---

## 2. 設計方針

### 2.1 統一ステップアルゴリズム

**コア概念**: 「次に発火するバー時刻」 = 全 kline buffer の `next_time_after(current_time)` の **min**（`replay_buffer_ready()` 済みの chart のみ）。

```rust
// 擬似コード
enum FireStatus {
    Ready(u64),   // 次バー時刻が決まっている（ready chart の next の min）
    Pending,      // ready chart が無いが backfill 中の chart がある（待機）
    Terminal,     // 全 ready 終端かつ backfill 中 chart も無い（Paused 遷移）
}

fn fire_status(dashboard: &Dashboard, current_time: u64) -> FireStatus {
    let mut min_time: Option<u64> = None;
    let mut has_pending = false;
    for c in dashboard.iter_all_kline_charts() {
        if !c.replay_buffer_ready() {
            has_pending = true;
            continue;
        }
        if let Some(t) = c.replay_next_kline_time(current_time) {
            min_time = Some(min_time.map_or(t, |m| m.min(t)));
        }
    }
    match (min_time, has_pending) {
        (Some(t), _)    => FireStatus::Ready(t),
        (None, true)    => FireStatus::Pending,
        (None, false)   => FireStatus::Terminal,
    }
}
```

毎 Tick の挙動（`pb.status == Playing` 前提）:

1. **Trades drain は常に毎 Tick 実行**（後述の穴 A 対応）: 全 trade_buffer に対し `drain_until(pb.current_time)` → `ingest_trades`。
   ただし `replay_kline_buffer_ready()` でない新規 pane の stream は除外（§2.3.1 穴 B 対応）。
2. `status = fire_status(dashboard, pb.current_time)` を計算。
3. `match status`:
   - `FireStatus::Terminal` → `pb.status = Paused`、return（`virtual_elapsed_ms` はリセットしてもしなくてもよいが、意図明確化のためゼロクリア）
   - `FireStatus::Pending` → **何もしない**で return。**`virtual_elapsed_ms` は据え置き（加算しない）**。累積を残したままペイン追加後にバックフィル完了するとジャンプが起きるため
   - `FireStatus::Ready(t)` → 4. へ
4. `Ready(t)` 経路:
   - `delta = t - pb.current_time`
   - `threshold = if delta >= COARSE_CUTOFF_MS { COARSE_BAR_MS } else { delta }` （§2.1.1 案 C, `>=` 決め打ち）
   - `pb.virtual_elapsed_ms += elapsed_ms * pb.speed`
   - `pb.virtual_elapsed_ms < threshold` → 未達で return（drain は 1. で既に実施済み）
   - `pb.virtual_elapsed_ms >= threshold` → `pb.current_time = t`、`pb.virtual_elapsed_ms -= threshold`、全 kline_chart で `replay_advance(pb.current_time)`。**この経路を通った Tick では drain を再度呼ぶ**（ジャンプ後の新しい current_time でもう一度 `drain_until`、新バー範囲の trades を取りこぼさない）

**穴 A の補足**: M1 単独の既存経路は毎 Tick `advance_time + drain_until` で trades が連続的に流れる。統合後も 1. で毎 Tick drain を回すことで M1 の UX を維持する。ジャンプ発生 Tick のみ「drain → advance → drain」の 2 段階になるが、`drain_until` は冪等（cursor ベース）なので二重実行しても問題ない。

**「終端」と「バックフィル待機」の区別**は §2.2 参照。単なる `Option<u64>` では不十分で、`FireStatus` enum で 3 状態を明示する。

### 2.1.1 `speed` セマンティクス（案 C = 実時間連動 + 粗 tf 補正）

**ひとことで言うと**: 案 B（実時間連動）が基本。`delta_to_next >= COARSE_CUTOFF_MS` のときだけ threshold を `COARSE_BAR_MS` に差し替える（= 粗 tf は定量ペース）。

過去の検討経緯（案 A: 常に bars/sec 固定 / 案 B: 常に実時間連動 / 案 C: その混合）は省略。**案 C 採用の理由**は M1 単独では既存の 1x = 実時間連動を維持しつつ、H1 以上では 1 bar/sec × speed で Phase 3 の D1 スロットリング互換を保てるため。

```rust
// 擬似コード（§2.1 case 4 を再掲）
const COARSE_CUTOFF_MS: u64 = 3_600_000;  // H1 以上（>= で比較）は粗 tf 扱い
const COARSE_BAR_MS: u64 = 1_000;         // 粗 tf は speed=1 で 1 バー/sec

let delta_to_next = next_fire.saturating_sub(pb.current_time);
let threshold = if delta_to_next >= COARSE_CUTOFF_MS {
    COARSE_BAR_MS   // H1 / H4 / D1 / W1: 1 バー = 実 1 秒 × speed
} else {
    delta_to_next   // M1 / M5 / M15 / M30: 実時間連動
};

pb.virtual_elapsed_ms += elapsed_ms * pb.speed;
if pb.virtual_elapsed_ms >= threshold as f64 {
    pb.current_time = next_fire;
    pb.virtual_elapsed_ms -= threshold as f64;
}
```

**検算**（speed=1.0）:

| tf | delta_to_next | threshold | 1 バーに要する実時間 | 期待 |
|---|---|---|---|---|
| M1 | 60_000 | 60_000 | 60 sec | 実時間連動 ✅ |
| M5 | 300_000 | 300_000 | 5 min | 実時間連動 ✅ |
| M30 | 1_800_000 | 1_800_000 | 30 min | 実時間連動 ✅ |
| **H1** | 3_600_000 | **1_000** | **1 sec** | **粗補正 ✅（`>=` で境界に入れる）** |
| H4 | 14_400_000 | 1_000 | 1 sec | 粗補正 ✅ |
| D1 | 86_400_000 | 1_000 | 1 sec | Phase 3 互換 ✅ |
| 混在 M1+D1 | 60_000 (=min) | 60_000 | M1: 60 sec, D1 は越境時に `replay_advance` で追従 | ✅ |

**境界判定を `>=` に決め打ちした根拠**: `>`（H1 を実時間側）にすると H1 Replay で 1 バー = 60 分待ちとなり実用不可。初期値は安全側の `>=` で H1 も粗補正に入れる。実機検証で「H1 は実時間で見たい」要望が出たら §8 に差し戻す。

**混在 M1+D1 の具体的挙動**: next_fire は常に次の M1 境界（M1 は D1 の約数なので D1 境界と重なる）。`delta_to_next ≤ 60_000` なので threshold は常に 60_000 → M1 の実時間ペースで current_time が進む。D1 は current_time が D1 境界を越えた Tick で `replay_advance_klines` が 1 本投入（次 Tick 以降で自然追従）。

### 2.2 `FireStatus` の 3 状態

`next_time_after()` が `None` を返すケースは 2 種類：
- **(A) buffer 末尾**: 全データ投入済み = 終端
- **(B) buffer 未到着**: バックフィル中の新規ペインで klines がまだ空

(A) と (B) を区別するため、`KlineChart` に **「リプレイ用 buffer に少なくとも 1 件入った状態か」** を判定するヘルパーを追加する。

```rust
impl KlineChart {
    /// リプレイ buffer が初期化済みかつ klines が 1 件以上あるか
    pub fn replay_buffer_ready(&self) -> bool {
        self.replay_kline_buffer
            .as_ref()
            .map(|b| !b.klines.is_empty())
            .unwrap_or(false)
    }
}
```

`fire_status()` は §2.1 の `FireStatus` enum を返す。Tick ハンドラは match で 3 分岐する:

| バリアント | 条件 | 遷移 | 用途 |
|:--|:--|:--|:--|
| `Ready(t)` | ready chart の min が `Some(t)` | `t` にジャンプ判定 | 通常再生。バックフィル中 chart は放置 |
| `Pending` | ready chart 全て終端 or ready 無し、かつ unready chart が 1 つ以上 | 何もしない（待機） | **初回起動 or mid-replay 追加直後**のバックフィル待ち |
| `Terminal` | ready chart 全て終端、かつ unready chart も無い | `Paused` に遷移 | 真の終端 |

これにより「初回起動時のバックフィル待ち」と「mid-replay 追加ペインのバックフィル待ち」を同じロジックでカバーする。

**注意**: `Pending` と `Terminal` を区別せずに `Paused` に落とすと、新規ペイン追加時に全体が Pause する。`FireStatus` enum で明示的に分けることで取りこぼしを防ぐ。

### 2.3 リプレイ中ペイン追加経路

**ユーザ操作の流れ**:
1. リプレイ中に Split Pane → Starter pane が生成される
2. ユーザが ticker と timeframe を選択 → `pane::State::set_content_and_streams()` が呼ばれる
3. `Dashboard` が `ResolveStreams` メッセージを発行 → `refresh_streams()` で `streams` 集合更新

**追加すべき分岐**:
- `set_content_and_streams()` の **後**で `self.replay.is_replay() && self.replay.playback.is_some()` ならバックフィルを発火
- バックフィル発火関数 `replay_backfill_pane(pane_id, kline_streams, trade_streams)`:
  - 対象ペインに対して `enable_replay_mode()` を呼ぶ（`rebuild_content_for_replay()` 相当）
  - kline ストリームに対して `kline_fetch_task(... fetch_start, end_ms ...)` を発行
  - trade ストリーム（Binance のみ）に対して `fetch_trades_batched()` を発火 → 既存の `TradesBatchReceived` 経路で `TradeBuffer` に追記

**重要な詳細**:
- `fetch_start = start_time.saturating_sub(450 * tf.to_milliseconds())` は既存と同じ
- `fetch_end = pb.end_time` は既存と同じ（リプレイ範囲全体を取得）
- バックフィル中は `replay_buffer_ready() == false` → min 計算から除外 → 既存ペインの自動再生は止まらない
- 完了後に `replay_advance(pb.current_time)` を 1 回呼べば、当該 chart は他ペインに追いつく

#### 2.3.1 新規ペインの TradeBuffer 流入タイミング（明確化）

**問題意識**: 新規ペインに対して `fetch_trades_batched()` を発火すると、リプレイ範囲全体 (`start_time → end_time`) の trades が `TradesBatchReceived` で流れ込む。このとき `pb.current_time` は既に再生の途中にあり、**過去分の trades をどう扱うか** を決めておかないと、既存ペインへの二重流入や UI 上の時刻逆走が起きる。

**既存仕様の確認**（調査結果、2026-04-12 時点）:
- [main.rs:1060-1070](../../src/main.rs#L1060) `TradesBatchReceived` ハンドラ: `pb.trade_buffers.entry(stream).or_insert_with(...).trades.extend(batch)`。新規 stream なら新しい `TradeBuffer { cursor: 0, trades: [] }` を作って append する。**cursor 操作は一切行わない**
- [main.rs:1040 周辺](../../src/main.rs#L1040) StepBackward 経路: 「`TradeBuffer` のカーソルをリセットし、new_time まで早送り」コメントあり → **cursor を指定時刻まで進めるロジックが既に存在**（流用可能）
- `TradeBuffer` は `(exchange, ticker)` キー。**同じ ticker を既存ペインが既に参照している場合、新規ペインでも stream キーは重複**するため `entry` で既存 buffer に merge される（cursor は既存の進捗を維持）

**決定する設計**:

| ケース | 流入経路 | cursor 操作 |
|------|--------|----------|
| **A: 新規 stream（ticker が既存ペインに無い）** | TradesBatchReceived で新規 TradeBuffer 作成 → trades 全量 append | `TradesFetchCompleted(stream)` 受信時に **`cursor = current_time` まで早送り** し、戻り値の過去 trades は **破棄**（UI には流さない）。その後は通常の Tick 経路で `drain_until` される |
| **B: 既存 stream（ticker が既存ペインにあり、timeframe だけ違う）** | TradesBatchReceived で既存 TradeBuffer に append。ただし時刻は重複する可能性あり | **append しない** が妥当。§2.5 timeframe 変更では trades は再利用と記載済み → バックフィル発火関数内で **「ticker が既存 stream 集合にあるか」判定し、あれば trade fetch をスキップ** |
| **C: 既存 TradeBuffer が既に `cursor > 0`（途中まで drain 済み）なのに過去 trades が届く** | 案 A の早送り操作で吸収 | `cursor` は `max(current_cursor, new_current_time_index)` で単調増加を保証 |

**実装の具体形**:

```rust
// replay.rs に追加
impl TradeBuffer {
    /// cursor を指定時刻まで進める。戻り値は早送りで読み飛ばした trades の件数（捨ててよい）。
    /// 既に cursor が指定時刻を超えていれば no-op（単調増加を保証）。
    pub fn advance_cursor_to(&mut self, target_time: u64) -> usize {
        let new_cursor = self
            .trades
            .iter()
            .position(|t| t.time > target_time)
            .unwrap_or(self.trades.len());
        let new_cursor = new_cursor.max(self.cursor);
        let skipped = new_cursor - self.cursor;
        self.cursor = new_cursor;
        skipped
    }
}

// main.rs TradesFetchCompleted ハンドラ (もしくは DataLoaded の個別版) に追加
ReplayMessage::TradesFetchCompleted(stream) => {
    if let Some(pb) = &mut self.replay.playback {
        if let Some(buffer) = pb.trade_buffers.get_mut(&stream) {
            let _skipped = buffer.advance_cursor_to(pb.current_time);
            // 早送りぶんは UI に流さない（既存ペインは既にその時刻の表示を持つ / 新規ペインはまだ描画していない）
        }
    }
}
```

**タイミング保証**:
1. `TradesBatchReceived` は **cursor を動かさない**（既存挙動維持）
2. `TradesFetchCompleted(stream)` 受信時に初めて `advance_cursor_to(current_time)` を呼ぶ
3. ケース B（既存 stream）はバックフィル発火時点で **trade fetch 自体をスキップ**するため `TradesFetchCompleted` は来ない → 既存 buffer は無傷
4. ケース C（部分 drain 済み + 過去 trades 到着）は `max()` で吸収。既存ペインの cursor 進行と干渉しない

**既存 Play ハンドラとの整合**: Play 初回でも `TradesFetchCompleted` は発火するが、その時点で `current_time == start_time` なので `advance_cursor_to(start_time)` は実質 no-op（`cursor = 0` 付近）。既存挙動は変わらない。

#### 2.3.1.1 穴 B: バックフィル中 stream を毎 Tick drain 対象から除外

**問題**: §2.1 case 1 で「毎 Tick drain_until を呼ぶ」設計にしたため、ケース A（新規 ticker）のバックフィル進行中に `TradesBatchReceived` で append された過去 trades を `drain_until(current_time)` が拾ってしまう。`TradesFetchCompleted` が来る前の時間窓で `ingest_trades` に流れ込み、バックフィル中の新規 pane or 既存 chart に過去 trades が紛れ込むリスクがある。

**設計**: `PlaybackState` に **`pending_trade_streams: HashSet<StreamKind>`** を追加し、バックフィル発火時に挿入、`TradesFetchCompleted` 受信時の `advance_cursor_to(current_time)` 直後に削除する。§2.1 case 1 の drain ループはこの集合に含まれる stream をスキップする:

```rust
// 擬似コード
for (stream, buffer) in pb.trade_buffers.iter_mut() {
    if pb.pending_trade_streams.contains(stream) { continue; }  // バックフィル中はスキップ
    let drained = buffer.drain_until(pb.current_time);
    // ... 既存の ingest_trades 経路 ...
}
```

**フロー**:
1. §2.3 `replay_backfill_pane()` で `fetch_trades_batched()` 発火と同時に `pb.pending_trade_streams.insert(stream)`
2. `TradesBatchReceived` は従来通り `trade_buffers[stream].trades.extend(batch)` のみ。`pending_trade_streams` は触らない
3. 完了通知 `TradesFetchCompleted(stream)` のハンドラで:
   - `buffer.advance_cursor_to(pb.current_time)` を呼ぶ
   - `pb.pending_trade_streams.remove(&stream)`
4. 次の Tick から通常の drain_until 対象に復帰

**ケース B（既存 stream 重複）との関係**: §2.3.1 の設計通り、重複時は trade fetch 自体を発火しないので `pending_trade_streams` にも追加しない → 既存 stream は一度も drain 除外されず無傷。

**ケース C（部分 drain + 過去 trades 到着）との関係**: このケースは「既存 stream に対して新しくバックフィルが走る」状況に相当するが、§2.3.1 ケース B でそもそも trade fetch をスキップする方針なので `pending_trade_streams` に入らない。従来の `advance_cursor_to` の `max()` ガードで十分。

### 2.4 リプレイ中ペイン削除経路

- `ClosePane` 相当のメッセージ受信時、リプレイ中であっても従来通り削除する
- 削除したペインの `KlineChart` ごと `ReplayKlineBuffer` も解放される（自動）
- 当該ペインの trade ストリームを **他のペインがまだ参照していない場合のみ** `pb.trade_buffers` から削除する
  - 参照判定: `dashboard.collect_trade_streams()` を再走査して **存在しない StreamKind** を `pb.trade_buffers` から remove
- これにより不要なメモリが解放される

### 2.5 リプレイ中 timeframe / ticker 変更経路

#### timeframe 変更 (例: M1 → M5)

`pane::State` で `set_basis(new_basis)` が呼ばれた直後の挙動：

- 既存 `replay_kline_buffer` の klines は **古い timeframe のもの**で再利用不可 → `replay_kline_buffer = Some(empty)` に再初期化
- 同 stream の **新 timeframe** に対して 2.3 と同じバックフィル経路を発火
- バックフィル中は `replay_buffer_ready() == false` で min 計算除外
- trades は (exchange, ticker) キーで stream 共有のため **そのまま再利用**（再フェッチ不要）

#### ticker 変更 (例: BTCUSDT → ETHUSDT)

- klines: 新 stream に対して `replay_kline_buffer = Some(empty)` で再初期化 + バックフィル発火（timeframe 変更と同じ）
- trades: **旧 ticker の TradeBuffer 解放が必要**。`set_content_and_streams()` の処理直後に §2.4 と同じ「全 dashboard を再走査して参照されない StreamKind を `pb.trade_buffers` から remove」を呼ぶ
- 新 ticker の trade stream を `pb.trade_buffers` に追加し、`fetch_trades_batched()` を発火

**変更 = remove + add の最小ケース**として、2.3 のバックフィル関数 + §2.4 のクリーンアップを組み合わせて再利用する。

### 2.6 バックフィル発火点の一元化

**事前 grep 調査結果（2026-04-12 実施）**。リプレイ中の stream 構成変更は以下の入口を経由する（全て `src/screen/dashboard.rs` / `src/screen/dashboard/pane.rs` / `src/main.rs`）:

#### 2.6.1 `refresh_streams()` 直接呼び出し（6 箇所）

| # | 行 | 文脈 | sync 必要か |
|:-:|---|------|:-:|
| 1 | [dashboard.rs:174](../../src/screen/dashboard.rs#L174) | Layout 初期化後（ポップアウトウィンドウ開き直し） | △ リプレイ中に layout 初期化は来ない想定 |
| 2 | [dashboard.rs:246](../../src/screen/dashboard.rs#L246) | 下流イベントのハンドラ内 | ○ |
| 3 | [dashboard.rs:980](../../src/screen/dashboard.rs#L980) | Ticker 関連の変更処理 | ○ |
| 4 | [dashboard.rs:1023](../../src/screen/dashboard.rs#L1023) | 同系統 | ○ |
| 5 | [dashboard.rs:1082](../../src/screen/dashboard.rs#L1082) | 同系統 | ○ |
| 6 | [dashboard.rs:1231](../../src/screen/dashboard.rs#L1231) | `close_pane` 系統の末尾 | ○ |

→ **refresh_streams 本体（[dashboard.rs:1369](../../src/screen/dashboard.rs#L1369)）の末尾に sync 呼び出しを 1 箇所追加**すれば 6 箇所全てカバーできる。個別の呼び出し元を編集する必要なし。

#### 2.6.2 `pane::Effect` 処理（Effect::RefreshStreams / Effect::RequestFetch）

`set_basis()` の全経路（[pane.rs:1363, 1374, 1398, 1435, 1474, 1507, 1526](../../src/screen/dashboard/pane.rs#L1363)）は **必ず `Effect::RefreshStreams` または `Effect::RequestFetch` を返す**設計になっている。これらの Effect は [dashboard.rs:369-393](../../src/screen/dashboard.rs#L369-L393) の 1 箇所に集約して処理されている。

→ `Effect::RefreshStreams` は `refresh_streams()` を呼ぶので 2.6.1 でカバー済み。`Effect::RequestFetch` 処理の末尾にも sync 呼び出しを追加する（timeframe 変更時の kline buffer 再初期化が必要なため）。

#### 2.6.3 `set_content_and_streams()` 直接呼び出し（4 箇所）

| # | 行 | 文脈 |
|:-:|---|------|
| 1 | [dashboard.rs:339](../../src/screen/dashboard.rs#L339) | `Effect` 処理経路内（Heatmap/Kline 切替） |
| 2 | [dashboard.rs:724](../../src/screen/dashboard.rs#L724) | SplitPane 後の初期化 |
| 3 | [dashboard.rs:759](../../src/screen/dashboard.rs#L759) | SplitPane 後の初期化（別経路） |
| 4 | [dashboard.rs:1193, 1196](../../src/screen/dashboard.rs#L1193) | ResolveStreams ハンドラ内（Bunch ticker 処理） |

→ `#1` は同じ Effect 処理内なので 2.6.2 でカバー。`#2-#4` は後続で `refresh_streams()` を呼ぶか、または `Message::ResolveStreams` を経由するので 2.6.1 / 2.6.5 でカバー。

#### 2.6.4 `Message::ResolveStreams` ハンドラ

[main.rs:481](../../src/main.rs#L481) の `Event::ResolveStreams` 経路は [dashboard.rs:1185](../../src/screen/dashboard.rs#L1185) の `pane::Action::ResolveStreams` が起点。最終的に `set_content_and_streams()` を呼び、続けて `refresh_streams()` を呼ぶ（[dashboard.rs:1231](../../src/screen/dashboard.rs#L1231)）→ 2.6.1 の #6 でカバー。

#### 2.6.5 集約結果

**最終的に sync_replay_buffers() の呼び出し点は 2 箇所**:

1. **`refresh_streams()` 本体の末尾** ([dashboard.rs:1369](../../src/screen/dashboard.rs#L1369) 内) → §2.6.1 の 6 入口 + §2.6.3 の後続経路 + §2.6.4 をカバー
2. **`Effect::RequestFetch` 処理の末尾** ([dashboard.rs:370-392](../../src/screen/dashboard.rs#L370-L392) 内) → §2.6.2 の timeframe 変更直接経路をカバー

これにより、個別の Message ハンドラや set_content_and_streams 呼び出し箇所に分散させる必要がない。冪等性が担保されていれば、`refresh_streams()` 内と `Effect::RequestFetch` 処理内で二重に呼ばれても問題ない（後者 → 前者の順で続けて呼ばれる場合がある）。

#### 2.6.6 旧記述（参考）

以前の表形式記述を参考に残す:

| 入口 | 経路 | 既存挙動 |
|------|------|----------|
| 新規 SplitPane → ticker 確定 | `pane::Action::ResolveStreams` → `Message::ResolveStreams` → `refresh_streams()` | streams 集合更新 |
| `set_basis(Kline)` (timeframe 変更) | `pane::Effect::RequestFetch` を返却 → `dashboard.rs:370` で fetch 発火 | 直接フェッチ。ResolveStreams は経由しない |
| `set_basis(Heatmap)` 系 | `pane::Effect::RefreshStreams` → `refresh_streams()` | streams 再計算 |
| `set_content_and_streams()` | 内容により上記いずれか | - |

```rust
// dashboard.rs 内のヘルパー
impl Dashboard {
    /// リプレイ中なら、現在の dashboard の stream 集合と pb.trade_buffers / replay_kline_buffer を
    /// 同期させる。新規 stream は backfill を発火、消えた stream は trade_buffers から remove。
    /// 冪等であることが重要。
    pub fn sync_replay_buffers(&mut self, pb: &mut PlaybackState, ...) -> Vec<Task<Message>> { ... }
}
```

**冪等性が要点**: 同じ stream 集合に対して 2 回呼ばれても二重フェッチしない。実装は `pb.trade_buffers` のキー集合と `replay_kline_buffer.is_some() && klines.is_empty()` フラグで「すでにバックフィル中か」を判定する。

これにより §2.6.5 の 2 箇所集約で、複数の入口を漏れなく、かつ二重発火させずにカバーできる。

---

## 3. データ構造の変更

### 3.1 `ReplayState` / `PlaybackState`

既存の `d1_virtual_elapsed_ms` フィールドを **Phase 1 内で `virtual_elapsed_ms` にリネーム**する。案 C では全 timeframe 共通の意味になるため、古い名前を残すと読み手を誤誘導する（レビュー指摘）。

```rust
pub struct ReplayState {
    pub mode: ReplayMode,
    pub range_input: ReplayRangeInput,
    pub playback: Option<PlaybackState>,
    pub last_tick: Option<std::time::Instant>,
    /// 次バー発火までの仮想時間累積（ms）。
    /// `elapsed_ms * pb.speed` を毎 Tick 加算し、§2.1.1 案 C の `comparison_threshold` に到達したら
    /// `pb.current_time` を進めて、しきい値ぶん減算する。
    pub virtual_elapsed_ms: f64,
}
```

**リネーム範囲**: `src/replay.rs` の struct 定義 + `src/main.rs` の Tick / Play / StepForward / StepBackward ハンドラ内の参照（grep で数ヶ所）。serialize 経路にはないため互換性懸念なし。

### 3.2 削除されるもの

- `Dashboard::is_all_d1_klines()`
- `all_kline_streams_are_d1()` 純粋関数 + テスト 6 件
- `replay::advance_d1()` / `replay::process_d1_tick()` の D1 限定ロジック（関数名は `process_tick` に改める）+ テスト 8 件
- `Dashboard::replay_next_kline_time()` のシグネチャを `fire_status()` に変更し `FireStatus` enum を返す
- `Dashboard::replay_prev_kline_time()` を「全 ready buffer の prev の max」のセマンティクスへ（Phase 1b）

### 3.3 追加されるもの

- `KlineChart::replay_buffer_ready() -> bool`
- `replay::FireStatus { Ready(u64), Pending, Terminal }` enum
- `Dashboard::fire_status(current, main_window) -> FireStatus`
- `Dashboard::prev_replay_fire_time(current, main_window) -> Option<u64>`（Phase 1b で追加）
- `replay::process_tick(pb, elapsed_ms, status) -> ...` 既存 `process_d1_tick` を案 C のしきい値補正付きで一般化。内部で毎 Tick drain を実行（§2.1 case 1）
- `const COARSE_CUTOFF_MS: u64 = 3_600_000;`（`replay.rs`。`delta >= これ` で粗補正モード）
- `const COARSE_BAR_MS: u64 = 1_000;`（`replay.rs`。粗補正モードでの比較しきい値 = 1 バー/sec × speed）
- `PlaybackState::pending_trade_streams: HashSet<StreamKind>`（§2.3.1.1 穴 B。バックフィル中の trade stream を毎 Tick drain から除外するフラグ集合）
- `TradeBuffer::advance_cursor_to(target_time) -> usize`（§2.3.1 実装例。StepBackward ロジックを抽出再利用）
- `Dashboard::sync_replay_buffers(pb, ...)` 冪等な stream 同期ヘルパー（Phase 3）
- `Dashboard::build_kline_backfill_task(...)` / `build_trades_backfill_task(...)` / `replay_backfill_pane(...)`（Phase 3-0 で抽出）

---

## 4. 実装フェーズ

| Phase | 概要 | 依存 | commit 単位 |
|:-:|------|------|------|
| 0 | ✅ **事前調査**（入口 grep + trade 流入タイミング設計） → §2.3.1 / §2.6 に結果反映済み | なし | コード変更なし |
| 1 | ✅ Tick ハンドラ統一（D1 分岐撤廃 + speed 案 C）**＋ 既存テストの移行**（ビルド通過まで含む） | Phase 0 | 1 commit（Phase 1b 吸収） |
| 1b | ✅ StepForward/Backward の離散ステップ統一 | Phase 1（実装上同 commit） | Phase 1 に吸収 |
| 3 | リプレイ中 SplitPane / 削除 / timeframe / ticker 変更の許容（3-0 バックフィル関数抽出 + 3-1〜 mid-replay 対応） | Phase 1 | 2-3 commit |
| 4 | **新規** テスト追加 + UI 表示調整 | Phase 1-3 | 1 commit |

> **Note (2026-04-12)**: Phase 1 と 1b は別 commit にする予定だったが、`ReplayMessage::StepForward` / `StepBackward` も `is_all_d1_klines()` を参照していたため、Phase 1 で該当関数を削除する時点で Phase 1b の離散化も同 commit で行う必要があった。ロールバック戦略は「Phase 1 全体を 1 commit として revert」に変更。

**旧 Phase 2（バックフィル関数抽出）は Phase 3 サブステップ 3-0 に吸収**。旧計画で独立 Phase になっていたが、抽出関数は Phase 3 でしか使われず、単独マージで dead code になるため統合。commit は 3-0（リファクタのみ）と 3-1 以降（機能追加）で分ける。

### Phase 0: 事前調査（2026-04-12 実施済み）

実装着手前のリスク軽減として以下の調査を行い、結果を §2.3.1 / §2.6 に反映済み。

| 調査項目 | 結果反映先 | 状態 |
|---------|---------|------|
| `refresh_streams()` / `set_content_and_streams()` / `set_basis()` / `Effect::*` 呼び出し入口の grep 網羅 | §2.6.1 〜 §2.6.5 | ✅ 完了。`sync_replay_buffers()` 呼び出し点は **2 箇所に集約**（refresh_streams 末尾 + Effect::RequestFetch 処理末尾） |
| 新規ペイン追加時の TradeBuffer 流入タイミングと既存ペイン干渉の回避設計 | §2.3.1 | ✅ 完了。`TradesFetchCompleted` 受信時に `advance_cursor_to(current_time)` を呼ぶ、既存 stream 共有時は trade fetch 自体をスキップ |
| 既存 `TradeBuffer` に cursor 早送り API が存在するか | §2.3.1 | ✅ 完了。[main.rs:1040](../../src/main.rs#L1040) に StepBackward 用の類似ロジックが既にあり、`TradeBuffer::advance_cursor_to()` として抽出して再利用可能 |

**この調査結果は Phase 1-3 の判断材料。実装中に前提が崩れたら §2.3.1 / §2.6 を差し戻す。**

**Phase 分割の根拠**（レビュー指摘反映）:
- Phase 1 で `is_all_d1_klines()` / `all_kline_streams_are_d1()` / `process_d1_tick()` / `advance_d1()` を削除すると、それらを参照する **既存ユニットテスト 14 件がコンパイル時点で壊れる**。したがって「実装コード変更」と「既存テストの移行/削除」は**同一 commit で行う必要がある**。旧 Phase 4 の「テスト書き換え」は Phase 1 に吸収する
- Phase 4 は純粋な新規追加（新規テスト + UI ツールチップ）のみとし、Phase 1-3 完了後でも独立に追加できるようにする
- Phase 2 はリファクタリングのみで挙動を変えないため、Phase 1 と独立に着手・マージ可能
- Phase 1b (StepForward の離散ステップ化) は UX 変更のため、Phase 1 から **別 commit に分離**して revert しやすくする

### Phase 1: Tick ハンドラ統一（D1 分岐撤廃）＋ 既存テスト移行

**目的**: `is_all_d1_klines()` 分岐を消し、全 timeframe で §2.1.1 案 C のしきい値切替ロジックに統一する。**この Phase の完了条件はビルドと `cargo test` がグリーン**であること。

#### 1-A: 実装コード変更

| Step | 内容 | ファイル |
|:-:|------|---------|
| 1-1 | `KlineChart::replay_buffer_ready()` を追加 | `src/chart/kline.rs` |
| 1-2 | `Dashboard::next_replay_fire_time()` を追加し `NextFire { time, has_pending_backfill }` を返す | `src/screen/dashboard.rs` |
| 1-3 | `replay::process_tick()` 関数を追加。case 分岐は §2.1 案 C のしきい値補正ロジック + §2.2 の 3 分岐。`COARSE_CUTOFF_MS` / `COARSE_BAR_MS` 定数を定義 | `src/replay.rs` |
| 1-4 | `ReplayState::d1_virtual_elapsed_ms` を `virtual_elapsed_ms` にリネーム（initializer 2 箇所 + テスト 14 箇所、計 16 ヶ所） | `src/replay.rs`, `src/main.rs` |
| 1-5 | `Message::Tick` ハンドラの D1 分岐 / `is_d1` ローカル変数 / 非 D1 経路を削除し、`process_tick()` 1 経路に置き換え | `src/main.rs` |
| 1-6 | `Dashboard::is_all_d1_klines()` / `all_kline_streams_are_d1()` 純粋関数を削除 | `src/screen/dashboard.rs` |
| 1-7 | `replay::process_d1_tick()` / `replay::advance_d1()` を削除 | `src/replay.rs` |
| 1-8 | Play ハンドラから「D1 のみなら Paused 開始」削除（Phase 3 で既に削除済みなら確認のみ） | `src/main.rs` |

#### 1-B: 既存テスト移行（同 commit 必須）

| Step | 内容 | ファイル |
|:-:|------|---------|
| 1-9 | `all_kline_streams_are_d1` 純粋関数テスト 6 件を **削除**（元関数が消えるため） | `src/screen/dashboard.rs` (旧 1453-1488) |
| 1-10 | `advance_d1` / `process_d1_tick` テスト 8 件を `process_tick` ベースに書き換え: D1 ラベルを削除し、`delta_to_next` と `comparison_threshold` を引数化。案 C 検算表の M1 / D1 両ケースを含む | `src/replay.rs` |
| 1-11 | 既存テスト内の `d1_virtual_elapsed_ms` 参照 14 箇所を `virtual_elapsed_ms` に置換（1-4 と同 commit） | `src/replay.rs` |

**完了条件**: `cargo build` / `cargo test` / `cargo clippy` がグリーン。既存 E2E テストで以下を実機確認:
- 単独 M1 Replay 自動再生が 1x で実時間相当（旧挙動と変化なし）
- 単独 D1 Replay（Tachibana 既存経路）自動再生が 1x で 1 D1/sec（Phase 3 互換）

**判断ポイント**:
- `process_tick()` の戻り値型は既存 `D1TickResult` を流用してよいが、引数で `next_fire: NextFire` を取るように改める。可能なら **`Dashboard` メソッドに引き上げて `&mut self` 単一借用**で書き直し、戻り値を `Vec<Task<Message>>` に統一すれば中継型自体不要
- 案 C の定数は `replay.rs` 内に 2 つ定義: `const COARSE_CUTOFF_MS: u64 = 3_600_000;` と `const COARSE_BAR_MS: u64 = 1_000;`（§2.1.1 の検算表を参照）
- `next_fire.time == None && !has_pending_backfill` で `pb.status = Paused`
- `next_fire.time == None && has_pending_backfill` で何もしない（バックフィル待ち、Phase 2-3 で本格化）。**このとき `virtual_elapsed_ms` は据え置く**（加算しない）

### Phase 1b: StepForward / StepBackward の離散ステップ統一（別 commit）

| Step | 内容 | ファイル |
|:-:|------|---------|
| 1b-1 | `StepForward` / `StepBackward` の D1 分岐を削除し、常に `next_replay_fire_time()` / `prev_replay_fire_time()` を使う離散ステップに統一 | `src/main.rs` |
| 1b-2 | `Dashboard::prev_replay_fire_time()` を追加（全 ready buffer の prev の max） | `src/screen/dashboard.rs` |

**StepForward の挙動変更（UX 影響あり）**:
- 現行: D1 = 離散ジャンプ、それ以外 = `+60_000ms` 固定
- 統合後: **常に離散ジャンプ**。M1 単独でも M1 buffer から `next_time_after(current)` を引いて次バーへ
- 「Step = 1 バー進める」というセマンティクス統一で、Replay モードのメンタルモデルが綺麗になる
- 副作用: M1 Replay で StepForward を押すと **次バー境界に揃う**（current_time が既にバー境界なら +60s、ずれていればその分短い）。実用上は問題なし
- **Phase 1 とは別 commit にする**ことで、UX 違和感時に Phase 1b だけを revert 可能にする

### Phase 3: リプレイ中ペイン操作の許容（バックフィル関数抽出を含む）

**目的**: (a) Play ハンドラ内のフェッチ発火ロジックを抽出し、(b) SplitPane / ClosePane / set_basis / set_content_and_streams をリプレイ中でも動作させる。§2.6.5 の 2 箇所集約で `sync_replay_buffers()` ヘルパーを呼ぶ。新規ペインの TradeBuffer 流入タイミングは §2.3.1 / §2.3.1.1 の設計に従う。

**commit 分割方針**:
- **3-0 commit**: 純リファクタ（Play ハンドラ挙動不変）
- **3-1〜 commit**: mid-replay 許容本体（機能追加）

| Step | 内容 | ファイル |
|:-:|------|---------|
| **3-0-a** | Play ハンドラの kline_tasks 構築ロジックを `Dashboard::build_kline_backfill_task(pane_id, stream, start_ms, end_ms, layout_id) -> Task<Message>` に抽出 | `src/screen/dashboard.rs` or `src/main.rs` |
| **3-0-b** | 同様に trades フェッチ発火を `build_trades_backfill_task(...)` に抽出 | 同上 |
| **3-0-c** | Play ハンドラを `for stream in kline_targets { tasks.push(build_kline_backfill_task(...)) }` に書き換え（挙動維持を確認） | `src/main.rs` |
| **3-0-d** | `Dashboard::replay_backfill_pane(pane_id, kline_targets, trade_targets, layout_id, start_ms, end_ms) -> Task<Message>` を新設し、上記 2 関数を呼ぶラッパとする。**trade 発火時は `pb.pending_trade_streams.insert(stream)` も同時実行** | `src/screen/dashboard.rs` |
| 3-1 | `Dashboard::view()` の `is_replay` ガードを **`on_drag` / `on_resize` のみ**に絞る（既に該当する。確認のみ） | `src/screen/dashboard.rs` |
| 3-2 | `TradeBuffer::advance_cursor_to(target_time) -> usize` を追加（§2.3.1 の実装例参照、StepBackward のロジックを抽出） | `src/replay.rs` |
| 3-3 | `Dashboard::sync_replay_buffers()` ヘルパーを実装。冪等性を持ち、stream 集合差分を計算して新規 stream には backfill 発火、消えた stream は trade_buffers + pending_trade_streams から remove する。**既存 stream との重複判定**も行い、重複時は trade fetch をスキップ（§2.3.1 ケース B） | `src/screen/dashboard.rs` |
| 3-4 | **`refresh_streams()` 本体の末尾**に `sync_replay_buffers()` 呼び出しを追加（§2.6.5 集約点 #1） | `src/screen/dashboard.rs:1369` |
| 3-5 | **`Effect::RequestFetch` 処理の末尾**に `sync_replay_buffers()` 呼び出しを追加（§2.6.5 集約点 #2） | `src/screen/dashboard.rs:370` |
| 3-6 | `ReplayMessage::TradesFetchCompleted(stream)` ハンドラに `advance_cursor_to(pb.current_time)` + `pb.pending_trade_streams.remove(&stream)` を追加（§2.3.1 + §2.3.1.1） | `src/main.rs:1071` |
| 3-7 | `close_pane()` 系統は `refresh_streams()` を末尾で呼ぶので 3-4 で自動カバー。明示呼び出しは不要（確認のみ） | `src/screen/dashboard.rs:1231` |
| 3-8 | バックフィル発火後の状態管理: 新規ペインに対して `pane::Status::Loading` を立て、フェッチ完了で `Ready` に戻す（既存 `ChangePaneStatus` 経路を流用） | `src/main.rs` |

**判断ポイント（3-0 部分）**:
- `Task<Message>` の構築には `layout_id` と `start_ms`/`end_ms` が必要 → `Dashboard` メソッドに引数で渡す（state を持たせない）
- バックフィル発火は **`enable_replay_mode()` を必ず先に呼ぶ**こと。これを忘れると新規 chart の `insert_hist_klines` がバッファ経由ではなく直接挿入されてしまう
- trades fetch は Binance 限定のフィルタを `replay_backfill_pane()` 内に閉じ込める

**判断ポイント**:
- 「mid-replay 追加で発火するフェッチ」は既存 `Message::DistributeFetchedData` 経路に流れ、`insert_hist_klines` が呼ばれる。この時点で当該 chart の `replay_kline_buffer` が `Some(empty)` であれば自動的にバッファに追記される（既存仕様）
- バックフィル中の chart は `replay_buffer_ready() == false` なので min 計算から除外され、他ペインの自動再生を止めない
- バックフィル完了後に `replay_advance(pb.current_time)` を 1 回呼ぶ必要があるが、これは次の Tick で自動的に実行される（`replay_advance_klines` が全ペインに対して呼ばれるため）
- timeframe 変更時の **既存 buffer クリア** を忘れると古い timeframe の klines が混在する → 必ず `replay_kline_buffer = Some(empty)` で再初期化
- 既存 stream と ticker が重なる場合は trade fetch をスキップし、`advance_cursor_to` も呼ばない（既存ペインの cursor 進行を壊さないため）

**競合状態の扱い**:
- バックフィル中に Play / Pause / 速度変更が来ても影響なし（pb.status だけ書き換わる）
- バックフィル中に Step Forward/Backward が来た場合 → 当該 chart は `ready=false` で除外、他 chart の next/prev を使ってジャンプ
- バックフィル中に **再度 timeframe 変更** が来た場合 → 古いフェッチタスクの結果が来ても、`req_id` 不一致で `insert_hist_klines` 内の `log::warn!` が出て無視される（既存仕様）

### Phase 4: 新規テスト追加 + UI 表示調整

既存テストの移行は Phase 1 で済んでいる前提。本 Phase は純粋な新規追加のみで、Phase 1-3 完了後に独立マージ可能。

| Step | 内容 | ファイル |
|:-:|------|---------|
| 4-1 | 新規ユニット: `next_replay_fire_time_returns_min_across_buffers` / `skips_unready_buffers` / `returns_none_when_all_terminal`（§6.1 #1-#3） | `src/screen/dashboard.rs` |
| 4-2 | 新規ユニット: `replay_buffer_ready_false_when_empty` / `true_after_insert`（§6.1 #10-#11） | `src/chart/kline.rs` |
| 4-3 | 新規ユニット: `process_tick_waits_when_pending_backfill` / `pauses_on_terminal_only_when_no_pending`（§6.1 #6-#7） | `src/replay.rs` |
| 4-4 | 新規ユニット: `sync_replay_buffers_idempotent` / `removes_orphan_trade_streams`（§6.1 #12-#13、Phase 3 依存） | `src/screen/dashboard.rs` |
| 4-5 | E2E: 「リプレイ中に SplitPane → ticker 選択 → kline が current_time に追いつく」シナリオ（§6.2 #5） | `tests/e2e_*.rs` |
| 4-6 | E2E: 「M1 Replay 中に StepForward を押すとバー境界に揃う」シナリオ（§6.2 #4、Phase 1b 依存） | 同上 |
| 4-7 | UI: Replay モードヘッダの speed 表示横にツールチップで「案 C: H1 以下は実時間連動 / H4 以上は 1 バー/sec × speed」を追記 | `src/main.rs` |

---

## 5. 影響範囲とリスク

| 領域 | 影響 | リスク | 緩和策 |
|------|------|------|--------|
| Tick ハンドラ | 完全書き換え | 高（リプレイ全体の挙動変化） | 既存 E2E テストで回帰検出。Phase 1 単体で commit して動作確認 |
| **Trades drain 頻度（穴 A）** | 統合後も M1 単独で毎 Tick drain を維持する設計 | **中** | §2.1 case 1 で「drain は常に毎 Tick、kline ジャンプとは独立」を明示。M1 単独 E2E で trades の連続流入を回帰検証 |
| **バックフィル中 stream の drain 除外（穴 B）** | `pending_trade_streams` セットで drain 対象外にしないと過去 trades が既存 pane に流入 | **中** | §2.3.1.1 の設計で発火時 insert / 完了時 remove。ユニット 1 件で冪等性を検証 |
| `speed` セマンティクス（案 C） | しきい値補正による暗黙の倍速（H1 以上） | 中 | M1〜M30 は実時間連動を維持。H1 以上は 1 bar/sec × speed である旨をツールチップで明示 |
| StepForward 離散化 (Phase 1b) | 挙動変化（M1 がバー境界に揃う） | 中 | Phase 1 から別 commit に分離し、revert 容易にする |
| `sync_replay_buffers()` の冪等性 | 入口が複数 (ResolveStreams / RequestFetch / RefreshStreams) | 中 | ヘルパー内で「すでにバックフィル中か」判定し二重発火防止。ユニットテストで冪等性を検証 |
| ペイン追加バックフィル | 新規経路 | 中（フェッチ失敗時の UI） | `pane::Status::Loading` で UI 表示。失敗時は既存の `DataLoadFailed` 経路に流す |
| timeframe / ticker 変更時のバッファ再初期化 | 競合状態の可能性 | 中 | `req_id` ベースの stale フェッチ無視は既存仕様で動く |
| trade_buffers の追加・削除 | 状態管理の複雑化 | 低 | 削除は「他ペインから参照されないもの」のみ。追加は既存の `TradesBatchReceived` 経路で吸収 |
| Tachibana D1 単独再生 | 動作不変（speed=1x で 1 D1/sec、案 C のしきい値補正で実現） | 低 | Phase 3 で動作検証済みのケース |
| 混在 M1+D1 | M1 実時間 + D1 は 1440 M1 バーごとに更新 | 受容（仕様） | リリースノートで「混在時は M1 が支配、D1 は M1 進捗に従う」と明記 |

---

## 6. 検証

### 6.1 ユニット

| # | テスト | 期待 |
|:-:|---------|------|
| 1 | `next_replay_fire_time_returns_min_across_buffers` | M1+D1 混在で次の M1 バー時刻を返す |
| 2 | `next_replay_fire_time_skips_unready_buffers` | バックフィル中の chart を min 計算から除外 |
| 3 | `next_replay_fire_time_returns_none_when_all_terminal` | 全 buffer 末尾で None |
| 4 | `process_tick_m1_jumps_after_60s_virtual_elapsed` | delta=60_000ms, speed=1.0, 累積 `virtual_elapsed_ms=60_000` でジャンプ（実時間連動、threshold=delta） |
| 5 | `process_tick_d1_jumps_after_1s_virtual_elapsed` | delta=86_400_000ms, speed=1.0, 累積 `virtual_elapsed_ms=1_000` でジャンプ（粗補正モード、threshold=`COARSE_BAR_MS`）。ジャンプ後の current_time は delta フルぶん進む |
| 5b | `process_tick_h1_stays_real_time` | delta=3_600_000ms, `COARSE_CUTOFF_MS=3_600_000` の境界で threshold=delta（実時間連動側）になることを確認 |
| 5c | `process_tick_h4_uses_coarse_threshold` | delta=14_400_000ms, threshold=`COARSE_BAR_MS` になり speed=1.0 で 1 バー/sec ペース |
| 6 | `process_tick_pauses_on_terminal_only_when_no_pending` | next_fire=None かつ has_pending_backfill=false で Paused |
| 7 | `process_tick_waits_when_pending_backfill` | next_fire=None かつ has_pending_backfill=true で Pause せず待機 |
| 8 | `process_tick_drains_trades_when_buffers_have_trades` | trades あれば drain 実行 |
| 9 | `process_tick_skips_drain_when_all_buffers_empty` | trade_buffers 全空ならスキップ（Tachibana ケース） |
| 10 | `replay_buffer_ready_false_when_empty` | klines が空なら ready=false |
| 11 | `replay_buffer_ready_true_after_insert` | 1 件追加で ready=true |
| 12 | `sync_replay_buffers_idempotent` | 同じ stream 集合で 2 回呼んでも二重 fetch しない |
| 13 | `sync_replay_buffers_removes_orphan_trade_streams` | ペイン削除後に参照無し stream を remove |
| 14 | `process_tick_drains_every_tick_even_without_jump` | **穴 A**: M1 シナリオで `virtual_elapsed_ms < threshold` の Tick でも `drain_until` が呼ばれる（current_time は変わらないが、蓄積 trades は drain される） |
| 15 | `process_tick_skips_drain_for_pending_trade_streams` | **穴 B**: `pending_trade_streams` に含まれる stream は drain されない（その他 stream は drain される） |
| 16 | `pending_trade_stream_cleared_on_fetch_completed` | `TradesFetchCompleted` ハンドラで `advance_cursor_to(current_time)` 直後に `pending_trade_streams.remove` される |

### 6.2 統合 / E2E

| # | シナリオ | 期待 |
|:-:|---------|------|
| 1 | 単独 M1 Replay → Play | 1x で 60 sec ごとに 1 M1 バー進む（実時間連動） |
| 2 | 単独 D1 Replay（Tachibana）→ Play | 1x で 1 sec ごとに 1 D1 バー進む（案 C 補正、Phase 3 互換） |
| 3 | 混在 M1+D1 Replay → Play | M1 は実時間連動、D1 は M1 が 1440 本進むごとに 1 本更新 |
| 4 | M1 Replay 中に StepForward | バー境界に揃う |
| 5 | リプレイ中 SplitPane → ticker 選択 | 新ペインがバックフィルされ、completion 後に既存ペインと同期 |
| 6 | リプレイ中 timeframe 変更 (M1 → M5) | 旧 buffer クリア、新 timeframe で再フェッチ、completion 後に同期 |
| 7 | リプレイ中 ClosePane | trade_buffers から不要 stream が削除される |
| 8 | リプレイ中 SplitPane → 即 Pause | バックフィル中でも Pause が効く |
| 9 | バックフィル中に既存 chart が自動再生継続 | バックフィル中の chart 待ちで他 chart が止まらない |
| 10 | バックフィル失敗時 (network error) | 該当 pane に Toast 表示、他は影響なし |

### 6.3 実機

- 立花証券 D1 単独で StepForward / 自動再生
- Binance M1 単独で StepForward / 自動再生
- Binance M1 + Tachibana D1 混在で Play
- Replay 中に SplitPane → Tachibana 7203 D1 を追加 → 当該日付までバックフィル → 同期確認
- Replay 中に timeframe を M1 → M5 に変更 → 同期確認

---

## 7. ロールバック計画

各 Phase は独立 commit にする。問題発覚時は以下の順序で revert：

1. Phase 4（新規テスト + UI ツールチップ）revert（最も影響小、挙動は変わらない）
2. Phase 3 の mid-replay 操作許容 commit（3-1〜）を revert（Phase 1 + 3-0 のリファクタは維持可能）
3. Phase 3-0 のバックフィル関数抽出 commit を revert（Phase 1 は維持可能）
4. Phase 1b の StepForward 離散化を revert（Phase 1 本体は維持可能）
5. Phase 1（Tick 統合 + 既存テスト移行）を revert（Phase 3 互換に戻る）

Phase 1 は実装コードと既存テストが同一 commit なので、**revert すれば一発で is_all_d1_klines / process_d1_tick / テスト 14 件がすべて復活**する（旧計画のように「テスト commit だけ先に revert」で壊れた状態になる心配はない）。Phase 1 を部分的にやり直したい場合は Phase 1 commit から枝分かれして issue を切り直す。

---

## 8. オープンクエスチョン

1. ~~**`speed` セマンティクス案の最終確定**~~ → **確定済み**。§2.1.1 で案 C を採用し、擬似コードを検算表と合わせて確定（2026-04-12 レビュー反映）。案変更時は §2.1 / §2.1.1 / §6.1 #4-#5c を差し戻す
2. ~~**案 C の `COARSE_CUTOFF_MS` 境界**~~ → **`>=` で決め打ち**（§2.1.1 レビュー反映）。H1 も粗補正側に入れる。実機検証で H1 を実時間側に戻したい要望が出たら `>` に差し戻す
3. ~~**`sync_replay_buffers()` 呼び出し点の網羅性**~~ → **Phase 0 で解消済み**。§2.6.1-§2.6.5 に grep 調査結果を記載、`refresh_streams()` 末尾 + `Effect::RequestFetch` 処理末尾の 2 箇所集約で全入口をカバー。Comparison chart は `set_basis()` 経由で `Effect::RequestFetch` を返すため同経路でカバー
4. ~~**新規ペインの TradeBuffer 流入タイミング**~~ → **Phase 0 で解消済み**。§2.3.1 で `TradesFetchCompleted(stream)` 受信時に `advance_cursor_to(current_time)` を呼ぶ設計に確定。既存 stream 重複時は trade fetch 自体をスキップ。§2.3.1.1 で `pending_trade_streams` を追加し、バックフィル中の drain 除外も担保
5. **「リプレイ中に Layout 切替」の挙動**: 別 Layout に切り替えると `active_dashboard()` が変わる。リプレイ状態は `Flowsurface` 直下なので維持されるが、別 dashboard の構成に対してバックフィルが必要 → **本計画スコープ外**、別 issue
6. **mid-replay でのペイン削除と D1 単独復帰**: 混在中に M1 ペインを全削除すると D1 単独になる。既存 `virtual_elapsed_ms` の累積はそのまま使えるか？ → 使える（次バー距離が自動的に 24h に変わり、案 C の `COARSE_CUTOFF_MS` 以上になるので threshold が `COARSE_BAR_MS = 1000` に切り替わる）。**ただし削除直後の 1 回の `virtual_elapsed_ms` は旧 M1 基準で積まれているため、次の jump タイミングが一時的に歪む**。実用上は無視できるが、気になる場合は削除時にリセット
7. **バックフィル中に同じペインへ再度 set_basis が来た場合**: 古いバックフィルタスクの結果を捨てる必要がある → 既存の `req_id` ベース stale 無視で自動処理される（[pane.rs:516-524](../../src/screen/dashboard/pane.rs#L516-L524)）

---

## 9. スコープ外

| 項目 | 理由 |
|------|------|
| Live モードの再設計 | Live は実時間連動のままで問題ない |
| `speed` の自動 timeframe 連動（min tf に応じて bars/sec を変える） | UX が複雑になる。固定の bars/sec で十分 |
| Tachibana の M1 / 時間足対応 | Tachibana API は D1 のみ提供 |
| Heatmap / Footprint の細粒度補間 | 1 Tick あたり 1 バー分の trades 一括 drain で受容 |
| Layout 切替時のリプレイ状態維持 | 別計画 |
| インジケータの mid-replay 再計算 | 既存の `replay_advance` 経路で自動処理される |

---

## 10. 参照

| 文書 | 参照箇所 |
|------|---------|
| [docs/replay_header.md](../replay_header.md) | §3 Phase 3（ペイン操作無効化）, §3 ReplayKlineBuffer 設計 |
| [docs/plan/tachibana_replay.md](tachibana_replay.md) | Phase 3（D1 自動再生スロットリング）, `process_d1_tick` 設計 |
| [docs/plan/refactor_tachibana_replay.md](refactor_tachibana_replay.md) | rebuild 系 API 統一案（本計画の Phase 3-3 と関連） |
| [src/replay.rs:283-361](../../src/replay.rs#L283-L361) | `advance_d1` / `process_d1_tick` 現行実装 |
| [src/main.rs:285-387](../../src/main.rs#L285-L387) | Tick ハンドラ |
| [src/main.rs:786-915](../../src/main.rs#L786-L915) | Play ハンドラ（バックフィル発火元） |
| [src/main.rs:936-1059](../../src/main.rs#L936-L1059) | StepForward / StepBackward |
| [src/screen/dashboard.rs:1095-1132](../../src/screen/dashboard.rs#L1095-L1132) | `replay_advance_klines` / `next_kline_time` / `is_all_d1_klines` |
| [src/screen/dashboard.rs:1325-1367](../../src/screen/dashboard.rs#L1325-L1367) | `prepare_replay` / `collect_trade_streams` |
| [src/chart/kline.rs:163-184](../../src/chart/kline.rs#L163-L184) | `ReplayKlineBuffer` 構造体 |
| [src/chart/kline.rs:337-410](../../src/chart/kline.rs#L337-L410) | `enable_replay_mode` / `replay_advance` |
| [src/screen/dashboard/pane.rs:453-499](../../src/screen/dashboard/pane.rs#L453-L499) | `rebuild_content(replay_mode)` |
| [src/main.rs:1060-1070](../../src/main.rs#L1060-L1070) | `TradesBatchReceived` ハンドラ（新規 TradeBuffer 作成 + trades append、§2.3.1） |
| [src/main.rs:1036-1052](../../src/main.rs#L1036-L1052) | StepBackward の cursor リセット + 早送りロジック（§2.3.1 で流用） |
| [src/screen/dashboard.rs:1369](../../src/screen/dashboard.rs#L1369) | `refresh_streams()` 本体（§2.6.5 集約点 #1、sync_replay_buffers 追加先） |
| [src/screen/dashboard.rs:369-393](../../src/screen/dashboard.rs#L369-L393) | `Effect::RefreshStreams` / `Effect::RequestFetch` 処理（§2.6.5 集約点 #2） |
| [src/screen/dashboard/pane.rs:1363-1530](../../src/screen/dashboard/pane.rs#L1363-L1530) | `set_basis` の複数経路（すべて `Effect` を返却し §2.6.2 に集約される） |
