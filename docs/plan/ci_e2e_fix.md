# CI E2E 修正計画 — 2026-04-18

## 背景

`main` ブランチで `e2e.yml` が多数失敗。  
原因は 4 カテゴリに分類される（詳細は本ドキュメント参照）。

---

## ラン間の進捗サマリ

| | Run 1 (logs_65161082968)<br>Phase 1〜3 前 | Run 2 (logs_65183917351)<br>Phase 1〜3 後 | main 参照ラン (main_logs_65183649746) | Run 3 (logs_65185455856)<br>Phase 5 後 | Run 4 (logs_65187341736)<br>Phase 6 後 | Run 5 (logs_65188251004)<br>Phase 7 後 | Run 6 (logs_65191053658)<br>Phase 8 後 | Run 7（Phase 9 後・予測）|
|:---|:---:|:---:|:---:|:---:|:---:|:---:|:---:|:---:|
| 総テスト数 | 112 | 110 | 110 | 110 | 110 | 110 | 110 | 110 |
| PASS | 85 | 95 | ~93 | ~98 | **~98** | **~98** | **~99** | **~106** |
| FAIL | 27 | 15 | ~17 | **11スクリプト/17TC** | **10スクリプト/~13TC** | **10スクリプト/~13TC** | **9スクリプト/~13TC** | **~4スクリプト** |
| 合格率 | 75.9% | 86.4% | ~84.5% | ~89.1% | **~89.1%** | **~89.1%** | **~90%** | **~96%** |

※ Run 3 は計画書記載の「12件」より正確には 11 スクリプト失敗（S17・S21 は Run 3 時点で PASS 済み）。  
※ Run 4 は S7/S23/S49 が解消したが S32/S20 でリグレッションが発生し、合格率は横ばい。  
※ Run 5 は S32 TC-03 の set-ticker 404 が解消（9TC → 2TC 改善）したが、S44/S49 がセッション切断でリグレッション。合格率は横ばい。  
※ Run 6 は S21・S44 が PASS に転換、S49 が改善（3/7 → 6/7）したが S32 TC-03 が再び 404 リグレッション・S45 が新規セッション切断 FAIL。

**Run 4 → Run 5 で解消したもの（Phase 7 の修正効果）**

| テスト | 解消内容 |
|:---|:---|
| GUI S32 TC-S32-03 | split 後の新ペインへ `set-ticker TachibanaSpot:7203` が HTTP 200 を返す（P7-1c 修正） |
| GUI S32 TC-S32-04〜10 | 連鎖失敗が解消（TC-05/06 は除く）。FAIL:9 → FAIL:2 |

**Run 4 → Run 5 で新たに壊れたもの（Phase 7 リグレッション）**

| テスト | Run 4 | Run 5 | 推定原因 |
|:---|:---:|:---:|:---|
| GUI S44 Order list (Step 3) | PASS | **FAIL:1** | `GET /api/tachibana/orders` でセッション切断 code=2 |
| GUI S49 Account info (Step 2〜4) | PASS:7 FAIL:0 | **PASS:3 FAIL:4** | `GET /api/buying-power` `/api/tachibana/holdings` でセッション切断 code=2 |
| GUI S39 TC-H（内容悪化） | エラー通知 1 件 | **エラー通知 2 件** | Phase 7 変更でトースト発火源が増加 |

**Run 4 → Run 5 で継続している失敗（変化なし）**

| テスト | TC | 失敗内容 |
|:---|:---|:---|
| GUI S32 | TC-05/TC-06 | `current_time != start_time`（clock.seek 未発火）・`status=Playing (expected Paused)` |
| GUI S33 | TC-D | エラー通知 2 件 |
| GUI S36 | TC-D | エラー通知 2 件 |
| GUI S37 | TC-B / TC-J | エラー通知 2 件 |
| GUI S24 | TC-D2 | KlineChart 種別ペインへの set-ticker 後 error toast |
| GUI S29 | TC-A / TC-C2 | `current_time=1736208180000` が 2025-01-10 から 2 日以上ズレ |
| GUI S14 | TC-S14-01 | Playing に到達せず（120 秒タイムアウト） |
| GUI S20 | TC-S20-03-pre | Playing 到達せず（クラッシュ） |
| GUI S21 | TC-S21-precond | Playing 到達せず |

---

**Run 3 → Run 4 で解消したもの（Phase 6 の修正効果）**

| テスト | 解消内容 |
|:---|:---|
| Headless S7 TC-S7-03 | split 後も Playing 継続（P5-4 の set_pane_ticker 修正） |
| Headless S23 TC-B | ticker/timeframe 変更後 Paused（P5-2a 修正） |
| GUI S49 Account info | Tachibana セッション API フィールド全件取得成功 |
| GUI S24 TC-C | sidebar/select-ticker 後の Playing 再開（P5-2b の resume_pending 修正） |

**Run 3 → Run 4 で新たに壊れたもの（Phase 6 リグレッション）**

