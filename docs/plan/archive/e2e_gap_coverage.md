# E2E カバレッジ補完計画

**作成日**: 2026-04-15
**ブランチ**: sasa/develop
**担当**: botterYosuke + Claude

## 概要

`docs/replay_header.md` に記載のユーザー操作と既存 E2E テスト (s1〜s26) を対照した結果、
以下の 4 件が未カバーと判明した。本計画書はその補完テスト (s27〜s29) の設計・実装・進捗を管理する。

---

## 未カバーのギャップ一覧

| # | 操作 | 仕様箇所 | 対処 |
|---|---|---|---|
| 1 | **CycleSpeed → current_time リセット検証** | §6.6 | s27 で追加 |
| 2 | **StartTimeChanged / EndTimeChanged → current_time リセット** | §6.6 | ※注1 参照 |
| 3 | **銘柄変更 Waiting（Loading）中** | §6.6 | s28 で追加 |
| 4 | **Tachibana StepBackward 休場日スキップ** | §10.1 | s29 で追加 |

### ※注1: StartTimeChanged / EndTimeChanged の HTTP API 不在

`StartTimeChanged` / `EndTimeChanged` は UI テキスト入力イベントであり、
HTTP 制御 API に対応するエンドポイントが存在しない。

仕様 §6.6 によると、これら 3 操作は **同一のリセットコードパスを共有する**:

```
操作: CycleSpeed / StartTimeChanged / EndTimeChanged
共通フロー（clock が Some の場合）:
  clock.pause() → clock.seek(range.start) → reset_charts → inject_klines_up_to(range.start)
```

したがって **s27 で CycleSpeed のリセット動作を検証することで、共通フローは間接的にカバーされる**。
StartTimeChanged / EndTimeChanged 固有のパスは UI 操作専用であり E2E HTTP テストからは検証不可。
将来 `/api/replay/set-range` エンドポイントを追加すれば直接テストできる（TODO として残す）。

---

## 実装するテストスクリプト

### s27_cyclespeed_reset.sh — §6.6 CycleSpeed リセット ✅ 実装済み

**目的**: CycleSpeed 呼び出し時に `clock` が `Some` であれば `current_time` が `range.start` に戻ること、
および speed label が正しくサイクルすることを確認する。

**テストケース**:

| ID | 前提 | 操作 | 期待 |
|---|---|---|---|
| TC-A | Playing 中 (current_time > start_time) | CycleSpeed | status=Paused, current_time≈start_time, speed=2x |
| TC-B | Paused (TC-A 後) | Resume | status=Playing |
| TC-C | Playing 中 (speed=2x) | CycleSpeed | status=Paused, current_time≈start_time, speed=5x |
| TC-D | Paused (TC-C 後) | Resume | status=Playing |

**フィクスチャ**: BinanceLinear:BTCUSDT M1, UTC[-3h, -1h] (auto-play)

**設計メモ**:
- current_time ≈ start_time の判定は ±60,000ms (1バー) 以内を許容
- `speed_to_10x()` ヘルパーは既に Resume を含んでいるが、
  本テストは「CycleSpeed 直後に Paused かつリセットされている」ことを確認するため Resume **前** に検証する
- s9 は speed label のサイクルのみ検証しており current_time のリセットは未確認

---

### s28_ticker_change_while_loading.sh — §6.6 Waiting 中リセット ✅ 実装済み

**目的**: `ClockStatus::Waiting`（Loading 表示）状態で銘柄変更しても
`current_time` が `range.start` にリセットされ、アプリがクラッシュしないことを確認。

**s23 との差分**: s23 は Playing 中・Paused 中をカバー。本テストは Waiting（Loading）中が対象。

**Waiting 状態の発生方法**:
1. Playing 到達後に `pane/split` で新ペインを追加
2. 新ペインに別 ticker (ETHUSDT) を設定 → 新ストリームのロードが始まる
3. `dispatch_tick` が未ロードストリームを検出し `clock.set_waiting()` → Loading 状態
4. この Loading 状態を 100ms ポーリングで検出し、即座に元ペインの ticker を変更

**テストケース**:

| ID | 前提 | 操作 | 期待 |
|---|---|---|---|
| TC-setup | Playing | split + ETHUSDT 設定 | status=Loading 遷移を確認 |
| TC-A | Loading 中 | 元ペイン ticker を SOLUSDT に変更 | クラッシュなし |
| TC-B | 変更後 | 待機 (最大 30s) | status=Paused |
| TC-C | Paused | 状態確認 | current_time ≈ start_time |
| TC-D | Paused | Resume | status=Playing 到達 |

**フィクスチャ**: BinanceLinear:BTCUSDT M1, UTC[-6h, -1h] (5h レンジで Loading を長くする)

