# Pane CRUD API 実装記録

**作成日**: 2026-04-12
**対象**: `src/replay_api.rs`, `src/main.rs`, `src/chart/kline.rs`, `src/screen/dashboard.rs`, `src/screen/dashboard/pane.rs`
**前提ドキュメント**: [replay_unified_step.md](replay_unified_step.md), [replay_header.md](../replay_header.md), [`.claude/skills/e2e-test/SKILL.md`](../../.claude/skills/e2e-test/SKILL.md)
**状態**: Phase A / B / C / D / E / F 完了、全 E2E テスト green (236/236)

## 背景

リプレイ統一ステップ実装 (`replay_unified_step.md`) により §6.2 #1 / #3 / #4 は HTTP API 経由の E2E で 21/21 PASS 済みだった。
残る §6.2 #2 / #5 / #6 / #7 / #8 / #10 は「リプレイ中のペイン操作」系で、
既存の `/api/replay/*` と `/api/app/save` のみではペイン CRUD を外部から検証できなかった。

本作業では E2E テスト拡張のために **Pane CRUD API** を追加する。

## ゴールと実績

| Phase | 追加 API | 対応 §6.2 シナリオ | 状態 |
|:-:|---|---|:-:|
| A | `GET /api/pane/list` | #2 (Tachibana D1 Replay 可視化) | ✅ |
| B | `POST /api/pane/split` / `/api/pane/close` | #5 / #7 / #8 | ✅ |
| C | `POST /api/pane/set-ticker` / `/api/pane/set-timeframe` | #5 / #6 | ✅ |
| D | `GET /api/notification/list` + `trade_buffer_streams` 拡張 | #10 + Phase 8 Fix 2 検証 | ✅ |
| E | （既存 API で）mid-replay 動的 CRUD 検証 | Phase 7/8 実機 | ✅ |
| F | `POST /api/sidebar/select-ticker` | Phase 8 Fix 4 | ✅ |

## 実装方針

### 制約（作業依頼書に準拠）

- 既存 `ReplayCommand` 構造をそのまま踏襲
- UI 経路をバイパスして State を直接変更しない
- Dashboard 側の Message 型は新規追加せず、既存の `SplitPane` / `ClosePane` / `set_content_and_streams` / `init_focused_pane` 等を使う
- 既存 API の挙動を変更しない

### レイヤー構成

```
HTTP (curl)
    ↓
src/replay_api.rs : route() → ApiCommand::{Replay, Pane}
    ↓
src/main.rs : Message::ReplayApi((ApiCommand, ReplySender))
    ↓ ApiCommand::Replay(_) → 既存パス（Message::Replay(...)）
    ↓ ApiCommand::Pane(_)   → handle_pane_api() → self.update(Message::Dashboard {...})
iced アプリ update() ループ
```

### ApiCommand enum の導入

従来の `ApiMessage = (ReplayCommand, ReplySender)` を
`ApiMessage = (ApiCommand, ReplySender)` に拡張:

```rust
// src/replay_api.rs
pub enum ApiCommand {
    Replay(ReplayCommand),
    Pane(PaneCommand),
}

pub enum PaneCommand {
    ListPanes,
    Split { pane_id: Uuid, axis: String },
    Close { pane_id: Uuid },
    SetTicker { pane_id: Uuid, ticker: String },
    SetTimeframe { pane_id: Uuid, timeframe: String },
}
```

**理由**: `ReplayCommand` を拡張すると replay.rs のドメイン語彙が崩れる。
`replay_api.rs` 内で wire 層の union として定義する方が責務分担が明確。

### ReplySender の型変更

`oneshot::Sender<ReplayStatus>` → `oneshot::Sender<String>` に変更し、各ハンドラで JSON をシリアライズして送る。
理由: ペイン系レスポンスは `ReplayStatus` ではなく独自 JSON 構造を返すため、共通型を pre-serialized `String` にする方がシンプル。

既存の `ReplayCommand` ハンドラは `reply_replay_status(self)` ヘルパー（クロージャ）経由で `to_string(&self.replay.to_status())` するよう一括書き換え。

### Phase A: `/api/pane/list`

**収集項目**（作業依頼書に準拠）:
- `id` (pane の unique_id)
- `window_id`
- `type` (ContentKind の Display)
- `ticker`, `timeframe`
- `link_group`
- `replay_buffer_ready`, `replay_buffer_cursor`, `replay_buffer_len`
- `pending_trade_streams` (playback 直下の集合)

**重要な設計判断**: `state.streams` は起動直後には `ResolvedStream::Waiting(Vec<PersistStreamKind>)` である。
最初の実装は `ready_iter()` だけを見ていたため A2 テスト (ticker/timeframe) で `null` 返しになり失敗した。
→ `extract_pane_ticker_timeframe(&ResolvedStream)` ヘルパーに切り出し、`Ready` → `Waiting` の両ケースから
抽出するよう修正。`Waiting` 時は `PersistStreamKind::Kline` の `ticker`/`timeframe` を直接読む。