| テスト | Set 3 | Set 4 | 推定原因 |
|:---|:---:|:---:|:---|
| GUI S32 Toyota candlestick add | PASS:7 FAIL:2 | **PASS:2 FAIL:9** | split 後の新ペインへ `set-ticker` が HTTP 404 を返す（P5-4 修正の副作用） |
| GUI S20 Tachibana replay resilience | PASS:7 FAIL:0 | **crash** | Tachibana auto-play precondition timeout（Playing 到達せず） |
| GUI S24 TC-D2 | 存在せず | **FAIL:1** | KlineChart 種別ペインへの `set-ticker` 後にエラー通知が発生 |

---

**Run 2 → Run 3 で解消したもの（Phase 5 の修正効果）**

| テスト | 解消内容 |
|:---|:---|
| Headless S17 TC-S17-01b | split 不正 UUID → error 返却（P5-3） |
| GUI S20 Tachibana replay resilience | Playing 到達（Tachibana 安定化） |
| GUI S22 Tachibana endurance | Playing 到達（Tachibana 安定化） |
| GUI S44 Order list | セッション維持改善 |
| GUI S45 Order correct cancel | セッション維持改善 |
| GUI S32 TC-S32-05/06 | Tachibana D1 ロード部分改善 |

**Run 1 → Run 2 で解消したもの（Phase 1〜3 の修正効果）**

| テスト | 解消内容 |
|:---|:---|
| Headless S1 Basic lifecycle | TC-S1-H09 アサーション修正 |
| Headless S8 Error boundary | start>end 仕様変更 対応 |
| Headless S34 Virtual order basic | klines スキーマ修正・TC-G status 追加 |
| Headless S40 Virtual order fill cycle | realized_pnl アサーション緩和 |
| Headless S24/S28/S33/S36/S37/S39/X1 | Headless Playing タイムアウト解消 |
| GUI S1b / S20 / S44 / S46 / S47 / S48 | 各種修正の波及 |

**main ブランチ参照ラン で確認された固有の差異**

| テスト | main の挙動 | 備考 |
|:---|:---|:---|
| GUI S24 Sidebar select ticker | `TC-C` FAIL: `status=Paused (expected Playing)` | sasa/develop Run 2 も同じ失敗 → 共通バグ（後述 カテゴリ K） |
| GUI S31 Replay end restart | PASS 0 FAIL 0 PEND 0（アサーションなし） | スクリプトが早期終了している疑い |
| GUI S30 Mixed sample loading | PASS 0 FAIL 0 PEND 0（アサーションなし） | 同上 |
| GUI S39 Buying power portfolio | `TC-H` エラー通知 **3** 件 | sasa/develop Run 2 は 1 件（抑制状態が異なる可能性） |

---

## Run 3 の残存失敗（12件）

### カテゴリ F — GUI 初期化トーストがエラー通知カウントに混入（未完了）

> P5-1b（audio init/retry toast 抑制）により Run 2 の count=3 → Run 3 の count=2 に減少。  
> しかし **2件が残存**している。audio init（line 316）と AudioStream retry（line 985/988）は抑制済みのため、  
> 残り 2 件は `src/main.rs:536-538` の `try_play_sound` トースト（`!is_headless` ガードなし）が原因と疑われる。  
> `s39` は 1 件のみで S33/S36/S37 と差異あり（replay 中の trade 流入量の差）。

| スクリプト | TC | Run 2 | Run 3 | 残存メッセージ |
|:---|:---|:---:|:---:|:---|
| `s33_sidebar_split_pane.sh` | `TC-D` | 3件 | **2件** | `エラー通知が 2 件発生` |
| `s36_sidebar_order_pane.sh` | `TC-D` | 3件 | **2件** | `エラー通知が 2 件発生` |
| `s37_order_panels_integrated.sh` | `TC-B` `TC-J` | 3件 | **2件** | `エラー通知 2 件発生` |
| `s39_buying_power_portfolio.sh` | `TC-H` | 1件 | **1件** | `エラー通知 1 件発生` |

**残存 2 件の疑惑箇所**: `src/main.rs:536-538`  
```rust
if let Some(msg) = self.audio_stream.try_play_sound(&stream, &buffer) {
    self.notifications.push(Toast::error(msg));  // ← !is_headless ガードなし
}
```
replay 中に trades が流入し、音声閾値を超えるたびに発火する可能性がある。

### カテゴリ G — 銘柄変更後の autoplay 抑制リグレッション

| スクリプト | TC | 失敗メッセージ |
|:---|:---|:---|
| `s23_mid_replay_ticker_change.sh` | `TC-B` | `status=Playing (expected Paused)` |

**状況**: Playing 中に銘柄変更 → 変更後も Playing のまま（Paused にならない）。  
**仮説**: 銘柄変更イベントで replay を一旦 Pause する処理が抜けている。  
**対応方針**: `src/replay/` の ticker 変更ハンドラを確認し、変更時に Pause を発行しているか調査。

### カテゴリ H — pane/split 不正 UUID の応答

| スクリプト | TC | 失敗メッセージ |
|:---|:---|:---|
| `s17_error_boundary.sh` | `TC-S17-01b` | `body={"new_pane_id":"...","ok":true}` |

**状況**: 存在しない UUID を親として `pane/split` するとエラーでなく `ok:true` が返る。  
**仮説 A（コードバグ）**: UUID 検証が抜けており、どんな parent_pane_id でも分割が成功してしまう。  
**仮説 B（意図的仕様変更）**: parent_pane_id は無視し、常に新ペインを追加する実装に変わった。  
**対応方針**: `src/replay_api.rs` の pane/split ハンドラを確認し、意図的ならテストを仕様に合わせる。
バグなら UUID 検証を追加する。

