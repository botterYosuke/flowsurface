# Agent ナラティブ基盤 仕様書（Phase 4a）

> **関連ドキュメント**:
> | 知りたいこと | 参照先 |
> |---|---|
> | HTTP API 全般・リプレイ制御 | [replay.md §11](replay.md#11-http-制御-api) |
> | 仮想約定エンジン（`VirtualOrder` / `FillEvent`）| [order.md §7](order.md#7-仮想約定エンジン) |
> | Python SDK 全体・E2E テストパターン | [python.md](python.md) |

**最終更新**: 2026-04-22
**対象ブランチ**: `sasa/develop`（Phase 4a 完了時点）

**実装計画ドキュメント**:
- [docs/plan/phase4a_narrative_foundation.md](../plan/phase4a_narrative_foundation.md) — Phase 4a 実装計画・進捗・実アプリ検証ログ
- [docs/plan/🔄ai_agent_platform_roadmap.md](../plan/🔄ai_agent_platform_roadmap.md) — 親ロードマップ

本書は flowsurface に追加されたナラティブ基盤を、実装・API 利用・運用に十分な粒度で説明するリファレンス仕様書である。Phase 4b（ASI 統合）以降の拡張点は §13 を参照。

---

## 目次

1. [概要](#1-概要)
2. [用語](#2-用語)
3. [データモデル](#3-データモデル)
4. [ストレージ設計](#4-ストレージ設計)
5. [HTTP API](#5-http-api)
6. [冪等性とバリデーション](#6-冪等性とバリデーション)
7. [FillEvent → outcome 自動更新](#7-fillevent--outcome-自動更新)
8. [チャート可視化](#8-チャート可視化)
9. [Python SDK](#9-python-sdk)
10. [E2E テスト](#10-e2e-テスト)
11. [定数と設計不変条件](#11-定数と設計不変条件)
12. [実装ファイルマップ](#12-実装ファイルマップ)
13. [スコープ外・Phase 4b 以降への申し送り](#13-スコープ外phase-4b-以降への申し送り)

---

## 1. 概要

flowsurface の **ナラティブ基盤** は、エージェント（Python スクリプト・人間・将来の uAgent）が行動するたびに **「観測・判断根拠・行動・結果」** をローカルに構造化保存する仕組みである。リプレイ中のチャートに判断根拠のマーカーを重ねて可視化し、Phase 4b の ASI 統合（Fetch.ai uAgent 経由の配信）の入力データ構造を確定させる。

### 1.1 主な機能

| 機能 | 内容 |
|---|---|
| ナラティブ記録 | `POST /api/agent/narrative` で reasoning / action / observation_snapshot を保存 |
| メタ/本体 分離保存 | メタは SQLite、observation_snapshot は gzip + sha256 でファイル保存 |
| 冪等性 | `idempotency_key` による再送安全化（`(agent_id, key)` UNIQUE） |
| 一覧・取得 | `GET /api/agent/narratives`（agent_id / ticker / since_ms / limit フィルタ）|
| 本体取得 | `GET /api/agent/narrative/:id/snapshot` で gzip 解凍 + sha256 検証済み JSON を返す |
| `FillEvent` 連携 | `linked_order_id` で紐付いた注文が約定すると `outcome` が自動更新 |
| 公開フラグ | `PATCH /api/agent/narrative/:id { public: bool }`（Phase 4b で配信入力となる） |
| ストレージ可観測性 | `GET /api/agent/narratives/storage`、孤児スナップショット検出 `/orphans` |
| チャートマーカー | リプレイ中のエントリー三角形 / エグジット矩形（`buy=緑` / `sell=赤`）|

### 1.2 非ゴール（Phase 4a）

- ASI Alliance（uAgents / Agentverse）との配信・フォロー
- ナラティブ削除 API（`DELETE` エンドポイント）
- 他エージェントのナラティブ購読
- ライブモードでのマーカー描画
- インジケーターハイライト・クリックで reasoning ポップアップ
- Ocean Protocol データ売買

---

## 2. 用語

| 用語 | 定義 |
|---|---|
| **Narrative** | 1 回のエージェント判断を表すレコード。観測 + 推論 + 行動 + 結果 |
| **Snapshot** | その判断時点の観測データ（OHLCV・インジケータ等の任意 JSON） |
| **SnapshotRef** | Snapshot 本体へのファイル参照 + サイズ + sha256 |
| **Outcome** | 約定結果（fill_price / fill_time_ms / closed_at_ms / realized_pnl）|
| **linked_order_id** | `VirtualOrder.order_id: String`（[order.md §7.2](order.md#72-型定義)）に紐付く |
| **idempotency_key** | 冪等再送用のクライアント指定キー（`(agent_id, key)` で UNIQUE） |
| **uagent_address** | Phase 4b で導入する Fetch.ai エージェントアドレス。4a では常に `null` 許容 |
| **NarrativeMarker** | チャート Canvas に描画する三角形（エントリー）/ 矩形（エグジット） |

時刻はすべて **Unix ミリ秒 (`i64`)** を基準とする。仮想時刻（`timestamp_ms`）は `StepClock::now_ms()` 由来、`created_at_ms` は実時間。

---

## 3. データモデル

### 3.1 Rust 型（`src/narrative/model.rs`）

```rust
pub struct Narrative {
    pub id: Uuid,                         // ローカル主キー（サーバー生成）
    pub agent_id: String,                 // エージェント識別子（Python 側で任意指定）
    pub uagent_address: Option<String>,   // Phase 4b 用。4a では null
    pub timestamp_ms: i64,                // 仮想時刻（StepClock::now_ms()）
    pub ticker: String,
    pub timeframe: String,                // "M1" / "H1" / "D1" 等
    pub snapshot_ref: SnapshotRef,
    pub reasoning: String,                // 自然言語の判断根拠
    pub action: NarrativeAction,
    pub confidence: f64,                  // 0.0 ..= 1.0
    pub outcome: Option<NarrativeOutcome>, // 約定後に自動更新
    pub linked_order_id: Option<String>,   // VirtualOrder.order_id と一致
    pub public: bool,                     // デフォルト false
    pub created_at_ms: i64,               // 実時間（監査用）
    pub idempotency_key: Option<String>,  // (agent_id, key) で UNIQUE
}

pub enum NarrativeSide { Buy, Sell }

pub struct NarrativeAction {
    pub side: NarrativeSide,
    pub qty: f64,
    pub price: f64,   // market 注文でも指定必須（4a では Option 化していない）
}

pub struct NarrativeOutcome {
    pub fill_price: f64,
    pub fill_time_ms: i64,
    pub closed_at_ms: Option<i64>,    // 決済（反対売買）時刻。4a では FillEvent 発火時に設定
    pub realized_pnl: Option<f64>,
}

pub struct SnapshotRef {
    pub path: PathBuf,       // data_path からの相対パス
    pub size_bytes: u64,     // 圧縮後バイト数
    pub sha256: String,      // 16 進 64 文字
}
```

### 3.2 型整合性メモ

- **`linked_order_id` は `Option<String>`**。`VirtualOrder.order_id: String`（[src/replay/virtual_exchange/order_book.rs:18](../../src/replay/virtual_exchange/order_book.rs#L18)）に揃えるため。
- **`Narrative.id` は `Uuid`** （SQLite には TEXT として保存）。`order_id` と異なる系統。
- **`NarrativeSide`** は API JSON で `"buy"` / `"sell"` を使う。内部の `PositionSide { Long, Short }` とは意図的に分離（SDK 利用者にとって `"buy"` / `"sell"` のほうが自然）。

---

## 4. ストレージ設計

### 4.1 分離戦略

メタデータ（軽量・クエリ対象）とスナップショット本体（大容量・個別取得時のみ）を分離する:

```
data_path()/
├── narratives.db                        # メタデータ SQLite
└── narratives/
    └── snapshots/
        └── {yyyy}/{mm}/{dd}/
            └── {uuid}.json.gz           # gzip 圧縮されたスナップショット本体
```

年月日で階層化することで 1 ディレクトリあたりの inode 数を抑える。

### 4.2 SQLite スキーマ

```sql
CREATE TABLE narratives (
    id TEXT PRIMARY KEY,
    agent_id TEXT NOT NULL,
    uagent_address TEXT,
    timestamp_ms INTEGER NOT NULL,
    ticker TEXT NOT NULL,
    timeframe TEXT NOT NULL,
    snapshot_path TEXT NOT NULL,
    snapshot_size_bytes INTEGER NOT NULL,
    snapshot_sha256 TEXT NOT NULL,
    reasoning TEXT NOT NULL,
    action_json TEXT NOT NULL,
    confidence REAL NOT NULL,
    outcome_json TEXT,
    linked_order_id TEXT,
    public INTEGER NOT NULL DEFAULT 0,
    created_at_ms INTEGER NOT NULL,
    idempotency_key TEXT
);

CREATE INDEX idx_narratives_agent_ticker ON narratives(agent_id, ticker);
CREATE INDEX idx_narratives_timestamp ON narratives(timestamp_ms);
CREATE INDEX idx_narratives_order      ON narratives(linked_order_id);
CREATE UNIQUE INDEX idx_narratives_idempotency
    ON narratives(agent_id, idempotency_key)
    WHERE idempotency_key IS NOT NULL;  -- NULL は制約対象外
```

### 4.3 並行性モデル

- **方針**: `Arc<tokio::sync::Mutex<rusqlite::Connection>>` で 1 本の接続を共有
- **実装**: すべての書き込み・読み込みは `tokio::task::spawn_blocking` 内で実行し UI スレッド・HTTP ハンドラーをブロックしない
- **抽象化**: `NarrativeStore` は trait として定義（[src/narrative/store.rs](../../src/narrative/store.rs)）。将来 `r2d2_sqlite` プール版に差し替え可能
- **採用理由**: ナラティブ書き込みは秒間数件レベルで Mutex の直列化で十分。依存クレートを最小化（YAGNI）

### 4.4 書き込み・読み込みフロー

- **`POST /api/agent/narrative` の流れ**:
  1. ルート層: サイズ early check（未圧縮バイト数）
  2. `SnapshotStore::write(id, timestamp_ms, &Value)`:
     - `flate2` で gzip 圧縮
     - `narratives/snapshots/{yyyy}/{mm}/{dd}/{uuid}.json.gz` に書き出し
     - sha256 と byte size を計算
  3. 成功したら `NarrativeStore::insert` でメタを INSERT
  4. INSERT 失敗時はファイルが孤児になる（[§4.5 参照](#45-整合性と孤児検出)）

- **`GET /api/agent/narratives` （一覧）**:
  - SQLite のみ参照。スナップショット本体は読まず、`snapshot_ref`（path / size / sha256）を返す

- **`GET /api/agent/narrative/:id/snapshot` （本体取得）**:
  - 明示的に呼ばれた時のみ gzip 解凍 + sha256 検証
  - sha256 不一致は `410 Gone`

### 4.5 整合性と孤児検出

- **孤児スナップショット**: 起動時および `GET /api/agent/narratives/orphans` で、SQLite に存在しないファイルを検出する（[`NarrativeStore::gc_orphans()`](../../src/narrative/store.rs)）。**自動削除はしない**（Phase 4a では検出のみ、GC 実行 API は Phase 4b 以降）
- **ファイル欠損**: `load_snapshot` 時に sha256 検証し欠損は 410 + ログ
- **削除 API**: Phase 4a では非スコープ（データ損失リスク vs 研究用途の価値のトレードオフ）

### 4.6 サイズ上限

| しきい値 | 挙動 |
|---|:-:|
| 圧縮前 10 MB 超 | `413 Payload Too Large` |
| 圧縮後 2 MB 超 | `413 Payload Too Large` |
| 圧縮後 256 KB 超 | WARN ログ出力（拒否はしない） |

> **注**: HTTP レイヤー（`read_full_request`）のボディ上限は **16 MB** に拡張済み（[§11.1](#111-定数)）。これは 10 MB ナラティブに余裕を見た値。

---

## 5. HTTP API

ベース仕様は [replay.md §11.1](replay.md#111-ベース仕様) と共通（`127.0.0.1:9876`、`Connection: close`、`application/json`）。

### 5.1 エンドポイント一覧

| メソッド | パス | ボディ/クエリ | レスポンス |
|---|---|---|---|
| `POST` | `/api/agent/narrative` | [Narrative 作成リクエスト JSON](#52-post-apiagentnarrative) | `{ "id", "snapshot_bytes", "idempotent_replay" }` 201 / 400 / 413 |
| `GET`  | `/api/agent/narratives` | query: `agent_id`, `ticker`, `since_ms`, `limit`（default=100, max=1000）| `{ "narratives": [...] }` 200 |
| `GET`  | `/api/agent/narrative/:id` | — | Narrative メタ JSON 200 / 404 |
| `GET`  | `/api/agent/narrative/:id/snapshot` | — | 解凍済み `observation_snapshot` JSON 200 / 404 / 410（sha256 不一致）|
| `PATCH`| `/api/agent/narrative/:id` | `{ "public": true \| false }` | 更新後の Narrative JSON 200 / 404 |
| `GET`  | `/api/agent/narratives/storage` | — | `{ "total_count", "total_bytes", "warn_count" }` 200 |
| `GET`  | `/api/agent/narratives/orphans` | — | `{ "orphan_files": [...] }` 200（削除は非スコープ）|

> **ルーティングの注意**: `GET /api/agent/narratives/storage` と `/orphans` は `GET /api/agent/narratives` より前にマッチさせる必要がある。実装では明示的な path セグメント比較で衝突を防いでいる。

### 5.2 POST `/api/agent/narrative`

#### リクエスト例

```jsonc
{
  "agent_id": "user_A_agent_v3",
  "uagent_address": null,
  "ticker": "BTCUSDT",
  "timeframe": "1h",
  "observation_snapshot": {          // 任意の JSON（大容量可。gzip + 別ファイル保存）
    "ohlcv": [{ "t": 1704067200000, "o": 92100, "h": 92800, "l": 91900, "c": 92500, "v": 1234.5 }],
    "indicators": { "rsi_4h": 28.3, "volume_ratio": 1.42 }
  },
  "reasoning": "RSI divergence on 4h, volume confirmed above 1.4x average",
  "action": { "side": "buy", "qty": 0.1, "price": 92500 },
  "confidence": 0.76,
  "linked_order_id": "ord_01JG...", // オプショナル。先に POST /api/replay/order で取得した String
  "timestamp_ms": 1704067200000,    // オプショナル。省略時は StepClock::now_ms() を使用
  "idempotency_key": "agent_A#step_42" // オプショナル。重複 POST 防止
}
```

#### レスポンス

- **201 Created**:
  ```json
  { "id": "<uuid>", "snapshot_bytes": 12345, "idempotent_replay": false }
  ```
  `idempotent_replay: true` は同じ `(agent_id, idempotency_key)` の再送で既存レコードを返したことを意味する。

- **400 Bad Request**: 不正 JSON、`confidence` 範囲外（0.0..=1.0 外）、空 `agent_id`、不明な `side`、`observation_snapshot` 欠落など。現状レスポンス本文は `{"error":"Bad Request: invalid JSON body"}` で一律（理由の細分化は Phase 4b DX 改善候補）

- **413 Payload Too Large**: 圧縮前 10 MB 超 または 圧縮後 2 MB 超

### 5.3 GET `/api/agent/narratives`

クエリパラメータ:

| 名前 | 型 | デフォルト | 説明 |
|---|---|---|---|
| `agent_id` | string | — | 完全一致フィルタ |
| `ticker` | string | — | 完全一致フィルタ（例 `"BTCUSDT"` / `"Tachibana:7203"`）|
| `since_ms` | int | — | `timestamp_ms >= since_ms` |
| `limit` | int | 100 | 最大 1000。超過値は 1000 に clamp |

レスポンスは `{ "narratives": [Narrative, ...] }`。スナップショット本体は含まない（`snapshot_ref` メタのみ）。

### 5.4 GET `/api/agent/narrative/:id/snapshot`

- **成功**: `200`、`Content-Type: application/json`、**gzip 解凍済み**のスナップショット本体 JSON を返す（バイナリではない）
- **sha256 不一致**: `410 Gone`（ディスク上のファイルが改ざんまたは破損している）
- **ファイル欠損**: `404 Not Found`

### 5.5 PATCH `/api/agent/narrative/:id`

```json
{ "public": true }
```

`true` / `false` 双方向をサポート（取消対応）。レスポンスは更新後の Narrative メタ JSON 全体。

> **API 設計メモ**: 親ロードマップ草案では `POST /api/agent/narrative/publish` だったが、REST 整合性と取消対応のため `PATCH` に一般化した。

### 5.6 GET `/api/agent/narratives/storage`

```json
{ "total_count": 128, "total_bytes": 13454, "warn_count": 0 }
```

| フィールド | 意味 |
|---|---|
| `total_count` | 登録ナラティブ件数 |
| `total_bytes` | スナップショット圧縮後サイズの合計 |
| `warn_count` | 圧縮後 256 KB を超えたレコードの数（WARN ログと連動）|

---

## 6. 冪等性とバリデーション

### 6.1 ID 生成責任

- **`Narrative.id`（UUID）は常にサーバー側で生成**。クライアントが `id` を指定しても無視する（ID 衝突事故の防止）
- **冪等性が必要な場合は `idempotency_key` を利用**:
  - `(agent_id, idempotency_key)` の複合 UNIQUE 制約（NULL 許容の部分インデックス）
  - 同一キーでの再送時は新規 INSERT せず既存 Narrative を返す（`idempotent_replay: true`）
  - キー未指定時は常に新規 INSERT（破壊的ではないため許容）

### 6.2 入力バリデーション（400 で拒否）

| 条件 | 例 |
|---|---|
| JSON が壊れている | `"{ broken"` |
| 必須フィールド欠落 | `agent_id` / `ticker` / `timeframe` / `reasoning` / `action` / `observation_snapshot` / `confidence` |
| `agent_id` が空文字 | `"agent_id": ""` |
| `confidence` 範囲外 | `-0.1` / `1.5`（`0.0..=1.0` 外）。`0.0` / `1.0` は受理 |
| 不明な `side` | `"flip"` / `"Long"` |

---

## 7. FillEvent → outcome 自動更新

`VirtualExchangeEngine::on_tick()` が返す `FillEvent`（[src/replay/virtual_exchange/order_book.rs](../../src/replay/virtual_exchange/order_book.rs)）を購読し、`linked_order_id == FillEvent.order_id` のナラティブの `outcome` を自動更新する。

### 7.1 FillEvent の生成箇所（3 カ所すべて配線済み）

| 経路 | ファイル |
|---|---|
| GUI 継続再生 | `app/handlers.rs::handle_tick` |
| GUI StepForward | `app/dashboard.rs` の `Message::Replay` 分岐 |
| headless | `headless.rs::HeadlessEngine::tick` および `step_forward`（後者は実アプリ検証で追加配線）|

> **実装知見**: 計画 §3.4 では `handle_virtual_order_filled()`（UI 通知専用）を想定していたが、そこでは `NarrativeStore` への参照が取れないため、**FillEvent 生成源（on_tick 直後）**で起動するほうが自然だった。headless の `step_forward` は計画書に明記されておらず、実アプリ検証で配線漏れが発覚 → 修正済み。

### 7.2 失敗時の挙動

- **ナラティブが見つからない**・**ストアが busy**: `log::warn!` のみで **UI には通知しない**（注文自体の処理はブロックしない）
- **配線方式**: GUI は `Task::perform(..., |()| Message::Noop)` で fire-and-forget、headless は `tokio::spawn`

### 7.3 outcome フィールドの現状

| フィールド | 4a での設定 |
|---|---|
| `fill_price` | `FillEvent.fill_price` |
| `fill_time_ms` | `FillEvent.fill_time_ms` |
| `closed_at_ms` | 反対売買時のみ。4a では単純設定 |
| `realized_pnl` | Phase 4b 以降。4a では未使用 |

---

## 8. チャート可視化

### 8.1 NarrativeMarker

[src/narrative/marker.rs](../../src/narrative/marker.rs) に定義。

```rust
pub struct NarrativeMarker {
    pub kind: MarkerKind,   // Entry / Exit
    pub side: NarrativeSide, // Buy / Sell
    pub time_ms: i64,
    pub price: f64,
}
```

- 1 ナラティブ → **1〜2 マーカー**（エントリー必須、`outcome` があればエグジット追加）
- `from_narrative(&Narrative) -> Vec<NarrativeMarker>` で生成

### 8.2 描画ルール

| 種別 | 形状 | 色 |
|---|---|---|
| Entry + Buy | 上向き三角形 | 緑 |
| Entry + Sell | 下向き三角形 | 赤 |
| Exit（両 side） | 矩形 + アルファ 0.75 | buy=緑 / sell=赤 |

- **可視範囲フィルタ**: `draw_markers` が `visible_range_ms` を使って範囲外を切り捨て
- **バー境界スナップ**: `time_ms` を `interval_ms = timeframe.to_milliseconds()` で割ってバー X 座標に変換（`Basis::Time(tf)` のみ。`Basis::Tick` は Phase 4a 対応外）

### 8.3 表示条件

- **リプレイモード時のみ描画**（`KlineChart::replay_mode` でガード）。ライブモードではナラティブ記録は可能だがマーカーは描画しない
- **常時表示**: サイドバートグル等の UI は Phase 4a では用意しない（Phase 4b で他者のナラティブが入った時点で再検討）

### 8.4 データ配信経路

`Message::SetNarrativeMarkers(Vec<NarrativeMarker>)` を新設。以下のトリガで `refresh_narrative_markers_task()` を起動し、全 KlineChart ペインに配信する:

1. `POST /api/agent/narrative` が 201 を返した時
2. `FillEvent` 発火時

`KlineChart::set_narrative_markers()` を呼ぶと `chart.cache.clear_all()` により再描画が強制される。

---

## 9. Python SDK

**ファイル**: `python/narrative.py`、`python/_client.py`（内部 HTTP クライアント）

### 9.1 インポートと基本利用

```python
import flowsurface as fs

# 作成
resp = fs.narrative.create(
    agent_id="my_agent",
    ticker="BTCUSDT",
    timeframe="1h",
    observation_snapshot={"rsi": 28.3, "volume_ratio": 1.42},
    reasoning="RSI divergence",
    action={"side": "buy", "qty": 0.1, "price": 92500.0},
    confidence=0.76,
    linked_order_id="ord_123",        # 省略可
    idempotency_key="step_42",        # 省略可
)
# resp = {"id": "...", "snapshot_bytes": 78, "idempotent_replay": False}

# 一覧・取得
narratives = fs.narrative.list(agent_id="my_agent", ticker="BTCUSDT", limit=50)
n = fs.narrative.get(resp["id"])            # -> Narrative dataclass
body = fs.narrative.snapshot(resp["id"])    # 解凍済み観測 JSON

# 公開・非公開
fs.narrative.publish(resp["id"])
fs.narrative.unpublish(resp["id"])

# 運用
stats = fs.narrative.storage_stats()        # {"total_count", "total_bytes", "warn_count"}
orphans = fs.narrative.orphans()            # list[str]
```

### 9.2 dataclass

| クラス | フィールド |
|---|---|
| `NarrativeAction` | `side` (`"buy"` / `"sell"`) / `qty` / `price` |
| `NarrativeOutcome` | `fill_price` / `fill_time_ms` / `closed_at_ms` / `realized_pnl` |
| `Narrative` | サーバー側レスポンス全体。`from_dict(d)` でデシリアライズ |

### 9.3 FlowsurfaceEnv 拡張（`python/env.py`）

| メソッド | 説明 |
|---|---|
| `env.record_narrative(*, agent_id, reasoning, side, qty, price, confidence, ...)` | `observation_snapshot` は現在の `obs` から自動生成。Env ユーザーのワンストップ API |
| `env.list_narratives(...)` | `fs.narrative.list` の委譲 |
| `env.publish_narrative(id, public=True)` | `public=False` で unpublish 相当 |

### 9.4 エラーモデル

- `FlowsurfaceNotRunningError` — アプリが起動していない（接続失敗）
- `ApiError(status, body)` — 4xx / 5xx 応答。`status=410` は sha256 不一致、`413` は payload too large

---

## 10. E2E テスト

`tests/e2e/` 配下に配置。命名規則は既存 S1〜S50 に続く **S51 以降**（[python.md §7](python.md#7-テストスイート一覧) 参照）。

| スイート | ファイル | 概要 |
|---|---|---|
| S51 | `s51_narrative_crud.py` | POST → GET → 404 → LIST フィルタ → PATCH true/false → idempotency_key 再送 → storage stats（9 TC）|
| S52 | `s52_narrative_outcome_link.py` | 仮想注文 → 約定 → outcome 自動更新（`linked_order_id` 紐付け、3 TC） |
| S53 | `s53_narrative_snapshot_size.py` | 11 MB → 413 / 正常 POST / sha256 破壊 → 410（3 TC） |
| S54 | — | **保留**（チャートオーバーレイの pixel-diff 比較。Phase 4b 以降）|

### 10.1 Rust ユニットテスト

- `src/narrative/model.rs`: serde 往復（4 件）
- `src/narrative/snapshot_store.rs`: gzip roundtrip / sha256 不一致検出 / サイズ上限 / 年月日ディレクトリ作成
- `src/narrative/store.rs`: insert / get / list フィルタ / idempotency / update_outcome_by_order_id / set_public / gc_orphans / storage_stats（合計 21 件）
- `src/narrative/marker.rs`: entry only / entry+exit / 色分け / 可視範囲フィルタ（4 件）
- `src/narrative/service.rs`: idempotent_create / integrity_mismatch → 410 / patch 404 / update_outcome_from_fill（5 件）
- `src/replay_api.rs`: 各ルート 15 件（POST / GET x5 / PATCH x2 / 413 / 400 バリデーション）

### 10.2 Python ユニットテスト

`tests/python/test_narrative.py` — 11 件（CRUD / idempotency / validation / dataclass roundtrip）。`pytest` で実行（アプリを別プロセスで起動した状態）。

---

## 11. 定数と設計不変条件

### 11.1 定数

| 定数 | 値 | 定義箇所 | 意味 |
|---|---|---|---|
| 最大未圧縮スナップショット | 10 MB | `src/narrative/snapshot_store.rs` | 413 しきい値（上限）|
| 最大圧縮後スナップショット | 2 MB | `src/narrative/snapshot_store.rs` | 413 しきい値（上限）|
| WARN しきい値 | 256 KB | `src/narrative/snapshot_store.rs` | ログ警告のみ（拒否はしない）|
| 一覧デフォルト / 最大 limit | 100 / 1000 | `src/replay_api.rs` | `GET /api/agent/narratives` クエリ |
| HTTP ヘッダ最大 | 64 KB | `src/replay_api.rs::MAX_HEADER_BYTES` | リクエスト行 + ヘッダ全体 |
| HTTP ボディ最大 | 16 MB | `src/replay_api.rs::MAX_BODY_BYTES` | 10 MB ナラティブ + 余裕 |

> **注**: HTTP バッファは動的拡張（初期 16 KB → 最大 16 MB）に変更済み。旧仕様（固定 8192 バイト）から変更されているため [replay.md §12.1](replay.md#121-定数一覧) の記述は当仕様で上書きされる。

### 11.2 設計上の不変条件

| # | 不変条件 | 破壊したときの症状 |
|:-:|---|---|
| 1 | `Narrative.id` はサーバー側で生成。クライアント指定を受け付けない | ID 衝突でデータ破壊 |
| 2 | `(agent_id, idempotency_key)` は部分 UNIQUE（`WHERE idempotency_key IS NOT NULL`）| NULL キーの重複で INSERT 失敗 |
| 3 | スナップショット書き込み成功後のみ SQLite INSERT | INSERT 失敗時は孤児ファイル（`gc_orphans()` で検出可能）|
| 4 | `load_snapshot` は sha256 を必ず検証 | 破損ファイルを JSON として返してしまう |
| 5 | `linked_order_id` は `VirtualOrder.order_id: String` と同型 | outcome 自動更新の紐付けが全スキップ |
| 6 | `NarrativeStore` のすべての SQL 操作は `spawn_blocking` 内で実行 | UI スレッドブロック |
| 7 | リプレイモード時のみマーカー描画（`KlineChart::replay_mode` ガード）| ライブモードで虚偽の過去マーカー表示 |
| 8 | FillEvent 生成源（`on_tick` 直後）すべてで `update_outcome_from_fill` を起動 | `step_forward` 経路で outcome が永遠 null（実アプリ検証で踏んだ実例）|
| 9 | `/api/replay/order` の `ticker` は `SerTicker`（`Exchange:Symbol`）形式を受けても symbol 単体に正規化する | `VirtualOrderBook::on_tick` の ticker 比較が全不一致で注文が永遠 Pending（silent failure の実例）|

---

## 12. 実装ファイルマップ

### 12.1 主要ファイル

| ファイル | 責務 |
|---|---|
| [src/narrative/mod.rs](../../src/narrative/mod.rs) | モジュール root |
| [src/narrative/model.rs](../../src/narrative/model.rs) | `Narrative` / `SnapshotRef` / `NarrativeAction` / `NarrativeOutcome` / `NarrativeSide` |
| [src/narrative/store.rs](../../src/narrative/store.rs) | `NarrativeStore` trait と rusqlite 実装。`insert` / `get` / `list` / `update_outcome_by_order_id` / `set_public` / `gc_orphans` / `storage_stats` |
| [src/narrative/snapshot_store.rs](../../src/narrative/snapshot_store.rs) | `SnapshotStore` — gzip + sha256 + 年月日ディレクトリ分割 |
| [src/narrative/service.rs](../../src/narrative/service.rs) | サービスレイヤー。GUI / headless 両方から呼ばれる。エラー → HTTP ステータス変換を一元化 |
| [src/narrative/marker.rs](../../src/narrative/marker.rs) | `NarrativeMarker` / `from_narrative` / `draw_markers` |
| [src/replay_api.rs](../../src/replay_api.rs) | HTTP ルーティング（`ApiCommand::Narrative`）。`parse_narrative_create` ほか |
| [src/app/api/narrative.rs](../../src/app/api/narrative.rs) | GUI 側 `NarrativeCommand` ハンドラ |
| [src/headless.rs](../../src/headless.rs) | headless 側 `handle_narrative_command` + `tick` / `step_forward` での FillEvent → outcome 連携 |
| [src/app/handlers.rs](../../src/app/handlers.rs) | GUI `handle_tick` での FillEvent → `update_outcome_from_fill` 起動 |
| [python/narrative.py](../../python/narrative.py) | Python SDK（`fs.narrative.*`）|
| [python/env.py](../../python/env.py) | `FlowsurfaceEnv.record_narrative` / `list_narratives` / `publish_narrative` |
| [tests/e2e/s51_narrative_crud.py](../../tests/e2e/s51_narrative_crud.py) | E2E CRUD |
| [tests/e2e/s52_narrative_outcome_link.py](../../tests/e2e/s52_narrative_outcome_link.py) | E2E outcome 自動更新 |
| [tests/e2e/s53_narrative_snapshot_size.py](../../tests/e2e/s53_narrative_snapshot_size.py) | E2E サイズ制限・sha256 改ざん検出 |
| [tests/python/test_narrative.py](../../tests/python/test_narrative.py) | Python ユニットテスト 11 件 |

### 12.2 依存クレート（新規）

| クレート | 用途 |
|---|---|
| `rusqlite`（`bundled` feature） | SQLite メタデータストア。外部 DLL 非依存 |
| `flate2` | gzip 圧縮 |
| `sha2` | スナップショット整合性検証 |
| `uuid`（`serde` feature を追加） | `Narrative.id` |

---

## 13. スコープ外・Phase 4b 以降への申し送り

### 13.1 スコープ外（Phase 4a で実装しない）

| 項目 | 理由 |
|---|---|
| ASI Alliance（uAgents / Agentverse）統合 | Phase 4b |
| ナラティブ削除 API | データ損失リスク vs 研究用途価値のトレードオフ |
| 他者のナラティブ購読・配信 | Phase 4b |
| ライブモードでのマーカー描画 | Open Question #8 の確定事項 |
| インジケーターハイライト・reasoning ポップアップ | Phase 4b |
| Ocean Protocol データ売買 | Phase 4c |
| Basis::Tick でのマーカー | ticker 基準 X 座標変換が未対応 |
| S54（チャートオーバーレイ pixel-diff E2E） | reference screenshot / tolerance インフラ未整備 |
| GC 実行 API | 検出のみ実装。実行は Phase 4b |
| バリデーションエラーの詳細コード | 現状は `"Bad Request: invalid JSON body"` 一律。DX 改善候補 |

### 13.2 実アプリ検証で踏み抜いた実行時バグ（修正済み）

| # | バグ | 修正 |
|:-:|---|---|
| 1 | headless `step_forward` が FillEvent を捨てて outcome が永遠 null | `tick` と同型で fills → `update_outcome_from_fill` を配線 |
| 2 | 512 KB 超ボディで接続が黙って切断（10 MB ナラティブが常に失敗） | `read_full_request` を動的バッファ化（16 KB→16 MB）＋ `ReadRequestOutcome::TooLarge` 追加 |
| 3 | `POST /api/replay/order` が `BinanceLinear:BTCUSDT` 形式をサイレント受理し、全注文が永遠 Pending | `parse_virtual_order_command` で `Exchange:Symbol` prefix を剥がし symbol だけに正規化 |

これらは [plan §9 実装ログ](../plan/phase4a_narrative_foundation.md#9-実装ログ作業者追記) に詳細あり。

### 13.3 既存バグ（未修正・Phase 4b 課題）

| # | バグ | 影響 |
|---|---|---|
| 1 | `data::data_path()` が `FLOWSURFACE_DATA_PATH` 上書き時に `path_name` suffix を無視 | env で temp dir を差し替えた場合に SQLite / snapshot dir が競合 |
| 2 | GUI 起動時に `--ticker` / `--timeframe` CLI 引数が無視される（headless 専用パース） | GUI モード E2E で初期 ticker を指定できない。起動後に `/api/pane/set-ticker` が必要 |
| 3 | `/api/replay/order` の `order_type` 欠落時デフォルト market（silent） | `{"type":"limit","price":X}` を送ると市場注文扱い |

### 13.4 Phase 4b に持ち越す DX 改善

- `NarrativeAction.price` を `Option<f64>` 化（market 注文で価格不明を自然表現）
- バリデーションエラーの詳細コード（`unknown_variant` / `out_of_range` 等）
- `snapshot_ref.path` の forward slash 正規化（Windows バックスラッシュ対策）
- GC 実行 API（`DELETE /api/agent/narratives/orphans` 等）
- `snapshot()` と `get()` の命名非対称の解消
