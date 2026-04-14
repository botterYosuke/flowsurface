# E2E テスト計画書 — 立花証券銘柄スイート S19〜S22

**作成日**: 2026-04-14  
**更新日**: 2026-04-14（本番データ移行 — e2e-mock 廃止）  
**対象ブランチ**: `sasa/develop`  
**テストスキル**: [.claude/skills/e2e-testing/SKILL.md](../../.claude/skills/e2e-testing/SKILL.md)  
**参照元計画書**: [e2e_new_features.md](e2e_new_features.md)（S15〜S18 の設計・知見）

---

## 1. 概要と目的

S15〜S18（BinanceLinear:BTCUSDT M1）で確認した以下の観点を、
**TachibanaSpot:7203 D1** 銘柄で再検証する。

| 対応スイート | 新スイート | 観点 |
|---|---|---|
| S15 | **S19** | chart-snapshot API（D1 ペイン） |
| S16 | **S20** | UI操作耐性（D1 再生中） |
| S17 | **S21** | エラー境界 / クラッシュしないこと |
| S22 | **S22** | 耐久テスト（D1、15〜30 分以内） |

---

## 2. 前提条件

### ビルド

```bash
cargo build   # 通常のデバッグビルド（--features e2e-mock は不要）
EXE="./target/debug/flowsurface.exe"
API="http://localhost:9876/api"
```

### 環境変数

実行前に以下の環境変数を設定済みであること（スクリプト内にハードコードしない）：

```bash
export DEV_USER_ID="<立花証券ユーザID>"
export DEV_PASSWORD="<パスワード>"
# export DEV_IS_DEMO="true"   # デモ口座を使う場合
```

未設定の場合、各スクリプトは即座に `exit 1` する。

### TachibanaSpot セットアップ手順

各スクリプトの `tachibana_replay_setup` 関数が以下を実行する：

1. **saved-state.json** を Live モードで書き込む（replay セクションなし）
2. `start_app`（デバッグビルドは起動時に DEV AUTO-LOGIN を自動発火）
3. `GET /api/auth/tachibana/status` をポーリング → `session="present"` まで待機（最大 120 秒）
4. `GET /api/pane/list` で pane_id を取得し `streams_ready=true` まで待機（D1 kline フェッチ完了）
5. `POST /api/replay/toggle` + `POST /api/replay/play` — リプレイ開始

> **重要**: モックデータ注入（inject-session / inject-master / inject-daily-history）は一切不要。
> 本番 Tachibana API から D1 kline が自動フェッチされる。

### D1 スペック

| 項目 | 値 |
|---|---|
| Ticker | `TachibanaSpot:7203` |
| Timeframe | `D1` |
| StepForward delta | `86400000` ms |
| 再生 range | `utc_offset -96` 〜 `utc_offset -24`（約 4 日分） |
| bar_count 上限 | 301（PRE_START_HISTORY_BARS=300 + 再生開始バー 1 本） |

---

## 3. スイート一覧と結果

| スイート | ファイル名 | TC 数 | 結果 |
|---|---|---|---|
| S19 | `s19_tachibana_chart_snapshot.sh` | 5 | 本番データ移行済み（要実行確認） |
| S20 | `s20_tachibana_replay_resilience.sh` | 7 | 本番データ移行済み（要実行確認） |
| S21 | `s21_tachibana_error_boundary.sh` | 7 | 本番データ移行済み（要実行確認） |
| S22 | `s22_tachibana_endurance.sh` | 4 | 本番データ移行済み（要実行確認） |

---

## 4. スイート S19: chart-snapshot API（TachibanaSpot）

**スクリプト**: `docs/plan/e2e_scripts/s19_tachibana_chart_snapshot.sh`  
**対応元**: S15 (`s15_chart_snapshot.sh`)

### TC 一覧

| TC ID | 内容 | 判定基準 |
|---|---|---|
| TC-S19-01 | Paused 直後の bar_count が 1〜301 | `1 <= bar_count <= 301` |
| TC-S19-02 | StepForward 後 bar_count が増加または同数 | `after >= before` |
| TC-S19-03 | StepBackward 後も snapshot 取得可能（クラッシュなし） | `bar_count` フィールドあり |
| TC-S19-04 | 不正 pane_id → `{"error":"..."}` かつアプリ生存 | `has_error && alive` |
| TC-S19-05 | Live モード中の snapshot 取得後もアプリ応答あり | API 応答あり |

### D1 特有の注意点

- StepForward delta = **86400000ms**（M1 の 1440 倍）
- bar_count は D1 データ量に依存するため、inject-daily-history で十分なバーを注入する

---

## 5. スイート S20: UI操作耐性（TachibanaSpot）

**スクリプト**: `docs/plan/e2e_scripts/s20_tachibana_replay_resilience.sh`  
**対応元**: S16 (`s16_replay_resilience.sh`)

### TC 一覧

| TC ID | 内容 | 判定基準 |
|---|---|---|
| TC-S20-01 | 速度ボタン 20 連打後も status=Playing | `status=Playing` |
| TC-S20-02a | D1 StepForward delta=86400000ms | `delta == 86400000` |
| TC-S20-02b | StepBackward 後 status=Paused | `status=Paused` |
| TC-S20-03 | Live↔Replay 10 連打後もアプリ応答あり | API 応答あり |
| TC-S20-04 | Playing 中の toggle → アプリ生存 | API 応答あり |
| TC-S20-05a | Paused → toggle → アプリ生存 | API 応答あり |
| TC-S20-05b | 2 回目 toggle 後もアプリ生存 | API 応答あり |

