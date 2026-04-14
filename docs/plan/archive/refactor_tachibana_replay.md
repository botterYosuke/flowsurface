# Tachibana + Replay リファクタリング計画書

**作成日**: 2026-04-12
**対象**: `docs/tachibana_spec.md`, `docs/replay_header.md` で完了した 2 機能のソースコード整理
**前提ドキュメント**: `docs/tachibana_spec.md`, `docs/replay_header.md`, `docs/plan/tachibana_replay.md`
**状態**: 未着手

---

## 0. この計画の立ち位置

立花証券 API 統合（Phase 完了、テスト 118 件）とリプレイヘッダー機能（Phase 1–5 完了）の実装中に発生した **構造的な負債** を、機能追加ではなく整理として解消する。機能追加や挙動変更は含まない（リファクタのみ）。

**ゴール**:
- `src/main.rs` の肥大化を抑え、リプレイ系ロジックを `src/replay.rs` 側に寄せる
- `tachibana.rs` のグローバル static / デッドコードを削減し、テスト独立性を高める
- マジックナンバーと String 時刻表現を型/定数で統一する

**非ゴール**:
- 新機能追加（fetch_ticker_stats、自動再ログイン、WS 実装 等）
- Phase 4.10（Shift-JIS `}` 境界バグ）の解消
- 仕様書に書かれている未対応課題（`docs/tachibana_spec.md` §7）の消化

---

## 1. 現状の負債サマリー

| # | カテゴリ | 症状 | 影響 |
|:-:|---------|------|------|
| 1 | main.rs 肥大化 | `Message::Replay` / `Message::ReplayApi` ハンドラが約 380 行 | 可読性・変更コスト |
| 2 | ネスト深度 | `StepForward` 分岐が 5 段ネスト（D1 / 非D1 / borrow 回避） | バグ混入リスク |
| 3 | Tachibana static | `ISSUE_MASTER_CACHE` / `EVENT_HTTP_URL` / `EVENT_WS_URL` がグローバル | テスト並列不可・層境界違反 |
| 4 | クロスクレート副作用 | `connector/auth.rs::store_session` が exchange crate の static を書く | 依存方向の逆転 |
| 5 | デッドコード | `spawn_init_issue_master`, `set_event_ws_url`, `EVENT_WS_URL`, `#[allow(dead_code)] clear_session` | 認知負荷 |
| 6 | `.unwrap()` 散在 | `tachibana.rs` 本番パスに `serde_json::to_string().unwrap()` | クラッシュ経路 |
| 7 | 命名の不統一 | `rebuild_content_for_replay` / `rebuild_for_live` / `rebuild_for_step_backward` | 読解コスト |
| 8 | String 時刻表現 | `ReplayRangeInput` / `ReplayMessage::Play` / `ReplayStatus` で重複 | 型安全性欠如 |
| 9 | マジックナンバー | `450`（kline backfill）、`9876`（API port）、`8192`（HTTP buf）等 | 散在と不整合リスク |
| 10 | 速度テーブル分離 | `SPEEDS = [1.0, 2.0, 5.0, 10.0]` と `speed_label()` の `format!` が別々 | 不整合リスク |
| 11 | match 重複 | `ingest_trades()` の Content 別 match × 5 ペイン種 | DRY 違反 |
| 12 | ヘルパ不在 | `iter_all_panes_mut` + kline 操作パターンが 4 箇所で反復 | 修正漏れリスク |

---

## 2. リファクタリング方針

### 2.1 優先順位の付け方

1. **High**: 追加機能のしやすさに直結するもの（main.rs スリム化、Tachibana static 撤去）
2. **Medium**: 一貫性・型安全性（命名統一、String → u64、定数化）
3. **Low**: 表層的なきれいさ（マジックナンバー、デッドコード削除、JSON 抽出ヘルパ）

High を優先して取り組み、Medium/Low は余力で拾う。

### 2.2 安全に進めるための原則

- **テストを先に走らせる**: 現状 118 件パスを基線として、各 Phase 後に全テストパスを確認してから次へ
- **ビルド + 実機起動確認**: TachibanaSession は keyring 経由で永続化されるため、リファクタ後に実ログインで validate
- **1 Phase = 1 コミット**: PR レビュー時に戻しやすいよう、Phase 単位で commit を分ける
- **挙動変更は一切行わない**: 出力（JSON レスポンス、UI、WS 再接続）がバイト単位で同じになることを確認

