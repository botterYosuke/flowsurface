---
name: agent-experience-verification
description: flowsurface の HTTP API を「ユーザーの分身としてのエージェント」として実際に叩き、ナラティブ基盤など機能が体験として成立しているかを検証するスキル。E2E テストでは拾えない実環境バグ（バッファ制限・配線漏れ・非決定性）を炙り出す。Phase 4a ナラティブ基盤の検証パターンに準拠。
---

# Agent Experience Verification

E2E テスト（仕様駆動）と単体テスト（実装駆動）の隙間を埋める「**実体験ベース検証**」のスキル。
あなたがエージェントとして HTTP API を叩き、観測 → 判断 → 発注 → 記録 → 振り返り のサイクルを回す。

仕様書通りに動くだけでなく、**実環境で違和感なく使えるか** を確かめる。

## いつ使うか

- 新機能（特に HTTP API + 永続化 + 非同期処理が絡むもの）の実装直後
- E2E と単体テストが全パスしているのに「本当に動くのか」不安が残るとき
- Phase X-a の verification-loop 直前の最終チェック
- 既存機能のリグレッション疑いを手早く確認したいとき

## 起動モード選定（最初に決める）

headless と GUI は別モードなので、検証目的に応じて選ぶ。**両方で回す必要があるケースが多い** — headless で回るだけだと GUI 固有の配線バグを見逃す（Phase 4a C-2 の `step_forward` 配線漏れが実例）。

| モード | 使う場面 | バイナリ | 注意 |
|---|---|---|---|
| **headless** | 仕様通り動作することの確認、CI での再現、純 API 経路 | `target/release/flowsurface.exe --headless --ticker ... --timeframe ...` | `--ticker` / `--timeframe` CLI が効く |
| **GUI (debug)** | GUI 経路でのみ発火するイベント配線・描画更新・Subscription 同期の検証、立花証券 API を含む検証 | `target/debug/flowsurface.exe`（CLI 引数は無視） | `#[cfg(debug_assertions)]` 下の `DEV_USER_ID` / `DEV_PASSWORD` 自動ログインは **debug ビルドのみ**有効 |
| **GUI (release)** | プロダクション配布バイナリの挙動確認のみ | `target/release/flowsurface.exe` | 自動ログイン無効・ログイン画面で手動操作必須。自動エージェント検証には不向き |

**GUI で `--ticker` / `--timeframe` は効かない**（`clap` 定義は `src/headless.rs` のみ）。GUI では保存済みダッシュボード設定（`AppData/Roaming/flowsurface/saved-state.json`）が復元されるので、違う ticker/timeframe を使いたければ **`POST /api/pane/set-ticker` / `POST /api/pane/set-timeframe` で動的に切り替える**。保存状態が想定外（前回 Tachibana:7203 のまま残っていた等）でも API で修正できる。

---

## 行動ループ（テンプレート）