**設計メモ**:
- Loading キャッチは 0.1s 間隔 × 最大 50 回（5秒）ポーリング
- Loading を確実にキャッチできない場合でも TC-A〜D は実行する
  （Loading をキャッチできなかった場合は Playing 中の変更となるが、それも §6.6 の対象）
- タイミング起因のフレーキーネス対策として、Loading キャッチ確認は PASS/FAIL でなく INFO 扱いにする

---

### s29_tachibana_holiday_skip.sh — §10.1 Tachibana 休場日スキップ ✅ 実装済み

**目的**: Tachibana D1 リプレイで `StepBackward` が土日をスキップし前の取引日 (金曜) に戻ることを確認する。

**前提条件**: Tachibana セッション (`DEV_USER_ID` / `DEV_PASSWORD` 環境変数)
セッションなしの場合は全テストを SKIP して exit 0。

**メカニズム**:
Tachibana の EventStore には土日祝の kline が存在しない。
`StepBackward` は `klines_in(0..current_time)` の最大 `time` を検索するため、
自然に前の取引日 kline を返す（明示的な holiday 判定ロジックは不要）。

**テスト対象レンジ**: `2025-01-07 00:00` 〜 `2025-01-15 00:00` (UTC)
- 2025-01-07 (火), 01-08 (水), 01-09 (木), 01-10 (金), [01-11 土, 01-12 日], 01-13 (月), 01-14 (火), 01-15 (水)

**テストケース**:

| ID | 前提 | 操作 | 期待 |
|---|---|---|---|
| TC-A | Playing 後 Pause | StepForward ×3 で 01-10 (金) 付近まで進める | current_time ≈ 01-10 |
| TC-B | current_time ≈ 01-10 (金) | StepForward 1回 | current_time ≈ 01-11 (土) → 土日 kline なし |
| TC-C | current_time ≈ 01-11 (土) | StepBackward | current_time = 01-10 (金) ← 休場日スキップ |
| TC-D | current_time = 01-10 (金) | StepBackward | current_time = 01-09 (木) |
| TC-E | StepBackward 連続 5 回 | 毎回取引日に着地すること | 土日曜に止まらない |

**設計メモ**:
- `utc_offset` は相対時間専用のため、固定日付は node で直接ミリ秒計算する
- Tachibana は autoplay なし。`toggle + play` を手動発火する
- セッション確立待ちは最大 120 秒
- Playing 到達待ちは最大 180 秒（D1 データ全期間フェッチに時間がかかる場合がある）

---

## 知見・Tips

### CycleSpeed と §6.6 リセットの関係

```
// CycleSpeed の処理フロー（src/main.rs）
ReplayMessage::CycleSpeed:
  if let Some(clock) = self.replay.clock.as_mut() {
    clock.cycle_speed();
    clock.pause();
    clock.seek(replay.range.start_ms);
    dashboard.reset_charts_for_seek(main_window);
    inject_klines_up_to(replay.range.start_ms);
  }
```

`speed_to_10x()` ヘルパー内の `curl .../replay/resume` は、
CycleSpeed 後の Paused 状態から Playing に戻すためのもの。
s9 のテストは speed 変更直後に `wait_status Playing` するため、
「Paused + リセット」の状態を検証していない。

### Loading 状態の確実なキャッチ

Binance の kline ロードは通常 1〜3 秒。5h レンジ (300 bars) でも
キャッシュなしで 2〜5 秒程度かかる場合がある。
100ms ポーリングで確実にキャッチするためには、**set-ticker と Loading ポーリングを同一ステップで行う**:

```bash
api_post /api/pane/set-ticker "..."
LOADING_CAUGHT="false"
for i in $(seq 1 50); do
  ST=$(get_status)
  if [ "$ST" = "Loading" ]; then LOADING_CAUGHT="true"; break; fi
  sleep 0.1
done
```

### Tachibana D1 のステップ挙動

- `StepForward` (Paused 中) = `current_time + STEP_D1 (86400000ms)`
  → 土曜日のタイムスタンプに landing することがある（kline はないが current_time は移動する）
- `StepBackward` (Paused 中) = EventStore から `current_time` 未満の最大 kline.time を検索
  → kline が存在する取引日にのみ landing する = 自動的に休場日スキップ

---

## 進捗

| スクリプト | 状態 | 備考 |
|---|---|---|
| 計画書作成 | ✅ 完了 | 本ファイル |
| s27_cyclespeed_reset.sh | ✅ 完了 | TC-A〜D: Playing 中 CycleSpeed × 2 回 + Resume 確認 |
| s28_ticker_change_while_loading.sh | ✅ 完了 | TC-setup〜D: Loading キャッチ + ticker 変更 + reset 確認 |
| s29_tachibana_holiday_skip.sh | ✅ 完了 | TC-A〜E: DEV_USER_ID/DEV_PASSWORD 必須, なければ SKIP |