---

## 3. Phase 1: main.rs スリム化（High）

**目的**: `Message::Replay` / `Message::ReplayApi` ハンドラを `src/replay.rs` に寄せ、`main.rs` はルーティングのみにする。

### 3.1 抽出先の新 API

```rust
// src/replay.rs
impl ReplayState {
    /// ReplayMessage を 1 つ処理して必要な副作用（Task）と新しい status を返す。
    pub fn apply(
        &mut self,
        message: ReplayMessage,
        dashboard: &mut Dashboard,
        // 必要なら fetcher / timezone 等も引数で受ける
    ) -> ReplayApplyOutcome;
}

pub struct ReplayApplyOutcome {
    pub task: Task<Message>,
    pub status: ReplayStatus,
    pub notifications: Vec<Notification>,
}
```

### 3.2 ステップ

| Step | 内容 | ファイル |
|:-:|------|---------|
| 1-1 | `ReplayApplyOutcome` 型を `src/replay.rs` に追加 | `src/replay.rs` |
| 1-2 | `ReplayState::apply()` を実装し、既存 `Message::Replay` ハンドラの各アームを 1 つずつ移植 | `src/replay.rs` |
| 1-3 | `main.rs::update()` の `Message::Replay(msg)` を `self.replay.apply(msg, &mut self.dashboards[...])` の 1 行に置き換え | `src/main.rs` |
| 1-4 | `Message::ReplayApi` ハンドラから `self.update(Message::Replay(...))` 再帰呼び出しを `ReplayState::apply` 直接呼び出しに変更 | `src/main.rs` |
| 1-5 | `replay_step_forward()` / `replay_step_backward()` / `replay_play()` を `ReplayState` のメソッドとして分離し、ネスト 5 段を 2-3 段に抑える | `src/replay.rs` |
| 1-6 | 既存テスト（16 件 + E2E 18 件）がパスすることを確認 | — |

### 3.3 判断が要るポイント

- `ReplayState::apply()` は `&mut Dashboard` を要求するため、`Flowsurface` 側から借用順序を整理する必要あり（すでに borrow checker 回避コメントが散在しているため、ここで整理する）
- `Task<Message>` が `ReplayState` 側で構築されるので、`Message` 型への依存が replay.rs に入る。これは既に `ReplayMessage` で入っているため問題なし
- 再帰 `self.update()` をやめると `ReplayApi` → `Replay` の dispatch 層が消え、JSON レスポンス組み立てロジックが明示的になる

### 3.4 検証

- `cargo test -p flowsurface -p exchange -p data` 全パス（118 件基線維持）
- 実機: `cargo run --release` → Live / Replay トグル、Play、Pause、StepForward、StepBackward、CycleSpeed の UI 動作確認
- E2E: `curl` での API 操作で JSON レスポンスが変化していないこと

---

## 4. Phase 2: Tachibana static 撤去 + クロスクレート副作用除去（High）

**目的**: `ISSUE_MASTER_CACHE` / `EVENT_HTTP_URL` / `EVENT_WS_URL` を struct フィールドに昇格し、`connector/auth.rs::store_session` から副作用を除去する。

### 4.1 方針

- **マスタキャッシュ**: `TachibanaSession` が保持する必要はない（4207 件、21MB）。`Flowsurface` 側で `tachibana_master: Option<Arc<Vec<MasterRecord>>>` として持つか、`ExchangeState` 相当の構造体に集約する
- **EVENT_HTTP_URL**: 既に `TachibanaSession.s_url_event` にあるため、`connect_event_stream()` の引数として session を受け取るよう変更
- **EVENT_WS_URL**: 仕様書 §4.1 より WS は 400 エラー確定 → 関連コード（`set_event_ws_url`, `EVENT_WS_URL`, `s_url_event_ws_fallback` 周辺）を **削除**

### 4.2 ステップ