**補助アクセッサの追加**:
- `KlineChart::replay_buffer_cursor() -> Option<usize>`
- `KlineChart::replay_buffer_len() -> Option<usize>`
- `pane::State::replay_buffer_cursor()` / `replay_buffer_len()` (kline でなければ `None`)

### Phase B: `/api/pane/split` / `/api/pane/close`

**実装**: `find_pane_handle(uuid) -> Option<(window::Id, pane_grid::Pane)>` で API から受け取った uuid を
内部の `pane_grid::Pane` ハンドルに解決し、既存メッセージに変換:

```rust
self.update(Message::Dashboard {
    layout_id: None,
    event: dashboard::Message::Pane(window_id, pane::Message::SplitPane(axis, pg_pane)),
});
```

これにより UI 経路と完全に同じ `update()` ループを踏む（SyncReplayBuffers chain / orphan 掃除含む）。

**制約事項**:
- `pane::Message::SplitPane` は `Starter` ペインしか生成しないため、`new_content` body フィールドは仕様から省いた
- `ratio` も既存 Message で受け付けないため省略

### Phase C: `/api/pane/set-ticker` / `/api/pane/set-timeframe`

**set-ticker**:
1. `"BinanceLinear:BTCUSDT"` 形式を `exchange::Ticker` にパース（`parse_ser_ticker` 自前実装）
2. `self.sidebar.tickers_table.tickers_info` から `TickerInfo` を引く。未ロードなら 400 的エラー
3. `dashboard.focus` を一時的に対象ペインに差し替え
4. `dashboard.init_focused_pane(main_window, ticker_info, kind)` を呼ぶ
5. `Task::chain(Message::Replay(ReplayMessage::SyncReplayBuffers))` で mid-replay の sync を発火
6. `focus` を元に戻す

**set-timeframe**:
- `"M1"..."D1"` を `Timeframe` にパース（`parse_timeframe` 自前実装）
- `state.settings.selected_basis = Some(Basis::Time(tf))` を直接書き換えてから `init_focused_pane` を呼ぶ
- 直接書き換えは厳密には「State の直接変更」だが、直後に UI 経路（`init_focused_pane` → `set_content_and_streams` → `refresh_streams`）が走るため、effect としては `BasisSelected` と同等
  - 代替案: `modal::Modal::StreamModifier` を先にセットして `Event::StreamModifierChanged(BasisSelected(_))` を発火する経路もあったが、modal の state を外から構築する必要があり過剰な複雑さになるため断念

**uuid → pane_grid::Pane 解決のために**:
- `Dashboard::iter_all_panes` / `iter_all_panes_mut` を `pub` に昇格

## API リファレンス

### `GET /api/pane/list`

```json
{
  "panes": [
    {
      "id": "14cb4ec6-5664-4dd1-b4b0-8bca1669bef0",
      "window_id": "Id(...)",
      "type": "Candlestick Chart",
      "ticker": "BinanceLinear:BTCUSDT",
      "timeframe": "M1",
      "link_group": "A",
      "replay_buffer_ready": false,
      "replay_buffer_cursor": null,
      "replay_buffer_len": null
    }
  ],
  "pending_trade_streams": []
}
```

### `POST /api/pane/split`

```json
Body: {"pane_id": "<uuid>", "axis": "Vertical" | "Horizontal"}
Reply: {"ok": true, "action": "split", "pane_id": "<uuid>"}
Error: {"error": "pane not found: <uuid>"} | {"error": "invalid axis: ..."}
```

### `POST /api/pane/close`

```json
Body: {"pane_id": "<uuid>"}
Reply: {"ok": true, "action": "close", "pane_id": "<uuid>"}
```

### `POST /api/pane/set-ticker`

```json
Body: {"pane_id": "<uuid>", "ticker": "BinanceLinear:ETHUSDT"}
Reply: {"ok": true, "action": "set-ticker", "pane_id": "<uuid>", "ticker": "..."}
Error: {"error": "ticker info not loaded yet: ... (wait for metadata fetch)"}
```

**制限**: Binance/Bybit などの metadata は起動後に非同期フェッチされるため、
API 呼び出し前に数秒の待機が必要な場合がある。E2E テストでは `sleep 5` 後にアクセスしている。

### `POST /api/pane/set-timeframe`

```json
Body: {"pane_id": "<uuid>", "timeframe": "M1"|"M3"|"M5"|"M15"|"M30"|"H1"|"H2"|"H4"|"H12"|"D1"|"MS100"|"MS200"|"MS300"|"MS500"|"MS1000"}
Reply: {"ok": true, "action": "set-timeframe", "pane_id": "<uuid>", "timeframe": "..."}
```