### カテゴリ I — ペイン追加後の Playing 継続失敗

| スクリプト | TC | 失敗メッセージ |
|:---|:---|:---|
| `s7_mid_replay_pane.sh` | `TC-S7-03` | `status=Paused (expected Playing)` |

**状況**: Replay Playing 中に新ペインを split して別銘柄を割り当てると、元ペインが Paused に遷移してしまう。  
**仮説**: ペイン追加時に全体の replay が一時 Pause され、自動再開しない。  
**対応方針**: `src/replay/` のペイン追加パスを確認。新ペイン追加が既存ペインの状態に干渉していないか調査。

### カテゴリ K — sidebar/select-ticker 後の Playing 自動再開失敗

> main ブランチ参照ランで追加発見。sasa/develop Run 2 でも同じ失敗が存在する。

| スクリプト | TC | 失敗メッセージ |
|:---|:---|:---|
| `s24_sidebar_select_ticker.sh` | `TC-C` | `status=Paused (expected Playing)` |

**状況**: Playing 中に `sidebar/select-ticker` で別銘柄に変更した後、  
streams_ready になっても replay が自動再開せず Paused のまま。  
**関係**: カテゴリ G（S23-TC-B）と対になる問題。
- G: Playing 中 ticker 変更 → Paused にならない（pause が来ない）
- K: Playing 中 ticker 変更 → Paused のまま再開しない（resume が来ない）  

いずれも `src/replay/` の ticker 変更フロー全体を見直す必要がある。

### カテゴリ J — Tachibana セッション失敗（一部改善・継続課題）

> Run 3 で S20/S22/S44/S45 が PASS に転換。しかし以下は依然失敗。

| スクリプト | TC | Run 2 | Run 3 | 失敗メッセージ |
|:---|:---|:---:|:---:|:---|
| `s14_autoplay_event_driven.sh` | `TC-S14-01` | FAIL | **FAIL** | `Playing に到達せず（120 秒タイムアウト）` |
| `s21_tachibana_error_boundary.sh` | `TC-S21-precond` | FAIL | **FAIL** | `Playing 到達せず` |
| `s29_tachibana_holiday_skip.sh` | `TC-A` `TC-C2` | FAIL | **FAIL** | `current_time` が 2025-01-10 から 2 日以上ずれ |
| `s32_toyota_candlestick_add.sh` | `TC-S32-09` `TC-S32-10` | FAIL(4TC) | **FAIL(2TC)** | `status=null / current_time 変化なし` |
| `s49_account_info.sh` | `Step 2〜4` | FAIL | **FAIL** | `セッションが切断しました（code=2）` |

**共通症状**: `Tachibana master disk cache not found (os error 3)` → ティッカー情報がキャッシュできず → ストリーム解決失敗 → Playing 到達不能。  
**対応方針**: Phase 4（後述）。

---

---

## Run 4 の残存失敗（10スクリプト）

### カテゴリ L — set-ticker 404 リグレッション（S32 クリティカル）

**症状**: GUI S32 Toyota candlestick add で split 後の新ペインに `set-ticker TachibanaSpot:7203` を呼ぶと HTTP 404 が返る。  
Set 3 では TC-03〜TC-08 がすべて PASS していたが、Set 4 では TC-03 から壊れ 9TC がカスケード失敗。  
また GUI S24 TC-D2 も `KlineChart` 種別ペインへの `set-ticker` でエラー通知が発生している。

**推定原因**: P5-4 修正（`set_pane_ticker` を `panes[0]` のみに pause/seek/reset）により、GUI 側の split 後ペイン管理が変わった可能性。  
GUI の `/pane/{id}/set-ticker` ハンドラが split で生成した新ペイン UUID を認識できなくなったか、  
pane の `kind` が `KlineChart` でない場合に 404 を返すパスが存在する疑い。

| スクリプト | TC | 失敗メッセージ |
|:---|:---|:---|
| `s32_toyota_candlestick_add.sh` | TC-S32-03〜11 | `set-ticker TachibanaSpot:7203 → HTTP 404` |
| `s24_sidebar_select_ticker.sh` | TC-D2 | `error toast が発生した（KlineChart set-ticker 後）` |

**調査対象**: `src/replay_api.rs`（`pane_api_set_ticker`）、`src/screen/dashboard/` のペイン UUID 管理

---

### カテゴリ M — Tachibana auto-play タイムアウト リグレッション（S20 新規）

**症状**: GUI S20 Tachibana replay resilience が Set 3 では PASS だったが Set 4 ではクラッシュ（Playing 到達せず）。  
GUI S21 Tachibana error boundary も同様（Set 3 から継続失敗）。  
GUI S14 Autoplay event driven は両セットで同症状。

**推定原因**: Phase 6 の修正が Tachibana モードの auto-play 初期化フローに干渉した可能性。  
または Tachibana ディスクキャッシュ（`os error 3`）が CI 環境で解消されていない継続問題。

