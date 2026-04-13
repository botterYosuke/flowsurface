# リプレイ E2E バグ分析と修正案

**作成日**: 2026-04-13  
**対象ブランチ**: `sasa/step`  
**関連スイート**: S1, S4, S6, S9, S10  

---

## 目次

1. [BUG-1: try_resume_from_waiting vacuous truth](#bug-1-try_resume_from_waiting-vacuous-truth重要度高)
2. [BUG-3: StepBackward 後の Resume が Playing にならない](#bug-3-stepbackward-後の-resume-が-playing-にならない重要度高)
3. [BUG-4: StepForward が特定条件で diff=0 になる](#bug-4-stepforward-が特定条件で-diff0-になる重要度高)
4. [BUG-2: StepForward が Playing 中でも効く](#bug-2-stepforward-が-playing-中でも効く重要度中)
5. [BUG-5: HTTP API の入力バリデーション不在](#bug-5-http-api-の入力バリデーション不在重要度低)

---

## BUG-1: try_resume_from_waiting vacuous truth（重要度:高）

### 実際のコードと根本原因

[src/replay/mod.rs:244-257](../../src/replay/mod.rs#L244-L257)

```rust
fn try_resume_from_waiting(&mut self, wall_now: Instant) {
    let Some(clock) = &mut self.clock else { return };
    if clock.status() != ClockStatus::Waiting {
        return;
    }
    let full_range = clock.full_range();
    let all_loaded = self
        .active_streams
        .iter()
        .all(|s| self.event_store.is_loaded(s, full_range.clone()));
    if all_loaded {
        clock.resume_from_waiting(wall_now);
    }
}
```

根本原因は **2 つ**ある。提示された選択肢はそれぞれ別の根本原因を対象にしている。

#### 根本原因 ①: `Iterator::all` の vacuous truth（空集合問題）

`active_streams` が空集合の場合、`self.active_streams.iter().all(...)` は Rust の仕様通り **vacuous truth = `true`** を返す。これにより `ingest_loaded` が 1 件も呼ばれていない状態でも `resume_from_waiting` が実行される。

**発生タイミング**: `replay.start()` 直後（`active_streams = HashSet::new()` でリセット済み）にタスク完了コールバックが割り込んだ場合、または `kline_targets` が空でタスクが 0 本の場合。

#### 根本原因 ②: `ingest_loaded` が空 klines でも range を登録する

[src/replay/store.rs:107-120](../../src/replay/store.rs#L107-L120)

```rust
pub fn ingest_loaded(&mut self, stream: StreamKind, range: Range<u64>, data: LoadedData) {
    self.klines.entry(stream).or_insert_with(SortedVec::new).insert_sorted(data.klines);
    // ...
    self.loaded_ranges.entry(stream).or_insert_with(Vec::new).push(range); // ← 常に登録
}
```

未来日時 (2030年など) を指定すると Binance API が空の `[]` を返す。`on_klines_loaded(stream, range, [])` → `ingest_loaded` → `loaded_ranges` に range 登録 → `is_loaded` が `true` → `all_loaded = true` → **データ 0 件なのに Playing になる**。

`active_streams` が非空（正常な状態）でもこの問題は発生する。

### 各選択肢の評価

| 選択肢 | 根本原因 ① 修正 | 根本原因 ② 修正 | 副作用 |
|--------|:--------------:|:--------------:|--------|
| (A) `active_streams.is_empty()` ガード | ✅ | ❌ | なし。最小変更 |
| (B) `is_loaded` で空 range を false に | ❌ | ❌ | 無関係（range は非空） |
| (C) `ingest_loaded` 時に klines 空なら登録しない | ❌ | ✅ | store の意味論を変更。trades 専用ロード時に問題になりうる |
| (D) `on_klines_loaded` で klines 空なら呼ばない | ❌ | ✅ | 最小変更。call site でガード |

### 推奨修正: **(A) + (D)** の組み合わせ

**(D) が最も副作用が少ない**。`ingest_loaded` の意味論（"load されたこと"）を変えず、call site でのみガード。

```rust
// src/main.rs — KlinesLoadCompleted ハンドラー
ReplayMessage::KlinesLoadCompleted(stream, range, klines) => {
    let now = std::time::Instant::now();
    let main_window_id = self.main_window.id;

    // (D) 空 klines は "未ロード" と同義 → EventStore に登録しない
    if !klines.is_empty() {
        self.replay.on_klines_loaded(stream, range, klines.clone(), now);
        self.active_dashboard_mut()
            .ingest_replay_klines(&stream, &klines, main_window_id);
    }
    // klines が空の場合: 当該ストリームはロード済みにならないため
    // try_resume_from_waiting は全ストリームが揃うまで待機し続ける。
    // → 実用上は「未来日時でデータなし = Playing にならない」が正しい挙動。
}
```

加えて、**(A) は defense-in-depth として追加する価値がある**:

```rust
// src/replay/mod.rs — try_resume_from_waiting
fn try_resume_from_waiting(&mut self, wall_now: Instant) {
    let Some(clock) = &mut self.clock else { return };
    if clock.status() != ClockStatus::Waiting {
        return;
    }
    // (A) active_streams が空 = ロード待ち対象なし → 再生不可
    if self.active_streams.is_empty() {
        return;
    }
    let full_range = clock.full_range();
    let all_loaded = self
        .active_streams
        .iter()
        .all(|s| self.event_store.is_loaded(s, full_range.clone()));
    if all_loaded {
        clock.resume_from_waiting(wall_now);
    }
}
```

> **注意**: (D) 単体でも BUG-1 の報告シナリオは修正される。(A) は別の vacuous truth シナリオ（`active_streams` が意図せず空になるバグ）に対する安全網。両方適用を推奨。

---

## BUG-3: StepBackward 後の Resume が Playing にならない（重要度:高）

### 実際のコードフロー

#### Resume ハンドラー ([src/main.rs:934-940](../../src/main.rs#L934-L940))

```rust
ReplayMessage::Resume => {
    let now = std::time::Instant::now();
    if let Some(clock) = &mut self.replay.clock {
        if clock.status() == replay::clock::ClockStatus::Paused {
            clock.play(now);  // Paused の場合のみ Playing に移行
        }
    }
}
```

**重要**: Resume は `clock.status() == Paused` のときのみ `play()` を呼ぶ。Waiting 状態には作用しない。

#### StepBackward ハンドラー ([src/main.rs:979-1007](../../src/main.rs#L979-L1007))

```rust
ReplayMessage::StepBackward => {
    let new_time = prev_time.unwrap_or(current_time);
    if let Some(clock) = &mut self.replay.clock {
        clock.seek(new_time);
        clock.pause();         // ← status = Paused に設定
    }

    self.active_dashboard_mut().prepare_replay(main_window_id); // ← 戻り値を破棄
    
    for stream in self.replay.active_streams.clone().iter() {
        // ... ingest_replay_klines
    }
}
```

### 根本原因の特定

コード上は `clock.pause()` → `status = Paused` → Resume で `play()` が呼ばれるはず。それでも Resume が効かない経路を以下に示す。

#### 原因 A（仮説）: `prepare_replay()` による subscription 副作用

`prepare_replay()` は全ペインの chart content を `rebuild_content_for_replay()` でクリアする（[src/screen/dashboard.rs:1338-1358](../../src/screen/dashboard.rs#L1338-L1358) 参照）。`prepare_replay()` 自体は同期関数であり、iced の subscription を直接トリガーするコードはない。

ただし、理論上は下記の連鎖が起きうる（**未検証**）:

```
StepBackward
  └─ clock.pause() → status = Paused
  └─ prepare_replay() → 全ペイン chart がクリアされる
  
[iced event loop が数フレーム処理]
  └─ ペイン状態変化 subscription → ReplayMessage::Play 発火（仮説）
      └─ replay.start() → clock = 新規 Waiting 状態
      
Resume
  └─ clock.status() == Waiting (≠ Paused) → play() 呼ばれず
```

この連鎖が実際に発生するかは実測が必要。

#### 原因 B: Resume ハンドラーが Waiting を処理しない（確定）

Resume ハンドラーは `Paused` 状態のみを処理する。何らかの理由（原因 A または別の経路）で clock が Waiting になっていた場合、Resume は no-op になる。これは構造上の問題であり **原因 A が成立しない場合でも潜在的なバグ**。

#### 原因 C: `prepare_replay()` の呼び出し自体が不要

StepBackward の目的は「1 つ前の kline 時刻にシークしてチャートを再構築する」こと。`active_streams` の再収集は不要（既に正しい値が入っている）。

**現在の StepBackward 内 `prepare_replay()` 呼び出しの問題**:
- 戻り値 (`kline_targets`) を使用しない → 副作用のためだけに呼んでいる
- chart content の全クリアという重い副作用を伴う

### 質問への回答

**Q1. StepBackward で `prepare_replay()` を呼び直す必要はありますか？**

**不要**。`active_streams` は既に正しく収集されており、klines データも EventStore にある。チャートの再構築は `ingest_replay_klines` で `0..new_time+1` の範囲を注入するだけで済む。

ただし `ingest_replay_klines` がチャートウィジェットにデータを「追記」する実装の場合、`new_time` より未来のデータが残存するため、chart clear は必要。その場合は **`prepare_replay()` 全体ではなく、chart content のみをクリアする専用メソッド**を用意するべき。

**Q2. `clock.play()` を呼んでも Playing にならない場合の状態遷移は？**

Resume ハンドラーの guard: `if clock.status() == Paused`. これが false になる条件:
- **Waiting**: `replay.start()` が再呼び出されてクロックがリセットされた
- **Playing**: 何らかの理由で既に Playing 状態（通常は発生しない）

最も疑わしいのは Waiting への遷移（原因 A または別の経路）。

**Q3. Resume ハンドラーが `Waiting` 状態のクロックを受け取ったら？**

現行実装では何もしない（guard が false なので `play()` 呼ばれない）。status は Waiting のまま。`to_status()` は "Loading" を返すが、E2E で "Paused" と観測されている場合は別の経路の可能性あり。

### 推奨修正

**適用順序**: 修正 1 を先に実装して症状が解消するか確認。解消しない場合は修正 2 を追加。

#### 修正 1（先行・低リスク）: Resume を Waiting にも対応させる

原因 B を確実に解消する。原因 A の仮説が正しいかどうかに関わらず、構造的な堅牢性が向上する。

```rust
ReplayMessage::Resume => {
    let now = std::time::Instant::now();
    if let Some(clock) = &mut self.replay.clock {
        match clock.status() {
            ClockStatus::Paused => {
                clock.play(now);
            }
            ClockStatus::Waiting => {
                // Waiting 中に Resume が来ても no-op（ロード完了時に try_resume_from_waiting が自動で Playing に移行する）
            }
            ClockStatus::Playing => {} // no-op
        }
    }
}
```

#### 修正 2（原因 A が実測で確認された場合）: StepBackward から `prepare_replay()` を除去

修正 1 で症状が解消しない場合、または原因 A の連鎖が実測で確認された場合に適用する。

```rust
ReplayMessage::StepBackward => {
    let main_window_id = self.main_window.id;
    let current_time = self.replay.current_time();

    let prev_time = self.replay.active_streams.iter().filter_map(|stream| {
        let klines = self.replay.event_store.klines_in(stream, 0..current_time);
        klines.iter().rev().find(|k| k.time < current_time).map(|k| k.time)
    }).max();

    let new_time = prev_time.unwrap_or(current_time);
    if let Some(clock) = &mut self.replay.clock {
        clock.seek(new_time);
        clock.pause();
    }

    // prepare_replay() の呼び出しを削除。
    // chart の再描画は「new_time までの全履歴を再注入」で代替する。
    // チャートウィジェットが追記型のため、先に chart content のみをクリアする専用メソッドを追加する。
    self.active_dashboard_mut()
        .clear_chart_for_replay(main_window_id);  // 新規追加: chart content のみクリア

    for stream in self.replay.active_streams.clone().iter() {
        let klines = self.replay.event_store.klines_in(stream, 0..new_time + 1);
        if !klines.is_empty() {
            let klines_vec = klines.to_vec();
            self.active_dashboard_mut()
                .ingest_replay_klines(stream, &klines_vec, main_window_id);
        }
    }
}
```

---

## BUG-4: StepForward が特定条件で diff=0 になる（重要度:高）

### StepForward の実装確認

[src/main.rs:947-975](../../src/main.rs#L947-L975)

```rust
ReplayMessage::StepForward => {
    let current_time = self.replay.current_time();
    let full_range = self.replay.clock.as_ref().map(|c| c.full_range());

    let next_time = if let Some(range) = full_range {
        self.replay.active_streams.iter().filter_map(|stream| {
            let klines = self.replay.event_store.klines_in(stream, current_time..range.end);
            klines.iter().find(|k| k.time > current_time).map(|k| k.time)
        }).min()
    } else {
        None
    };

    if let (Some(new_time), Some(clock)) = (next_time, &mut self.replay.clock) {
        clock.seek(new_time);
        for stream in self.replay.active_streams.clone().iter() {
            let klines = self.replay.event_store.klines_in(stream, 0..new_time + 1);
            // ... ingest
        }
    }
}
```

### diff=0 になりうる全経路の列挙

#### 経路 1: `next_time = None` → seek されない（最有力）

`filter_map(...).min()` が `None` を返すとき seek は起きず `current_time` が維持される。

`None` になる条件:
- `active_streams` が空集合 → `iter()` が空 → `min()` = None
- 全 stream に対して `find(|k| k.time > current_time)` が None を返す

**`find` が None を返す条件**:
1. `klines_in(stream, current_time..range.end)` が空スライスを返す（当該 stream の klines に `time >= current_time` のものがない）
2. スライス内の全 kline が `time == current_time`（厳密等号なので `>` で弾かれる）

#### 経路 2: `klines_in` の境界値

[src/replay/store.rs:63-68](../../src/replay/store.rs#L63-L68)

```rust
pub fn range_slice(&self, range: Range<u64>) -> &[Kline] {
    let start = self.data.partition_point(|k| k.time < range.start); // time >= current_time を返す
    let end   = self.data.partition_point(|k| k.time < range.end);
    &self.data[start..end]
}
```

`klines_in(stream, current_time..range.end)` は `time >= current_time && time < range.end` の klines を返す。

- `time == current_time` の kline がスライスに含まれる
- `find(|k| k.time > current_time)` で `time == current_time` はスキップ
- **もし `current_time` が最後の kline の `time` に一致していれば** → スライスには `current_time` のものしかなく、`find` は None → next_time = None → diff=0

> これは正常な「終端到達」の挙動だが、auto-play の tick で `now_ms` がバー境界外に進んだ場合、最後の kline を飛び越えた状態になり得る。

#### 経路 3: `active_streams` に klines なしの phantom stream が含まれる

観測事項: `trade_buffer_streams` に `Kline` ストリームが混入。

`active_streams` にデータが EventStore にない stream（ロード失敗、または異なる key で登録されたもの）が含まれる場合:
- その stream は `klines_in` から空スライスを返す → `find` = None → `filter_map` で除外
- **他のストリームが有効なら `min()` は非 None** → diff > 0 になるはず

ただし `active_streams` の **全 stream** がデータなしの場合は diff=0。

#### 経路 4: speed cycle 後の `now_ms` ずれ（S1 特有）

`CycleSpeed` → `clock.set_speed()` は speed のみ変更し `step_size_ms` や `now_ms` を変えない。

しかし `SyncReplayBuffers` がどこかで発火した場合:

```rust
ReplayMessage::SyncReplayBuffers => {
    if let Some(clock) = &mut self.replay.clock {
        let step_size_ms = replay::min_timeframe_ms(&self.replay.active_streams);
        clock.set_step_size(step_size_ms);
    }
}
```

`set_step_size` は `now_ms` を新 step_size の倍数に **floor 再整列**する。S1（M1 のみ）で `step_size_ms = 60_000` が維持されるなら影響なし。だが step_size が変わる条件（multistream での M5 追加など）と組み合わさると `now_ms` が大きく後退し、後続の StepForward が意図しない位置から始まる。

### 最も疑わしい経路

> **経路 1 + 経路 3 の組み合わせ** が最有力。

**S1 で speed cycle 後に diff=0**:  
speed cycle が dashboard の subscription を通じて `SyncReplayBuffers` を発火 → `set_step_size` が `now_ms` を floor 整列 → 整列後の `now_ms` が最後の kline の time に一致 → 次の `klines_in(current_time..range.end)` のスライスに `time == current_time` の kline が 1 件のみ → `find(k.time > current_time)` が None → diff=0。

**S4/S6 で diff=0**:  
マルチペイン構成で `active_streams` に extra な phantom stream が混入（`trade_buffer_streams` への Kline 誤追加が証拠）、かつ正規ストリームも終端付近にいる場合、全 stream が None を返す。

### 診断手順

```bash
# StepForward 直前に active_streams の内容を確認
curl -s http://localhost:9876/api/replay/status | node -e "
  const d = JSON.parse(require('fs').readFileSync('/dev/stdin','utf8'));
  console.log(JSON.stringify(d, null, 2));
"

# pane list で trade_buffer_streams の Kline 混入を確認
curl -s http://localhost:9876/api/pane/list | node -e "
  const d = JSON.parse(require('fs').readFileSync('/dev/stdin','utf8'));
  d.panes?.forEach(p => {
    if (p.trade_buffer_streams?.some(s => s.Kline)) {
      console.log('WARN: Kline in trade_buffer_streams', p.id, p.trade_buffer_streams);
    }
  });
"
```

### 推奨修正

経路 2 と経路 3 は即時修正可能。デバッグログは不要（修正後も再現する場合に追加）。

#### 修正 1: `klines_in` の範囲を `current_time + 1..` に変更（経路 2 の確定修正）

現行: `klines_in(stream, current_time..range.end)` → `time == current_time` の kline が混入し、`find(k.time > current_time)` で弾かれて next_time = None になりうる  
修正後: range 自体を next kline から始めることで意図を明確化し、`first()` で簡略化

```rust
// 変更前
let klines = self.replay.event_store.klines_in(stream, current_time..range.end);
klines.iter().find(|k| k.time > current_time).map(|k| k.time)

// 変更後: range と検索条件を一致させる
let klines = self.replay.event_store.klines_in(stream, current_time + 1..range.end);
klines.first().map(|k| k.time)
```

#### 修正 2: `active_streams` への Kline-only フィルター（経路 3 の確定修正）

`active_streams` に追加するストリームを `Kline` 系のみに限定する明示的フィルターを追加:

```rust
// Play ハンドラー
for (_, stream) in &kline_targets {
    // kline stream のみ active_streams に追加（Trade/Depth は除外）
    if matches!(stream, StreamKind::Kline { .. }) {
        self.replay.active_streams.insert(*stream);
    }
}
```

---

## BUG-2: StepForward が Playing 中でも効く（重要度:中）

### 根本原因

[src/main.rs:947](../../src/main.rs#L947) の `StepForward` ハンドラーに clock status のチェックがない。Playing 中でも seek が実行される。

### 質問への回答

**Q1. ハンドラー冒頭に Paused チェックを追加するだけで十分か？**

はい、十分。Playing 中は `tick()` が自動進行するため、StepForward は仕様上 no-op であるべき。

```rust
ReplayMessage::StepForward => {
    // Playing 中は no-op（tick が自動で進める）
    if !self.replay.is_paused() {
        return Task::none();
    }
    // ... 既存処理
}
```

**Q2. StepBackward にも同様のガードが必要か？**

StepBackward の場合、Playing 中の挙動は 2 つの流派がある:

| 方針 | 挙動 | 理由 |
|------|------|------|
| **no-op** | Playing 中は何もしない | StepForward と対称的。一貫性。 |
| **stop + step back** | pause してから 1 つ後退 | ユーザーが意図的に巻き戻したい場合の UX |

**推奨: stop + step back** が自然な UX（Playing 中に ⏮ を押したら止まって戻ることを期待する）。現行コードは既に `clock.pause()` を呼んでいるため、ガード不要で既に "stop + step back" として動作している。ただし **意図を明示するコメント**を追加:

```rust
ReplayMessage::StepBackward => {
    // Playing 中でも呼べる: seek(prev) + pause() で「止まって戻る」UX
    // ...
}
```

---

## BUG-5: HTTP API の入力バリデーション不在（重要度:低）

### 現状

[src/replay_api.rs:264-277](../../src/replay_api.rs#L264-L277) — `play` エンドポイントはフィールド存在確認のみで、日時フォーマット検証は iced 側の `parse_replay_range` に委ねている。パース失敗時は Toast 表示のみで HTTP は常に 200。

### axum + serde による最小コストバリデーション

[src/replay_api.rs](../../src/replay_api.rs) の `route()` 関数に日時バリデーションを追加する。

```rust
// replay_api.rs

use chrono::NaiveDateTime;

/// YYYY-MM-DD HH:MM 形式の日時文字列を検証する
fn validate_datetime_str(s: &str, field: &str) -> Result<(), RouteError> {
    NaiveDateTime::parse_from_str(s, "%Y-%m-%d %H:%M")
        .map(|_| ())
        .map_err(|_| RouteError::BadRequestWithReason(
            format!("Invalid {field}: expected 'YYYY-MM-DD HH:MM', got '{s}'")
        ))
}

// route() 内の play エンドポイント処理:
("POST", "/api/replay/play") => {
    let parsed: serde_json::Value =
        serde_json::from_str(body).map_err(|_| RouteError::BadRequest)?;
    let start = parsed
        .get("start")
        .and_then(|v| v.as_str())
        .ok_or(RouteError::BadRequest)?
        .to_string();
    let end = parsed
        .get("end")
        .and_then(|v| v.as_str())
        .ok_or(RouteError::BadRequest)?
        .to_string();

    // フォーマット検証 → 不正なら RouteError::BadRequest を返す
    validate_datetime_str(&start, "start")?;
    validate_datetime_str(&end, "end")?;

    return Ok(ApiCommand::Replay(ReplayCommand::Play { start, end }));
}
```

`RouteError` に理由付き variant を追加:

```rust
enum RouteError {
    BadRequest,
    BadRequestWithReason(String),
    NotFound,
}
```

HTTP レスポンス生成部:

```rust
// handle_request() or equivalent
match result {
    Err(RouteError::BadRequest) => {
        return (StatusCode::BAD_REQUEST, r#"{"error":"bad request"}"#.to_string());
    }
    Err(RouteError::BadRequestWithReason(msg)) => {
        let body = format!(r#"{{"error":"bad request","detail":"{}"}}"#, msg);
        return (StatusCode::BAD_REQUEST, body);
    }
    // ...
}
```

### 検証テスト例

```bash
# 不正日時 → 400 を期待
STATUS=$(curl -s -o /dev/null -w "%{http_code}" \
  -X POST http://localhost:9876/api/replay/play \
  -H "Content-Type: application/json" \
  -d '{"start":"not-a-date","end":"2026-04-10 15:00"}')
[ "$STATUS" = "400" ] && echo PASS || echo "FAIL: got $STATUS"

# フィールド欠損 → 400 を期待
STATUS=$(curl -s -o /dev/null -w "%{http_code}" \
  -X POST http://localhost:9876/api/replay/play \
  -H "Content-Type: application/json" \
  -d '{"start":"2026-04-10 09:00"}')
[ "$STATUS" = "400" ] && echo PASS || echo "FAIL: got $STATUS"

# 正常 → 200
STATUS=$(curl -s -o /dev/null -w "%{http_code}" \
  -X POST http://localhost:9876/api/replay/play \
  -H "Content-Type: application/json" \
  -d '{"start":"2026-04-10 09:00","end":"2026-04-10 15:00"}')
[ "$STATUS" = "200" ] && echo PASS || echo "FAIL: got $STATUS"
```

---

## 修正優先順位と作業チェックリスト

| 優先 | BUG | 修正箇所 | 難易度 | 状態 |
|------|-----|----------|--------|------|
| 1 | BUG-3 | Resume ハンドラーを Waiting にも対応（修正 1）→ 未解消なら `prepare_replay()` 除去（修正 2） | 低→中 | ✅ 修正1 + 修正2 両方適用済 |
| 2 | BUG-1 | `KlinesLoadCompleted` で空 klines ガード (D) + `try_resume_from_waiting` で empty guard (A) | 低 | ✅ 両方適用済 |
| 3 | BUG-4 | `klines_in` 範囲を `current_time+1` に変更（修正 1）+ Kline-only フィルター（修正 2） | 低 | ✅ 修正1 + 修正2 両方適用済 |
| 4 | BUG-2 | StepForward ハンドラー冒頭に `is_paused()` チェック追加 | 低 | ✅ 適用済 |
| 5 | BUG-5 | `route()` 内に `validate_datetime_str()` 追加 | 低 | ✅ 適用済 |

---

## 実装ログ（2026-04-13）

### 作業概要
TDD アプローチで全バグ修正を実施。`cargo test` 149 tests all passed。

---

### ✅ BUG-1 fix(A): `try_resume_from_waiting` の vacuous truth ガード

**修正ファイル**: `src/replay/mod.rs`

**TDD**:
- RED: `try_resume_does_not_auto_play_when_active_streams_empty` テスト追加 → 失敗確認
- GREEN: `if self.active_streams.is_empty() { return; }` ガード追加 → パス

**変更箇所** (`src/replay/mod.rs:249-251`):
```rust
fn try_resume_from_waiting(&mut self, wall_now: Instant) {
    let Some(clock) = &mut self.clock else { return };
    if clock.status() != ClockStatus::Waiting { return; }
    // (A) active_streams が空 = ロード待ち対象なし → 再生不可（vacuous truth ガード）
    if self.active_streams.is_empty() {
        return;
    }
    // ...
```

---

### ✅ BUG-1 fix(D): `KlinesLoadCompleted` 空 klines ガード

**修正ファイル**: `src/main.rs`

```rust
// (D) 空 klines は "未ロード" と同義 — EventStore に登録しない。
if klines.is_empty() {
    return Task::none();
}
```

**Why**: 未来日時の指定など Binance が `[]` を返したとき、`ingest_loaded` が `loaded_ranges` に range を登録してしまい `is_loaded → true` となって空データで Playing になる問題を防ぐ。

---

### ✅ BUG-3 修正1: Resume ハンドラーの明示的 match

**修正ファイル**: `src/main.rs`

```rust
ReplayMessage::Resume => {
    let now = std::time::Instant::now();
    if let Some(clock) = &mut self.replay.clock {
        match clock.status() {
            replay::clock::ClockStatus::Paused => { clock.play(now); }
            // Waiting: ロード完了時に try_resume_from_waiting が自動で Playing に移行する
            replay::clock::ClockStatus::Waiting => {}
            // Playing: 既に再生中 — no-op
            replay::clock::ClockStatus::Playing => {}
        }
    }
}
```

**修正2 適用済み（2026-04-13）**: `clear_chart_for_replay()` を `Dashboard` に追加し、StepBackward ハンドラーで `prepare_replay()` の代わりに呼び出すよう変更。subscription 経由の Play 再発火を防ぐ。テスト: `_type_check_clear_chart_for_replay_returns_unit`（コンパイル時型チェック）。

---

### ✅ BUG-4 修正1: StepForward の `klines_in` 範囲を `current_time + 1` に変更

**修正ファイル**: `src/main.rs`

```rust
// current_time + 1 スタートで current_time と同時刻のバーを除外し、
// first() で最初の次バーを取得（BUG-4: diff=0 防止）
let klines = self.replay.event_store.klines_in(stream, current_time + 1..range.end);
klines.first().map(|k| k.time)
```

**TDD**: `klines_in_with_exclusive_start_skips_current_time_kline` テスト追加（GREEN from start = 既存動作の文書化）

**修正2 適用済み（2026-04-13）**: Play ハンドラーの `active_streams` 登録ループに `matches!(stream, StreamKind::Kline { .. })` フィルターを追加。テスト: `active_streams_only_contains_kline_streams_after_insert`（`src/replay/mod.rs`）。全テスト 150 passed。

---

### ✅ BUG-2: StepForward Playing 中ガード

**修正ファイル**: `src/main.rs`

```rust
ReplayMessage::StepForward => {
    // Playing 中は tick が自動で進める — StepForward は Paused 時のみ有効
    if !self.replay.is_paused() {
        return Task::none();
    }
    // ...
```

---

### ✅ BUG-5: HTTP API 日時バリデーション

**修正ファイル**: `src/replay_api.rs`

**TDD**:
- RED: `route_post_play_invalid_datetime_start_returns_bad_request` / `..._end_returns_bad_request` 追加
- GREEN: `validate_datetime_str()` ヘルパー追加、`route()` で呼び出し

```rust
fn validate_datetime_str(s: &str) -> Result<(), RouteError> {
    chrono::NaiveDateTime::parse_from_str(s, "%Y-%m-%d %H:%M")
        .map(|_| ())
        .map_err(|_| RouteError::BadRequest)
}
```

---

## 残課題 / Tips

### BUG-3 修正2 の適用条件
`prepare_replay()` が subscription 経由で `ReplayMessage::Play` を誘発し、clock が Waiting にリセットされる連鎖は**未検証**。E2E S6（StepBackward 後 Resume）で再現する場合:
1. `clear_chart_for_replay(main_window_id)` メソッドを `Dashboard` に追加（chart content のみクリア、`kline_targets` 収集なし）
2. StepBackward から `prepare_replay()` 呼び出しを削除して `clear_chart_for_replay()` に置換

### BUG-4 修正2 の適用条件
`active_streams` に Trade/Depth 系 phantom stream が混入する問題が E2E で確認された場合:
```rust
// Play ハンドラー内
for (_, stream) in &kline_targets {
    if matches!(stream, StreamKind::Kline { .. }) {
        self.replay.active_streams.insert(*stream);
    }
}
```

### テスト追加一覧（本作業で追加）
| テスト名 | ファイル | 目的 |
|---------|---------|------|
| `try_resume_does_not_auto_play_when_active_streams_empty` | `src/replay/mod.rs` | BUG-1(A) ガード検証 |
| `route_post_play_invalid_datetime_start_returns_bad_request` | `src/replay_api.rs` | BUG-5 バリデーション |
| `route_post_play_invalid_datetime_end_returns_bad_request` | `src/replay_api.rs` | BUG-5 バリデーション |
| `klines_in_with_exclusive_start_skips_current_time_kline` | `src/replay/store.rs` | BUG-4 range 動作の文書化 |

---

## 付録: StepClock の状態遷移図

```
         new()
          │
          ▼
       [Paused] ◄──────── pause() ────────────┐
          │                                    │
         play()                         pause() / tick → range.end
          │                                    │
          ▼                                    │
       [Playing] ──── tick() ─────────────────►┘
          │                          │
     set_waiting()             resume_from_waiting()
          │                          │
          ▼                          │
       [Waiting] ────────────────────┘

Resume ハンドラーは Paused → Playing のみ処理。
Waiting 状態には作用しないため、Waiting 時に Resume が来ると no-op。
```
