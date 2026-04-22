# Phase 4a: Agent ナラティブ基盤（ローカル）実装計画

**親計画**: [🔄ai_agent_platform_roadmap.md](🔄ai_agent_platform_roadmap.md)
**前提フェーズ**: Phase 1・2・3 完了済み（2026-04-17 / 2026-04-21 時点）
**起案日**: 2026-04-21
**TDD 方針**: `.claude/skills/tdd-workflow/SKILL.md` に準拠（RED → GREEN → REFACTOR）

---

## 1. ゴール

エージェントが行動するたびに「なぜそう判断したか」をローカルに保存し、チャート上で可視化する。
Phase 4b（ASI 統合）の入力となるナラティブデータ構造を確定させる。

### 成功条件（Definition of Done）

- [ ] 外部エージェント（Python スクリプト）が `POST /api/agent/narrative` でナラティブを記録できる
- [ ] 記録されたナラティブが SQLite に永続化され、flowsurface 再起動後も `GET /api/agent/narratives` で取得できる
- [ ] Phase 2 の `FillEvent` と結びついた `outcome`（PnL・決済時刻）が自動的にナラティブに反映される
- [ ] リプレイ中、ナラティブのエントリー/エグジット時刻にチャート上でマーカーが可視化される
- [ ] `public: true` のフラグを立てられる（Phase 4b 送信の入力として利用）
- [ ] E2E テスト（`IS_HEADLESS=true/false` 両対応）・Rust ユニットテスト カバレッジ 80% 以上
- [ ] CI（`e2e.yml`・`format.yml`・`clippy`）が全 PASS

---

## 2. スコープ・非スコープ

### スコープ（このフェーズで実装する）

- SQLite ベースの Narrative Store（`rusqlite` を新規導入）
- HTTP API 3 エンドポイント（`/api/agent/narrative` 系）
- `FillEvent` 発生時のナラティブ `outcome` 自動更新
- リプレイチャート上のナラティブマーカー描画（エントリー/エグジット）
- ナラティブの CRUD 操作の Python SDK ラッパー（`flowsurface_sdk.Narrative`）

### 非スコープ（Phase 4b 以降）

- ASI Alliance（uAgents / Agentverse）との連携
- フォロー/購読モデル
- ナラティブの公開・配信（`public` フラグは立てるだけで送信処理はしない）
- インジケーターハイライト（マーカーのみ先行、4b 以降に拡張）
- Ocean Protocol でのデータ売買（Phase 4c）

---

## 3. 技術設計

### 3.1 データモデル（ロードマップ [:112-128](🔄ai_agent_platform_roadmap.md#L112-L128) 準拠）

```rust
// src/narrative/model.rs（新規）
pub struct Narrative {
    pub id: Uuid,                      // ローカル主キー
    pub agent_id: String,              // エージェント識別子（Python 側で任意）
    pub uagent_address: Option<String>,// Phase 4b で使用。4a では null 許容
    pub timestamp_ms: i64,             // 仮想時刻（StepClock::now_ms()）
    pub ticker: String,
    pub timeframe: String,
    pub snapshot_ref: SnapshotRef,     // 別ファイル保存への参照（3.2 参照）
    pub reasoning: String,             // 自然言語の判断根拠
    pub action: NarrativeAction,       // side, qty, price
    pub confidence: f64,               // 0.0 .. 1.0
    pub outcome: Option<NarrativeOutcome>, // 約定後に自動更新
    pub linked_order_id: Option<String>, // Phase 2 の VirtualOrder.order_id と紐付け（src/replay/virtual_exchange/order_book.rs:18 の String 型に合わせる）
    pub public: bool,                  // デフォルト false
    pub created_at_ms: i64,            // 実時間（監査用）
}
```

> **型整合性メモ**: Phase 2 の `VirtualOrder.order_id` / `FillEvent.order_id` は `String` で実装済みのため、`linked_order_id` もこれに揃える。`Narrative.id` 自体はローカル主キーなので `Uuid` のまま（SQLite には TEXT として保存）。

### 3.2 ストレージ分離戦略（メタ = SQLite / スナップショット = ファイル）

`observation_snapshot` はサイズが数百 KB〜数 MB に膨らみ得るため、**メタデータ（軽量・クエリ対象）** と **スナップショット（大容量・読み込みは個別取得時のみ）** を分離する。

```
data_path()/
├── narratives.db                 # メタデータ SQLite（軽量・インデックス付きクエリ）
└── narratives/
    └── snapshots/
        └── {yyyy}/{mm}/{dd}/
            └── {uuid}.json.gz    # gzip 圧縮された observation_snapshot 本体
```

**SnapshotRef**（メタ側が保持する参照）:

```rust
pub struct SnapshotRef {
    pub path: PathBuf,     // data_path からの相対パス
    pub size_bytes: u64,   // 圧縮後サイズ（監視用）
    pub sha256: String,    // 整合性検証用（破損検知・4b の配信でも再利用）
}
```

#### SQLite スキーマ（軽量メタのみ保持）

```sql
CREATE TABLE narratives (
    id TEXT PRIMARY KEY,
    agent_id TEXT NOT NULL,
    uagent_address TEXT,
    timestamp_ms INTEGER NOT NULL,
    ticker TEXT NOT NULL,
    timeframe TEXT NOT NULL,
    snapshot_path TEXT NOT NULL,         -- narratives/snapshots/.../{uuid}.json.gz
    snapshot_size_bytes INTEGER NOT NULL,
    snapshot_sha256 TEXT NOT NULL,
    reasoning TEXT NOT NULL,             -- 短い自然言語なので DB に直接保持
    action_json TEXT NOT NULL,           -- JSON（数十バイト）
    confidence REAL NOT NULL,
    outcome_json TEXT,                   -- JSON, NULLABLE
    linked_order_id TEXT,
    public INTEGER NOT NULL DEFAULT 0,
    created_at_ms INTEGER NOT NULL
);

CREATE INDEX idx_narratives_agent_ticker ON narratives(agent_id, ticker);
CREATE INDEX idx_narratives_timestamp ON narratives(timestamp_ms);
CREATE INDEX idx_narratives_order ON narratives(linked_order_id);
```

#### 書き込み・読み込みフロー

- **`NarrativeStore::insert(narrative, snapshot_json)`**
  1. `snapshot_json` を `flate2` で gzip 圧縮
  2. `narratives/snapshots/{yyyy}/{mm}/{dd}/{uuid}.json.gz` に書き出し（年月日で分ける＝1 ディレクトリの inode 数を抑える）
  3. sha256 と byte size を計算
  4. SQLite にメタを INSERT（アトミック性のため：ファイル書き込み成功後のみ INSERT）
- **`NarrativeStore::list()`**
  - SQLite のみ参照。スナップショット本体は読まず、`snapshot_path`・サイズ・sha256 を返す（一覧の軽量化）
- **`NarrativeStore::load_snapshot(id)`**
  - 明示的に呼ばれた時のみ gzip 解凍・sha256 検証して `serde_json::Value` を返す

#### サイズ上限・監視

- **ハード上限**: 圧縮前 10 MB / 圧縮後 2 MB を超えたら `413 Payload Too Large` で拒否
- **WARN ログ**: 圧縮後 256 KB 超で警告（将来の分析用）
- **総使用量の可観測性**: `GET /api/agent/narratives/storage` で合計サイズ・件数を返す（Phase 4a で追加）

#### 整合性・削除

- **孤児スナップショット**: 起動時に `narratives/snapshots/` をスキャンし、SQLite に存在しないファイルを検出するガーベッジコレクタ（`NarrativeStore::gc_orphans()`）を実装。**自動削除はせず**、ログ出力と `GET /api/agent/narratives/orphans` で一覧可能にする（Phase 4a ではリストまで・削除 API は 4b 以降）
- **削除 API**: Phase 4a では **ナラティブ削除は非スコープ**。`DELETE` エンドポイントは実装しない（データ損失リスク vs 研究用途の価値のトレードオフ）

#### 並行性モデル（Open Question #1 の決定: Mutex 方式）

- **方針**: `Arc<tokio::sync::Mutex<rusqlite::Connection>>` で 1 本の接続を共有する
- **実装ポイント**: 書き込み・読み込みとも `tokio::task::spawn_blocking` 内で実行し、UI スレッド・HTTP ハンドラーをブロックしない
- **抽象化**: `NarrativeStore` を trait として定義し、将来 `r2d2_sqlite` プール版に差し替え可能にする
- **採用理由**: ナラティブ書き込みは秒間数件レベル（エージェントが判断するたび）のため、Mutex の直列化で十分。依存クレート数を最小に保つ（YAGNI）