### S16-TC-02 からの変更点

S16-TC-02 は「UTC 0:00 越え」を M1 連続再生でテストした。
D1 データでは 1 ステップで 1 日分進むため、代わりに **StepForward の delta 検証** に差し替えた。

---

## 6. スイート S21: エラー境界（TachibanaSpot）

**スクリプト**: `docs/plan/e2e_scripts/s21_tachibana_error_boundary.sh`  
**対応元**: S17 (`s17_error_boundary.sh`)

### TC 一覧

| TC ID | 内容 | 判定基準 |
|---|---|---|
| TC-S21-01 | `pane/split` 不正 UUID → HTTP 200 & アプリ生存 | `HTTP==200 && alive` |
| TC-S21-02 | `pane/close` 不正 UUID → HTTP 200 & アプリ生存 | `HTTP==200 && alive` |
| TC-S21-03 | `pane/set-ticker` 不正 UUID → HTTP 200 & アプリ生存 | `HTTP==200 && alive` |
| TC-S21-04 | 空 range (start == end) でもアプリ生存 | API 応答あり |
| TC-S21-05 | 未来 range でもアプリ生存 | API 応答あり |
| TC-S21-06 | StepForward 50 連打 → クラッシュなし, status=Paused | `!crash && status=Paused` |
| TC-S21-07 | split 上限テスト → クラッシュなし | API 応答あり |

---

## 7. スイート S22: 耐久テスト（TachibanaSpot）

**スクリプト**: `docs/plan/e2e_scripts/s22_tachibana_endurance.sh`  
**対応元**: S18 (`s18_endurance.sh`)  
**所要時間の目安**: 15〜30 分

### TC 一覧

| TC ID | 内容 | 判定基準 |
|---|---|---|
| TC-S22-01 | 4 日 range を 10x 速度で再生し終了 → Paused | `status=Paused` (180s 以内) |
| TC-S22-02-fwd | StepForward × 50 → クラッシュなし, status=Paused | `!crash && status=Paused` |
| TC-S22-02-bwd | StepBackward × 50 → クラッシュなし, status=Paused | `!crash && status=Paused` |
| TC-S22-03 | Playing 中 split→close × 20 サイクル → status=Playing | `status=Playing` 維持 |

### S18 からの変更点

- **TC-S22-01 待機時間**: D1 データが少ないため Paused 到達を 180 秒待機（S18 は 900 秒）
- **TC-S22-02 ステップ数**: D1 ステップは 86400000ms なので広い range が必要。50 ステップ × 2 方向で実施（S18 は 500 回）
- **TC-S22-02 range**: `-1300h` 〜 `-24h`（約 54 日分）の D1 kline を注入

---

## 8. 実装メモ・知見

### 2026-04-14 初版

#### tachibana_replay_setup 関数

各スクリプト冒頭で定義する共通セットアップ関数。

```bash
tachibana_replay_setup() {
  local start=$1 end=$2
  # Live モード saved-state → start_app → inject → play
}
```

#### inject_daily_history 関数

start/end 範囲内に日次境界アラインされた D1 kline を生成して注入する。

```javascript
// Node.js 内で start_ms, end_ms から日次アラインを計算
const day = 86400000;
const firstBoundary = Math.ceil(startMs / day) * day;
for (let t = firstBoundary; t <= endMs; t += day) { ... }
```

#### D1 StepForward delta

TachibanaSpot:7203 単独ペイン（D1 のみ）では delta = **86400000ms**。
M1 と混在させると最小 TF の M1（60000ms）になるので注意（S5-TC-07 参照）。

---

## 9. 修正・知見ログ

### inject_daily_history のペイロードサイズ問題

- **問題**: `inject-daily-history` エンドポイントへのリクエストが失敗する（curl exit 56 / "Argument list too long"）
- **原因 1**: replay_api.rs の TCP 受信バッファが 8KB 固定だったため、90 bars 超のペイロード（~8KB）で切断発生
- **修正 1**: `src/replay_api.rs` のバッファを `8192` → `524288`（512KB）に拡大
- **原因 2**: shell 変数に klines JSON を格納して `curl -d "$body"` で渡すと引数長制限（~32KB）を超える
- **修正 2**: s22 の inject_daily_history を一時ファイル経由（`curl --data-binary "@$tmpfile"`）に変更

### init_issue_master の HTTP フェッチ問題

- **問題**: e2e-mock ビルドでも `https://e2e-mock.invalid/master/` に HTTP リクエストを送って失敗
- **修正**: `exchange/src/adapter/tachibana.rs` の `init_issue_master()` に `#[cfg(feature = "e2e-mock")]` ブランチを追加して即 `Ok(())` を返す

### D1 range のバー数設計

- D1 は 100ms/bar at 1x なので少バー range では瞬時に再生完了してしまう
- TC-S20-01（速度連打）: -2400h/-24h（100 bars ≒ 10 秒）
- TC-S22-03（CRUD 20 サイクル）: -18000h/-24h（750 bars ≒ 75 秒）

## 10. 進捗ログ

| 日時 | 内容 |
|---|---|
| 2026-04-14 | 計画書作成、スクリプト実装開始 |
| 2026-04-14 | S19〜S22 全 TC PASS 確認（e2e-mock ビルド版。修正履歴は §9 参照） |
| 2026-04-14 | S19〜S22 を本番データ移行。`--features e2e-mock` → 通常デバッグビルド、inject 系 POST → DEV AUTO-LOGIN + `wait_tachibana_session` ポーリングに変更 |