```
1. ビルド: cargo build && cargo build --release
   ※ debug / release 両方必須。FlowsurfaceEnv._find_binary() は FLOWSURFACE_BINARY 未指定で
     target/debug を優先するため、release だけビルドすると古い debug で再体験することになる。

2. 既存プロセス確認（ポート衝突予防）:
   netstat -ano | grep "9876 " | grep LISTENING
   tasklist //FI "IMAGENAME eq flowsurface.exe"
   ※ 見つかったら taskkill //PID <N> //F で落としてから起動する。
     既存プロセスが 9876 を掴んだままだと、新しく起動した方は sub ライブラリ bind に失敗するが
     プロセス自体は生き続ける「無言フェイルオープン」状態になり、
     curl の応答は既存プロセスから返る → 違うバイナリを検証していることに気付けない。
     起動後のログで `Failed to bind replay API server on 127.0.0.1:9876 (os error 10048)`
     が出ていないか grep で必ず確認する。

3. 起動:
   ## (a) headless モード
   ./target/release/flowsurface.exe --headless --ticker BinanceLinear:BTCUSDT --timeframe M1 &

   ## (b) GUI debug モード（立花証券 API 検証を含む場合・自動ログインが必要な場合）
   set -a; source .env; set +a                           # .env から DEV_USER_ID/PASSWORD/IS_DEMO を読み込む
   ./target/debug/flowsurface.exe &                      # CLI 引数は効かないので渡さなくて良い
   until curl -s --max-time 1 http://127.0.0.1:9876/api/replay/state >/dev/null; do sleep 2; done
   grep "Failed to bind" <log>                           # 空であることを確認（衝突検知）
   grep -iE "Attempting to restore tachibana|Loaded tachibana|auto[-_]login" <log>
                                                         # 自動ログインまたは keyring 復元成功を確認

   ※ FLOWSURFACE_DATA_PATH は設定しない（既定 AppData を使う）。env 上書きすると
     data_path() の path_name 引数が捨てられる既知バグ (data/src/lib.rs:133-144) を踏む。

4. 検証対象機能のセットアップ
   ## GUI モードの場合、saved-state のペイン構成が想定と違うことがあるので最初に確認:
   curl -s http://127.0.0.1:9876/api/pane/list | python -m json.tool

   ## 必要に応じて API で ticker / timeframe を切り替える:
   curl -s -X POST http://127.0.0.1:9876/api/pane/set-ticker  -d '{"pane_id":"<candlestick pane id>","ticker":"BinanceLinear:BTCUSDT"}'
   curl -s -X POST http://127.0.0.1:9876/api/pane/set-timeframe -d '{"pane_id":"<candlestick pane id>","timeframe":"M1"}'
   ※ "ticker info not loaded yet" が返る場合はメタデータ fetch を待つ（最大 10 秒程度リトライ）。

   ## リプレイ開始:
   curl -s -X POST http://127.0.0.1:9876/api/replay/play -d '{"start":"2023-11-14 12:00","end":"2023-11-14 14:00"}'
   curl -s -X POST http://127.0.0.1:9876/api/replay/pause
   curl -s http://127.0.0.1:9876/api/replay/state                   # klines が 0 件なら準備未完了

5. エージェントとして 1 サイクル実行:
   a. 状態観測（GET /api/replay/state など）
      - 同じ状態を 2-3 回観測して決定的か確認
   b. 観測から判断（reasoning は自然言語で自分で書く）
   c. アクション実行（POST /api/replay/order など）
   d. 判断を記録（POST /api/agent/narrative）
   e. 時間を進める（POST /api/replay/step-forward × 数回）
   f. 結果を確認（GET /api/agent/narrative/:id で outcome 自動充填確認）

6. バリエーションを変えて 3-5 サイクル繰り返す:
   - 成行 / 指値 × buy / sell
   - idempotency_key あり/なし（再送で結果安定するか）
   - public フラグ true / false 往復
   - payload サイズ 極小 / 大きめ（数百 KB） / 上限超え

7. 振り返り:
   - 履歴一覧（GET /api/agent/narratives?...）
   - スナップショット復元（GET /api/agent/narrative/:id/snapshot で記録時と一致するか）
   - 蓄積量（GET /api/agent/narratives/storage）

8. Python SDK でも同サイクルが回るか:
   import flowsurface as fs
   fs.narrative.create(...) / list / get / publish / unpublish / snapshot / storage_stats

9. 破綻を検知 → 根本修正 → debug + release 両方リビルド → 3 へ戻る。
```

---

## GUI 経路で遭遇しがちなハマりどころ

### H1. ポート 9876 の「無言フェイルオープン」
二重起動時、後から起動した側は `Failed to bind replay API server on 127.0.0.1:9876 (os error 10048)` を吐いたまま GUI は生きる。curl は既存プロセスに届き、検証対象と違うバイナリを触る事故が起きる。起動直後に必ずログを grep で検査する。

### H2. `--ticker` / `--timeframe` は GUI では無視される
CLI 引数は `src/headless.rs` の `clap` でのみ定義。GUI は保存済みレイアウトを復元するため、想定外の ticker（例: 前回セッションで Tachibana:7203 を使っていた）で立ち上がる。**`/api/pane/list` で必ず確認**し、違えば `/api/pane/set-ticker` と `/api/pane/set-timeframe` で動的に合わせる。

### H3. `NarrativeAction.price` は `f64`（Option ではない）
成行注文でも `price: 0.0` を明示的に送る必要がある。`price: null` / `price` 欠落は **400 "invalid JSON body"** になる（`src/narrative/model.rs:77-82`）。エラーメッセージからは原因が特定できない（§9 の DX 観察項目と一致）。

### H4. auto-login は debug ビルド専用
`.env` の `DEV_USER_ID` / `DEV_PASSWORD` / `DEV_IS_DEMO` / `DEV_SECOND_PASSWORD` は `#[cfg(debug_assertions)]` 下でのみ読まれる（`src/screen/login.rs:140`, `src/connector/auth.rs:91-100`）。release GUI で検証すると手動ログインが必須となり自動エージェント化できない。**立花証券 API を含む検証は必ず debug GUI で行う。**