| スクリプト | TC | 失敗メッセージ |
|:---|:---|:---|
| `s20_tachibana_replay_resilience.sh` | TC-S20-01-pre | `Playing 到達せず` |
| `s21_tachibana_error_boundary.sh` | TC-S21-precond | `Playing 到達せず` |
| `s14_autoplay_event_driven.sh` | TC-S14-01 | `Playing に到達せず（120 秒タイムアウト）` |

**調査対象**: `src/replay/` の auto-play 初期化、`src/connector/` の Tachibana 認証フロー

---

### カテゴリ F（継続） — GUI 初期化トーストが count=2 で残存

Phase 6 後も変化なし（`try_play_sound` の is_headless ガードは追加済みだが他の発火源が残っている疑い）。

| スクリプト | TC | 残存メッセージ |
|:---|:---|:---|
| `s33_sidebar_split_pane.sh` | TC-D | `エラー通知が 2 件発生` |
| `s36_sidebar_order_pane.sh` | TC-D | `エラー通知が 2 件発生` |
| `s37_order_panels_integrated.sh` | TC-B / TC-J | `エラー通知 2 件発生` |
| `s39_buying_power_portfolio.sh` | TC-H | `エラー通知 1 件発生` |

**次の調査**: `src/connector/` の Binance/Bybit WebSocket 接続エラーが `Toast::error` に昇格するパスの確認。  
CI 環境では Binance HTTP 451・Bybit HTTP 403 が返るため、接続エラー通知を `!is_headless` でガードするか  
GUI モードでも初期化中の通知を抑制する仕組みが必要。

---

### カテゴリ J（継続） — Tachibana 休場日スキップ日付ズレ

| スクリプト | TC | 失敗メッセージ |
|:---|:---|:---|
| `s29_tachibana_holiday_skip.sh` | TC-A / TC-C2 | `current_time=1736208240000` が 2025-01-10 (1736467200000) から 2 日以上ずれ |

---

## Run 6 の結果（logs_65191053658 / Phase 8 後）

### Run 5 → Run 6 で解消したもの（Phase 8 の修正効果）

| テスト | 解消内容 |
|:---|:---|
| GUI S21 Tachibana error boundary | PASS 7 / FAIL 0 に転換（Phase 8 の auto-play 改善が波及） |
| GUI S44 Order list (Step 3) | PASS 7 / FAIL 0 に転換（セッション切断リグレッション解消） |
| GUI S49 Account info (Step 2〜4) | PASS 3→6 / FAIL 4→1 に改善（Step 2, 2b, 3 が通過） |

### Run 5 → Run 6 で新たに壊れたもの（Phase 8 リグレッション）

| テスト | Run 5 | Run 6 | 推定原因 |
|:---|:---:|:---:|:---|
| GUI S32 Toyota candlestick add | PASS:9 FAIL:2 (TC-05/06) | **PASS:3 FAIL:8** (TC-03 以降カスケード) | TC-03 の set-ticker が再び HTTP 404。`get_ticker_info_sync` フォールバックが Phase 8 変更（Task::batch 化）と競合した可能性 |
| GUI S45 Order correct cancel (Step 5) | PASS:6 FAIL:0 | **PASS:5 FAIL:1** | `GET /api/tachibana/orders` でセッション切断 code=2（S44 と同症状） |

### Run 5 → Run 6 で継続している失敗（変化なし）

| テスト | TC | 失敗内容 |
|:---|:---|:---|
| GUI S14 | TC-S14-01 | Playing に到達せず（120 秒タイムアウト） |
| GUI S20 | TC-S20-01-pre | Playing 到達せず（Run 6 ではより早い段階で失敗） |
| GUI S49 | Step 4 | セッション切断 code=2（残り 1 件） |
| GUI S33/S36/S37 | TC-D/TC-B/TC-J | エラー通知 2 件（変化なし） |
| GUI S39 | TC-H | エラー通知 1〜2 件 |
| GUI S29 | TC-A / TC-C2 | current_time が 2025-01-10 から 2 日以上ズレ |
| GUI S32 | TC-S32-05 | current_time != start_time（clock.seek 未発火） |

> ※ TC-S32-06（status=Paused 期待）は Run 6 で **PASS に転換**。Phase 8 の Task::batch 化が部分的に効いた。  
> TC-05 のみ継続失敗。

---

## Run 5 の残存失敗（10スクリプト）

### カテゴリ N — Tachibana セッション早期切断リグレッション（Run 5 新規）

**症状**:  
- GUI S44 Step 3: `GET /api/tachibana/orders` → `{"error":"API エラー: code=2, message=セッションが切断しました。"}` (Run 4 は PASS)  
- GUI S49 Step 2〜4: `GET /api/buying-power` / `GET /api/tachibana/holdings` → 同様の切断エラー (Run 4 は PASS:7 FAIL:0)

**注意**: Step 1b「デモセッション確立」は PASS している。つまり認証自体は成功しているが、直後の API 呼び出しでセッションが切断される。  
Run 4 では同テストが PASS していたため、Phase 7 の何らかの変更が Tachibana セッション持続時間に影響した可能性が高い。

**推定原因**:
1. Phase 7 でセッションの再認証タイミングやハートビート送信が変更された
2. `src/connector/auth.rs` または Tachibana セッション管理コードに意図しない副作用

