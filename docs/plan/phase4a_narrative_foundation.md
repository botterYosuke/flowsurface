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
    pub linked_order_id: Option<Uuid>, // Phase 2 の VirtualOrder.order_id と紐付け
    pub public: bool,                  // デフォルト false
    pub created_at_ms: i64,            // 実時間（監査用）
}
```

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

| メソッド | パス | ボディ/クエリ | レスポンス |
|---|---|---|---|
| `POST` | `/api/agent/narrative` | Narrative JSON（`id`・`outcome`・`created_at_ms` は省略可） | `{ "id": "<uuid>" }` 201 |
| `GET` | `/api/agent/narratives` | クエリ: `agent_id`, `ticker`, `limit`, `since_ms` | `{ "narratives": [...] }` 200 |
| `GET` | `/api/agent/narrative/:id` | — | Narrative JSON 200 / 404 |
| `POST` | `/api/agent/narrative/:id/publish` | `{ "public": true }` | `{ "id": "<uuid>", "public": true }` 200 |

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

- [ ] **A-1**: `Cargo.toml` に `rusqlite`（`bundled` feature）・`flate2`・`sha2` を追加、`data_path()` 配下に DB を開く helper を作成
  - テスト: `narrative_store::tests::opens_db_in_data_path()`
- [ ] **A-2**: `Narrative` / `SnapshotRef` / `NarrativeAction` / `NarrativeOutcome` モデル定義・serde 往復テスト
  - テスト: `narrative::model::tests::roundtrip_json()`
- [ ] **A-3**: `SnapshotStore`（ファイル書き込み層）実装
  - `write(snapshot_json) -> SnapshotRef`: gzip 圧縮 → 年月日ディレクトリ作成 → 書き込み → sha256 計算
  - `read(snapshot_ref) -> serde_json::Value`: 読み込み → sha256 検証 → 解凍
  - サイズ上限（圧縮前 10 MB / 圧縮後 2 MB）超過時のエラー
  - テスト: 一時ディレクトリで書き込み→読み戻し、sha256 不一致検出、サイズ上限
- [ ] **A-4**: `NarrativeStore::insert()` — SnapshotStore に書き出した後 SQLite INSERT（ファイル書き込み失敗時は INSERT しないアトミック性）
  - テスト: ファイル I/O 失敗を注入して SQLite に残らないことを確認
- [ ] **A-5**: `NarrativeStore::get()` / `list()`（filter: agent_id / ticker / since_ms / limit）
  - テスト: インメモリ DB（`:memory:`）で CRUD 全ケース。`list()` はスナップショット本体を読まないことも検証
- [ ] **A-6**: `NarrativeStore::load_snapshot(id)` — 明示取得（lazy load）
- [ ] **A-7**: `NarrativeStore::update_outcome()` ・ `set_public()`
- [ ] **A-8**: マイグレーション（初回起動時に CREATE TABLE IF NOT EXISTS）
- [ ] **A-9**: `NarrativeStore::gc_orphans()` — 孤児スナップショット検出（ログ出力のみ、削除はしない）
- [ ] **A-10**: `NarrativeStore::storage_stats()` — 総件数・総バイトサイズ・最大ファイルサイズを返す

### サブフェーズ B: HTTP API

- [ ] **B-1**: `POST /api/agent/narrative` — リクエストパースと store.insert 結線
  - テスト: `replay_api::tests::accepts_narrative_post()`
- [ ] **B-2**: `GET /api/agent/narratives`（agent_id / ticker / since_ms / limit フィルタ、スナップショット本体は含めない）
- [ ] **B-3**: `GET /api/agent/narrative/:id`（メタ + スナップショット本体を含む）・404 ハンドリング
- [ ] **B-4**: `GET /api/agent/narrative/:id/snapshot`（スナップショットのみを gzip 解凍済み JSON で返す）
- [ ] **B-5**: `POST /api/agent/narrative/:id/publish`
- [ ] **B-6**: `GET /api/agent/narratives/storage`（総件数・総バイトサイズ・最大ファイルサイズ）
- [ ] **B-7**: `GET /api/agent/narratives/orphans`（孤児スナップショット一覧）
- [ ] **B-8**: バリデーション（不正 JSON、confidence 範囲外、空 agent_id、スナップショットサイズ超過→413）

### サブフェーズ C: FillEvent 連携

- [ ] **C-1**: `linked_order_id` フィールドを活用し、FillEvent で outcome を自動埋め込み
  - テスト: モック NarrativeStore + FillEvent で outcome が入ることを検証
- [ ] **C-2**: `handle_virtual_order_filled()` から Narrative Store に通知するイベントバス配線

### サブフェーズ D: チャート可視化

- [ ] **D-1**: `NarrativeMarker` 構造体・Canvas への描画実装
  - ビジュアルテスト（`e2e` 内スクショ比較）
- [ ] **D-2**: リプレイの `current_time` 範囲内のナラティブのみ描画
- [ ] **D-3**: マーカー種別の色分け（buy=緑三角 / sell=赤三角）

### サブフェーズ E: Python SDK 拡張

- [ ] **E-1**: `flowsurface_sdk.Narrative` データクラス（dataclasses + `to_dict()`）
- [ ] **E-2**: `FlowsurfaceEnv.record_narrative(reasoning, confidence, ...)` ヘルパー
- [ ] **E-3**: `env.list_narratives()` / `env.publish_narrative(id)`

### サブフェーズ F: E2E テスト

- [ ] **F-1**: `tests/s33_narrative_crud.py`（POST → GET → publish のライフサイクル）
- [ ] **F-2**: `tests/s34_narrative_outcome_link.py`（注文 → 約定 → outcome 自動更新）
- [ ] **F-3**: `tests/s35_narrative_chart_overlay.py`（GUI 起動時のみ、マーカー描画確認）
- [ ] **F-4**: CI（`e2e.yml`）に headless ステップ追加（S33/S34）

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
| `linked_order_id` が Phase 2 の `VirtualOrder.order_id` と型互換でない | 結線失敗 | 初手で両者の Uuid フォーマット互換性をテスト（A-5 に含める） |
| Canvas 描画の座標変換が既存インジケーターと競合 | UI 崩れ | D-1 でビジュアルテストを先に書き、退行を検出 |

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

tests/s33_narrative_crud.py          # 新規 E2E
tests/s34_narrative_outcome_link.py
tests/s35_narrative_chart_overlay.py

Cargo.toml                           # rusqlite 追加
.github/workflows/e2e.yml            # S33/S34 headless ステップ追加
```

