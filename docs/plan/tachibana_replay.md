# 立花証券リプレイ対応 — 修正計画書

**作成日**: 2026-04-12
**最終更新**: 2026-04-12（Phase 3 レビュー反映）
**対象**: 立花証券（Tachibana）銘柄でリプレイ機能を利用可能にする
**前提ドキュメント**: `docs/replay_header.md`, `docs/tachibana_spec.md`
**状態**: Phase 1・Phase 2 完了 ✅ / Phase 3 未実装 ⏳

---

## 0. 現状の問題

リプレイモードで Play を押すと、以下の2つのデータを取得する:

| データ | 取得関数 | Tachibana の状況 |
|--------|----------|-----------------|
| Kline | `kline_fetch_task()` | `fetch_tachibana_daily_klines()` で日足のみ取得可。**range フィルタなし**（全履歴を返す） |
| Trades | `fetch_trades_batched()` | Binance 以外はスキップ。**Tachibana 未対応** |

### 具体的な障害

1. **Kline は動くが非効率**: `fetch_tachibana_daily_klines()` は range 引数を無視して全履歴（~20年分）を返す。リプレイには不要なデータまで取得してしまう
2. **日足しかない**: 立花証券 API は分足・時間足を提供しない（`supports_kline_timeframe` は `D1` のみ）。リプレイの時間軸が日足単位に制限される
3. **Trades なし**: 過去の歩み値を取得する API が存在しない。EVENT I/F の ST コマンドはリアルタイム配信のみ
4. **Depth なし**: 過去の板スナップショットを取得する API が存在しない

---

## 1. 方針

立花証券 API の制約上、**日足レベルのリプレイのみ対応**とする。Trades / Depth の再生は API が存在しないため対象外。

ユースケース: 日足チャートを1本ずつ進めて「次の日の値動きを予測する」練習。StepForward で1日進む、StepBackward で1日戻る。

---

## 2. 修正箇所

### Phase 1: Kline の range フィルタ追加 ✅

**ファイル**: `src/connector/fetcher.rs`

現状の `fetch_tachibana_daily_klines()` は range を受け取らないため、全履歴を返す。リプレイ時は `kline_fetch_task()` に渡される `range: Option<(u64, u64)>` を `fetch_tachibana_daily_klines()` に伝播させ、取得後にフィルタする。

```
変更前:
  kline_fetch_task() → fetch_tachibana_daily_klines(issue_code)
                        → 全履歴を返す

変更後:
  kline_fetch_task() → fetch_tachibana_daily_klines(issue_code, range)
                        → 全履歴を取得後、range でフィルタして返す
```

具体的な変更:

1. `fetch_tachibana_daily_klines()` のシグネチャに `range: Option<(u64, u64)>` を追加
2. 取得した klines を `range` でフィルタ（`kline.time >= start && kline.time <= end`）
3. `kline_fetch_task()` 内の Tachibana 分岐で `range` を渡す
4. 既存の呼び出し元（ライブモード）は `None` を渡す

### Phase 2: StepForward / StepBackward の離散ステップ化 + D1 Paused 開始 ✅

**ファイル**: `src/main.rs` (ReplayMessage::Play / StepForward / StepBackward ハンドラ)

#### 問題点

現状の StepForward は `current_time += 60_000`（1分進む）固定。日足リプレイでは 1分 では意味がないうえ、株式市場は土日祝が休場のため、固定幅 `+86_400_000`（1日）にしても休場日のタイムスタンプに止まり、対応する kline が存在しない「空振りステップ」が発生する。

#### 変更: kline ベースの離散ステップ

プリフェッチ済みの kline timestamp リストから次/前の kline を探す方式にする。これにより休場日を自動スキップする。

```rust
// StepForward: 現在時刻より後の最初の kline に進む
let next_time = klines.iter()
    .find(|k| k.time > pb.current_time)
    .map(|k| k.time);
if let Some(t) = next_time {
    pb.current_time = t;
}

// StepBackward: 現在時刻より前の最後の kline に戻る
let prev_time = klines.iter()
    .rev()
    .find(|k| k.time < pb.current_time)
    .map(|k| k.time);
if let Some(t) = prev_time {
    pb.current_time = t;
}
```

D1 以外の timeframe では現状通り `+= 60_000` を維持する（分足は連続データのため固定幅で問題ない）。

#### 変更: D1 は Paused で開始

Play ボタン押下時に D1 のみの場合は直接 `Paused` 状態で開始する。日足の自動再生は UX として不自然（チャートがパタパタ切り替わるだけ）で、「次の足を予測する」ユースケースには StepForward/StepBackward 操作が最適。

これにより `replay.rs` の Tick 自動進行ロジックの変更は不要（D1 では Tick に到達しない）。