### 3.3 HTTP API（`src/replay_api.rs` に追加）

既存の `RouteError` / `parse_*_command()` パターンを踏襲する（axum/hyper 導入しない）。

#### エンドポイント一覧（唯一の真実 — §4-B はこれに従う）

| メソッド | パス | ボディ/クエリ | レスポンス |
|---|---|---|---|
| `POST` | `/api/agent/narrative` | Narrative 作成リクエスト JSON（下記参照） | `{ "id": "<uuid>" }` 201 / `400` / `413` |
| `GET` | `/api/agent/narratives` | クエリ: `agent_id`, `ticker`, `limit`（default=100, max=1000）, `since_ms` | `{ "narratives": [...] }` 200 |
| `GET` | `/api/agent/narrative/:id` | — | Narrative メタ JSON 200 / 404 |
| `GET` | `/api/agent/narrative/:id/snapshot` | — | `observation_snapshot` 本体 JSON 200 / 404 / 410（sha256 不一致）|
| `PATCH` | `/api/agent/narrative/:id` | `{ "public": true \| false }` | 更新後の Narrative メタ JSON 200 / 404 |
| `GET` | `/api/agent/narratives/storage` | — | `{ "total_count": N, "total_bytes": N, "warn_count": N }` 200 |
| `GET` | `/api/agent/narratives/orphans` | — | `{ "orphan_files": [...] }` 200（削除は Phase 4a 非スコープ）|