---

## 7. 進捗トラッキング

作業着手時にこのセクションを更新。完了項目に ✅ を付与。

- [ ] サブフェーズ A（Narrative Store）
- [ ] サブフェーズ B（HTTP API）
- [ ] サブフェーズ C（FillEvent 連携）
- [ ] サブフェーズ D（チャート可視化）
- [ ] サブフェーズ E（Python SDK 拡張）
- [ ] サブフェーズ F（E2E テスト）
- [ ] `/verification-loop` 通過
- [ ] PR 作成・CI 全 PASS

---

## 8. Open Questions（着手前に要確定）

1. ~~**SQLite 書き込みスレッド**~~ → ✅ **確定（2026-04-21）**: `Arc<tokio::sync::Mutex<Connection>>` で 1 本の接続を共有。書き込み・読み込みは `tokio::task::spawn_blocking` 内で実行し UI をブロックしない。`NarrativeStore` trait で抽象化し、将来 `r2d2_sqlite` プールへ差し替え可能にする（詳細: 3.2「並行性モデル」）
2. ~~**ナラティブの最大サイズ制限**~~ → ✅ **確定（2026-04-21）**: 別ストレージ分離方式を採用。メタは SQLite、`observation_snapshot` は `narratives/snapshots/{yyyy}/{mm}/{dd}/{uuid}.json.gz` に gzip + sha256 付きで保存。圧縮前 10 MB / 圧縮後 2 MB をハード上限、256 KB で WARN ログ
3. **マーカー表示の ON/OFF**: 常時表示か、設定でトグルできるようにするか？（サイドバー「ナラティブ表示」チェックボックス）
4. **agent_id の重複許容**: 同一 agent_id を複数ユーザーが使うケースは想定外としてよいか？（Phase 4b で `uagent_address` が一意識別子になるため、4a では緩くしてよい想定）

サブフェーズ A 着手前に残り 2 問（Q3・Q4）を確定させる。
