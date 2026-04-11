# リプレイ状態の永続化 — 実装プラン

**作成日**: 2026-04-11
**対象**: saved-state.json にリプレイモード・範囲入力を保存し、再起動後も復元する
**状態**: ✅ 完了 (2026-04-11)
**追加対応**: 6時間制限撤廃 + fetch_klines ページング + E2E テスト基盤整備 (2026-04-11)

---

## 1. 背景と目的

現在、`ReplayState` は起動時に `ReplayState::default()` で初期化され、常に Live モードで開始する。リプレイの範囲入力（Start/End）やモード選択は毎回手入力が必要。

**ゴール**: アプリ終了時のリプレイ設定を保存し、次回起動時に復元する。

### 保存する項目

| 項目 | 型 | 保存する理由 |
|------|-----|-------------|
| Live/Replay モード | `ReplayMode` | 前回のモードを復元 |
| 開始日時（文字列） | `String` | 手入力を省略 |
| 終了日時（文字列） | `String` | 手入力を省略 |

### 保存しない項目

| 項目 | 理由 |
|------|------|
| `PlaybackState` (current_time, speed, status) | 再生位置は揮発的。復元しても trade_buffers が空で意味がない |
| `trade_buffers` | データサイズ大。再生開始時に再フェッチすれば良い |
| `last_tick` | `std::time::Instant` は serialize 不可。フレーム間計測用で永続化不要 |

---

## 2. 現状のアーキテクチャ

### 保存の流れ

```
save_state_to_disk() (src/main.rs)
  → data::State::from_parts(...) で State 構築
  → serde_json::to_string(&state)
  → data::write_json_to_file(json, SAVED_STATE_PATH)
```

### 読込の流れ

```
load_saved_state() (src/layout.rs)
  → data::read_from_file(SAVED_STATE_PATH)
  → serde_json::from_str::<data::State>(json)
  → SavedState に変換して返す
```

### 関連構造体

```
data::State (data/src/config/state.rs)     -- JSON 永続化用 (#[derive(Serialize, Deserialize)])
SavedState  (src/layout.rs)                -- ロード後の中間表現（derive なし）
ReplayState (src/replay.rs)                -- ランタイム状態（derive なし）
```

---

## 3. 設計

### 3.1 新しい永続化用構造体

`data/src/config/state.rs` に追加:

```rust
#[derive(Clone, Serialize, Deserialize, Default)]
#[serde(default)]
pub struct ReplayConfig {
    /// "live" or "replay"
    pub mode: String,
    /// 開始日時の入力文字列 (例: "2026-04-10 09:00")
    pub range_start: String,
    /// 終了日時の入力文字列 (例: "2026-04-10 15:00")
    pub range_end: String,
}
```

**設計判断**: `ReplayMode` enum を直接 Serialize するのではなく `String` を使う。
- 理由: `data` crate は `src/replay.rs` の型に依存しない。`"live"` / `"replay"` 文字列なら依存関係が増えない。
- `#[serde(default)]` により、既存の saved-state.json に `replay` フィールドがなくても `ReplayConfig::default()` (= Live, 空文字) で安全にデシリアライズされる。

### 3.2 State 構造体への追加

```rust
#[derive(Default, Clone, Deserialize, Serialize)]
#[serde(default)]
pub struct State {
    // ... 既存フィールド ...
    pub replay: ReplayConfig,  // 追加
}
```

### 3.3 SavedState への追加

`src/layout.rs`:

```rust
pub struct SavedState {
    // ... 既存フィールド ...
    pub replay_config: data::ReplayConfig,  // data crate の型をそのまま保持
}
```

**設計判断**: `SavedState` は既に `data::Theme`, `data::Sidebar` 等の `data` crate 型を直接使っている。`ReplayConfig` も同様に持たせることで、`load_saved_state()` 内での String→enum 変換が不要になり、変換ロジックを `Flowsurface::new()` の1箇所に集約できる。

### 3.4 JSON 出力例

```json
{
  "layout_manager": { ... },
  "timezone": "UTC",
  "replay": {
    "mode": "replay",
    "range_start": "2026-04-10 09:00",
    "range_end": "2026-04-10 15:00"
  },
  ...
}
```

既存の saved-state.json には `replay` キーがないが、`#[serde(default)]` により `ReplayConfig::default()` = Live モードとして読み込まれる。**後方互換性に問題なし**。

---

## 4. 変更箇所

### Phase 1: data crate 側 (永続化用型の追加) ✅

**ファイル: `data/src/config/state.rs`**

1. `ReplayConfig` 構造体を追加
2. `State` に `pub replay: ReplayConfig` フィールドを追加
3. `State::from_parts()` に `replay: ReplayConfig` 引数を追加

### Phase 2: 保存 (save_state_to_disk) ✅

**ファイル: `src/main.rs` — `save_state_to_disk()`**

1. `self.replay` から `ReplayConfig` を構築:
   ```rust
   let replay_cfg = data::ReplayConfig {
       mode: match self.replay.mode {
           ReplayMode::Live => "live".into(),
           ReplayMode::Replay => "replay".into(),
       },
       range_start: self.replay.range_input.start.clone(),
       range_end: self.replay.range_input.end.clone(),
   };
   ```