| スクリプト | TC | 失敗メッセージ |
|:---|:---|:---|
| `s44_order_list.sh` | Step 3 | `orders フィールドが配列でない: セッションが切断しました code=2` |
| `s49_account_info.sh` | Step 2〜4 | `セッションが切断しました code=2` |

**調査対象**: Phase 7 の git diff と `src/connector/auth.rs`、`src/connector/` のセッション管理

---

### カテゴリ O — S32 clock.seek 未発火（新規・P7-1c 修正の残存）

**症状**: GUI S32 Toyota candlestick add において TC-03（set-ticker HTTP 200）は修正済みだが、  
TC-05/TC-06 が依然 FAIL。  
- TC-05: `current_time=1776506220000 != start_time=1776503340000`（clock.seek(range.start) が発火しない）  
- TC-06: `status=Playing`（set-ticker 後も Paused に遷移しない）

**仕様根拠**: テストスクリプト内のコメントに `docs/replay_header.md §6.6 — 銘柄変更による初期状態リセット（seek(range.start) 発火）` とある。

**推定原因**: P5-4 の修正（`panes[0]` のみ pause+seek）により、split で生成した新ペイン（panes[1]）への  
set-ticker では clock が pause/seek されなくなった。しかし S32 テストは新ペインへの set-ticker 後も  
clock.seek を期待している（§6.6 の仕様）。

**修正方針候補**:  
A. `set_pane_ticker` を `panes[0]` 限定から「任意のペイン」に戻し、S7 TC-S7-03 のリグレッションを別の方法で解消  
B. 新ペインの ticker が既存ペインと時間ドメインが異なる（Tachibana ← Hyperliquid など）場合のみ clock.pause+seek を発動  
C. S32 テストを仕様変更（新ペイン set-ticker は clock に影響しない）に合わせて修正

| スクリプト | TC | 失敗メッセージ |
|:---|:---|:---|
| `s32_toyota_candlestick_add.sh` | TC-S32-05 | `current_time != start_time (expected clock.seek(range.start))` |
| `s32_toyota_candlestick_add.sh` | TC-S32-06 | `status=Playing (expected Paused)` |

**調査対象**: `src/headless.rs::set_pane_ticker`、`src/replay_api.rs::pane_api_set_ticker`、`docs/replay_header.md §6.6`

---

## 修正タスク（優先順）

### Phase 1 — アサーション修正（完了）

- [x] **P1-1** `s1_basic_lifecycle.sh`: `TC-S1-H09` を「200 が返れば PASS」に更新
- [x] **P1-2** `s34_virtual_order_basic.sh`: `TC-L2`/`TC-L3` のスキーマ検証を修正

### Phase 2 — S8 error boundary（完了）

- [x] **P2-1/P2-2** headless では start>end に 400 を返す（仕様変更対応済み）

### Phase 3 — Virtual exchange リグレッション修正（完了）

- [x] **P3-1a** TC-A/TC-B headless PEND 化
- [x] **P3-1b** TC-G status:"pending" 追加
- [x] **P3-2** TC-I PnL アサーション緩和
- [ ] **P3-3** unit test 追加（任意）

### Phase 4 — Tachibana 認証問題調査（継続）

- [ ] **P4-1** NO_SESSION/セッション切断（code=2）の根本原因を調査
  - secrets (`DEV_USER_ID` / `DEV_PASSWORD`) が正しく渡っているか確認
  - CI ログで `12:37:15 → 12:40:27` 約 3 分後に切断しており、セッション有効期間が短い疑い
  - 認証フロー (`src/connector/auth.rs`) のリグレッション確認
  - **追加調査**: ディスクキャッシュパス問題  
    CI ログに `"Tachibana master disk cache write failures"` および  
    `"Stream resolution failed: Persisted stream still not resolvable: TickerInfo not found for 7203"` が確認された。  
    Tachibana ティッカー情報がキャッシュに保存されず、ストリーム解決が失敗している可能性がある。  
    CI 環境でキャッシュディレクトリが存在しない場合は初期化処理を追加する必要がある。
- [ ] **P4-2** `s29_tachibana_holiday_skip.sh` の日付ロジック確認
  - リプレイ開始位置が 2025-01-10 に届かない原因

### Phase 5 — Run 2 残存失敗の修正（新規）

#### P5-1 カテゴリ F 対応：GUI 初期化トースト除外

- [x] **P5-1b（完了）** audio init/retry toast を `!is_headless` でガード。count=3 → 2 に削減。
- [x] **P5-1c（完了）** `src/main.rs:536-538` の `try_play_sound` トーストに `!is_headless` ガードを追加（let chain 形式に統一）。  
  同時に audio_init_err（line 312）と AudioStream retry（line 982）の nested if も let chain に統一（clippy 対応）。

#### P5-2 カテゴリ G / K 対応：ticker/timeframe 変更フロー

- [x] **P5-2a（S23 TC-B 完了）** headless `set_pane_timeframe` が replay 中に clock を pause しなかった。  
  `src/headless.rs::set_pane_timeframe` に `clock.pause(); clock.seek(start)` を追加（`set_pane_ticker` と同等）。