### Phase 3: D1 自動再生（Tick ハンドラ内スロットリング）⏳ 未実装

**ファイル**: `src/main.rs` (Message::Tick ハンドラ), `src/replay.rs` (`ReplayState` にフィールド追加)

#### 背景

Phase 2 で「D1 は Paused で開始」としたが、長期間のヒストリカルスキャンや連続的な値動き確認を手動 Step だけに任せるのは不便。一方で現行の Tick 機構は `advance_time(elapsed_ms)` が `elapsed_ms * speed` を加算する方式のため、1x では日足 1 本を跨ぐのに実時間 24 時間、最大 10x でも ~2.4 時間かかり実用不能。

#### 現行 Tick 機構の事実確認

- Tick サブスクリプションは `iced::window::frames().map(Message::Tick)` ([main.rs:1356](../../src/main.rs#L1356))。ディスプレイのリフレッシュレート固定で、間引き設定は存在しない。
- `PlaybackState::advance_time` は `elapsed_ms * speed` で進むのみ ([replay.rs:270](../../src/replay.rs#L270))。
- `src/screen/dashboard.rs` に `tick_interval` のような設定は存在しない。

したがって Phase 3 で「tick_interval を延長する」という外部設定の導入は不要・不可能で、**Tick ハンドラ内で前回 D1 ジャンプ時刻からの経過で自前スロットリングする**のが最小変更。

#### 変更方針: Tick ハンドラ内スロットリング（仮想時間累積方式）

「1 tick = 次の kline に離散ジャンプ」方式。ただし毎フレーム進めるのではなく、`speed` を「bars/sec」として再解釈した間隔でジャンプする。

実装は `Instant` ベースではなく **仮想時間累積** 方式を採用する。Tick ごとに `elapsed_ms * speed` を仮想カウンタに足し、1000ms を超えたら次 kline にジャンプする。Pause 中は Tick 自体が来ないためカウンタは自然に止まり、`Instant` の引き算や Pause/Resume 時のリセット管理が不要になる。

1. **`PlaybackState` にフィールド追加** (`src/replay.rs`):
   - `is_d1_only: bool` — Play ハンドラ構築時に `Dashboard::is_all_d1_klines()` の結果を一度だけ記録。Tick ハンドラは毎フレーム dashboard を走査せず、このフラグを見る
   - `d1_virtual_elapsed_ms: f64` — 仮想時間累積カウンタ。ジャンプ発火時に 0 へリセット
2. **`PlaybackState::advance_d1()` を追加**: 純粋関数として `elapsed_ms`・次 kline 時刻を受け取り、`(jumped: bool, reached_end: bool)` を返す（ユニットテスト可能にするため）
   ```rust
   // interval_ms = 1000.0 / speed （1x=1000ms/本, 10x=100ms/本）
   self.d1_virtual_elapsed_ms += elapsed_ms * self.speed;
   if self.d1_virtual_elapsed_ms < 1000.0 { return (false, false); }
   self.d1_virtual_elapsed_ms = 0.0;
   match next_kline_time {
       Some(t) => { self.current_time = t; (true, false) }
       None => (false, true),  // 終端
   }
   ```
3. **Tick ハンドラの分岐** ([main.rs:284 付近](../../src/main.rs#L284)): `pb.is_d1_only && pb.status == Playing` の場合、`advance_time()` の代わりに上記 `advance_d1()` を呼ぶ:
   - `jumped == true` → `replay_current_time = Some(pb.current_time)` を返し、後続の `replay_advance_klines()` 経由でチャートへ反映
   - `jumped == false && reached_end == false` → `replay_current_time = None`（UI 更新スキップ、チャートは現状維持）
   - `reached_end == true` → `pb.status = Paused`
   - **trade_buffers の drain はスキップ** する（D1-only 時は Trades が存在しないため）
4. **既存の終端判定の移設**: 現行 [main.rs:316-318](../../src/main.rs#L316-L318) の `pb.current_time >= pb.end_time` 分岐は D1 以外の else 側にのみ残す（D1 経路では `advance_d1` の `reached_end` で判定）
5. **Play ハンドラは `is_d1_only` をセット**: Phase 2 で既に使っている `is_all_d1_klines()` の結果を `PlaybackState` 構築時に保存する。「D1 のみなら `resume_status = Paused` で開始」の既存挙動は維持
6. **手動 Step ハンドラ**: StepForward/StepBackward 側では `d1_virtual_elapsed_ms = 0.0` にリセットする（Step 直後に自動ジャンプが連続するのを防ぐ）
7. **速度スライダの UI 変更はしない**: `SPEEDS = [1.0, 2.0, 5.0, 10.0]` をそのまま「bars/sec」として再解釈するだけで、ラベル表示 (`speed_label()`) も変更不要

```rust
// Message::Tick 内（擬似コード）
let (replay_trades, replay_current_time) = if let Some(pb) = &mut self.replay.playback {
    if pb.status != PlaybackStatus::Playing {
        (None, None)
    } else if pb.is_d1_only {
        let next = self.active_dashboard()
            .replay_next_kline_time(pb.current_time, main_window_id);
        let (jumped, reached_end) = pb.advance_d1(elapsed_ms, next);
        if reached_end {
            pb.status = PlaybackStatus::Paused;
        }
        // D1-only: trade_buffers は空なので drain しない
        (None, jumped.then_some(pb.current_time))
    } else {
        let current_time = pb.advance_time(elapsed_ms);
        // 既存の trade_buffers.drain_until(...) ロジック
        // ... collected を構築 ...
        if pb.current_time >= pb.end_time {
            pb.status = PlaybackStatus::Paused;
        }
        (Some(collected), Some(current_time))
    }
} else {
    (None, None)
};
```

**重要**: `replay_current_time.is_some()` のときだけ [main.rs:331-334](../../src/main.rs#L331-L334) の `replay_advance_klines()` が呼ばれるため、D1 経路でも **ジャンプ発火時は必ず `Some(pb.current_time)` を返す** こと。これを忘れるとバッファは進むがチャートが更新されない。

#### テスト追加予定

`advance_d1()` を純粋関数にしたことで、以下のテストは `PlaybackState` 単体でユニットテスト可能。

| # | テスト | 検証内容 |
|---|--------|---------|
| 8 | `advance_d1` ジャンプ発火 | `elapsed_ms * speed >= 1000` を満たした Tick で `(jumped=true, reached_end=false)` を返し、`current_time` が next_kline_time に更新されること |
| 9 | `advance_d1` 終端検知 | `next_kline_time = None` のとき `(jumped=false, reached_end=true)` を返すこと |
| 10 | `advance_d1` スロットル | `speed=1.0` で `elapsed_ms=100` を 9 回呼んでも `jumped=false`、10 回目で `jumped=true` になること（仮想時間累積） |
| 11 | `advance_d1` speed スケール | `speed=10.0`・`elapsed_ms=100` 単発で `jumped=true` になること（10x = 100ms/本） |
| 12 | Step 後のカウンタリセット | StepForward/StepBackward ハンドラ呼び出し後に `d1_virtual_elapsed_ms == 0.0` になっていること |

---

## 3. 変更ファイル一覧

| ファイル | 変更内容 | 状態 |
|---------|---------|------|
| `src/connector/fetcher.rs` | `fetch_tachibana_daily_klines()` に range フィルタ追加 | ✅ Phase 1 |
| `src/main.rs` | StepForward/StepBackward の kline ベース離散ステップ化、D1 Paused 開始 | ✅ Phase 2 |
| `src/main.rs` | Tick ハンドラで `pb.is_d1_only` 分岐を追加、`advance_d1()` 呼び出し、終端で Paused 遷移、既存 `end_time` 判定を else 側へ移設 | ⏳ Phase 3 |
| `src/main.rs` | StepForward/StepBackward ハンドラで `d1_virtual_elapsed_ms` をリセット | ⏳ Phase 3 |
| `src/replay.rs` | `PlaybackState` に `is_d1_only: bool`・`d1_virtual_elapsed_ms: f64` を追加、`advance_d1()` メソッド実装 | ⏳ Phase 3 |
| `src/main.rs` | Play ハンドラで `PlaybackState` 構築時に `is_d1_only` をセット | ⏳ Phase 3 |

---

## 4. 対象外（API 制約により不可能）

| 項目 | 理由 |
|------|------|
| 分足・時間足リプレイ | 立花証券 API は日足のみ提供 |
| Trades（歩み値）リプレイ | 過去の歩み値を取得する API が存在しない |
| Depth（板情報）リプレイ | 過去の板スナップショットを取得する API が存在しない |
| Heatmap リプレイ | Trades / Depth が必要 |

---

## 5. テスト計画

| # | テスト | 検証内容 |
|---|--------|---------|
| 1 | `fetch_tachibana_daily_klines` range フィルタ | range 指定時に範囲内の kline のみ返すこと |
| 2 | `fetch_tachibana_daily_klines` range なし | `None` の場合は全履歴を返すこと（既存動作維持） |
| 3 | リプレイ Play → D1 ペイン | Loading → Paused に遷移し、チャートにプリフェッチ分の日足が表示されること |
| 4 | StepForward（D1） | 休場日をスキップして次の営業日の日足が表示されること |
| 5 | StepBackward（D1） | 休場日をスキップして前の営業日に戻り、最後の日足が非表示になること |
| 6 | StepForward（D1・終端） | 最後の kline 以降で StepForward しても current_time が変わらないこと |
| 7 | ライブ復帰 | ToggleMode → Live で PlaybackState が破棄され、WebSocket が再購読されること |

---

## 6. 将来の拡張可能性

- **ローカル録画**: EVENT I/F のリアルタイムデータ（ST/FD）をファイルに記録し、後からリプレイする。API 制約を回避する唯一の方法
- **KP コマンド活用**: 約5秒間隔で受信する現在値データを記録すれば、疑似的な分足を構築できる可能性がある

---

## 7. 実装記録

### Phase 1 実装（2026-04-12）

**変更ファイル**: `src/connector/fetcher.rs`

- `fetch_tachibana_daily_klines()` に `range: Option<(u64, u64)>` パラメータを追加
- range 指定時は取得後に `klines.retain(|k| k.time >= start && k.time <= end)` でフィルタ
- `kline_fetch_task()` の Tachibana 分岐で `range` を伝播
- テスト 4 件追加（セッション未設定エラー、変換、range フィルタ、range=None 全件返却）

**設計判断**: API 側でのフィルタは不可能（全履歴を一括返却する仕様）のため、取得後のクライアントサイドフィルタを採用。

### Phase 2 実装（2026-04-12）

**変更ファイル**: `src/main.rs`, `src/chart/kline.rs`, `src/screen/dashboard/pane.rs`, `src/screen/dashboard.rs`

- `KlineChart` に `replay_next_kline_time()` / `replay_prev_kline_time()` を追加
- `pane::State`, `Dashboard` に同名のラッパーメソッドを追加（全ペイン横断で min/max を取得）
- `Dashboard::is_all_d1_klines()` で全 kline ストリームが D1 かを判定
- `StepForward`: D1 の場合はバッファから次の kline timestamp へ離散ジャンプ（休場日自動スキップ）
- `StepBackward`: D1 の場合はバッファから前の kline timestamp へ離散ジャンプ
- D1 以外は従来通り `±60_000ms` 固定ステップを維持
- `Play` ハンドラ: 全 kline ストリームが D1 のみの場合、`resume_status = Paused` に設定
- テスト 4 件追加（next/prev の休場日スキップ、終端/始端での停止）

**設計判断**:
- borrow checker 制約のため、`replay_next_kline_time()` / `replay_prev_kline_time()` を immutable borrow で先に呼び出し、結果を変数に保存してから `&mut self.replay.playback` を取得する構成にした
- `prepare_replay()` と `collect_trade_streams()` を `active_dashboard_mut()` のスコープブロックに閉じ込め、D1 判定・`resume_status` 更新を外側で行う構成にした

**Tips**:
- `ReplayKlineBuffer.klines` はソート済みなので、`iter().find()` / `iter().rev().find()` で O(n) の線形探索。バッファサイズは高々 ~6000 本（20年分日足）なので問題なし

### Phase 2 レビュー対応（2026-04-12）

レビュー指摘 4 件を修正:

1. **kline.rs テストが実装を検証していなかった**（高）: `find_next`/`find_prev` というローカルコピーでテストしていたため、実メソッドが壊れても検出不可。
   - 対応: `ReplayKlineBuffer` に `next_time_after()` / `prev_time_before()` メソッドを追加し、`KlineChart::replay_next_kline_time` / `replay_prev_kline_time` はこれに委譲。テストは実メソッドを直接呼ぶ形に書き換え。

2. **D1 判定ロジックの重複**（中）: `main.rs` の Play ハンドラ内でインライン判定と `Dashboard::is_all_d1_klines()` が並存していた。
   - 対応: Play ハンドラも `is_all_d1_klines()` を呼ぶように統一。

3. **Dashboard 層のテスト欠落**（中）: `is_all_d1_klines` のユニットテストがなかった。
   - 対応: 判定ロジックを純粋関数 `all_kline_streams_are_d1<I: IntoIterator<Item = &StreamKind>>` に抽出し、6 ケースのテストを追加（D1 のみ、複数 D1、非 D1 混在、kline なし、空、trade 無視）。

4. **冗長な clamp**（低）: `next_kline_time.unwrap_or(...).min(pb.end_time)` の clamp は replay_kline_buffer が `[start, end]` 範囲で構築されるため効果なし。
   - 対応: D1 分岐の `.min/.max` を削除し、「バッファ由来で範囲内」の旨をコメント化。

**設計判断**:
- D1 判定を `StreamKind` イテレータを受け取る純粋関数に抽出したことで、`Dashboard` 全体を組み立てずにテスト可能になった
- `ReplayKlineBuffer` に検索メソッドを追加したことで、`KlineChart` の border 層（`replay_kline_buffer.as_ref()` の Option ラップ）と検索ロジックが分離された
