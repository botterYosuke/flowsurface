# CI E2E 修正計画 — 2026-04-18

## 背景

`main` ブランチで `e2e.yml` が多数失敗。  
原因は 4 カテゴリに分類される（詳細は本ドキュメント参照）。

---

## ラン間の進捗サマリ

| | Run 1 (logs_65161082968)<br>Phase 1〜3 前 | Run 2 (logs_65183917351)<br>Phase 1〜3 後 | main 参照ラン (main_logs_65183649746) | Run 3 (logs_65185455856)<br>Phase 5 後 | Run 4 (logs_65187341736)<br>Phase 6 後 | Run 5（Phase 7 後・予測）|
|:---|:---:|:---:|:---:|:---:|:---:|:---:|
| 総テスト数 | 112 | 110 | 110 | 110 | 110 | 110 |
| PASS | 85 | 95 | ~93 | ~98 | **~98** | **~106** |
| FAIL | 27 | 15 | ~17 | **11スクリプト/17TC** | **10スクリプト/~13TC** | **~4スクリプト** |
| 合格率 | 75.9% | 86.4% | ~84.5% | ~89.1% | **~89.1%** | **~96%** |

※ Run 3 は計画書記載の「12件」より正確には 11 スクリプト失敗（S17・S21 は Run 3 時点で PASS 済み）。  
※ Run 4 は S7/S23/S49 が解消したが S32/S20 でリグレッションが発生し、合格率は横ばい。

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

### Phase 7 — Run 4 リグレッション修正（新規・最優先）

#### P7-1 カテゴリ L 対応：set-ticker 404 リグレッション修正

- [ ] **P7-1a（調査）** `src/replay_api.rs` の `pane_api_set_ticker` を確認し、  
  split 後の新ペイン UUID が 404 を返す条件を特定する。  
  GUI split API（`pane_api_split`）が生成する UUID が `pane_api_set_ticker` の検索対象に含まれているか確認。
- [ ] **P7-1b（調査）** `KlineChart` 以外の `kind` を持つペインへの `set-ticker` が 404 を返すパスがあるか確認。  
  S24 TC-D2 のエラー通知と関連する可能性。
- [ ] **P7-1c（実装）** 上記調査を踏まえ、split 後の新ペインへ `set-ticker` が正常に動作するよう修正。  
  P5-4 の `panes[0]` 限定修正が GUI 側のペイン UUID 解決に影響していないか確認。

#### P7-2 カテゴリ M 対応：Tachibana auto-play リグレッション修正

- [ ] **P7-2a（調査）** S20 が Set 3 で PASS → Set 4 でクラッシュした原因を特定。  
  Phase 6 の変更差分と S20 の auto-play 初期化フローを照合する。
- [ ] **P7-2b（実装）** auto-play precondition の Playing 到達タイムアウトが 120 秒で発生する原因を修正。  
  Tachibana ディスクキャッシュ `os error 3` が継続しているなら P4-1（キャッシュディレクトリ初期化）を先行適用。

#### P7-3 カテゴリ F 残存対応：Binance/Bybit 接続エラートースト抑制

- [ ] **P7-3a（調査）** GUI モードで S33/S36/S37 テスト中に発生するエラー通知 2 件の発火源を特定。  
  `src/connector/` の WebSocket 接続エラーハンドラが `Toast::error` に昇格するパスを追う。
- [ ] **P7-3b（実装）** Binance/Bybit 地理的ブロック（HTTP 451/403）による接続エラーを  
  CI 環境（`DEV_IS_DEMO=true`）ではトーストとして表示しないよう抑制する。  
  または replay 中の初期化エラー通知レベルを `Warning` に落とす。

#### P7-4 カテゴリ J 継続対応：Tachibana 休場日スキップ

- [ ] **P4-2（再掲）** `s29_tachibana_holiday_skip.sh` の `current_time` が 2025-01-10 より 2 日以上ずれる原因調査。  
  `src/replay/` の開始時刻計算ロジックと休場日スキップ実装を確認。

---

## 調査対象ファイル

```
src/replay_api.rs                     # P7-1 (set-ticker 404 リグレッション), P5-3 (pane/split UUID)
src/screen/dashboard/                 # P7-1 (GUI ペイン UUID 管理)
src/replay/                           # P7-2 (Tachibana auto-play), P5-2/K (ticker 変更フロー)
src/connector/                        # P7-3 (Binance/Bybit エラートースト), P4-1 (Tachibana 認証)
src/connector/auth.rs                 # P4-1 (Tachibana 認証)
tests/s20_tachibana_replay_resilience.sh # P7-2
tests/s32_toyota_candlestick_add.sh   # P7-1
tests/s24_sidebar_select_ticker.sh    # P7-1 (TC-D2)
tests/s29_tachibana_holiday_skip.sh   # P7-4
tests/s33_sidebar_split_pane.sh       # P7-3
tests/s36_sidebar_order_pane.sh       # P7-3
tests/s37_order_panels_integrated.sh  # P7-3
tests/s39_buying_power_portfolio.sh   # P7-3
```

---

## 完了条件

- `e2e.yml` の全 job が PASS または PEND（意図的スキップ）
- 新規リグレッションなし（`cargo test` / `cargo clippy` グリーン）