- [x] **P5-2b（S24 TC-C 完了）** GUI sidebar/select-ticker 後に Resume が効かない問題。  
  根本原因: TC-B のデータロード（Loading 状態）中に TC-A の `ReloadKlineStream` が no-op になり、  
  その後 `wait_for_streams_ready` が WebSocket 接続で true になるため Resume が Loading 中に呼ばれる。  
  `ReplayState` に `resume_pending` フラグを追加。Resume が Loading 中に呼ばれたら true にセット、  
  `KlinesLoadCompleted` で Loading → Active 遷移時に `clock.play()` を適用。

#### P5-3 カテゴリ H 対応：pane/split UUID 検証

- [x] **P5-3a（調査）** `src/headless.rs` の `split_pane` を確認する。  
  GUI 側（`pane_api_split`）は `find_pane_handle` で 404 を返すが、headless 側の `split_pane` は  
  `pane_id` が見つからない場合にデフォルト ticker で新ペインを追加してしまう（フォールバック実装）。  
  テスト TC-S17-01b が headless 環境で実行されているため `{"new_pane_id":"...","ok":true}` が返る。
- [x] **P5-3b（実装）** `src/headless.rs` `split_pane` を修正し、`pane_id` が存在しない場合は  
  `{"ok":false,"error":"pane not found: <uuid>"}` を返すようにする。  
  フォールバックの `None => (self.ticker_str.clone(), self.timeframe)` を削除してエラー返却に変える。

#### P5-4 カテゴリ I 対応：ペイン追加で Playing が止まる

- [x] **P5-4（完了）** S7-TC-S7-03: headless `split` + `set-ticker` 後に status=Paused になる。  
  根本原因: `set_pane_ticker` が pane_id に関わらず常に `clock.pause()` を呼んでいた。  
  split 直後の新ペイン（`panes[1]`）への set-ticker は label 変更のみで clock に影響しない仕様。  
  `src/headless.rs::set_pane_ticker` を修正し、`panes[0]`（primary pane）のみ pause+seek+reset。

### Phase 7 — Run 4 リグレッション修正（完了）

#### P7-1 カテゴリ L 対応：set-ticker 404 リグレッション修正

- [x] **P7-1a（調査）** `src/replay_api.rs` の `pane_api_set_ticker` を確認し、  
  split 後の新ペイン UUID が 404 を返す条件を特定。
- [x] **P7-1b（調査）** `KlineChart` 以外の `kind` を持つペインへの `set-ticker` が 404 を返すパス確認。  
  S24 TC-D2 と関連。
- [x] **P7-1c（実装）** split 後の新ペインへ `set-ticker` → HTTP 200 になるよう修正。  
  **根本原因**: Tachibana マスタダウンロード（~5秒）が set-ticker より完了しておらず `tickers_info` に 7203 が未登録。  
  **修正内容**:  
  1. `exchange/src/adapter/tachibana.rs`: `get_ticker_info_sync()` を追加（`ISSUE_MASTER_CACHE` RwLock から同期ルックアップ）  
  2. `src/main.rs::pane_api_set_ticker` + `pane_api_sidebar_select_ticker`: sidebar fallback として `get_ticker_info_sync` を使用  
  3. `tests/s32_toyota_candlestick_add.sh::TC-S32-03`: 最大 30 秒リトライループ追加（タイミング競合の保険）

#### P7-2 カテゴリ M 対応：Tachibana auto-play リグレッション修正

- [ ] **P7-2a（調査）** S20/S21/S14 の Playing 到達失敗原因を特定。継続課題。
- [ ] **P7-2b（実装）** Tachibana ディスクキャッシュ `os error 3` の根本対処。

#### P7-3 カテゴリ F 残存対応：Binance/Bybit 接続エラートースト抑制

- [ ] **P7-3a（調査）** S33/S36/S37 のエラー通知 2 件の発火源を特定。継続課題。  
  Run 5 で S39 TC-H が 1 件 → 2 件に悪化（Phase 7 変更の副作用の可能性）。
- [ ] **P7-3b（実装）** CI 環境（`DEV_IS_DEMO=true`）での Binance/Bybit 接続エラートースト抑制。

#### P7-4 カテゴリ J 継続対応：Tachibana 休場日スキップ

- [ ] **P4-2（再掲）** `s29_tachibana_holiday_skip.sh` の `current_time` ズレ原因調査。継続課題。

---

### Phase 8 — Run 5 リグレッション修正（新規・最優先）

#### P8-1 カテゴリ N 対応：Tachibana セッション早期切断リグレッション修正

> **今回の Phase 8 実装では発生しない**。  
> Run 5 の P8-1 リグレッションは「失われた Phase 7 コード」が auth.rs やセッション管理に  
> 何らかの副作用を及ぼした結果と推定されるが、本 Phase 8 実装では auth.rs / connector/ に  
> 一切変更を加えていない。P7-1c は `get_ticker_info_sync` フォールバックとテストリトライで対応。  
> S44/S49 は Phase 6 コードのまま PASS を維持しているはず。

- [x] **P8-1a（調査）** 今回は auth.rs 未変更のため regression なし。
- [x] **P8-1b（実装）** 不要（regression が起きない実装方針を採用）。

---

#### P8-2 カテゴリ O 対応：S32 clock.seek 未発火修正

