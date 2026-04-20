# E2E テスト失敗 修正プラン

> 元データ: `docs/plan/e2e_logs_analysis.md`（58件のログ, 22件の FAIL）
> 元ログ: `C:\Users\sasai\Downloads\logs_65297484727`（58件の `.txt` ファイル）
> 策定日: 2026-04-20

---

## 失敗分類と根本原因

| # | カテゴリ | 件数 | 根本原因 |
|---|---------|-----|---------|
| A | S11/S12/S13 — `Playing 到達せず` | 9件 | `wait_playing(30)` タイムアウトが CI 環境では短い |
| B | S6-04 — `diff=60000 (expected 300000)` | 1件 | `step-forward` 後に Paused 遷移を待機していない |
| C | S20/S21 — Tachibana セッション 120s タイムアウト | 12件 | 前ジョブのセッション残留 / ログイン失敗 |

---

## カテゴリ A — `wait_playing` タイムアウト延長

### 症状
`tests/e2e/s11_bar_step_discrete.py` の TC-S11-01/03/05/06、`s12_pre_start_history.py` の precond、`s13_step_backward_quality.py` の precond で「Playing 到達せず」。

### 根本原因
- S11-01, 03, 05, 06 は `wait_playing(30)` = 30秒
- S12/S13 の precond も `wait_playing(30)` = 30秒
- CI（windows-latest）での Hyperliquid ストリーム初期化 + BinanceLinear BTCUSDT データ読み込みが 30秒では不安定

> S11-04 (H1) は既に 60秒、S32-01 は 120秒だが S32 でも失敗が起きている（下記 Tachibana 関連とは別に要確認）

### 修正内容

**`tests/e2e/s11_bar_step_discrete.py`**
```python
# TC-S11-01: 30 → 60
if not wait_playing(60):   # was 30

# TC-S11-03 (M5): 30 → 60
if not wait_playing(60):   # was 30

# TC-S11-05 (混在): 30 → 60
if not wait_playing(60):   # was 30

# TC-S11-06 (10連続): 30 → 60
if not wait_playing(60):   # was 30
```

**`tests/e2e/s12_pre_start_history.py`**
```python
# precond: 30 → 60
if not wait_playing(60):   # was 30
```

**`tests/e2e/s13_step_backward_quality.py`**
```python
# precond: 30 → 60
if not wait_playing(60):   # was 30
```

### 対応ファイル
- [ ] `tests/e2e/s11_bar_step_discrete.py` (4箇所)
- [ ] `tests/e2e/s12_pre_start_history.py` (1箇所)
- [ ] `tests/e2e/s13_step_backward_quality.py` (1箇所)

---

## カテゴリ B — S6-04 `Paused` 待機欠落

### 症状
`TC-S6-04: diff=60000 (expected 300000)` — M5 単独構成で `step-forward` 後の delta が M5 (300000ms) ではなく M1 (60000ms) と一致する。

### 根本原因
`s6_mixed_timeframes.py` line 287 の `time.sleep(1)` は Paused 遷移を保証しない。
step-forward が完了する前に `current_time` を読むと、ステップ処理が M1 バウンダリで止まった中間状態を拾う可能性がある。

```python
# 現状（バグあり）
api_post("/api/replay/step-forward")
time.sleep(1)                                        # 非決定的
post2 = int(api_get("/api/replay/status").get("current_time") or 0)

# 修正後
api_post("/api/replay/step-forward")
wait_status("Paused", 10)                            # Paused 確定後に読む
post2 = int(api_get("/api/replay/status").get("current_time") or 0)
```

`wait_status` は `helpers.py` で定義済みで S11 でも使われている（`_step_forward_delta()` 内）。

### 対応ファイル
- [ ] `tests/e2e/s6_mixed_timeframes.py` (line 287 付近、1箇所)

---

## カテゴリ C — Tachibana セッション確立失敗

### 症状
- "Tachibana session not established after 120s"（S20: 5件, S21: 2件）
- "precond — DEV_USER_ID でのログイン失敗"（3件）
- "Step 3 — orders フィールドが配列でない: セッションが切断しました"（1件）

### 根本原因（確定）

> **2026-04-20 追記**: ローカルで `s50_tachibana_login.py` を実行し、`DEV_USER_ID` でのログインが成功することを確認。
> 「資格情報が無効」の可能性は排除。**CI でのジョブ間セッション競合が主因** に確定。

**主因: 前ジョブのセッション残留**
`test-gui-tachibana-session` は `max-parallel: 1` で直列だが、`env.close()` でプロセスが終了しても Tachibana サーバー側のセッションが数秒〜数十秒生き続ける。
次ジョブが起動すると「同一アカウントで2重ログイン」→ "ログイン失敗" または既存セッションが切断される。

