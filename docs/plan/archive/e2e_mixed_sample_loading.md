# 計画書: サンプルデータ（Tachibana + 仮想通貨混合）起動時の E2E テスト

**作成日**: 2026-04-15
**ブランチ**: sasa/develop
**対象ファイル**: `src/replay/controller.rs`, `src/main.rs`

---

## 概要

Tachibana（立花証券 D1）と仮想通貨（ETHUSDT 1m）が混在したリプレイ状態から
再生 ▶ できないバグを修正した 3 件を E2E テストで検証する。

### 修正内容

| ファイル | 修正内容 |
| :--- | :--- |
| `src/replay/controller.rs` | Play 時に各ストリームの **固有 timeframe** で `compute_load_range` を計算（D1 なら 300 日分） |
| `src/replay/controller.rs` | `KlinesLoadCompleted` で klines が空でも `on_klines_loaded` を呼んでロード済みマーク |
| `src/main.rs` | ▶ ボタンが終端（`now_ms >= range.end`）で Paused のとき `Play`（再スタート）を発行 |

---

## テストシナリオ

### S30: 混合データ・Loading 解消テスト
**スクリプト**: `tests/s30_mixed_sample_loading.sh`
**ビルド**: `cargo build --release --features e2e-mock`

| TC | 操作 | 期待結果 |
|---|---|---|
| TC-A | D1 履歴あり + Play → 待機 | `Loading` → `Playing` or `Paused` に遷移（`Loading` 固定でない） |
| TC-B | D1 履歴なし（空 klines）+ Play → 待機 | `Loading` → `Playing` or `Paused` に遷移（空 klines でも脱出できる） |
| TC-C | TC-A 後ペイン確認 | Tachibana D1 と ETHUSDT M1 双方の `streams_ready=true` |

### S31: 終端 ▶ 再スタートテスト
**スクリプト**: `tests/s31_replay_end_restart.sh`
**ビルド**: `cargo build --release`（Binance のみ、e2e-mock 不要）

| TC | 操作 | 期待結果 |
|---|---|---|
| TC-A | Play → 10x 加速 → 待機 | `Paused` 状態で `current_time ≈ end_time` になる |
| TC-B | 終端到達後 Play 再呼び出し | `Loading` または `Playing` に遷移（再スタート開始） |
| TC-C | Playing 到達後 `current_time` 確認 | `current_time` が `start_time` 付近（end_time 付近のままでない） |

---

## 実行コマンド

```bash
# S30 (e2e-mock ビルドが必要)
cargo build --release --features e2e-mock
bash tests/s30_mixed_sample_loading.sh

# S31 (通常 release ビルド)
cargo build --release
bash tests/s31_replay_end_restart.sh
```

---

## 実行コマンド（debug ビルド使用）

```bash
# debug ビルド（inject-* エンドポイントが debug_assertions でのみ有効のため）
cargo build

# S30
FLOWSURFACE_EXE=./target/debug/flowsurface.exe bash tests/s30_mixed_sample_loading.sh 2>/dev/null

# S31
FLOWSURFACE_EXE=./target/debug/flowsurface.exe bash tests/s31_replay_end_restart.sh 2>/dev/null
```

**注意事項:**
- どちらのテストも Tachibana セッション（keyring）が必要
- セッションなし環境では SKIP して exit 0 する
- `inject-master` / `inject-daily-history` エンドポイントは現在未実装（404）のため実際の Tachibana API を使用

## 進捗

- ✅ 計画書作成
- ✅ S30 テストスクリプト実装 (`tests/s30_mixed_sample_loading.sh`)
- ✅ S31 テストスクリプト実装 (`tests/s31_replay_end_restart.sh`)
- ✅ bash 構文チェック通過（`bash -n`）
- ✅ S30 実行確認: **4/4 PASS**
- ✅ S31 実行確認: **3/3 PASS**