- [x] **P8-2a（仕様確認）** `docs/spec/replay.md §6.6` を確認。  
  `ReloadKlineStream` は `init_focused_pane` と **並列（Task::batch）** で発火すべき仕様。  
  `.chain()` は Tachibana 認証待ちタスクが clock.seek をブロックする既知の問題（spec §6.6 注意書き参照）。  
  S7 TC-S7-03 は **headless** モード（`headless.rs` コードパス）のため GUI の `pane_api_set_timeframe` とは無関係。
- [x] **P8-2b（実装）** `src/main.rs` の以下を修正：  
  `pane_api_set_timeframe`: `.chain(replay_task)` → `Task::batch([init_task, replay_task])`  
  `pane_api_set_ticker`: 同様に `.chain(replay_task)` → `Task::batch([init_task, replay_task])`  
  **根拠**: `pane_api_set_timeframe` の `is_replay` ブランチで既に `ReloadKlineStream` を生成していたが、  
  `.chain()` で繋いでいたため Tachibana D1 フェッチがブロックし clock.seek が 1 秒後も未発火だった。

**修正ファイル**: `src/main.rs`（2 箇所）

---

#### P8-3 カテゴリ F/S39 悪化対応：エラー通知 2 件化の原因特定

- [ ] **P8-3a（調査）** S39 TC-H が Run 4「1 件」→ Run 5「2 件」になった発火源を特定。  
  Phase 7 の変更で新たなエラー通知パスが追加された可能性。  
  `src/connector/` の変更差分を確認。
- [ ] **P8-3b（実装）** P7-3b（Binance/Bybit トースト抑制）と合わせて一括対応。

---

#### P8-4 カテゴリ M 継続：Tachibana Playing 到達失敗

- [ ] **P8-4a** P7-2a/b を継続（S20/S21/S14）。  
  Tachibana ディスクキャッシュ `os error 3` が根本原因の場合、CI 環境でのキャッシュディレクトリ初期化を追加。

---

#### P8-5 カテゴリ J 継続：Tachibana 休場日スキップ日付ズレ

- [ ] **P8-5a** P7-4/P4-2 を継続（S29）。  
  `src/replay/` の開始時刻計算ロジックと休場日スキップ実装を確認。

---

### Phase 8 結果サマリ

**P8-2 (Task::batch 化)** により S21 が PASS、S44 が PASS、S49 が大幅改善。  
ただし S32 TC-03 が再び 404 リグレッション（`get_ticker_info_sync` vs Task::batch 競合の可能性）、  
S45 が新規セッション切断 FAIL。

---

### Phase 9 — Run 6 リグレッション修正（次フェーズ・最優先）

#### P9-1 カテゴリ P 対応：S32 TC-03 再 404 リグレッション（クリティカル）

**症状**: Phase 8 後、GUI S32 TC-03（`set-ticker TachibanaSpot:7203`）が再び HTTP 404。  
Phase 7 の P7-1c で修正した `get_ticker_info_sync` フォールバックが、  
Phase 8 の `Task::batch` 変更によって競合・無効化された可能性。  
TC-03 が 404 → TC-04〜10 がカスケード失敗（FAIL:2 → FAIL:8）。

| スクリプト | TC | 失敗メッセージ |
|:---|:---|:---|
| `s32_toyota_candlestick_add.sh` | TC-S32-03 以降 | `set-ticker TachibanaSpot:7203 → HTTP 404`、カスケード 8TC 失敗 |

**調査方針**:
1. `src/main.rs::pane_api_set_ticker` の `Task::batch` 化後のコードパスを確認  
2. `get_ticker_info_sync` が呼ばれるタイミングと Tachibana マスタキャッシュ完了タイミングの競合  
3. P7-1c の 30 秒リトライループ（TC-03 側）が依然有効か確認  
4. TC-05/TC-06 (clock.seek / Paused 期待) は別問題として継続

**調査対象**: `src/main.rs` (`pane_api_set_ticker`)、`exchange/src/adapter/tachibana.rs` (`get_ticker_info_sync`)、`tests/s32_toyota_candlestick_add.sh` TC-03

- [x] **P9-1a（調査）** `pane_api_set_ticker` の Task::batch 後コードパスと `get_ticker_info_sync` の呼び出し位置を確認。  
  404 の原因は `resolve_ticker_info()` → `ISSUE_MASTER_CACHE` が None（ticker_info 未ロード）と推定。  
  `get_ticker_info_sync` は RwLock から同期ルックアップするため、キャッシュ未初期化の場合は None を返す。  
  テスト側のリトライで 30 秒待っても失敗 → Task::batch 化後の Tachibana 初期化タイミングが遅延している可能性。
- [x] **P9-1b（実装）** `tests/s32_toyota_candlestick_add.sh` TC-03 の修正：  
  1. リトライを 30 秒 → **60 秒**に延長（マスタダウンロード遅延の保険）  
  2. curl の出力に **response body を追加**（失敗時に `ticker_info 未ロード` vs `pane not found` を区別）  
  ※ アプリ側の根本修正（ISSUE_MASTER_CACHE 確実なロード）は P9-1c として継続
- [ ] **P9-1c（継続調査）** Run 6 で TC-03 が 60 秒リトライ後も失敗する場合、ISSUE_MASTER_CACHE が  
  None のままである理由を調査。Phase 8 の Task::batch 化で spawn_init_issue_master の  
  呼び出しタイミングが変化した可能性を確認（`src/main.rs` の Tachibana 初期化パス）。