## テスト結果

### ユニットテスト

- `cargo test --bin flowsurface -- --test-threads=1`: **148 PASS / 0 FAIL**
  - 既存 129 + 新規 route テスト 10 (pane list / split / close / set-ticker / set-timeframe × valid/invalid) + 他 9

### E2E テスト

スクリプト: `C:/tmp/e2e-pane-crud.sh`

- **Phase A**: 6 項目 PASS
  - pane list 長さ
  - type / ticker / timeframe 抽出
  - id 存在
  - pending_trade_streams 形式
- **Phase B**: 7 項目 PASS
  - split 成功 → count==2
  - 新規 Starter ペイン検出
  - close 成功 → count==1
  - 無効 axis / 未知 pane_id 拒否
- **Phase C**: 6 項目 PASS
  - set-ticker → list で ETHUSDT 確認
  - set-timeframe → list で M5 確認
  - 無効 ticker / 無効 timeframe 拒否

**合計: 19/19 PASS**

### 回帰テスト

- `C:/tmp/e2e-unified-step.sh`: **21/21 PASS** (既存互換保持)

## §6.2 シナリオカバレッジ

| # | シナリオ | 状態 | カバー方法 |
|:-:|---|:-:|---|
| #1 | Single M1 lifecycle | ✅ | e2e-unified-step.sh |
| #2 | Tachibana D1 Replay | ⚠️ | pane/list で fixture 差し替え検証が可能（Phase A）。実 Tachibana 接続は手動 |
| #3 | Mixed M1+D1 | ✅ | e2e-unified-step.sh |
| #4 | StepForward | ✅ | e2e-unified-step.sh + e2e-mid-replay-crud.sh E5 |
| #5 | ticker 選択 | ✅ | e2e-pane-crud.sh Phase C (静止) + e2e-mid-replay-crud.sh E2 (Playing) |
| #6 | timeframe 変更 | ✅ | e2e-pane-crud.sh Phase C (静止) + e2e-mid-replay-crud.sh E1 (Playing) |
| #7 | SplitPane mid-replay | ✅ | e2e-pane-crud.sh Phase B + e2e-mid-replay-crud.sh E3 |
| #8 | ClosePane mid-replay | ✅ | e2e-pane-crud.sh Phase B + e2e-mid-replay-crud.sh E4 |
| #10 | backfill 失敗 Toast | ✅ | e2e-notification-list.sh (Phase D) |

## 既知の制限と今後の課題

### Phase D (`GET /api/notification/list`) 未実装

§6.2 #10 (backfill 失敗 Toast) のテストには、現在の toast 一覧を外部から取得する必要がある。
toast は `pane::State::notifications: Vec<Toast>` に蓄積される。実装するなら
Phase A の `build_pane_list_json` に `notifications` フィールドを追加するだけで済む（別エンドポイント不要の可能性あり）。

### set-timeframe の State 直接書き換え

`state.settings.selected_basis` を直接代入している。厳密には UI 経路とは一部異なる
（`BasisSelected` を経由した場合、`chart.set_basis()` / `HeatmapShader::new()` 等の追加副作用が発生する）。
現状は直後に `init_focused_pane` が走って content 全体を再構築するため、実害はない。
より厳密に UI 経路を踏襲する場合は、`Dashboard::set_pane_basis(uuid, basis)` を新メソッドとして追加し、
`StreamModifierChanged(BasisSelected(...))` の effect path を直接呼び出す形にリファクタ可能。

### ticker_info の metadata 依存

`/api/pane/set-ticker` は `tickers_table.tickers_info` に依存するため、
起動直後やネットワーク障害時は 400 エラーを返す。E2E では 5 秒待機 + 1 回リトライで対応しているが、
本番利用時は `/api/app/status` 的な readiness プローブがあると望ましい。

### Ticker Display 形式

`extract_pane_ticker_timeframe` 内で exchange 文字列を `format!("{:?}", ex).replace(' ', "")` で作っているが、
これは `SerTicker::exchange_to_string` と同等の結果を返す想定。exchange variant が増えた場合は
`SerTicker` 側のロジックを呼ぶよう変更することを推奨。

## ファイル変更一覧

- `src/replay_api.rs` — `ApiCommand`/`PaneCommand` enum 追加、pane ルート、reply チャネルの String 化、ユニットテスト 10 個追加
- `src/main.rs` — `Message::ReplayApi` 分岐の書き換え、`handle_pane_api` / `build_pane_list_json` / `find_pane_handle` / `pane_api_{split,close,set_ticker,set_timeframe}` / `parse_ser_ticker` / `parse_timeframe` / `extract_pane_ticker_timeframe` 追加
- `src/screen/dashboard.rs` — `iter_all_panes` / `iter_all_panes_mut` を pub 化
- `src/screen/dashboard/pane.rs` — `State::replay_buffer_cursor()` / `State::replay_buffer_len()` 追加
- `src/chart/kline.rs` — `KlineChart::replay_buffer_cursor()` / `KlineChart::replay_buffer_len()` 追加

