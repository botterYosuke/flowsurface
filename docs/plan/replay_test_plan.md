# リプレイ機能テスト計画

**作成日**: 2026-04-11
**最終更新**: 2026-04-11
**対象**: `replay.rs` / `replay_api.rs` のユニットテスト + E2E API テスト
**状態**: ✅ 全テスト完了

---

## 0. 背景

リプレイ機能（Phase 1-5）の実装完了後、既存テストは 16 件（`replay.rs` のみ）。
`replay_api.rs` にテストなし、エッジケースや状態遷移の網羅も不十分。
ユーザー目線で「迷いなく動作する」品質を担保するため、以下の観点で網羅的にテストを追加する。

---

## 1. テスト対象と方針

### 1.1 replay.rs ユニットテスト追加（既存 16 件 → 目標 30+ 件）

| カテゴリ | テスト内容 | 状態 |
|---------|-----------|------|
| **to_status()** | Live モード（playback なし）→ mode="Live", 他フィールド None | ✅ |
| **to_status()** | Replay + Playing → 全フィールド populated | ✅ |
| **to_status()** | Replay + Loading → status="Loading" | ✅ |
| **to_status()** | Replay + Paused → status="Paused" | ✅ |
| **serialization** | Live: optional フィールドが JSON に含まれない | ✅ |
| **serialization** | Replay: 全フィールドが JSON に含まれる | ✅ |
| **TradeBuffer** | 空バッファで drain_until → 空スライス、is_exhausted=true | ✅ |
| **TradeBuffer** | 同一時刻の複数 trades → 全て返される | ✅ |
| **TradeBuffer** | 連続 drain で全消費 → is_exhausted=true + 追加 drain は空 | ✅ |
| **advance_time** | elapsed=0 → current_time 変化なし | ✅ |
| **advance_time** | current_time == end_time → それ以上進まない | ✅ |
| **advance_time** | 非常に大きな elapsed → end_time にクランプ | ✅ |
| **speed_label** | 全プリセット値 (1x/2x/5x/10x) の表示確認 | ✅ |
| **speed_label** | 小数速度 (1.5x) の表示確認 | ✅ |
| **parse_replay_range** | start == end → StartAfterEnd エラー | ✅ |
| **parse_replay_range** | 秒付きフォーマット → InvalidStartFormat | ✅ |
| **parse_replay_range** | 空文字列 → InvalidStartFormat | ✅ |
| **parse_replay_range** | 1 分間隔 → OK | ✅ |
| **toggle_mode** | playback がある状態で Live に戻す → playback=None | ✅ |
| **toggle_mode** | 2 回トグル（Live→Replay→Live）で元に戻る | ✅ |
| **cycle_speed** | 不明な速度値からのリカバリ | ✅ |
| **format_current_time** | Replay + playback=None → realtime fallback | ✅ |

### 1.2 replay_api.rs ユニットテスト追加（現在 0 件 → 目標 15+ 件）

| カテゴリ | テスト内容 | 状態 |
|---------|-----------|------|
| **parse_request** | 正常な GET リクエスト | ✅ |
| **parse_request** | 正常な POST + body | ✅ |
| **parse_request** | 空文字列 → None | ✅ |
| **parse_request** | 不正フォーマット（method のみ） → None | ✅ |
| **parse_request** | body なし POST → body 空文字列 | ✅ |
| **parse_request** | \r\n\r\n なし → body 空 | ✅ |
| **route** | 全 8 エンドポイントの正常ルーティング (status/toggle/play/pause/resume/step-forward/step-backward/speed) | ✅ |
| **route** | 不明パス → NotFound | ✅ |
| **route** | ルートパス (/) → NotFound | ✅ |
| **route** | GET で POST エンドポイントにアクセス → NotFound | ✅ |
| **route** | POST で GET エンドポイントにアクセス → NotFound | ✅ |
| **route** | POST /api/replay/play に不正 JSON → BadRequest | ✅ |
| **route** | POST /api/replay/play に start 欠落 → BadRequest | ✅ |
| **route** | POST /api/replay/play に end 欠落 → BadRequest | ✅ |
| **route** | POST /api/replay/play に空 body → BadRequest | ✅ |
| **route** | POST /api/replay/play に数値 start/end → BadRequest | ✅ |
| **route** | POST /api/replay/play に正常 JSON → Play コマンド | ✅ |

### 1.3 E2E API テスト