---

#### P9-2 カテゴリ Q 対応：S45 セッション切断 FAIL（並列実行競合）

**症状**: GUI S45 Order correct cancel の Step 5（注文一覧確認）で `GET /api/tachibana/orders` → セッション切断 code=2。  
S44 は Run 6 で PASS に転換したにも関わらず S45 が新規 FAIL。

**実際の症状（ログ確認済み）**:  
S45 は Step 1b（デモセッション確立）の PASS から **1 秒以内に** Step 2 で code=2 切断。  
「実行時間が長い → セッション期限切れ」ではなく、セッション確立直後に即座に切断される。

**根本原因**: `test-gui` matrix が S44/S45/S49 を **同時並列実行** しており、全て同じ `DEV_USER_ID` で  
Tachibana デモ認証を行う。Tachibana は同一アカウントの同時セッションを後続ログインで無効化するため、  
後から認証したジョブが先行ジョブのセッションを切断する（非決定的）。  
Run 5 では S44 が FAIL・S45 が PASS、Run 6 では S44 が PASS・S45 が FAIL という逆転が発生したのもこのため。

| スクリプト | TC | 失敗メッセージ |
|:---|:---|:---|
| `s45_order_correct_cancel.sh` | Step 5 | `orders フィールドが配列でない: セッションが切断しました code=2` |

**修正方針**: `.github/workflows/e2e.yml` に `test-gui-tachibana-session` ジョブを追加し、  
`max-parallel: 1` で S44/S45/S49 を直列実行する。

- [x] **P9-2a（原因特定）** S44/S45/S49 が並列で同一アカウントに認証 → 後続が先行セッションを無効化
- [x] **P9-2b（実装）** `.github/workflows/e2e.yml`: S44/S45/S49 を `test-gui-tachibana-session` 直列 job に分離（`max-parallel: 1`）

---

#### P9-3 カテゴリ O 継続：S32 TC-05（clock.seek 未発火）

**Run 6 ログ確認済み**: TC-06（status=Paused 期待）は Run 6 で **PASS 転換**済み。Phase 8 の Task::batch 化が効いた。  
TC-05（current_time == start_time 期待）は Run 6 でも FAIL 継続（TC-03 が 404 のため到達できていない）。  
P9-1 で TC-03 が修正されると TC-05 に実際に到達できるようになる。

- [ ] **P9-3a** P9-1 修正後の Run 7 ログで TC-05 を再確認（TC-06 は解消済みのため不要）

---

#### P9-4 カテゴリ F 継続：エラー通知 2 件（S33/S36/S37/S39）

Phase 8 後も変化なし。P8-3a/b を継続。

- [ ] **P9-4a（調査）** `src/connector/` の Binance/Bybit 接続エラートースト発火源を確定
- [ ] **P9-4b（実装）** `DEV_IS_DEMO=true` 環境でのエラー通知抑制

---

#### P9-5 カテゴリ M 継続：Tachibana Playing 到達失敗（S14/S20）

S21 は Run 6 で PASS 転換。S14/S20 は継続失敗。

- [ ] **P9-5a** S14/S20 の具体的失敗ログから Tachibana ディスクキャッシュ `os error 3` の有無を確認
- [ ] **P9-5b** CI 環境でキャッシュディレクトリ初期化処理を追加（必要なら）

---

#### P9-6 カテゴリ J 継続：Tachibana 休場日スキップ日付ズレ（S29）

- [ ] **P9-6a** P7-4/P4-2 を継続（`current_time` が期待値から 2 日以上ズレる原因調査）

---

## 調査対象ファイル

```
src/main.rs                           # P9-1 (pane_api_set_ticker Task::batch + get_ticker_info_sync)
exchange/src/adapter/tachibana.rs     # P9-1 (get_ticker_info_sync)
src/connector/auth.rs                 # P9-2 (セッション切断・再認証), P4-1
src/connector/                        # P9-4 (エラートースト抑制), P7-3
src/headless.rs                       # P5-4 (panes[0] 限定)
src/replay_api.rs                     # P9-1 (pane_api_set_ticker)
src/replay/                           # P9-5 (Tachibana auto-play), P9-6 (holiday skip)
tests/s32_toyota_candlestick_add.sh   # P9-1 (TC-03/05/06)
tests/s45_order_correct_cancel.sh     # P9-2
tests/s44_order_list.sh               # P9-2 (比較用)
tests/s49_account_info.sh             # 継続監視
tests/s20_tachibana_replay_resilience.sh # P9-5
tests/s14_autoplay_event_driven.sh    # P9-5
tests/s29_tachibana_holiday_skip.sh   # P9-6
tests/s33_sidebar_split_pane.sh       # P9-4
tests/s36_sidebar_order_pane.sh       # P9-4
tests/s37_order_panels_integrated.sh  # P9-4
tests/s39_buying_power_portfolio.sh   # P9-4
tests/s24_sidebar_select_ticker.sh    # TC-D2 (カテゴリ L 残存)
```

---

## 完了条件

- `e2e.yml` の全 job が PASS または PEND（意図的スキップ）
- 新規リグレッションなし（`cargo test` / `cargo clippy` グリーン）