| Step | 内容 | ファイル |
|:-:|------|---------|
| 2-1 | `EVENT_WS_URL` / `set_event_ws_url` / WS 関連デッドコードを削除 | `exchange/src/adapter/tachibana.rs` |
| 2-2 | `connect_event_stream(ticker_info, push_freq)` の引数に `session: &TachibanaSession` を追加し、`EVENT_HTTP_URL` 参照を session field に置換 | `exchange/src/adapter/tachibana.rs`, `exchange/src/connect.rs` |
| 2-3 | `EVENT_HTTP_URL` static + `set_event_http_url()` を削除 | `exchange/src/adapter/tachibana.rs` |
| 2-4 | `connector/auth.rs::store_session` から `set_event_*_url()` 呼び出しを削除 | `src/connector/auth.rs` |
| 2-5 | `ISSUE_MASTER_CACHE` を削除し、`cached_ticker_metadata()` に `&[MasterRecord]` を引数で渡す形へ | `exchange/src/adapter/tachibana.rs` |
| 2-6 | `Flowsurface` に `tachibana_master: Option<Arc<Vec<MasterRecord>>>` を持たせ、`start_master_download` で格納 | `src/main.rs` |
| 2-7 | `spawn_init_issue_master` 削除（仕様書 §7 #9 のデッドコード） | `exchange/src/adapter/tachibana.rs` |
| 2-8 | 既存テスト全パス。`ISSUE_MASTER_CACHE` を触っていたテストは引数渡しに変更 | — |

### 4.3 判断が要るポイント

- `connect_event_stream` は `impl Stream` を返すため、session を move で取り込む必要がある。`Arc<TachibanaSession>` にするか、必要 field だけ `String` で取り出して渡す
- マスタキャッシュを `Flowsurface` に持たせると `fetch_ticker_metadata(Tachibana)` が context を要求する（現在は free function）。これは `Exchange` trait の責務論と絡むため、**現時点では Flowsurface → exchange に参照を渡す関数シグネチャに留める**（trait 変更は非スコープ）

### 4.4 検証

- `cargo test -p exchange`: Tachibana 95 件パス
- 実機: ログイン → マスタ DL → 銘柄検索「7203」→ トヨタ表示 → 板受信 → 日足表示
- keyring 復元シナリオ: 一度起動 → 終了 → 再起動で session 復元 → 上記が動く

---

## 5. Phase 3: 命名統一・型設計（Medium）

### 5.1 rebuild 系 API の統合

`rebuild_content_for_replay` / `rebuild_for_live` / `rebuild_for_step_backward` の 3 つを `rebuild_content(RebuildReason)` 1 関数に統合。

```rust
pub enum RebuildReason {
    EnterReplay,          // バッファクリア
    StepBackward,         // ReplayKlineBuffer 保持、cursor リセット
    ExitReplay,           // ライブ復帰
}
```

| Step | 内容 | ファイル |
|:-:|------|---------|
| 3-1 | `RebuildReason` enum を `src/screen/dashboard/pane.rs` に追加 | `src/screen/dashboard/pane.rs` |
| 3-2 | `rebuild_content(reason)` を実装し、3 つの旧関数を wrap → 呼び出し元を一括置換 → 旧関数削除 | `src/screen/dashboard.rs`, `src/screen/dashboard/pane.rs` |
| 3-3 | `KlineChart::enable_replay_mode()` を `rebuild_content(EnterReplay)` 内に寄せて呼び出しポイントを 1 箇所に | `src/chart/kline.rs` |

### 5.2 String 時刻表現の統一

`ReplayRangeInput` は UI 入力テキストのため String 維持。一方 `ReplayMessage::Play { start: String, end: String }` は `ReplayMessage::Play { range: ReplayRange }` に変更し、パースは呼び出し側（UI または API）で行う。

| Step | 内容 | ファイル |
|:-:|------|---------|
| 3-4 | `ReplayRange { start_ms: u64, end_ms: u64 }` 型を追加 | `src/replay.rs` |
| 3-5 | `ReplayMessage::Play` のペイロードを `ReplayRange` に変更 | `src/replay.rs`, `src/main.rs` |
| 3-6 | `replay_api.rs` の `/api/replay/play` ハンドラで JSON → `ReplayRange` にパース | `src/replay_api.rs` |

### 5.3 速度テーブル統合

```rust
const SPEEDS: &[(f64, &str)] = &[(1.0, "1x"), (2.0, "2x"), (5.0, "5x"), (10.0, "10x")];
```

`PlaybackState::speed_label()` の `format!` を撤廃し、テーブル lookup に統一。

---

## 6. Phase 4: マジックナンバー・定数化（Low）