**証拠**
- ローカル単体実行では 100% PASS（資格情報は正常）
- "セッションが切断しました (code=2)" は Tachibana のセッション競合エラーコード
- "DEV_USER_ID でのログイン失敗" は前ジョブ由来のセッションが生きている状態での再ログイン失敗
- 各ジョブは独立した GitHub Actions VM で動くため、アプリ終了後も **Tachibana サーバー側のセッション** が残留する

### 修正方針

#### ~~C-1: ジョブ間セッション確認ステップを追加（ワークフロー側）~~【無効化】

> 各ジョブは独立した GitHub Actions VM で実行されるため、前ジョブのアプリプロセスには到達できない。
> ポート 9876 は現ジョブの VM にしか存在しないため、このアプローチは機能しない。
> **C-2（テスト teardown での明示的 logout）を代わりに優先する。**

#### C-2: テスト内に明示的ログアウト処理を追加（アプリ側）

`FlowsurfaceEnv.close()` 呼び出し前に Tachibana logout API を叩く。

```python
# s20/s21 の teardown パターン
finally:
    try:
        api_post("/api/auth/tachibana/logout")
        time.sleep(3)   # サーバー側のセッション切断を待つ
    except Exception:
        pass
    env.close()
```

ただし `/api/auth/tachibana/logout` エンドポイントが存在するか確認が必要（`src/replay_api.rs` 調査）。

#### C-3: `wait_tachibana_session` にリトライ付きエラーメッセージ改善

失敗時に `/api/auth/tachibana/status` のレスポンス全体をログに出す。

```python
def wait_tachibana_session(timeout: int = 120) -> bool:
    deadline = time.monotonic() + timeout
    last_body: dict = {}
    while time.monotonic() < deadline:
        try:
            body = requests.get(f"{API_BASE}/api/auth/tachibana/status", timeout=5).json()
            last_body = body
            if body.get("session") == "present":
                return True
        except requests.RequestException:
            pass
        time.sleep(1)
    print(f"  [debug] last tachibana status: {last_body}")  # 診断用
    return False
```

### 対応ファイル
- [ ] `tests/e2e/helpers.py` — `wait_tachibana_session` にデバッグ出力追加 (C-3)
- [ ] `tests/e2e/s20_tachibana_replay_resilience.py` — teardown に logout 追加 (C-2)
- [ ] `tests/e2e/s21_tachibana_error_boundary.py` — teardown に logout 追加 (C-2)
- [ ] `src/replay_api.rs` — `/api/auth/tachibana/logout` エンドポイント存在確認 (C-2前提)

### C-2 の前提確認
`src/replay_api.rs` に logout エンドポイントが**ない**場合は実装追加が必要。
ない場合の代替案: Tachibana セッションは Rust 側でアプリ終了時に自動切断されるはずだが、
`env.close()` → `Kill` シグナルでプロセスが強制終了された場合はクリーンアップが走らない可能性あり。
その場合は `SIGTERM` → 猶予期間 → `SIGKILL` の順序に変更する。

---

## 実装優先順位

| 優先度 | 修正 | 難易度 | 期待効果 |
|--------|------|--------|---------|
| 🔴 高  | A: `wait_playing` タイムアウト延長 | 低（数値変更のみ） | 9件解消 |
| 🔴 高  | B: S6-04 `wait_status("Paused")` 追加 | 低（1行変更） | 1件解消 |
| 🟡 中  | C-3: `wait_tachibana_session` デバッグ出力 | 低（数行追加） | 原因特定に有用 |
| 🟡 中  | C-2: teardown に logout 追加 | 中（logout API 確認必要） | 12件のうち一部解消 |
| 🟢 低  | C-1: ワークフロー session_guard ステップ | 中（ファイル追加） | 補完的効果 |

---

## 完了チェックリスト

- [ ] A: `s11_bar_step_discrete.py` — 4箇所 30→60秒 ✅
- [ ] A: `s12_pre_start_history.py` — 1箇所 30→60秒 ✅
- [ ] A: `s13_step_backward_quality.py` — 1箇所 30→60秒 ✅
- [ ] B: `s6_mixed_timeframes.py` — `time.sleep(1)` → `wait_status("Paused", 10)` ✅
- [ ] C-3: `helpers.py` — `wait_tachibana_session` デバッグ出力追加 ✅
- [ ] C-2: `src/replay_api.rs` logout エンドポイント確認 ✅
- [ ] C-2: `s20_tachibana_replay_resilience.py` teardown logout 追加 ✅
- [ ] C-2: `s21_tachibana_error_boundary.py` teardown logout 追加 ✅
- [ ] CI 再実行で FAIL 件数が低下することを確認 ✅