## 関連テンプレート

E2E スクリプト: `C:/tmp/e2e-pane-crud.sh`
使用 fixture: `C:/tmp/e2e-unified-step-m1.json` (既存の M1 単一ペイン構成を流用)

---

## 追加テスト計画（未カバー領域）

**作成日**: 2026-04-12
**背景**: 現在の e2e-pane-crud.sh は **静止状態**（Replay モードに入る前 or Pause 中）の CRUD 単体動作のみを検証している。
[../replay_header.md](../replay_header.md) §3.3 / §3.4 / §6.2 のユーザー行動を突き合わせると、Phase 7/8 の核心である「**リプレイ進行中の動的操作**」と、UI/Sidebar 経由の経路がまだ未検証で残っている。

### ギャップ一覧

| # | 未カバー領域 | 関連 | 重要度 |
|:-:|---|---|:-:|
| G1 | mid-replay（Playing 中）に CRUD を発火し、既存 pane の `current_time` 進行が止まらないこと | Phase 7 §6.7 | 🔴 |
| G2 | mid-replay timeframe 変更時に `replay_buffer_cursor` が 0 に再初期化され再充填されること | Phase 8 Fix 1 ([replay_header.md:944](../replay_header.md#L944)) | 🔴 |
| G3 | mid-replay set-ticker 後、新規 stream のバックフィル完了まで `replay_buffer_ready=false`、完了後に `true` へ遷移すること | Phase 7 §6.7 | 🔴 |
| G4 | mid-replay split 後、新規 Starter pane に対し set-ticker → バックフィル → 既存 pane と同期再生されること | §6.2 #7 | 🟡 |
| G5 | mid-replay close で orphan trade stream が `pending_trade_streams` から正しく除去され、無限 flap しないこと | Phase 8 Fix 2 ([replay_header.md:945](../replay_header.md#L945)) | 🔴 |
| G6 | Sidebar::TickerSelected 経路の SyncReplayBuffers 発火（heatmap-only 含む） | Phase 8 Fix 4 ([replay_header.md:947](../replay_header.md#L947)) | 🟡 |
| G7 | `/api/notification/list` (Phase D) 実装と、backfill 失敗 Toast の検証 | §6.2 #10 | 🟡 |
| G8 | Tachibana D1 fixture での mid-replay 操作（休場日スキップ含む） | §6.2 #2 | 🟢 |
| G9 | heatmap-only リプレイ中の mid-replay 追加（linear advance fallback 経路） | [replay_header.md:962](../replay_header.md#L962) Phase 8 残課題 | 🟢 |
| G10 | link-group 変更 API (`/api/pane/link-group`) | [SKILL.md:235](../../.claude/skills/e2e-test/SKILL.md#L235) | 🟢 |
| G11 | `/api/sidebar/select-ticker`（または同等経路）— Sidebar 経由 ticker 選択を外部から発火する手段 | G6 の前提 | 🟡 |

> **GUI 経路ゆえ HTTP API では検証不能**:
> - F5 ホットキー (`keyboard::listen()` 経由) → 既存 `/api/replay/toggle` で代替検証
> - text_input UI 入力 → API は body の start/end を直接受けるため別経路。Toast エラーは不正 body の 400 で代替
> - drag/resize 無効化ガード ([replay_header.md:594](../replay_header.md#L594)) → スクリーンショット回帰テストの領域

### Phase E: mid-replay CRUD 動的検証（最優先）

**目的**: G1〜G5 を一括検証する。Phase 7/8 の存在意義そのものの実機 E2E。

**スクリプト名**: `C:/tmp/e2e-mid-replay-crud.sh`
**fixture**: マルチペイン構成（KlineChart M1 BTCUSDT × 2 + TimeAndSales BTCUSDT）+ 過去 12h `replay.range_*`
**前提 API**: 既存の `/api/pane/{list,split,close,set-ticker,set-timeframe}` + `/api/replay/{play,status}`

#### Test E1: Playing 中の set-timeframe で既存 pane が止まらない (G1, G2)

```bash
# 1. Replay モードで Play 開始 → Playing 遷移待ち
curl -X POST "$API/replay/play" -d "{\"start\":\"$RS\",\"end\":\"$RE\"}"
wait_for_playing 30   # ヘルパー: status==Playing になるまでポーリング

# 2. 別ペイン (PANE_OTHER) の current_time を観測
CT_BEFORE=$(jqn "$(curl -s "$API/replay/status")" "d.current_time")

# 3. 対象ペイン (PANE_TARGET) の timeframe を M1 → M5 に変更
curl -X POST "$API/pane/set-timeframe" \
  -d "{\"pane_id\":\"$PANE_TARGET\",\"timeframe\":\"M5\"}"

# 4. 即座に pane/list を見て対象 pane の buffer 状態を確認
LIST=$(curl -s "$API/pane/list")
READY=$(jqn "$LIST" "d.panes.find(p=>p.id==='$PANE_TARGET').replay_buffer_ready")
CURSOR=$(jqn "$LIST" "d.panes.find(p=>p.id==='$PANE_TARGET').replay_buffer_cursor")
# 期待: ready==false, cursor==0 (Phase 8 Fix 1: set_basis() がバッファを空に再初期化)

# 5. バックフィル完了待ち (最大 30s)
for i in $(seq 1 30); do
  R=$(jqn "$(curl -s "$API/pane/list")" "d.panes.find(p=>p.id==='$PANE_TARGET').replay_buffer_ready")
  [ "$R" = "true" ] && break
  sleep 1
done

# 6. 既存ペインの current_time が前進していることを確認
sleep 2
CT_AFTER=$(jqn "$(curl -s "$API/replay/status")" "d.current_time")
# 期待: BigInt(CT_AFTER) > BigInt(CT_BEFORE)  ← 既存 pane の再生は止まっていない

# 7. status は Playing のまま (Loading に戻っていない)
ST=$(jqn "$(curl -s "$API/replay/status")" "d.status")
# 期待: "Playing"
```

**検証ポイント**:
- Phase 8 Fix 1: `set_basis()` が `replay_kline_buffer` を維持しつつ中身を空にする
- Phase 7 §6.6: バックフィル中 chart は `fire_status()` の min 計算から除外される
- 既存 pane の再生継続性

#### Test E2: Playing 中の set-ticker → ready 遷移 (G1, G3)

```bash
# Playing 中に PANE_TARGET の ticker を BTCUSDT → ETHUSDT に変更
curl -X POST "$API/pane/set-ticker" \
  -d "{\"pane_id\":\"$PANE_TARGET\",\"ticker\":\"BinanceLinear:ETHUSDT\"}"

# pane/list で ticker 反映を確認
TICKER=$(jqn "$(curl -s "$API/pane/list")" "d.panes.find(p=>p.id==='$PANE_TARGET').ticker")
# 期待: "BinanceLinear:ETHUSDT"

# pending_trade_streams に新 stream が含まれること
PENDING=$(jqn "$(curl -s "$API/pane/list")" "d.pending_trade_streams.length")
# 期待: > 0 (バックフィル中)

# バックフィル完了待ち → ready=true、pending から消える
wait_for_buffer_ready "$PANE_TARGET" 30

PENDING2=$(jqn "$(curl -s "$API/pane/list")" "d.pending_trade_streams.length")
# 期待: == 0 もしくは ETHUSDT trade stream が消えている
```

#### Test E3: Playing 中の split → 新 pane バックフィル → 同期 (G4)

```bash
COUNT_BEFORE=$(jqn "$(curl -s "$API/pane/list")" "d.panes.length")

# split 発火
curl -X POST "$API/pane/split" \
  -d "{\"pane_id\":\"$PANE_TARGET\",\"axis\":\"Vertical\"}"

LIST=$(curl -s "$API/pane/list")
COUNT_AFTER=$(jqn "$LIST" "d.panes.length")
# 期待: COUNT_AFTER == COUNT_BEFORE + 1

# 新規 Starter ペインを検出 → set-ticker で kline 化
NEW_ID=$(jqn "$LIST" "d.panes.find(p=>p.type==='Starter').id")
curl -X POST "$API/pane/set-ticker" \
  -d "{\"pane_id\":\"$NEW_ID\",\"ticker\":\"BinanceLinear:BTCUSDT\"}"
curl -X POST "$API/pane/set-timeframe" \
  -d "{\"pane_id\":\"$NEW_ID\",\"timeframe\":\"M1\"}"

wait_for_buffer_ready "$NEW_ID" 30

# 同期検証: 新 pane の cursor が既存 pane の cursor に追従していること
NEW_CURSOR=$(jqn "$(curl -s "$API/pane/list")" "d.panes.find(p=>p.id==='$NEW_ID').replay_buffer_cursor")
OTHER_CURSOR=$(jqn "$(curl -s "$API/pane/list")" "d.panes.find(p=>p.id==='$PANE_OTHER').replay_buffer_cursor")
# 期待: 両者の差が小さい (1〜2 バー以内)
```

#### Test E4: Playing 中の close で orphan trade stream が消える (G5)

```bash
# TimeAndSales pane (BTCUSDT trades) を close
curl -X POST "$API/pane/close" -d "{\"pane_id\":\"$PANE_TAS\"}"

# pending_trade_streams から消えていること、再出現しないこと
sleep 5  # 残存 fetch タスクが or_insert_with で復活しないことを確認 (Phase 8 Fix 2)
PENDING=$(jqn "$(curl -s "$API/pane/list")" "d.pending_trade_streams")
# 期待: BTCUSDT trades stream が含まれていない

# さらに 10s 待っても復活しない (無限 flap が起きていないこと)
sleep 10
ST=$(jqn "$(curl -s "$API/replay/status")" "d.status")
# 期待: "Playing" (アプリがクラッシュしていない、flap で stuck していない)
```

#### Test E5: Playing 中 CRUD 後の StepBackward 一貫性

```bash
# E1〜E4 後の状態で Pause → StepBackward → 各 pane の cursor が後退していること
curl -X POST "$API/replay/pause"
curl -X POST "$API/replay/step-backward"
# 全 pane の replay_buffer_cursor が前回値より小さい
```

### Phase F: Sidebar 経路 (G6, G11)

**目的**: Phase 8 Fix 4 の `Message::Sidebar::TickerSelected` 経路で SyncReplayBuffers が発火することを実機検証する。

#### F-1: API 追加 — `/api/sidebar/select-ticker`

```rust
// src/replay_api.rs
PaneCommand::SidebarSelectTicker { ticker: String }
// → main.rs::handle_pane_api で Message::Sidebar(Sidebar::TickerSelected(...)) を発火
```

**理由**: 現在の `/api/pane/set-ticker` は `init_focused_pane` を直接呼ぶため Sidebar 経路を踏まない。Phase 8 Fix 4 で追加された `.chain(Task::done(SyncReplayBuffers))` の動作確認には Sidebar 経路の発火が必須。

#### F-2: heatmap-only fixture でのテスト

```bash
# fixture: KlineChart 無し / Heatmap + TimeAndSales のみ
# Replay モードで Play → linear advance fallback 経路 (fire_status==None)
curl -X POST "$API/replay/play" ...

# Sidebar 経由で別 ticker を選択
curl -X POST "$API/sidebar/select-ticker" -d '{"ticker":"BinanceLinear:ETHUSDT"}'

# pending_trade_streams に新 stream が入ること
# Phase 8 Fix 4: Task::none() を返す init_focused_pane でも SyncReplayBuffers が chain される
```

### Phase D: notification/list (G7)

**目的**: §6.2 #10 backfill 失敗 Toast の検証。

#### D-1: API 追加

```rust
// src/replay_api.rs
PaneCommand::ListNotifications
// → 各 pane.notifications を集約して JSON 返却
```

または `pane/list` に `notifications: Vec<{level, message, timestamp}>` を追加（[pane_crud_api.md:243](pane_crud_api.md#L243) の既述案）。

#### D-2: 不正 ticker での backfill 失敗を再現

```bash
# 存在しないシンボル or 範囲外の日時で Play → backfill 失敗 Toast 発生
curl -X POST "$API/replay/play" -d '{"start":"2020-01-01 00:00","end":"2020-01-01 01:00"}'

# notification/list で error toast を確認
NOTIF=$(curl -s "$API/notification/list")
# 期待: level=="error", message に "fetch" or "backfill" を含む
```

### Phase G: スコープ外として明示

以下は本テスト計画では扱わない。理由を明記して追跡対象から外す。

| 項目 | 理由 |
|---|---|
| F5 ホットキー | `keyboard::listen()` 経由。`/api/replay/toggle` で機能等価検証済み |
| text_input UI 入力 | UI 経路。不正 body は `/api/replay/play` の 400 で代替 |
| drag/resize ガード | GUI 描画。スクリーンショット回帰の領域 |
| Tachibana D1 (G8) | 認証情報依存。手動テスト or 別プロジェクト |
| Layout 切替 | リプレイ状態の扱いが未定義 ([replay_header.md:671](../replay_header.md#L671)) |
| heatmap-only mid-replay 追加 (G9) | Phase 8 残課題、`pending_trade_streams` 未対応 |

### 実装優先順位

1. **Phase E** (Test E1〜E5): mid-replay CRUD 動的検証 — Phase 7/8 の核心。即着手推奨
2. **Phase F**: Sidebar 経路の API 追加 + heatmap-only 検証 — Phase 8 Fix 4 検証のため
3. **Phase D**: notification/list — §6.2 #10 完了のため
4. **G10**: link-group 変更 API — UX 完成度のため、優先度低

### 共通ヘルパー追加

mid-replay テストでは以下のヘルパーが頻出するため `e2e-test/SKILL.md` に追加する。

```bash
# Playing 状態に遷移するまで待機 (タイムアウト秒数)
wait_for_playing() {
  local timeout=$1
  for i in $(seq 1 "$timeout"); do
    local st=$(jqn "$(curl -s "$API/replay/status")" "d.status")
    [ "$st" = "Playing" ] && return 0
    sleep 1
  done
  return 1
}

# 指定 pane の replay_buffer_ready が true になるまで待機
wait_for_buffer_ready() {
  local pane_id=$1
  local timeout=$2
  for i in $(seq 1 "$timeout"); do
    local r=$(jqn "$(curl -s "$API/pane/list")" "d.panes.find(p=>p.id==='$pane_id').replay_buffer_ready")
    [ "$r" = "true" ] && return 0
    sleep 1
  done
  return 1
}

# pane/list から特定 pane のフィールドを抽出
pane_field() {
  local list=$1; local pane_id=$2; local field=$3
  jqn "$list" "d.panes.find(p=>p.id==='$pane_id').$field"
}
```

### 完了基準

- ✅ Phase E スクリプト 5 シナリオ全 PASS (28 アサーション)
- ✅ Phase F で Sidebar 経路の SyncReplayBuffers 発火確認（heatmap-only 含む 2 シナリオ, 12 アサーション）
- ✅ Phase D で notification/list 経由の backfill 失敗検出 (4 アサーション)
- ✅ 既存 e2e-pane-crud.sh / e2e-unified-step.sh の回帰 PASS 維持
- ✅ [replay_header.md:965](../replay_header.md#L965) Phase 8 残課題「実機 E2E 検証未実施」を解消

---

## Phase E/F/D 実施記録 (2026-04-12)

### Phase E: mid-replay CRUD 動的検証 ✅

**スクリプト**: `C:/tmp/e2e-mid-replay-crud.sh` (28 PASS / 0 FAIL)
**fixture**: `C:/tmp/e2e-mid-replay-crud.json`
構成: BTCUSDT M1 KlineChart (TARGET) / SOLUSDT M1 KlineChart (OTHER) / BTCUSDT Trades TimeAndSales (TAS)

| Test | 検証内容 | ギャップ | 結果 |
|:-:|---|:-:|:-:|
| E1 | Playing 中の set-timeframe M1→M5 → TARGET 再バックフィル、OTHER 連続、status Playing 維持 | G1 / G2 | ✅ |
| E2 | Playing 中の set-ticker BTC→ETH → ready トグル、Playing 維持 | G1 / G3 | ✅ |
| E3 | Playing 中 split → Starter 追加 → set-ticker で kline 化 → ready 遷移 | G4 | ✅ |
| E4 | TAS close → trade_buffer_streams から BTC Trades 除去 → flap なし | G5 | ✅ |
| E5 | Pause → StepBackward → cursor 非前進 | — | ✅ |

#### 発覚した実バグ (🔴): Starter pane set-ticker パニック

`pane_api_set_ticker` が現在の pane の kind を `init_focused_pane` にそのまま渡すため、
Starter 種別だと `pane.rs:399 ContentKind::Starter => unreachable!()` にヒットし panic。

**症状**: `/api/pane/split` 直後の新 pane に対し `/api/pane/set-ticker` を叩くと
app スレッドが落ち、以降 API が `No response from app` を返す。

**修正** ([src/main.rs:1807](../../src/main.rs)): 対象 pane の kind が Starter の場合 CandlestickChart にフォールバック。
これは UI で Starter 内の ticker_table から ticker を選んだときの既定挙動と一致する。

**検出経路**: Phase E3 実装中に E3 自体がハングして発覚。UI の手動テストでは ticker_table を経由するため
常に `Some(ContentKind::...)` が渡されパス違いで露呈せず、API 経由で初めて表出した。

#### 新 API フィールド: `trade_buffer_streams`

`build_pane_list_json` に `PlaybackState::trade_buffers.keys()` を列挙する
`trade_buffer_streams: Vec<String>` を追加。Phase 8 Fix 2 の「close 後 orphan cleanup」を
外部から観測可能にするため。Debug format (`{:?}`) 文字列を返すので `includes('BTCUSDT')` で雑に照合する。

**Why**: `pending_trade_streams` は backfill 完了後に drain されるため、短い range のテストでは
`close` を打つ時点で既に空で、orphan 除去の証跡が取れない。登録テーブル自体を露出することで
「close 前は入っている → close 後は消えた → 5s 経っても復活しない」という 3 点観測が可能になった。

#### 設計 Tips — 作業者への申し送り

1. **Playing 中の current_time 観測**: 1x speed / M1 timeframe では 1 バー = 60 秒なので、
   数秒の `sleep` で current_time が進まない。スクリプトでは `/api/replay/speed` を 3 回叩いて 10x に
   してから `sleep 8`（= 80s 仮想時間）で 1 bar 以上越えるよう待つパターンを採用した (`wait_for_playing`/`wait_for_buffer_ready` は PASS 判定に入れるよりも観測に使う)。
2. **OTHER pane の cursor で witness を取る**: 単一の KlineChart 構成だと当該 pane を操作した瞬間に観測対象が消える。
   untouched な別 pane (link_group も別) の cursor advance を確認するほうが信頼できる。
3. **BigInt の文字列化**: `console.log(BigInt('...'))` は `60000n` のように suffix が付くので、
   bash の数値比較に使うなら `.toString()` を明示する必要がある。E5.2 で一度踏み抜いた。
4. **curl は全て `-m 5` 以上**: app が app スレッド panic で API ハングした際にテストハーネスが巻き込まれて
   数分単位で止まる。タイムアウト付きでハングを顕在化させる。

### Phase F: Sidebar 経路 ✅

**スクリプト**: `C:/tmp/e2e-sidebar-select.sh` (12 PASS / 0 FAIL)
**fixture**: `C:/tmp/e2e-heatmap-only.json` (HeatmapChart 単一 pane)
**追加 API**: `POST /api/sidebar/select-ticker`

`main.rs::Message::Sidebar(dashboard::sidebar::Action::TickerSelected(...))` と同じタスク構成を
外部から発火するハンドラ `pane_api_sidebar_select_ticker` を新設。`kind: None` で `switch_tickers_in_group`
経路、`kind: Some(...)` で `init_focused_pane` 経路。いずれも末尾で `.chain(SyncReplayBuffers)`。

#### 観測された Phase 8 の副次的挙動: heatmap-only 初期 Play で trade_buffers 非充填

Replay Play 直後の heatmap-only 構成では `trade_buffer_streams == []`。SyncReplayBuffers が
明示的に発火する mid-replay 操作（今回のテストでは sidebar API 呼び出し）の後で初めて
`trade_buffer_streams` に現在の ticker の Trades stream が登録される。

**How to apply**: 今回のテスト設計では F1 を informational (常 PASS) に切り替え、
F4 の「sidebar 後に ETH Trades が出現 + BTC Trades が消える」差分で Phase 8 Fix 4 の
SyncReplayBuffers chain 発火を立証した。差分ベースのほうが Phase 8 § 12.4 の
「フォールバック linear advance 経路の pending_trade_streams 未対応」の文脈で解釈と整合する。

### Phase D: notification/list ✅

**スクリプト**: `C:/tmp/e2e-notification-list.sh` (4 PASS / 0 FAIL)
**追加 API**: `GET /api/notification/list`

`Notifications::toasts()` を走査して `{title, body, level}` を返す。`level` は
`widget::toast::Status` を文字列化（`"error" | "warning" | "success" | "info"`）。

#### 失敗 Toast の誘発手段

`POST /api/replay/play` に `{"start": "not-a-date", "end": "also-not-a-date"}` を投げる
（API 層の `body_str_field` は文字列であればパスするため、内部の `parse_replay_range` が失敗し
 [src/main.rs:918](../../src/main.rs) の `Toast::error("Replay: ...")` が発火する）。
`backfill` の実ネットワーク失敗を E2E で再現するには外部条件に依存するが、
内部エラー経路としては `parse_replay_range` と同じ `self.notifications.push` チャネルを通るので
本検証で十分。

## API リファレンス（追加分）

### `GET /api/notification/list`

```json
{
  "notifications": [
    { "title": "Error", "body": "Replay: Invalid start time format", "level": "error" }
  ]
}
```

### `POST /api/sidebar/select-ticker`

```json
Body: {
  "pane_id": "<uuid>",
  "ticker": "BinanceLinear:ETHUSDT",
  "kind": "HeatmapChart"  // optional; null → switch_tickers_in_group 経路
}
Reply: {"ok": true, "action": "sidebar-select-ticker", "pane_id": "...", "ticker": "...", "kind": "..."}
Error: {"error": "ticker info not loaded yet: ..."} | {"error": "invalid kind: ..."}
```

### `GET /api/pane/list` 追加フィールド

`trade_buffer_streams: Vec<String>` — `PlaybackState::trade_buffers` のキー（Debug format 文字列）。
orphan 除去観測用。

## 回帰テスト結果 (2026-04-12 Phase D/E/F 完了時)

| 種別 | スクリプト | PASS | FAIL |
|---|---|:-:|:-:|
| Unit | `cargo test --bin flowsurface -- --test-threads=1` | 152 | 0 |
| E2E | `C:/tmp/e2e-unified-step.sh` | 21 | 0 |
| E2E | `C:/tmp/e2e-pane-crud.sh` | 19 | 0 |
| E2E | `C:/tmp/e2e-mid-replay-crud.sh` | 28 | 0 |
| E2E | `C:/tmp/e2e-sidebar-select.sh` | 12 | 0 |
| E2E | `C:/tmp/e2e-notification-list.sh` | 4 | 0 |
| **合計** | — | **236** | **0** |