| 定数 | 値 | 現在の出現箇所 | 置き場所 |
|------|----|---------------|---------|
| `KLINE_REPLAY_BACKFILL_BARS` | `450` | `main.rs:836, 842`, `chart/kline.rs:446` | `src/replay.rs` |
| `DEFAULT_REPLAY_API_PORT` | `9876` | `replay_api.rs:39` + E2E テスト | `src/replay_api.rs` |
| `MAX_HTTP_REQUEST_BYTES` | `8192` | `replay_api.rs:72` | `src/replay_api.rs` |
| `REPLAY_API_BIND_ADDR` | `"127.0.0.1"` | `replay_api.rs` | `src/replay_api.rs` |

**追加作業**:
- `replay_api.rs` の JSON body 抽出を `extract_string(&Value, key) -> Result<String, RouteError>` ヘルパー関数に統一
- HTTP body 上限超過時に `413 Payload Too Large` を返すよう改修（現状は上限超過を無視）

---

## 7. Phase 5: ヘルパ関数抽出（Low）

### 7.1 Kline バッファ操作ヘルパ

`dashboard.rs` の `replay_advance_klines` / `replay_next_kline_time` / `replay_prev_kline_time` / `rebuild_for_step_backward` で、`iter_all_panes_mut` + Content::Kline マッチが反復している。

```rust
impl Dashboard {
    fn for_each_kline_chart_mut<F: FnMut(&mut KlineChart)>(&mut self, mut f: F) { ... }
}
```

### 7.2 Trade ingest の match 縮約

`ingest_trades()` の Content 列挙子ごとの match arms を縮約する。`trait TradeIngest { fn insert_trades(&mut self, trades: &[Trade], latest_t: u64); }` を `Heatmap/ShaderHeatmap/Kline/TimeAndSales/Ladder` に実装し、match を 1 行化。

### 7.3 .unwrap() 撲滅

`tachibana.rs` 実装パス（非テスト）の `serde_json::to_string().unwrap()` を `Result<_, TachibanaError>` 経由で伝播。テストコードの `.unwrap()` は対象外。

---

## 8. Phase 別の見積もり・リスク

| Phase | 見積もり | リスク | ブロッカー |
|:-:|:-:|------|------|
| 1 | 中（1-2 日） | borrow checker との戦い。`ReplayState::apply(&mut Dashboard)` の借用順序 | 無 |
| 2 | 中（1 日） | keyring 復元と session URL の整合性。Tachibana 実機テストが必要 | 立花証券デモ口座ログイン |
| 3 | 小（半日） | E2E JSON レスポンスフォーマット変更の波及 | 無 |
| 4 | 小（半日） | E2E テストが `9876` をハードコードしている場合の更新 | 無 |
| 5 | 小（半日） | `trait TradeIngest` 追加で既存 Content の再 import が波及する可能性 | 無 |

**全体の前提**: 現状 118 件テストが緑。`cargo test --all --release` を Phase 完了ごとに実行。

---

## 9. スコープ外

| 項目 | 理由 |
|------|------|
| Shift-JIS `}` 境界バグ修正 | 仕様書 §4.10 未解決課題。機能追加扱い |
| `fetch_ticker_stats` 実装 | 未実装機能の追加 |
| 自動再ログイン | 仕様書 §7 #2 未実装機能 |
| 旧取引所アダプター削除 | 仕様書 §7 #10。別 PR で扱う |
| `Exchange` trait の責務見直し | Tachibana の free function 寄せの原因だが、trait 変更は波及が大きい |
| インジケータ再計算、Depth リプレイ | `replay_header.md` §7 スコープ外 |

---

## 10. 参照

| 文書 | 参照箇所 |
|------|---------|
| `docs/tachibana_spec.md` | §4.1（WS 400）, §4.9（銘柄検索）, §7（未対応課題）, §9（変更ファイル一覧） |
| `docs/replay_header.md` | §3.1（ReplayState 定義）, §3.4（データフロー）, §3.5（subscription）, §9（Phase 5 API） |
| `docs/plan/tachibana_replay.md` | 立花証券リプレイ対応 Phase 1-3 の前提 |
| `exchange/src/adapter/tachibana.rs` | Static 群、`connect_event_stream`, `ISSUE_MASTER_CACHE` |
| `src/main.rs` | `Message::Replay` / `ReplayApi` ハンドラ、`subscription()` |
| `src/replay.rs` | `ReplayState` / `PlaybackState` / `ReplayMessage` / `ReplayStatus` |
| `src/replay_api.rs` | HTTP サーバー、JSON 抽出、定数 |
| `src/screen/dashboard.rs` | `prepare_replay` / `rebuild_for_*` / `ingest_trades` / `replay_advance_klines` |