アプリを実際に起動し、HTTP API エンドポイントを curl で叩いて動作確認する。

| テストケース | 手順 | 状態 |
|------------|------|------|
| status 取得 | `GET /api/replay/status` → `{"mode":"Live"}` | ✅ |
| toggle | `POST /api/replay/toggle` → Live↔Replay 切替 | ✅ |
| status 確認 | toggle 後に status → mode="Replay" 確認 | ✅ |
| play | toggle → play → status で Playing 確認 | ✅ |
| speed cycle | speed を 4 回叩いて 2x→5x→10x→1x を確認 | ✅ |
| pause | pause → mode="Replay" 確認 | ✅ |
| step-forward | pause 後に step-forward → current_time +60s 増加 | ✅ |
| step-backward | step-backward → current_time -60s 減少 | ✅ |
| resume | resume → status="Playing" 確認 | ✅ |
| toggle to Live | Replay→Live 切替 | ✅ |
| Live 確認 | status で mode="Live" + playback フィールドなし | ✅ |
| 不正パス | `GET /api/replay/unknown` → 404 | ✅ |
| 不正 body | `POST /api/replay/play` with bad JSON → 400 | ✅ |
| フィールド欠落 | `POST /api/replay/play` with missing end → 400 | ✅ |
| 空 body | `POST /api/replay/play` with empty body → 400 | ✅ |
| 多重 toggle | Live→Replay→Live→Replay→Live ラウンドトリップ | ✅ |

---

## 2. 設計思想

- **クラッシュしないこと**: エッジケース（空バッファ、境界値、不正入力）を網羅
- **ユーザー目線**: 状態遷移が正しいこと（Live→Replay→Play→Pause→Resume→Live）
- **API の堅牢性**: 不正リクエストで 400/404 が返り、クラッシュしないこと
- **速度**: ユニットテストは瞬時に完了、E2E テストのみアプリ起動が必要

---

## 3. 結果

### ユニットテスト: 62 件全パス ✅

| ファイル | 既存 | 追加 | 合計 |
|---------|------|------|------|
| `replay.rs` | 16 | 23 | 39 |
| `replay_api.rs` | 0 | 23 | 23 |
| **合計** | **16** | **46** | **62** |

```
cargo test --bin flowsurface replay
→ test result: ok. 62 passed; 0 failed; 0 ignored; 0 measured; 20 filtered out; finished in 0.01s
```

### E2E API テスト: 22 件全パス ✅

```
bash tests/e2e_replay_api.sh
→ Passed: 22, Failed: 0 — All tests passed!
```

テスト用環境:
- `FLOWSURFACE_DATA_PATH=/tmp/flowsurface-e2e-test` で本番データ隔離
- `FLOWSURFACE_API_PORT=9877` でポート競合回避
- 最小構成 `saved-state.json`（KlineChart 1 枚、BinanceLinear:BTCUSDT M1）

---

## 4. 知見・Tips

### 4.1 変更点

- `replay_api.rs` の `RouteError` に `#[derive(Debug)]` を追加（テストの `.unwrap()` に必要）

### 4.2 Windows 環境の注意

- `python3`/`jq` が使えない環境が多い → `grep -o` + `sed` で JSON パース
- `date -u -d` は Git Bash では使えない場合がある → 固定日時を使用
- `taskkill //f //im flowsurface.exe` でスラッシュをエスケープ

### 4.3 E2E テストの実行手順

```bash
# 1. テスト用データディレクトリ準備
mkdir -p /tmp/flowsurface-e2e-test
cp tests/fixtures/minimal-saved-state.json /tmp/flowsurface-e2e-test/saved-state.json

# 2. テスト用ポートでアプリ起動
FLOWSURFACE_DATA_PATH=/tmp/flowsurface-e2e-test \
FLOWSURFACE_API_PORT=9877 \
target/release/flowsurface.exe &

# 3. テスト実行
FLOWSURFACE_API_PORT=9877 bash tests/e2e_replay_api.sh

# 4. アプリ停止
taskkill //f //im flowsurface.exe
```

### 4.4 step-backward の挙動

- step-backward は `current_time -= 60s` の後、チャート再構築 + Kline 再フェッチが走る
- E2E テストでは `sleep 2` を入れてフェッチ完了を待つ必要がある
- レスポンスの `current_time` は即座に更新されるが、チャート表示の更新は非同期

