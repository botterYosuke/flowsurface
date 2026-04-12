# 立花証券 E2E テスト Phase T1 — fixture 注入 API 追加 実装記録

**作成日**: 2026-04-12
**対象**: `Cargo.toml`, `exchange/Cargo.toml`, `exchange/src/adapter/tachibana.rs`, `src/connector/auth.rs`, `src/connector/fetcher.rs`, `src/replay_api.rs`, `src/main.rs`
**前提ドキュメント**: [pane_crud_api.md](pane_crud_api.md), [replay_header.md](../replay_header.md), [tachibana_spec.md](../tachibana_spec.md), [`.claude/skills/e2e-test/SKILL.md`](../../.claude/skills/e2e-test/SKILL.md)
**状態**: Phase T1 完了、ユニットテスト 157/157 PASS、E2E 18/18 PASS（回帰含め 254/254 PASS）

## 背景

[pane_crud_api.md §6.2](pane_crud_api.md#シナリオカバレッジ) 表の #2 (Tachibana D1 Replay) は「実 Tachibana 接続は手動」とだけ記され、
また [追加テスト計画 G8](pane_crud_api.md#ギャップ一覧) でも「認証情報依存。手動テスト or 別プロジェクト」とスコープ外扱いだった。

しかし `docs/replay_header.md §10.1`（Tachibana D1 特記事項）・`§7.1`（粗補正モード）で記述されている
**休場日スキップ / 離散ステップ / 粗補正モード / range フィルタ / 株式分割調整値** は、本番ビルドで実際に動かさないとリグレッションを検出できない。

本作業では、**認証をバイパスして fixture を直接注入する test backdoor API** を追加し、
Tachibana D1 リプレイの中核経路を E2E で自動検証可能にする。

## ゴールと実績

| 項目 | 結果 |
|---|:-:|
| `e2e-mock` feature 配下で 3 つの inject API (`/api/test/tachibana/{inject-session,inject-master,inject-daily-history}`) が動作 | ✅ |
| `e2e-tachibana-d1.sh` の T-1〜T-6 が全 PASS | ✅ (18/18) |
| `cargo build --release`（feature 無し）で test backdoor が一切リンクされない | ✅ |
| ユニットテスト / 既存 E2E の回帰 | ✅ (157 unit + 21 + 19 + 28 E2E) |

## 実装方針

### 制約（作業依頼書に準拠）

- **本番ビルドには絶対に含めない**。全コードを `#[cfg(feature = "e2e-mock")]` で gate
- 既存 API の挙動を一切変更しない
- 認証 (`login()`)、EVENT I/F、`fetch_market_prices`、keyring 永続化は触らない（Phase T2/T3）
- mock データは `Vec<Kline>` をそのまま受け取る（`DailyHistoryRecord` のパースは経由しない）

### レイヤー構成

```
HTTP (curl)
    ↓
src/replay_api.rs : route() → ApiCommand::Test(TestCommand::...)
    ↓
src/main.rs : Message::ReplayApi((ApiCommand, ReplySender))
    ↓ ApiCommand::Test(_) → self.handle_test_api(cmd)
    ↓ inject 先:
        · connector::auth::inject_dummy_session()
        · exchange::adapter::tachibana::e2e_mock::inject_master_cache()
        · exchange::adapter::tachibana::e2e_mock::inject_daily_klines()
    ↓ 副作用:
        · inject-master の後に Task::perform で Sidebar::UpdateMetadata を発火
```

### Feature 定義

`Cargo.toml`:
```toml
[features]
debug = ["iced/hot"]
e2e-mock = ["exchange/e2e-mock"]
```

`exchange/Cargo.toml`:
```toml
[features]
e2e-mock = []
```

本番リリースは `cargo build --release` でビルドされ、test backdoor は `#[cfg(feature = "e2e-mock")]` で
完全に弾かれる。E2E 用ビルドは `cargo build --features e2e-mock --release`。

### TestCommand enum の導入

`src/replay_api.rs`:
```rust
#[derive(Debug, Clone)]
pub enum ApiCommand {
    Replay(ReplayCommand),
    Pane(PaneCommand),
    #[cfg(feature = "e2e-mock")]
    Test(TestCommand),
}

#[cfg(feature = "e2e-mock")]
#[derive(Debug, Clone)]
pub enum TestCommand {
    TachibanaInjectSession,
    TachibanaInjectMaster { raw_body: String },
    TachibanaInjectDailyHistory { raw_body: String },
}
```

**理由**: [pane_crud_api.md §ApiCommand enum の導入](pane_crud_api.md#ApiCommand-enum-の導入) と同じく
wire 層の union に新バリアントを追加する。raw_body のまま受け渡すのは main.rs 側で
`MasterRecord` / `Vec<Kline>` の構築を行うため（パース失敗を 1 箇所に寄せる）。

### RwLock の `std::sync` 化（ISSUE_MASTER_CACHE）

`exchange/src/adapter/tachibana.rs` の `ISSUE_MASTER_CACHE` は従来 `tokio::sync::RwLock` だったが、
test backdoor から sync context（iced `update()` ループ）で `.write()` する必要があるため
**`std::sync::RwLock` に置き換え**た。

```rust
// Before
use tokio::sync::RwLock;
static ISSUE_MASTER_CACHE: RwLock<...> = RwLock::const_new(None);
*ISSUE_MASTER_CACHE.write().await = Some(Arc::new(records));

// After
use std::sync::RwLock;
static ISSUE_MASTER_CACHE: RwLock<...> = RwLock::new(None);
if let Ok(mut guard) = ISSUE_MASTER_CACHE.write() {
    *guard = Some(Arc::new(records));
}
```

`init_issue_master` / `get_cached_issue_master` / `cached_ticker_metadata` の `async` シグネチャは
そのまま維持。内部的には std::sync の RwLock を触るだけで `.await` は消えた。
このクレート内で該当ロックを跨ぐ async 処理がないため単純な置き換えで済んだ。

### `e2e_mock` サブモジュールの導入

`exchange/src/adapter/tachibana.rs` に `#[cfg(feature = "e2e-mock")] pub mod e2e_mock` を追加:

```rust
pub(super) static MOCK_DAILY_HISTORY: RwLock<Option<HashMap<String, Vec<Kline>>>> =
    RwLock::new(None);

pub fn inject_master_cache(records: Vec<MasterRecord>);
pub fn inject_daily_klines(issue_code: String, klines: Vec<Kline>);
pub fn get_mock_daily_klines(issue_code: &str) -> Option<Vec<Kline>>;
pub fn clear_daily_klines();
pub fn clear_master_cache();
```

すべて sync API（std::sync::RwLock を触るだけ）。

### `fetch_daily_history` の mock 分岐

依頼書は「`fetch_daily_history` を mock 経路へ分岐させる」と書いているが、`fetch_daily_history` の
返り値は `Vec<DailyHistoryRecord>` であり mock データ (`Vec<Kline>`) と型が異なる。
「mock データは `Vec<Kline>` をそのまま受け取る形でよい（`DailyHistoryRecord` のパースは経由しない）」
という指示を優先し、分岐点を呼び出し側の `fetch_tachibana_daily_klines`（`src/connector/fetcher.rs`）に
置いた。これにより `fetch_daily_history` 自体は無変更で済み、かつ mock 経路は
`DailyHistoryRecord` を経由せず `Vec<Kline>` を直接返す（range フィルタは mock 経路でもそのまま効く）。

```rust
// src/connector/fetcher.rs
pub async fn fetch_tachibana_daily_klines(
    issue_code: &str,
    range: Option<(u64, u64)>,
) -> Result<Vec<Kline>, String> {
    #[cfg(feature = "e2e-mock")]
    if let Some(mock) = exchange::adapter::tachibana::e2e_mock::get_mock_daily_klines(issue_code) {
        let mut klines = mock;
        if let Some((start, end)) = range {
            klines.retain(|k| k.time >= start && k.time <= end);
        }
        return Ok(klines);
    }
    // … 既存の実ネットワーク経路 …
}
```

### `inject_dummy_session` — keyring 非経由のセッション格納

`src/connector/auth.rs`:
```rust
#[cfg(feature = "e2e-mock")]
pub fn inject_dummy_session() {
    let session = exchange::adapter::tachibana::TachibanaSession {
        url_request: "https://e2e-mock.invalid/request/".to_string(),
        // … (全 URL をダミー "https://e2e-mock.invalid/…")
    };
    store_session(session);
}
```

`store_session` だけを呼び、`persist_session`（keyring 書き込み）は経由しない。
EVENT I/F と REST は mock 経路に分岐するのでダミー URL は実際に叩かれない。

### `handle_test_api` — main.rs

`Message::ReplayApi` の match arm に `ApiCommand::Test(_)` を追加し、
`handle_test_api(&mut self, TestCommand) -> (String, Task<Message>)` で分岐する。

**重要な副作用**: `TestCommand::TachibanaInjectMaster` ではキャッシュ注入の直後に
`Task::perform(cached_ticker_metadata(), |m| Message::Sidebar(TickersTable(UpdateMetadata(Tachibana, m))))` を
発火して sidebar の `tickers_info` を populate する。これを忘れると後続の
`/api/pane/set-ticker` が "ticker info not loaded yet" で必ず落ちる。

### API 仕様

#### `POST /api/test/tachibana/inject-session`

```json
Body: {} （または空）
Reply: {"ok": true, "action": "inject-session"}
```

ダミー `TachibanaSession` をメモリに格納する。以降 `connector::auth::get_session()` が
`Some(...)` を返すようになり、`fetch_tachibana_daily_klines` のセッションチェックを通過する。

#### `POST /api/test/tachibana/inject-master`

```json
Body: {
  "records": [
    {"sIssueCode": "7203", "sIssueName": "トヨタ自動車"},
    {"sIssueCode": "6501", "sIssueName": "日立製作所"}
  ]
}
Reply: {"ok": true, "action": "inject-master", "count": 2}
Error: {"error": "missing 'records' field"} | {"error": "failed to parse records: ..."}
```

`ISSUE_MASTER_CACHE` に `MasterRecord` を直接格納する。`sCLMID` が空のレコードは自動的に
`"CLMIssueMstKabu"` で埋めるので、テスト側は `sIssueCode` と任意のメタデータだけ書けば良い。

**注意**: `sIssueNameEizi`（英語名 = display symbol）を**含めない**こと。含めると
`master_record_to_ticker_info` が `Ticker::new_with_display(_, _, Some("..."))` を作るため、
後続の `/api/pane/set-ticker "TachibanaSpot:7203"` 側が構築する display なし Ticker と
ハッシュ不一致になり `tickers_info.get()` が None を返して "ticker info not loaded yet" 扱いになる。
これは `pane_api_set_ticker` の `parse_ser_ticker` が display なし Ticker を作る既存挙動と連動しており、
E2E fixture では一貫して display を省略するのが正しい。

#### `POST /api/test/tachibana/inject-daily-history`

```json
Body: {
  "issue_code": "7203",
  "klines": [
    {"time": 1736089200000, "open": 3010.0, "high": 3030.0, "low": 2995.0, "close": 3015.0, "volume": 100000.0},
    ...
  ]
}
Reply: {"ok": true, "action": "inject-daily-history", "issue_code": "7203", "count": 60}
Error: {"error": "missing 'issue_code' field"} | {"error": "kline missing 'time' (u64)"} | ...
```

`time` は Unix **ms**（`daily_record_to_kline` と同じく JST 深夜0時推奨）。
`volume` は `f64` → `exchange::unit::qty::Qty::from_f32`。
既存キーへの上書き扱い（累積マージではなく同じ issue_code なら差し替え）。

## ユニットテスト

`src/replay_api.rs` の route テストに 6 個追加:

- `route_post_tachibana_inject_session` — 正常系
- `route_post_tachibana_inject_master_valid` — raw_body 保持
- `route_post_tachibana_inject_master_invalid_json` — 400
- `route_post_tachibana_inject_daily_history_valid` — raw_body 保持
- `route_post_tachibana_inject_daily_history_invalid_json` — 400
- `route_test_backdoor_disabled_when_feature_off` — feature OFF 時の 404 確認

```
cargo test --bin flowsurface -- --test-threads=1            → 153 PASS / 0 FAIL
cargo test --bin flowsurface --features e2e-mock -- ...     → 157 PASS / 0 FAIL
cargo test -p flowsurface-exchange --features e2e-mock      → 115 + 1 PASS / 0 FAIL
```

## E2E テスト: `C:/tmp/e2e-tachibana-d1.sh`

**fixture**: `C:/tmp/e2e-tachibana-d1.json` — BinanceLinear:BTCUSDT M1 の最小単一 pane
（アプリ起動時点で metadata が整う経路を使いたいだけの土台、後続 API で Tachibana に差し替える）

**前提ビルド**: `cargo build --features e2e-mock --release`

**試験内容**:

| Test | 検証 | PASS |
|:-:|---|:-:|
| T-1 | inject-session / inject-master が成功する。sidebar の Tachibana metadata 到達後に pane/list が取得できる | ✅ |
| T-2 | inject-daily-history → `/api/pane/set-ticker TachibanaSpot:7203` → `/api/pane/set-timeframe D1` → pane/list で ticker/timeframe 反映 | ✅ |
| T-3 | `/api/replay/play` → Loading→Playing 遷移 → `replay_buffer_len > 0`（= 60） | ✅ |
| T-4 | `/api/replay/step-forward` 5 回で cursor が 5 バー進む（fixture に土日バーなし ⇒ 実質 weekend skip を確認） | ✅ |
| T-5 | `/api/replay/speed` で 10x まで上げて current_time が前進する（粗補正モード） | ✅ |
| T-6 | `inject-daily-history` で 10 本だけ再注入、狭い range で Play → buffer_len が 10 未満（範囲フィルタ動作） | ✅ |

**合計**: 18/18 PASS

### 設計 Tips — 作業者への申し送り

1. **sIssueNameEizi は省略する**。`Ticker::has_display_symbol` の有無でハッシュが変わり
   `tickers_info.get(&Ticker::new(...))` と一致しなくなる。Phase T2 以降で display を効かせたい場合は
   `pane_api_set_ticker` 側で display-aware な lookup が必要（本 Phase では触らない）。

2. **pane/list の ticker フィールドは `"Tachibana:7203"`（Spot 修飾子なし）**。
   `extract_pane_ticker_timeframe` が `format!("{:?}", ex).replace(' ', "")` を使っているため、
   Tachibana variant は Display と Debug で異なる文字列を返す（Debug は "Tachibana" だけ）。
   set-ticker 側は SerTicker 形式 "TachibanaSpot:7203" を受け付ける。この非対称は既存 Pane CRUD API の
   [pane_crud_api.md §既知の制限](pane_crud_api.md#既知の制限と今後の課題) と同じ構造で、本 Phase の対象外。

3. **cursor 差分が weekend-skip の真の witness**。注入 kline 列に土日を含めないので、
   `step-forward` でバーを 1 つ進めるたびに実カレンダーでは 1〜3 日飛ぶ。
   `current_time` の delta は coarse correction mode の仮想時刻（1 秒/バー × speed）に沿うため
   日数換算では検証できない。cursor が連続 5 ずつ進むことで「離散ステップが機能している」と判断する。

4. **set-ticker → set-timeframe の順**。逆順だと `set-timeframe` で
   `"pane has no active ticker to rebase timeframe"` エラーになる（pane の `stream_pair()` が None の場合
   set-timeframe は拒否される）。先に set-ticker で Tachibana 銘柄を流し込んでから timeframe を D1 に上げる。

5. **curl は `-m 5` 以上**。mock 経路は sync なので通常は即応するが、set-ticker が内部で
   `Task::perform(fetch_tachibana_daily_klines)` を走らせる期間は数百 ms かかり得る。
   ハーネス側のタイムアウトを短くし過ぎると一見失敗に見える。

## スコープ外（明示）

- **実 Tachibana サーバーへの接続テスト** — Phase T1 では認証バイパスのみ
- **ログイン UI の自動化** — Phase T3
- **EVENT I/F (FD/ST フレーム) のモック** — Phase T2
- **`fetch_market_prices` の mock** — Phase T2 で同 MASTER キャッシュ方式と同様に追加予定
- **株式分割調整値 (`pDOPxK` 系) の fixture 経路** — 現在の mock は `Vec<Kline>` を
  そのまま受け取るため調整値の差分は既に事前計算済み扱い。未調整/調整の切替検証は Phase T2

## ファイル変更一覧

- `Cargo.toml` — `e2e-mock` feature 定義
- `exchange/Cargo.toml` — `e2e-mock` feature 定義
- `exchange/src/adapter/tachibana.rs`
  - `ISSUE_MASTER_CACHE` を `tokio::sync::RwLock` → `std::sync::RwLock` に変更
  - `init_issue_master` / `get_cached_issue_master` を sync RwLock に合わせて書き換え
  - `MasterRecord::clm_id` に `#[serde(default)]` 追加（fixture 省略許容）
  - `pub mod e2e_mock` を追加（feature gate）: `MOCK_DAILY_HISTORY` 静的 + inject/get/clear 関数群
- `src/connector/fetcher.rs` — `fetch_tachibana_daily_klines` の先頭に mock 分岐を追加（feature gate）
- `src/connector/auth.rs` — `inject_dummy_session` 関数を追加（feature gate）
- `src/replay_api.rs`
  - `ApiCommand::Test(TestCommand)` バリアント追加（feature gate）
  - `TestCommand` enum 追加（3 バリアント、feature gate）
  - `route()` に 3 つのバックドア path を追加（feature gate）
  - ユニットテスト 6 個追加（feature on/off 両方）
- `src/main.rs`
  - `ApiCommand::Test(_)` arm を追加（feature gate）
  - `Self::handle_test_api(&mut self, TestCommand) -> (String, Task<Message>)` を追加
  - inject-master 成功時に `Task::perform` で `Sidebar::UpdateMetadata(Tachibana, ..)` を発火

## テスト結果（最終）

| 種別 | スクリプト | PASS | FAIL |
|---|---|:-:|:-:|
| Unit (no feature) | `cargo test --bin flowsurface -- --test-threads=1` | 153 | 0 |
| Unit (feature ON) | `cargo test --bin flowsurface --features e2e-mock -- --test-threads=1` | 157 | 0 |
| Unit (exchange) | `cargo test -p flowsurface-exchange --features e2e-mock` | 116 | 0 |
| Build | `cargo build --release` (no feature) | ✅ | — |
| Build | `cargo build --features e2e-mock --release` | ✅ | — |
| E2E | `C:/tmp/e2e-tachibana-d1.sh` (new) | 18 | 0 |
| E2E 回帰 | `C:/tmp/e2e-unified-step.sh` | 21 | 0 |
| E2E 回帰 | `C:/tmp/e2e-pane-crud.sh` | 19 | 0 |
| E2E 回帰 | `C:/tmp/e2e-mid-replay-crud.sh` | 28 | 0 |

## 次フェーズ (依頼者確認待ち)

- **Phase T2**: EVENT I/F (FD/ST バイナリフレーム) のモックと `fetch_market_prices` mock。
  ライブ ticker の stream_type を E2E で検証可能にする。
- **Phase T3**: ログイン UI の自動化 or `/api/auth/*` テスト専用エンドポイント。
  keyring 永続化パスの E2E 検証、および Phase T1 の backdoor を非有効化するモード切替。

**Phase T1 完了後の T2/T3 着手判断は依頼者に確認すること**（作業依頼書のスコープ外に明記）。