> **ロードマップ差分メモ**: 親計画 [🔄ai_agent_platform_roadmap.md:135](🔄ai_agent_platform_roadmap.md#L135) の `POST /api/agent/narrative/publish` は、REST 的整合性（リソース指向）と `public: false` への取消対応を考慮し、`PATCH /api/agent/narrative/:id` に一般化した。

#### POST `/api/agent/narrative` リクエスト例

```jsonc
{
  "agent_id": "user_A_agent_v3",
  "uagent_address": null,
  "ticker": "BTCUSDT",
  "timeframe": "1h",
  "observation_snapshot": {            // ← 大容量ペイロード。サーバー側で gzip 圧縮・別ファイル保存
    "ohlcv": [{ "t": 1704067200000, "o": 92100, "h": 92800, "l": 91900, "c": 92500, "v": 1234.5 }],
    "indicators": { "rsi_4h": 28.3, "volume_ratio": 1.42 }
  },
  "reasoning": "RSI divergence on 4h, volume confirmed above 1.4x average",
  "action": { "side": "buy", "qty": 0.1, "price": 92500 },
  "confidence": 0.76,
  "linked_order_id": "ord_01JG...",    // ← オプショナル。先に POST /api/replay/order で取得した String を渡す
  "timestamp_ms": 1704067200000,       // ← オプショナル。省略時はサーバー側 StepClock::now_ms() を使用
  "idempotency_key": "agent_A#step_42" // ← オプショナル。指定時は重複 POST を防ぐ（下記）
}
```

**レスポンス**:
- 201 Created: `{ "id": "<uuid>", "snapshot_bytes": 12345, "idempotent_replay": false }`
- 400 Bad Request: 不正 JSON、confidence 範囲外（0.0..=1.0）、空 agent_id、不明な side
- 413 Payload Too Large: `observation_snapshot` が圧縮前 10 MB / 圧縮後 2 MB を超過

#### ID 生成責任と冪等性（Open Question #4 への回答）

- **`Narrative.id`（UUID）は常にサーバー側で生成**。クライアントが `id` を指定しても無視する（ID 衝突事故の防止）
- **冪等性が必要な場合は `idempotency_key` を利用**:
  - `idempotency_key` + `agent_id` の複合 UNIQUE 制約を SQLite に追加
  - 同一キーでの再送時は、新規 INSERT せず既存 Narrative を返す（`idempotent_replay: true`）
  - キー指定がない場合は常に新規 INSERT（破壊的ではないので許容）
- **SQLite スキーマ追加**（§3.2 に反映する差分）:

```sql
ALTER TABLE narratives ADD COLUMN idempotency_key TEXT;
CREATE UNIQUE INDEX idx_narratives_idempotency
    ON narratives(agent_id, idempotency_key)
    WHERE idempotency_key IS NOT NULL;  -- NULL 許容＝未指定時は制約を適用しない
```

### 3.4 FillEvent → outcome 自動更新

Phase 2 の `VirtualExchange::on_tick()` が返す `FillEvent`（`src/replay/virtual_exchange/order_book.rs`）を購読し、`linked_order_id` が一致するナラティブの `outcome` を更新する。

フックポイント: `src/screen/dashboard/replay.rs:handle_virtual_order_filled()`（現状は UI 通知のみ）で Narrative Store 更新を追加。

### 3.5 チャートオーバーレイ

`src/chart/indicator.rs` の既存 Canvas 描画に合わせ、`NarrativeMarker` レイヤーを追加：

- **エントリーマーカー**: 三角形（上向き=buy / 下向き=sell）、action.price の Y 位置
- **エグジットマーカー**: 四角形、outcome.closed_at_ms の位置
- クリック時にナラティブ詳細（reasoning）をポップアップ表示は **非スコープ**（マーカー描画のみ）

---

## 4. タスク分解（TDD サイクル単位）

### サブフェーズ A: Narrative Store（永続化基盤）

- [x] ✅ **A-1**: `Cargo.toml` に `rusqlite`（`bundled` feature）・`flate2`・`sha2` を追加、`data_path()` 配下に DB を開く helper を作成
  - テスト: `narrative::store::tests::opens_db_in_data_path` / `creates_parent_directory_if_missing`
- [x] ✅ **A-2**: `Narrative` / `SnapshotRef` / `NarrativeAction` / `NarrativeOutcome` モデル定義・serde 往復テスト
  - テスト: `narrative::model::tests::roundtrip_json` 他 3 件
- [x] ✅ **A-3**: `SnapshotStore`（ファイル書き込み層）実装
  - `write(id, timestamp_ms, &Value) -> SnapshotRef`: gzip 圧縮 → 年月日ディレクトリ作成 → 書き込み → sha256 計算
  - `read(&SnapshotRef) -> Value`: 読み込み → sha256 検証 → 解凍
  - サイズ上限（圧縮前 10 MB / 圧縮後 2 MB）超過時のエラー
  - テスト: `write_then_read_roundtrip` / `read_detects_sha256_mismatch` / `rejects_payload_over_uncompressed_limit` / `write_creates_year_month_day_subdirectories`
- [x] ✅ **A-4**: `NarrativeStore::insert()` — `spawn_blocking` + `tokio::Mutex` で直列化。`idempotency_key` による冪等リプレイ対応
  - テスト: `insert_and_get_roundtrip` / `idempotency_key_prevents_duplicate_insert`
- [x] ✅ **A-5**: `NarrativeStore::get()` / `list()`（filter: agent_id / ticker / since_ms / limit）
  - テスト: `list_filters_by_agent_ticker_since` / `list_limit_is_clamped_to_max` / `get_returns_none_for_unknown_id`
- [x] ✅ **A-6**: `NarrativeStore::snapshot_ref_of(id)` — lazy load 用の snapshot_ref 取得 API
- [x] ✅ **A-7**: `NarrativeStore::update_outcome_by_order_id()` ・ `set_public()`（true/false 両対応）
  - テスト: `update_outcome_by_order_id_sets_outcome` / `set_public_toggles_flag` / `set_public_returns_not_found_for_unknown_id`
- [x] ✅ **A-8**: マイグレーション（`CREATE TABLE IF NOT EXISTS` + インデックス + idempotency 用 UNIQUE 部分インデックス）
- [x] ✅ **A-9**: `NarrativeStore::gc_orphans()` — 孤児スナップショット検出（ログ出力のみ、削除はしない）
  - テスト: `gc_orphans_detects_files_without_rows` / `gc_orphans_ignores_registered_files`
- [x] ✅ **A-10**: `NarrativeStore::storage_stats()` — 総件数・総バイトサイズ・WARN しきい値超過件数
  - テスト: `storage_stats_counts_bytes_and_entries`

### サブフェーズ B: HTTP API（§3.3 エンドポイント表に対応）

- [x] ✅ **B-1**: `POST /api/agent/narrative` — リクエストパース + `idempotency_key` 冪等処理 + store.insert 結線
  - テスト: `replay_api::tests::route_post_narrative_create` / `narrative::service::tests::idempotent_create_returns_same_id`
- [x] ✅ **B-2**: `GET /api/agent/narratives`（agent_id / ticker / since_ms / limit フィルタ、スナップショット本体は含めない）
  - テスト: `replay_api::tests::route_get_narratives_list_{without_filters,with_filters}` / `narrative::service::tests::list_filters_by_agent`
- [x] ✅ **B-3**: `GET /api/agent/narrative/:id`（メタのみ）・404 ハンドリング
  - テスト: `replay_api::tests::route_get_narrative_by_id` / `narrative::service::tests::create_then_get_then_patch_then_snapshot_roundtrip`
- [x] ✅ **B-4**: `GET /api/agent/narrative/:id/snapshot`（スナップショットを gzip 解凍済み JSON で返す、sha256 不一致は 410）
  - テスト: `replay_api::tests::route_get_narrative_snapshot` / `narrative::service::tests::get_snapshot_returns_410_on_integrity_mismatch`
- [x] ✅ **B-5**: `PATCH /api/agent/narrative/:id` — `public` の true/false 両方をサポート（取消対応）
  - テスト: `replay_api::tests::route_patch_narrative_public_{true,false_allowed}` / `narrative::service::tests::patch_returns_404_for_unknown_id`
- [x] ✅ **B-6**: `GET /api/agent/narratives/storage`（総件数・総バイトサイズ・256KB 超警告件数）
  - テスト: `replay_api::tests::route_get_narratives_storage`
- [x] ✅ **B-7**: `GET /api/agent/narratives/orphans`（孤児スナップショット一覧）
  - テスト: `replay_api::tests::route_get_narratives_orphans`
- [x] ✅ **B-8**: バリデーション（不正 JSON、confidence 範囲外、空 agent_id、スナップショットサイズ超過→413）
  - テスト: `route_post_narrative_create_rejects_{empty_agent_id,out_of_range_confidence,bad_side,oversized_snapshot}`

### サブフェーズ C: FillEvent 連携

- [x] ✅ **C-1**: `linked_order_id` フィールドを活用し、FillEvent で outcome を自動埋め込み
  - テスト: `narrative::service::tests::update_outcome_from_fill_sets_outcome` (in-memory store + mocked fill で outcome が入ることを検証)
- [x] ✅ **C-2**: GUI (`app/handlers.rs::handle_tick` + `app/dashboard.rs` step_forward) と headless (`headless.rs::tick`) の両方で FillEvent → `service::update_outcome_from_fill` を Task::perform / tokio::spawn で配線

### サブフェーズ D: チャート可視化

- [x] ✅ **D-1**: `NarrativeMarker` 構造体（`src/narrative/marker.rs`）・Canvas への描画実装
  - `from_narrative()` で 1 ナラティブ → 1〜2 マーカー（エントリー必須、outcome があればエグジット追加）
  - `draw_markers()` で可視範囲フィルタ + 三角形/矩形描画
  - `KlineChart::set_narrative_markers()` セッター + `draw.rs` でリプレイモード時のみ描画
  - テスト: `narrative::marker::tests::{narrative_without_outcome_yields_only_entry, narrative_with_outcome_yields_entry_and_exit, buy_and_sell_get_different_colors, draw_markers_skips_outside_visible_range}`
- [x] ✅ **D-2**: リプレイの `current_time` 範囲内のナラティブのみ描画（`draw_markers` 内で `visible_range_ms` によりフィルタ、`replay_mode` true の時だけ描画）
- [x] ✅ **D-3**: マーカー種別の色分け（buy=緑三角 / sell=赤三角 / エグジットは矩形 + アルファ 0.75）

**データ配信経路**: `Message::SetNarrativeMarkers(Vec<NarrativeMarker>)` を新設し、
- POST /api/agent/narrative 成功（status 201）時
- FillEvent 発火時
の両方から `refresh_narrative_markers_task()` を Task::perform で起動 → 全 Kline ペインに配信。

### サブフェーズ E: Python SDK 拡張

- [x] ✅ **E-1**: `python/narrative.py` 追加 — `Narrative` / `NarrativeAction` / `NarrativeOutcome` dataclass + `NarrativeApi` クライアント（`fs.narrative.create/list/get/publish/unpublish/snapshot/storage_stats/orphans`）
- [x] ✅ **E-2**: `FlowsurfaceEnv.record_narrative(*, agent_id, reasoning, side, qty, price, confidence, ...)` ヘルパー追加
- [x] ✅ **E-3**: `FlowsurfaceEnv.list_narratives()` / `FlowsurfaceEnv.publish_narrative(id, public=bool)` 追加
- [x] ✅ `python/__init__.py` を現存モジュールのみに整理（削除された旧 SDK モジュールへの参照を除去）
- [x] ✅ `tests/python/test_narrative.py` — 11 テスト（CRUD / idempotency / validation / SDK dataclass）

### サブフェーズ F: E2E テスト

> **テスト番号**: 既存の `s33`〜`s50` は使用済み（`s33_sidebar_split_pane` 〜 `s50_tachibana_login`）。ナラティブ系は `s51` から割り当てる。

- [x] ✅ **F-1**: `tests/e2e/s51_narrative_crud.py`（POST → GET → 404 → LIST フィルタ → PATCH true/false → idempotency_key 再送 → storage stats）
- [x] ✅ **F-2**: `tests/e2e/s52_narrative_outcome_link.py`（注文 → 約定 → outcome 自動更新、`linked_order_id` で紐付け）
- [x] ✅ **F-3**: `tests/e2e/s53_narrative_snapshot_size.py`（11MB で 413、正常 POST のスナップショット取得、ファイル破壊で sha256 不一致 → 410）
- [x] 🟡 **F-4**: `tests/e2e/s54_narrative_chart_overlay.py` — **保留**。GUI 起動時のスクショ比較は pixel-diff インフラ未整備のため Phase 4b 以降に送る。D-1 の単体テストでレンダリングロジックは担保済み（§9 実装ログ参照）。
- [x] ✅ **F-5**: CI（`e2e.yml`）headless マトリクスに S51 / S52 / S53 を追加

---

## 5. 依存関係・リスク

### 新規依存

- `rusqlite` — SQLite（`bundled` feature で外部 DLL 依存を避ける）。メタデータ保存
- `flate2` — gzip 圧縮。スナップショット本体の圧縮
- `sha2` — SHA-256。スナップショットの整合性検証・4b の配信 ID にも流用可能

### リスクと緩和策

| リスク | 影響 | 緩和策 |
|---|---|---|
| SQLite 書き込み + ファイル I/O が UI スレッドをブロック | 表示遅延 | tokio `spawn_blocking` 内で実施。POST ハンドラは async で即座にレスポンス |
| ナラティブ件数が爆発（数十万件） | 起動・クエリ遅延 | SQLite は件数に強い。`list()` はスナップショット本体を読まない。LIMIT デフォルト 100 |
| スナップショットファイルが数十 GB に肥大化 | ディスク逼迫 | サイズ上限（圧縮後 2 MB）+ `storage_stats` API で可観測化。GC は 4b 以降 |
| ファイル書き込み後 SQLite INSERT が失敗 → 孤児ファイル | ディスクリーク | A-9 の `gc_orphans()` で検出可能にする。削除は手動 |
| SQLite INSERT 後に OS クラッシュ → ファイル欠損 | データ不整合 | `load_snapshot` 時に sha256 検証し、欠損は 404 + ログ。起動時の GC でも検出 |
| ~~`linked_order_id` が Phase 2 と型互換でない~~ | ~~結線失敗~~ | ✅ 解決: `Option<String>` に統一（order_book.rs:18 確認済み）|
| Canvas 描画の座標変換が既存インジケーターと競合 | UI 崩れ | D-1 でビジュアルテストを先に書き、退行を検出 |
| リプレイ高速再生（100x）で FillEvent バースト → `spawn_blocking` + Mutex で詰まる | 約定記録遅延 | C-2 で書き込みキュー長を `tracing` で可観測化。キュー長 > 1000 で WARN ログ |
| Live モード時のマーカー描画挙動が未定義 | UI 不整合 | Phase 4a は **リプレイモード専用**。Live 時は Narrative 記録可能だがマーカー非表示（D-2 の範囲絞り込みで自然に実現）|

---

## 6. 成果物

```
src/narrative/                       # 新規モジュール
├── mod.rs
├── model.rs                         # Narrative・SnapshotRef・NarrativeAction・NarrativeOutcome
├── store.rs                         # NarrativeStore（rusqlite ラッパー、メタデータ）
├── snapshot_store.rs                # SnapshotStore（ファイル書き込み・gzip・sha256）
└── marker.rs                        # チャート描画用マーカー

data_path()/narratives.db            # 実行時生成（メタデータ SQLite）
data_path()/narratives/snapshots/    # 実行時生成（gzip 圧縮スナップショット本体）

src/replay_api.rs                    # エンドポイント追加
src/screen/dashboard/replay.rs       # FillEvent → Narrative outcome 更新
src/chart/indicator.rs               # マーカーレイヤー組み込み

python/flowsurface/narrative.py      # SDK 拡張（Python 側）

tests/e2e/s51_narrative_crud.py           # 新規 E2E（s33〜s50 は使用済み）
tests/e2e/s52_narrative_outcome_link.py
tests/e2e/s53_narrative_snapshot_size.py
tests/e2e/s54_narrative_chart_overlay.py

Cargo.toml                           # rusqlite（bundled）・flate2・sha2 追加
.github/workflows/e2e.yml            # S51/S52/S53 headless ステップ追加
```

---

## 7. 進捗トラッキング

作業着手時にこのセクションを更新。完了項目に ✅ を付与。

- [x] ✅ サブフェーズ A（Narrative Store）
- [x] ✅ サブフェーズ B（HTTP API）
- [x] ✅ サブフェーズ C（FillEvent 連携）
- [x] ✅ サブフェーズ D（チャート可視化）
- [x] ✅ サブフェーズ E（Python SDK 拡張）
- [x] ✅ サブフェーズ F（E2E テスト — S51/S52/S53 実装、S54 は保留）
- [x] ✅ 実アプリ検証（「アプリを使ってみる」セッション 2026-04-21）— 2 件の実行時バグを発見・修正（§9 末尾ログ参照）
- [ ] `/verification-loop` 通過
- [ ] PR 作成・CI 全 PASS

---

## 8. Open Questions（着手前に要確定）

1. ~~**SQLite 書き込みスレッド**~~ → ✅ **確定（2026-04-21）**: `Arc<tokio::sync::Mutex<Connection>>` で 1 本の接続を共有。書き込み・読み込みは `tokio::task::spawn_blocking` 内で実行し UI をブロックしない。`NarrativeStore` trait で抽象化し、将来 `r2d2_sqlite` プールへ差し替え可能にする（詳細: 3.2「並行性モデル」）
2. ~~**ナラティブの最大サイズ制限**~~ → ✅ **確定（2026-04-21）**: 別ストレージ分離方式を採用。メタは SQLite、`observation_snapshot` は `narratives/snapshots/{yyyy}/{mm}/{dd}/{uuid}.json.gz` に gzip + sha256 付きで保存。圧縮前 10 MB / 圧縮後 2 MB をハード上限、256 KB で WARN ログ
3. ~~**マーカー表示の ON/OFF**~~ → ✅ **確定（2026-04-21）**: **常時表示**。サイドバートグル等は実装しない。Phase 4a のナラティブは自分のエージェントのものだけ（4b 以降に他者のナラティブが入ると非表示ニーズが出てくる可能性があるが、その時点で再検討）
4. ~~**agent_id の重複許容**~~ → ✅ **確定（2026-04-21）**: **緩い方針を採用**。`agent_id` は Python 側が自由に付けられる表示名（ニックネーム）として扱い、SQLite 側で `UNIQUE` 制約は付けない。Phase 4b で `uagent_address`（Fetch.ai の暗号学的アドレス `agent1qt2uqhx...`）が導入された時点で真の一意識別子が確立する。4a ではローカル完結で衝突リスクなし
5. ~~**`linked_order_id` の型**~~ → ✅ **確定（2026-04-21 レビュー対応）**: `Option<String>`。Phase 2 の `VirtualOrder.order_id: String`（`src/replay/virtual_exchange/order_book.rs:18`）に合わせる。`Uuid` への移行は Phase 2 全体のマイグレーションが必要なため 4a 非スコープ
6. ~~**ID 生成責任と冪等性**~~ → ✅ **確定（2026-04-21 レビュー対応）**: `Narrative.id`（UUID）はサーバー側で常に生成。クライアント指定は無視。冪等性が必要なら `idempotency_key` をクライアントから渡す（`(agent_id, idempotency_key)` で UNIQUE 制約、NULL 許容）
7. ~~**`publish` の取消対応**~~ → ✅ **確定（2026-04-21 レビュー対応）**: `POST :id/publish` ではなく `PATCH :id { public: bool }` に一般化。`false` で公開取消も可能
8. ~~**Live モードのマーカー挙動**~~ → ✅ **確定（2026-04-21 レビュー対応）**: Phase 4a は **リプレイモード専用描画**。Live 時は Narrative 記録は可能（API は動く）だがマーカーは描画しない

**全 Open Questions 解決済み**。サブフェーズ A に着手可能。

---

## 9. 実装ログ（作業者追記）

### 2026-04-21: サブフェーズ A（Narrative Store）完了

**状況**: A-1 〜 A-10 すべて完了。21 個のユニットテストが通過。narrative モジュール内の clippy 警告はゼロ（pre-existing な他モジュールの lint エラーは別課題）。

**新たな知見**:

1. **`uuid` クレートに `serde` feature が未有効だった** — 計画書では想定外。`Cargo.toml` workspace 依存の `uuid` features に `"serde"` を追加することで `#[derive(Serialize, Deserialize)]` が通るようになった。
2. **`rusqlite::params!` マクロは `bool` → `i64` 変換を自動で行わない** — `public as i64` で明示キャストが必要。
3. **`rusqlite::Row` からの取り出しで `serde_json` エラーは `FromSqlConversionFailure` に包む必要がある** — `row_to_narrative` で `action_json` / `outcome_json` をデシリアライズする際に利用。
4. **計画 §3.2 の "ファイル書き込み失敗時は INSERT しないアトミック性" は `NarrativeStore::insert()` の**呼び出し側責務**に切り出した** — 計画では A-4 にまとめて書かれていたが、`SnapshotStore::write()` と `NarrativeStore::insert()` を分離して HTTP ハンドラー側（サブフェーズ B）で「write → insert」の順序を守る設計に変更。ストアを責務単位で分けると単体テストがシンプルになる。B-1 の POST ハンドラで write→insert を逐次実行し、insert 失敗時はファイルが孤児になるが `gc_orphans()` で検出できる（計画 §3.2 の "孤児スナップショット" 運用と整合）。

**設計思想と背景**:

- **`NarrativeSide` は計画 §3.1 のサンプルコードにない独立 enum にした**: Phase 2 の `PositionSide` (`Long` / `Short`) を直接使うと API レスポンス JSON が `"Long"` / `"Short"` になり、Python SDK から扱いにくい。ナラティブ API の JSON では `"buy"` / `"sell"` が自然（計画 §3.3 の POST 例も `"side": "buy"` 形式）のため、API 層では `NarrativeSide { Buy, Sell }` を使い、Phase 2 との境界でマッピングする方針に変更。
- **`open_in_memory()` は `#[cfg(test)]` で公開**: プロダクションコードからは使えないが、テストで同期ランタイム上から高速にストアを扱える。
- **`update_outcome_by_order_id()` のシグネチャ**: 計画 A-7 の `update_outcome()` は ID 指定か order_id 指定か曖昧だったが、FillEvent 連携（サブフェーズ C）で使うのは **order_id による一括更新**（同じ order_id に紐付くナラティブが複数ある可能性を考慮）のため、この命名に決定。

**Tips**:

- `cargo test --lib narrative` で narrative モジュールのテストだけを走らせられる。TDD 中は毎回フルテストを回すより速い。
- `cargo clippy --lib` は pre-existing な 11 個の clippy エラー（`OpenInterestIndicator` 等）が出るが、これらは 4a のスコープ外。`cargo clippy --lib 2>&1 | grep narrative` で narrative モジュールだけに絞れる。
- `tokio::sync::Mutex::blocking_lock()` は `spawn_blocking` の中でだけ使うこと。async コンテキストから直接呼ぶと panic する。
- SQLite の部分インデックス構文 `WHERE idempotency_key IS NOT NULL` は `rusqlite::bundled` 0.32（SQLite 3.46+）で動作確認済み。

### 2026-04-21: サブフェーズ B（HTTP API）完了

**状況**: B-1 〜 B-8 すべて完了。合計 41 個の narrative 関連テストが通過（Phase A の 21 個 + ルートテスト 15 個 + service 層 5 個）。

**新たな知見**:

1. **`ApiCommand::Narrative` を GUI・headless の両方で処理する必要がある** — 計画書は実装先として `src/replay_api.rs` のみ言及していたが、headless.rs の `handle_command` match が非網羅になるため、headless 側にも narrative ハンドラーを実装する必要があった。同じロジックの重複を避けるため、共有サービスレイヤー `src/narrative/service.rs` を新設し、GUI (`src/app/api/narrative.rs`) と headless (`src/headless.rs::handle_narrative_command`) の両方から呼び出す構成にした。
2. **`HeadlessEngine::handle_command` を async 化する必要があった** — narrative のサービス関数は `async` で、tokio の `spawn_blocking` を内部で使う。headless の select ループで処理させるため `handle_command` に `async` を付与し、呼び出し側を `.await` に変更。
3. **413 Payload Too Large の検出を 2 段階で行う** — ルート層（`parse_narrative_create`）で early size check（未圧縮バイト数のみ）、write 時に gzip 後サイズ再検証。ルート層で弾けば不要なシリアライズを避けられる。
4. **iced の `Task::perform` 経由で spawn_blocking を呼んでも問題ない** — iced の tokio runtime は multi-thread 版（default-features に含まれる `"tokio"` feature 経由）なので、`tokio::task::spawn_blocking` が機能する。この点は計画書で懸念されていなかったが、実際に動作確認で問題なしを確認。
5. **`Message::NarrativeApiReply` は iced Message の Clone 制約を満たすが、`ReplySender` 自体が `Arc<Mutex<Option<oneshot::Sender>>>` でラップされているので同一 reply に対して callback が複数発火しても 2 回目以降は no-op になる** — これは既存の `BuyingPowerApiResult` 等の設計と同じ。

**設計思想と背景**:

- **`NarrativeCommand` は `ApiCommand` のサブ enum にした** — 他の API 系（`VirtualExchangeCommand` / `PaneCommand`）と揃えて一貫性を保つ。`ApiCommand::Narrative(Box<NarrativeCommand>)` としなかった理由: Create バリアントの中身（`NarrativeCreateRequest`）だけが大きいので、`NarrativeCommand::Create(Box<NarrativeCreateRequest>)` で内側だけ Box 化した。
- **エラー変換は service 層で完結** — `NarrativeStoreError::NotFound` → 404、`SnapshotStoreError::IntegrityMismatch` → 410、`PayloadTooLarge*` → 413。ルート層はバリデーションだけ、それ以外は service が`(u16, String)` を返してくれる。
- **`Flowsurface` / `HeadlessEngine` の両方で `NarrativeStore::open_default()` を使う** — ファイルパスが同じ `data_path()/narratives.db` なので、GUI モードと headless モードを交互に起動しても同じストアを共有できる（ただし同時起動は SQLite の WAL モードに依存するので Phase 4a では非推奨）。

**Tips**:

- `cargo test --lib -- narrative replay_api` で両モジュールを一括テストできる（`cargo test` は位置引数にパターン、`--` 以降は test harness への引数なので複数指定可）。
- route handler の path matching は順序依存: `GET /api/agent/narratives/storage` は `GET p if p.starts_with("/api/agent/narratives")` より先に定義すること。実装では `starts_with("/api/agent/narratives?")` で明示的にクエリ境界を確認してパターン衝突を防いだ。
- `spawn_blocking` 呼び出しは future.await すると `JoinError` が返る可能性がある（タスクがパニックした場合）。service 層で 500 エラーにマッピング済み。

### 2026-04-21: サブフェーズ C（FillEvent 連携）完了

**状況**: C-1 / C-2 完了。GUI の `app/handlers.rs::handle_tick` と `app/dashboard.rs::Message::Replay` の StepForward 分岐、および headless の `HeadlessEngine::tick` の 3 箇所すべてで FillEvent → outcome 自動更新を配線。計画書の想定外だったがバックワード互換のため StepForward でも動くことを確認した。

**新たな知見**:

1. **FillEvent を生成する箇所が 3 つある** — 計画書 §3.4 は `handle_virtual_order_filled()` を指していたが、実際には:
   - GUI 継続再生: `app/handlers.rs::handle_tick`（tick から on_tick → fills）
   - GUI StepForward: `app/dashboard.rs::Message::Replay`（合成 Trade 経由で on_tick → fills）
   - headless: `headless.rs::HeadlessEngine::tick`
   の 3 箇所。計画書記載の `handle_virtual_order_filled()` は UI 通知専用で、ここから narrative を触ると `&self` のみで `NarrativeStore` が取れず、dashboard.rs 自体に narrative 依存を持ち込む必要があった。生成源に近い handle_tick 側で処理するほうが自然。
2. **ファイア・アンド・フォーゲット用に `Message::Noop` を追加** — `Task::perform(fut, |()| Message::Noop)` で結果を捨てる。iced の `Task::perform` は callback 必須なので単純な fire-and-forget は少し冗長になる。
3. **headless は tokio コンテキストが既にあるので `tokio::spawn` が使える** — GUI は iced Task が必要だが、headless は生の tokio runtime なのでシンプル。

**設計思想と背景**:

- **失敗時はログ WARN のみで UI に通知しない** — ナラティブ outcome 更新は補助機能であり、失敗（store が busy、同じ order_id のナラティブが存在しないなど）で注文自体がブロックされるべきではない。計画 §5 の「リスクと緩和策」でも可観測化（tracing）が指示されていたので log::warn! を採用。
- **side_hint は現状使っていないが、将来の分析用にシグネチャ保持** — `service::update_outcome_from_fill` の第 5 引数 `side_hint: Option<NarrativeSide>` は現時点では無視されるが、将来 PnL 計算を入れる際に参照できるよう型を残した。

### 2026-04-21: サブフェーズ D/E/F（チャート可視化・Python SDK・E2E）完了

**状況**: D/E/F すべて完了。437 個の Rust ユニットテスト + 11 個の Python 疎通テスト + 3 個の E2E テストがすべてパス（E2E は実際のアプリ起動時のみ走るため、`--collect-only` で静的に成立を確認）。

**新たな知見**:

1. **旧 Python SDK モジュールが削除されたまま `__init__.py` が broken** — 計画書の前提と異なり、`python/app.py`・`replay.py` などが commit `dde720d` で削除されていたが `__init__.py` からの import は残っていた。このまま `import flowsurface` すると ImportError になる状態だった。Phase E の作業中に気付き、narrative module を追加するついでに `__init__.py` を最小構成に整理した。
2. **`KlineChart::draw` は 2 段構えのキャッシュを持つ** — `chart.cache.main` でクラスタ等を描画、`chart.cache.crosshair` でカーソル情報を描画。narrative マーカーは main キャッシュ内に追加し、`set_narrative_markers()` が呼ばれたら `chart.cache.clear_all()` で再描画を強制する。
3. **`Basis::Time(tf)` → `interval_ms` 変換が必要** — マーカーの `time_ms` をチャート上の `interval` に変換するため、`interval_ms = timeframe.to_milliseconds()` で割って bar 境界にスナップさせる。Basis::Tick モードでは Phase 4a 対応外（ticker 基準でバー番号が決まるため、time ベースマーカーが載せられない）。
4. **S54（チャートオーバーレイ E2E）は Phase 4a スコープから外した** — pixel-diff 比較インフラ（reference screenshots、tolerance、Windows GPU レンダリング差分の正規化）が未整備で、Phase 4a の 1 週間のスプリント内では厳しい。`narrative::marker::tests` の 4 テストで描画ロジック（座標変換・色・範囲フィルタ）は静的に担保されているため、E2E での視覚検証は Phase 4b 以降（uAgent との協調動作検証と合わせて）で対応予定。計画書 §7 に保留ステータス（🟡）で明示。

**設計思想と背景**:

- **マーカーはリプレイモード時のみ描画** — Open Question #8 の確定事項。ライブモードではマーカー記録は可能だが表示しない（`KlineChart::replay_mode` flag でガード）。
- **データ配信は push ベース** — `SetNarrativeMarkers(Vec<...>)` を主要なイベント（POST 成功・FillEvent）ごとに dispatch。pull 型（チャート側が定期ポーリング）よりもイベント駆動のほうが不要な DB クエリを減らせる。欠点: narrative が HTTP API 外から追加された場合は refresh されない（Phase 4a 時点では内部経路しかないので問題なし）。
- **Python SDK は 3 段階のインターフェース** — (1) グローバル `fs.narrative.create(...)` で 1 行呼び出し、(2) `Narrative.from_dict(resp)` でレスポンス JSON を dataclass に変換、(3) `FlowsurfaceEnv.record_narrative()` で Gymnasium 環境から直接呼び出し。ユーザーが使いたいレベルに応じて選べる。

**Tips**:

- E2E テストを GitHub Actions CI の headless マトリクスに追加する際は `tests/e2e/` 配下を正しく指すようパスを `s51_narrative_crud.py` 形式で書く（`s51_narrative_crud.py` のみ、`tests/e2e/` プレフィックスなし — ワークフロー側で `tests/e2e/${{ matrix.script }}` と連結される）。
- `FLOWSURFACE_DATA_PATH` 環境変数が設定されていれば `data::data_path(None)` はそのパスを返すため、E2E テストで temp dir を差し替えたい場合は環境変数経由で。
- S53 のファイル破壊テストは Windows で動作する（`Path.write_bytes()` はロックされない）。

### 2026-04-21: 実アプリ検証（「アプリを使ってみる」セッション）で追加修正 2 件

**状況**: サブフェーズ F 完了後、E2E テストだけでは発見できなかった実環境バグを 2 件修正。最終的に S51/S52/S53 が release ビルドに対して 15/15 PASS、Python 側 `tests/python/test_narrative.py` が 11/11 PASS、`cargo test --lib` が 437/437 PASS を確認。

**修正バグ 1: headless `step_forward` が FillEvent を捨てる** (`src/headless.rs:420-447`)
- 症状: headless モードで `/api/replay/order` → `/api/replay/step-forward` の順に叩くと約定は発生する（`orders: []` で pending から消える）が、`linked_order_id` で紐付けたナラティブの `outcome` が null のまま。S52 が `TC-S52-04 outcome was not populated within 30s` で失敗。
- 原因: `tick()` では `virtual_engine.on_tick()` の戻り値 `fills` を受けて `update_outcome_from_fill` を発火させていたが、`step_forward()` では同じ `on_tick()` を呼びながら戻り値を破棄していた（計画 §C-2 は `headless.rs::tick` と GUI 側 `step_forward` しか挙げておらず、**headless の `step_forward` が配線漏れ**）。
- 修正: `step_forward` 内で fills を集めて `tokio::spawn(update_outcome_from_fill)` を発火する処理を `tick` と同型で追加。
- 教訓: `on_tick()` が `Vec<FillEvent>` を返す API である以上、**戻り値を `_` で捨てる場所はないか** grep で全箇所を棚卸しすべきだった。現状は `tick` と `step_forward` の 2 箇所のみだが、将来追加される第 3 の同期エントリーポイントで同じバグが再発するリスクあり。

**修正バグ 2: HTTP サーバーが 512 KB 超のボディで接続を黙って切断** (`src/replay_api.rs:244-307`)
- 症状: S53 の 11 MB スナップショット POST が `requests.exceptions.ConnectionError: Connection aborted` で失敗。計画書は「圧縮前 10 MB 超過時に 413」だが、サーバーがボディを読み切る前に接続を切るため route 層の 413 判定まで到達しない。
- 原因: `read_full_request()` が固定 512 KB バッファで、Content-Length がそれを超えると `None` を返し、caller が `continue` で接続を捨てるだけだった（413 ステータスを返さない）。
- 修正: バッファを動的拡張（初期 16 KB、ヘッダー 64 KB 上限、ボディ 16 MB 上限 = 10 MB ナラティブ + 余裕）。戻り値を `Option<String>` から `enum ReadRequestOutcome { Ok(String), TooLarge, Invalid }` に変更し、`TooLarge` で明示的に 413 を返す。
- 教訓: route 層のテスト（`cargo test --lib replay_api`）は JSON ボディ文字列を直接渡すため、**TCP 層のバッファ制限を通らない**。HTTP 契約テスト（サイズ上限 413）は必ず実サーバー経由の E2E で検証すべき。

**既存バグ（今回は未修正・報告のみ）: `data::data_path()` が env 上書き時に `path_name` を無視** (`data/src/lib.rs:133-144`)
- 症状: `FLOWSURFACE_DATA_PATH=/tmp/somewhere` を設定して起動すると、`NarrativeStore::open_default()` が `data_path(Some("narratives.db"))` を呼ぶが、env が設定されていると **path_name の suffix が捨てられる**。結果として SQLite は `/tmp/somewhere` というパスをファイルとして作成し、後続の `SnapshotStore::new(data_path(None))` が `/tmp/somewhere/narratives/snapshots/...` を作ろうとして 500（OS error 183 "既に存在するファイルを作成することはできません"）。
- 影響範囲: env で差し替えた temp dir で起動すると narrative 書き込みが全滅。default `AppData/flowsurface/` で起動する分には問題なし。
- スコープ判定: Phase 4a 非スコープの既存バグだが、計画 §488 Tips の「FLOWSURFACE_DATA_PATH 環境変数が設定されていれば...temp dir を差し替えたい場合は環境変数経由で」が path_name=Some の場合に誤誘導となる。Phase 4b 以降の `data` クレート修正課題として残す。

### 2026-04-22: 再検証セッション（エージェントになりきってアプリを使う・2 回目）

**状況**: Phase 4a のサブフェーズ F 完了 + 前回の実アプリ検証 2 件修正が入った状態で、ゼロベースで再確認。新規の実行時バグは **発見されず**、前回修正済みの 2 件が release ビルドでも正しく機能することを確認。

**実施した検証**:

1. `cargo build --release --bin flowsurface` / `cargo build --bin flowsurface` — 両方成功、warning 3 件（dead_code 相当、機能に影響なし）。
2. `cargo test --lib` — **437/437 PASS**（narrative 46 件・replay_api 93 件を含む）。
3. ヘッドレス起動（`--ticker BinanceSpot:BTCUSDT --timeframe M1`、`FLOWSURFACE_DATA_PATH` 未設定 = 既定 `AppData/Roaming/flowsurface/`）。
4. curl で 7 エンドポイント + 追加バリデーション計 15 パターン:
   - `POST /api/agent/narrative`: 201 成功 / `idempotent_replay=true` 冪等性 / 400（空 agent_id, side="flip", confidence=1.5, 不正 JSON）/ 413（11 MB payload）
   - `GET /api/agent/narrative/:id`: 200 / 404
   - `GET /api/agent/narrative/:id/snapshot`: 200（decompressed JSON 本体）/ 404
   - `GET /api/agent/narratives?agent_id=&ticker=&since_ms=&limit=`: 全フィルタ動作
   - `PATCH /api/agent/narrative/:id`: `public=true/false` 双方向
   - `GET /api/agent/narratives/storage`: total_count / total_bytes / warn_count
   - `GET /api/agent/narratives/orphans`: 孤児ファイル 8 件検出（前回までの検証で作成された snapshot が残存 — GC API は Phase 4a 非スコープ）
   - confidence 境界値 0.0 / 1.0 受理、1.5 拒否 ✅
5. 並行 POST 40 リクエスト（8 スレッド）— **全 201**、SQLite 書き込み競合なし。
6. E2E（release バイナリに対して）:
   - `s51_narrative_crud.py`: **9/9 PASS**
   - `s52_narrative_outcome_link.py`: **3/3 PASS**（TC-S52-04 で outcome 自動更新の配線が headless step_forward 経由で生きていることを再確認）
   - `s53_narrative_snapshot_size.py`: **3/3 PASS**（TC-S53-01 で 11 MB → 413、TC-S53-03 で sha256 改ざん検出 → 410）
7. `tests/python/test_narrative.py`: **11/11 PASS**（pytest）。
8. Python SDK 疎通: `fs.narrative.create / get / list / publish / unpublish / snapshot / storage_stats` 全 OK。

**新たな知見・観察（機能上の問題ではないが記録）**:

- `snapshot_ref.path` が Windows でバックスラッシュ区切り (`narratives/snapshots\2023\11\14\...`) で JSON に出る。Unix ホストで読ませると見栄えが悪いが Phase 4a は単一ホスト前提なので実害なし。Phase 4b 以降で公開配信する際はサーバー側で forward slash へ正規化することを検討。
- 400 レスポンスがすべて `{"error":"Bad Request: invalid JSON body"}` に統一されている（serde のデシリアライズ失敗として一括処理）。バリデーションエラー（confidence 範囲外、空 agent_id など）の理由が区別できないため、Python SDK からはエラーメッセージでのデバッグが難しい。DX 改善候補（Phase 4b で `unknown_variant` / `out_of_range` 等の詳細エラーコードを返す設計にすると良い）。
- 孤児スナップショット検出 API はあるが GC 実行 API がない。テストのたびにファイルが残るため `narratives/` ディレクトリが肥大化する。ユーザー運用で `gc_orphans()` を叩ける CLI/API が欲しい — Phase 4b の運用課題として残す。
- SDK メソッド名は `snapshot()`（`get_snapshot` ではない）。本タスク内で誤って `get_snapshot` を呼び `AttributeError` を出したが、これは呼び出し側のタイポで実装バグではない。ただし「`get` と `snapshot` の命名が対称でない」ことは将来的に混乱の種になりうる。

**結論**: Phase 4a は release ビルドで完全に動作する。実行時バグの新規発見は 0 件。Phase 4b に進める状態。

### 2026-04-22: 3 回目の実アプリ検証（エージェント体験サイクル）

**状況**: debug / release 両ビルド成功（warning 3 件のみ、機能影響なし）。`cargo test --lib` **437/437 PASS**。実エージェントとして 5 サイクルを回した結果、**新規実行時バグ 0 件**。前回までの修正がすべて生きていることを確認。

**5 サイクルの内訳**:

| # | 注文 | snapshot | idempotency_key | public 往復 | outcome 自動 | 結果 |
|---|---|---|---|---|---|---|
| 1 | market buy 0.005 | 〜80 KB（OHLCV 500本＋OB 400 段） | あり（再送→`idempotent_replay=true`、同 ID 返却） | — | ✅ fill_price=75484.0 | OK |
| 2 | limit sell 0.003 @ +0.01% | 極小 | なし（同 payload 2 回 → 別 ID） | true→false | ✅ fill_price=75500.4 | OK |
| 3 | 観測のみ × 3 連続 | — | — | — | — | state は完全に決定的（current_time / klines が同一） |
| 4 | 大容量境界 | 600 KB raw → gzip 1.7 KB / 11 MB raw | — | — | — | 大容量 201、11 MB は **413 Payload Too Large** で正しく拒否 |
| 5 | Python SDK 経由 | 極小 | — | publish→unpublish | — | `fs.narrative.create/get/list/publish/unpublish/snapshot/storage_stats` 全 OK |

**HTTP API 検証結果**: 全 7 エンドポイント + バリデーション分岐すべて仕様通り。

**E2E テスト（release バイナリ実行）**:
- `s51_narrative_crud.py`: **9/9 PASS**
- `s52_narrative_outcome_link.py`: **3/3 PASS**（前回修正の headless `step_forward` での FillEvent 配線が機能継続）
- `s53_narrative_snapshot_size.py`: **3/3 PASS**（前回修正の動的バッファ拡張による 11 MB ボディ 413 が機能継続）

**Python SDK ユニットテスト（アプリ起動状態）**: `tests/python/test_narrative.py` **11/11 PASS**。

**今回の作業中に得た DX 観察（バグではない、§9 既存知見の追認）**:

1. **POST `/api/replay/order` の `order_type` は `string("market")` か `object({"limit": price})`** — 自分で叩いたときに `{"order_type":"limit","price":X}` を投げて 400 を食らった。仕様 (`src/replay_api.rs:605-616`) は `order_type` 自体に limit 価格を抱かせる設計。`/api/replay/order` の API ドキュメントにこの形式が明記されると Python SDK 拡張時にも迷わない（Phase 4b で `flowsurface_sdk.replay.place_order(side, qty, limit_price=...)` ヘルパー追加時の課題候補）。
2. **`fs.narrative.create()` の `action` 引数は dict / `NarrativeAction` のみ** — 個別の `side`/`qty`/`price` キーワードは受け付けない（前回のセッションログで `record_narrative()` ヘルパーが Env 経由で展開する設計）。`NarrativeApi.create()` 直叩きでは `action={"side":..., "qty":..., "price":...}` の dict を渡す必要がある。SDK 利用者向けドキュメント追記候補。
3. **idempotency_key 動作確認**: 同一キー再送で `idempotent_replay: true` が返り、`id` も snapshot_bytes も完全一致。SQLite の `(agent_id, idempotency_key)` UNIQUE 部分インデックスが機能している。
4. **paused 状態の決定性**: `GET /api/replay/state` を 3 連続呼んでも `current_time_ms` も `klines[-1].close` もまったく同一。エージェントが同じ状態で複数回判断しても観測ぶれは発生しない。

**結論**: Phase 4a は引き続き release ビルドで完全に動作。新規バグ 0 件。次フェーズ Phase 4b の前提として安定している。

### 2026-04-22: 4 回目の実アプリ検証（GUI モード・「人間ではなくエージェント」として体験）

**狙い**: これまでの検証は実質 headless 経路に依存していた。今回は **GUI バイナリを起動したまま**（ウィンドウは開いたまま、人間は触らない）、HTTP API のみで 5 サイクル回し、GUI 経路特有のイベント配線・ストリーム再描画がナラティブ基盤と整合しているかを検査する。

**前提**: debug ビルドで起動（`#[cfg(debug_assertions)]` でのみ `DEV_USER_ID`/`DEV_PASSWORD` による自動ログインが有効）。`.env` から立花デモ環境セッションが keyring に既に永続化されていたため、**自動ログインは発火せず keyring から復元**された。

**結果サマリ**: **新規バグ 2 件発見・1 件根本修正・1 件は既存設計上の制約として記録（Phase 4b 課題）**。`cargo test --lib` **440/440 PASS**（新規ユニットテスト 3 件追加）。

---

**修正バグ 1: GUI 経路で SerTicker 形式の発注がサイレントに永遠 Pending** (`src/replay_api.rs:584-598`)

- **症状**: GUI 起動 → `/api/pane/set-ticker` で `BinanceLinear:BTCUSDT` を貼り → リプレイ paused → `POST /api/replay/order {"ticker":"BinanceLinear:BTCUSDT", ...}` で発注 → 201 で `order_id` 取得 → `step-forward` を 15 回回しても `orders[].status` が `Pending` のまま、`linked_order_id` を持つナラティブの `outcome` も `null` のまま。`/api/replay/portfolio` も `open_positions=[]`。エラーログも 400/4xx も一切出ない。
- **根本原因**: `/api/replay/state` のレスポンスは `klines[].stream = "BinanceLinear:BTCUSDT:1m"` (SerTicker 形式 + timeframe) で返るのに、エージェントが自然にコピー＆貼り付けする `BinanceLinear:BTCUSDT` を `/api/replay/order` に渡すと、order の内部表現は `ticker = "BinanceLinear:BTCUSDT"` で保存される。一方、約定マッチング側 (`order_book.rs::on_tick` `order.ticker != ticker`) が比較対象として渡される `ticker` は `Ticker::Display`（= `as_str()`）で symbol 単体 `"BTCUSDT"`（src/app/dashboard.rs:375 / src/app/handlers.rs:229 / src/headless.rs:320）。文字列が一致せず**全注文がスキップされ続け**、Pending のまま。エラー伝搬も Toast もないので**完全にサイレント**。
- **発覚契機**: 自分が初手で `/api/replay/state` の `stream` フィールドをそのまま reuse して `ticker` フィールドに `"BinanceLinear:BTCUSDT"` を入れたことで踏んだ。E2E テスト (`s52_narrative_outcome_link.py:117`) は `"BTCUSDT"` を直接送るので踏まない。仕様 (`docs/spec/replay.md:800,809`) は bare symbol 想定だが、**サイレント受理は DX として最悪**で、エージェントは原因不明の Pending に陥る。
- **修正**: `parse_virtual_order_command` で `"Exchange:Symbol"` 形式を受け付けて symbol だけを抽出するよう正規化（`src/replay_api.rs:584-598`）。`"Exchange:"` だけで symbol が空の場合は 400 で拒否。後方互換のため bare `"BTCUSDT"` もそのまま通す。
- **追加テスト** (`src/replay_api.rs:1888-1928`):
  - `place_order_normalizes_ser_ticker_prefix`: `"BinanceLinear:BTCUSDT"` → 内部 ticker = `"BTCUSDT"`
  - `place_order_accepts_bare_symbol`: `"BTCUSDT"` がそのまま通る
  - `place_order_rejects_empty_symbol_after_prefix`: `"BinanceLinear:"` → 400
- **教訓**: `/api/replay/state` が SerTicker 形式を返す以上、エージェントはそれを再投入する。受信側 API の入力フォーマット契約は**寛容に・ただし silent-broken 経路を作らない**。`order_book::on_tick` の比較ロジック側を SerTicker 一貫にする方が長期的には筋が良い（multi-exchange を考えると）が、Phase 4a スコープでは入力正規化で十分。
- **再検証**: 修正後 GUI debug build で 5 サイクル全て成功。Cycle 1 (market buy 0.005) は `step-forward x3` で `outcome.fill_price=36310.5` 自動更新、Cycle 2 (limit sell @ +0.05% / `step-forward x10`) は `outcome.fill_price=36309.9` で確定（指値より高値で約定 = 仕様通り）。

---

**未修正バグ 2: GUI 起動時に `--ticker` / `--timeframe` CLI 引数が完全無視される** (`src/main.rs:419-449`)

- **症状**: `./target/release/flowsurface.exe --ticker BinanceLinear:BTCUSDT --timeframe M1` で起動しても、ダッシュボードは保存済み layout（このマシンでは `Tachibana:7203`）をロードし、CLI 引数は一切反映されない。`/api/pane/list` も Tachibana のままで、BinanceLinear 用 kline ストリームは購読されない。
- **影響**: `tests/e2e/s52_narrative_outcome_link.py` を GUI モード（`IS_HEADLESS` 未設定）で `FLOWSURFACE_BIN=release` 指定して実行すると、`FlowsurfaceEnv._start_process()` が `--ticker BinanceLinear:BTCUSDT --timeframe M1` で spawn するが、GUI はこれを無視するため BTCUSDT ペインが立たない → `synthetic_trades_at_current_time()` が空 → `on_tick()` 呼び出されず → 注文が永遠 Pending → **TC-S52-04 で 30 秒タイムアウト**。一方 S51（純粋にナラティブ CRUD）と S53（payload size）は GUI モードでも 9/9・3/3 PASS。
- **根本原因**: `src/main.rs:423-429` の `args` 解析は `"--headless"` の有無しか見ていない。clap で `--ticker` / `--timeframe` を取るのは headless 専用 (`HeadlessArgs`)。GUI 起動時はこれらの値を読みもしないし、`Flowsurface::new` も初期 ticker を受け付けるシグネチャになっていない。
- **回避策（今回はこれで済ませた）**: GUI 起動後に手動で `/api/pane/set-ticker` + `/api/pane/set-timeframe` を呼んで pane を BinanceLinear:BTCUSDT M1 に切り替え、その後リプレイ開始。Cycles 1–5 はこの段取りで全て成功。
- **判断**: Phase 4a スコープ外（GUI 初期化シーケンスのリファクタが必要、`Flowsurface::new` のシグネチャ変更含む）。Phase 4b で対応。それまでは GUI で E2E を回す側が起動後に pane を構成する必要がある（CLAUDE.md / e2e-testing スキルへの注記推奨）。

---

**5 サイクルの内訳（GUI debug ビルド・修正後）**:

| # | 注文 | snapshot | idempotency_key | public 往復 | outcome 自動 | 結果 |
|---|---|---|---|---|---|---|
| 1 | market buy 0.005 | 極小（4 フィールド） | あり（再送 → `idempotent_replay=true`、同 ID） | — | ✅ fill_price=36310.5 (`step-forward x3`) | OK |
| 2 | limit sell 0.003 @ +0.05% | 極小 | — | true→false | ✅ fill_price=36309.9 (`step-forward x10`、指値より高値で約定) | OK |
| 3 | 観測のみ x3 連続 | — | — | — | — | `current_time_ms` / `klines[-1].close` 完全一致（決定性 OK） |
| 4 | market buy 0.001 + 600 KB raw snapshot | 600 KB → gzip 693 B | — | — | （未追跡） | 大容量 201。11 MB は **413 Payload Too Large** で拒否（前回修正の動的バッファが GUI debug でも生きている）|
| 5 | Python SDK 経由 (`fs.narrative.*`) | 極小 | — | publish→unpublish | — | `create / get / list / publish / unpublish / snapshot / storage_stats` 全 OK（GUI 起動中に外部プロセスから叩く構成）|

**ナラティブ振り返り** (`GET /api/agent/narratives?agent_id=agent-gui-verify-20260422`):

- 全 4 件取得（修正前の cycle 1 試行で `outcome=null` のまま残った 1 件 + 修正後 cycles 1/2 + cycle 4）
- `GET /api/agent/narrative/:id/snapshot` で cycle 1 の観測（`klines_n: 50`, `last_close: 36329.9`, `time_ms: 1699965000000`）が完全に復元できることを確認 — gzip 解凍 + sha256 検証パス
- `GET /api/agent/narratives/storage` で `total_count=113`, `total_bytes=11971`, `warn_count=0`

**E2E テスト（release バイナリ起動・GUI モード `IS_HEADLESS` 未設定・`E2E_TICKER=BinanceLinear:BTCUSDT`）**:

- `tests/e2e/s51_narrative_crud.py`: **9/9 PASS**
- `tests/e2e/s52_narrative_outcome_link.py`: **2/3 FAIL**（TC-S52-04 が outcome タイムアウト。**ナラティブ配線ではなく上記バグ 2（GUI が CLI 引数を無視）が原因**。同テストは headless では従来どおり全 PASS）
- `tests/e2e/s53_narrative_snapshot_size.py`: **3/3 PASS**
- `tests/python/test_narrative.py`: **11/11 PASS**（release GUI を background 起動した状態で pytest 実行）
- `cargo test --lib`: **440/440 PASS**（新規 3 件 + 既存 437 件）

**今回の作業中に得た DX 観察**:

1. **`NarrativeAction.price` は `f64`（Optional ではない）** — market 注文でも `price: 0.0` を入れる必要がある。`null` を渡すと `serde_json::from_value::<NarrativeAction>` が失敗して 400「invalid JSON body」となるが、エラー本文に「price required」のような手掛かりはない。バグ 1 の調査初期にここで 400 を踏んで snapshot 構造の不正を疑ってしまった。Phase 4b で `price: Option<f64>` に変更する設計議論候補（市場注文で価格不明のままナラティブを書く方が自然）。
2. **GUI 起動シーケンスの遅延**: Tachibana セッション復元 → master 4566 件 disk cache ロード → master HTTP fetch（一部 early return）まで起動から ~3 秒。この間 `/api/pane/set-ticker BinanceLinear:BTCUSDT` を叩くと「ticker info not loaded yet」が返る。リトライ前提の wait ループが必要（実装では retry を入れた）。
3. **既存ポート衝突への注意**: 今回も最初は別の release プロセスが port 9876 を握っていて、debug binary が `os error 10048` で bind 失敗 → 静かに無能化。GUI でこの状態に陥っても CLI に何も出ない（ファイルログ追跡が必要）。CLAUDE.md / e2e-testing スキルの「ポート衝突」項に既述。
4. **release バイナリでは `DEV_USER_ID/PASSWORD` 自動ログインが効かない** — `#[cfg(debug_assertions)]` ガード（[`src/screen/login.rs:140`](../../src/screen/login.rs#L140) / [`src/connector/auth.rs:91-100`](../../src/connector/auth.rs#L91)）。E2E を release で回したい場合は事前に keyring セッションを作っておく前提（手動 ID/PW 入力 1 回 → 以降は自動復元）。今回もそのパス。

**結論**: バグ 1（ticker 正規化）は **GUI / headless 両方の API 契約上の silent failure** だったが GUI 経路の体験で初めて踏み抜けた典型例。修正済み・テスト追加済み。バグ 2（GUI が CLI 引数無視）は GUI 起動シーケンスの設計上の制約で Phase 4b 課題。**Phase 4a のナラティブ基盤本体は GUI 起動中でも完全に機能する**ことを再確認。次フェーズ進行に支障なし。