2. `State::from_parts()` 呼び出しに `replay_cfg` を追加

### Phase 3: 読込 (load_saved_state) ✅

**ファイル: `src/layout.rs`**

1. `SavedState` に `pub replay_config: data::ReplayConfig` を追加
2. `SavedState::default()` で `replay_config: data::ReplayConfig::default()` をセット
3. `load_saved_state()` 内で `data::State::replay` をそのまま渡す:
   ```rust
   replay_config: state.replay,
   ```

### Phase 4: 起動時の復元 ✅

**ファイル: `src/main.rs` — `Flowsurface::new()`**

1. `ReplayState::default()` の代わりに `SavedState` から復元。String→enum 変換はここの1箇所のみ:
   ```rust
   let replay_mode = match saved_state.replay_config.mode.as_str() {
       "replay" => ReplayMode::Replay,
       _ => ReplayMode::Live,
   };
   
   replay: ReplayState {
       mode: replay_mode,
       range_input: ReplayRangeInput {
           start: saved_state.replay_config.range_start,
           end: saved_state.replay_config.range_end,
       },
       playback: None,
       last_tick: None,
   },
   ```

### Phase 5: テスト ✅

1. `cargo test --bin flowsurface replay` で既存テストが通ることを確認
2. saved-state.json テストテンプレートに `replay` フィールドを追加して動作確認:
   ```json
   {
     "replay": {
       "mode": "replay",
       "range_start": "2026-04-10 09:00",
       "range_end": "2026-04-10 15:00"
     },
     "layout_manager": { ... }
   }
   ```
3. `replay` フィールドなしの既存 JSON で起動して Live モードになることを確認（後方互換）
4. E2E: 起動 → Replay モードに切替 → 日時入力 → 終了 → 再起動 → `/api/replay/status` で mode="replay" を確認

---

## 5. 変更ファイル一覧

| ファイル | 変更内容 |
|---------|---------|
| `data/src/config/state.rs` | `ReplayConfig` 追加、`State` にフィールド追加、`from_parts()` 引数追加 |
| `src/layout.rs` | `SavedState` にフィールド追加、`load_saved_state()` で変換 |
| `src/main.rs` | `save_state_to_disk()` で replay 保存、`new()` で replay 復元、`SaveState` コマンド処理追加 |
| `src/replay.rs` | `ReplayStatus` に `range_start`/`range_end` 追加、6時間制限(`MAX_REPLAY_DURATION_MS`/`RangeTooLong`)を撤廃 |
| `src/replay_api.rs` | `POST /api/app/save` ルート追加 |
| `exchange/src/adapter.rs` | `fetch_klines` にページング追加（取引所ごとの上限で自動分割取得） |

---

## 6. リスクと注意点

| リスク | 対策 |
|--------|------|
| 古い saved-state.json に `replay` キーがない | `#[serde(default)]` で安全にフォールバック。E2E テスト（後方互換テスト）で検証済み |
| Replay モードで保存→再起動すると、playback=None で UI が中途半端 | 復元時は mode=Replay + 範囲入力済みだが Paused 相当。ユーザーが Play を押して開始する形で自然 |
| `data` crate に `ReplayMode` 依存を持ち込む | String ベースの `ReplayConfig` で回避済み |
| `taskkill //f` で強制終了すると保存されない | `POST /api/app/save` で明示的に保存してから kill する（E2E テストで検証済み） |
| 長大な範囲のリプレイでメモリ圧迫 | `fetch_klines` はページング取得するが、trades バッファは全量メモリに保持。極端な範囲（数ヶ月以上）は実用的でない可能性あり |

---

## 7. 実装ログ

**2026-04-11 永続化実装完了**

- TDD アプローチで実装。data crate に 6 テスト追加（RED→GREEN 確認済み）
- 既存の replay テスト 62 件 + data crate 9 件 = 計 71 テスト全パス
- `ReplayConfig::default()` の `mode` は `"live"` (空文字ではない)。`#[serde(default)]` と組み合わせて後方互換を実現
- `src/replay.rs` に存在した `is_exhausted()` メソッドの削除（ワーキングツリー上の未コミット変更）を復元して修正

**2026-04-11 E2E テスト基盤整備 + 6時間制限撤廃**

- `ReplayStatus` に `range_start`/`range_end` フィールドを追加（永続化検証用）
- `POST /api/app/save` エンドポイントを追加（`taskkill //f` では保存されないため）
- 6時間制限（`MAX_REPLAY_DURATION_MS` / `ParseRangeError::RangeTooLong`）を撤廃
- `exchange/src/adapter.rs` の `fetch_klines` にページング追加:
  - 取引所ごとの1リクエスト上限: OKEx 300, Hyperliquid 500, Binance/Bybit/MEXC 1000
  - 範囲が上限を超える場合、自動で chunk_ms 単位に分割して結合
- ユニットテスト: 78 件 PASS（replay + replay_api）
- E2E テスト結果:
  - 永続化テスト: 11/11 PASS（復元・後方互換・往復保存）
  - 再生ライフサイクル: 20/20 PASS（Play→Pause→Resume→Speed→Step→Toggle）
  - 6時間超テスト: 5/5 PASS（12時間範囲で Loading→Playing 遷移確認）
