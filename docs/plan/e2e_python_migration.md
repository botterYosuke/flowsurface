# E2E テスト Python 移行計画

## 概要

67 個の bash E2E テストスクリプト（`tests/*.sh`）を Python / pytest に移行する。

**目標:**
- `bash + curl + node` 依存を排除し、`pytest + requests` に統一
- CI の `e2e.yml` もそのまま動かせるよう互換性を維持
- bash では困難だった型安全・リトライ・パラメータ化を導入

---

## アーキテクチャ方針

### 新設するモジュール構成

```
python/
├── flowsurface_sdk/
│   ├── __init__.py          # FlowsurfaceEnv (既存 RL 用、変更なし)
│   └── env.py
├── flowsurface_e2e/         # ← 新設（E2E テスト専用）
│   ├── __init__.py
│   ├── client.py            # FlowsurfaceClient — HTTP API ラッパー
│   └── helpers.py           # bash common_helpers.sh 相当のユーティリティ
└── tests/
    ├── e2e/                 # ← 移行先
    │   ├── conftest.py      # pytest fixture（プロセス起動・クリーンアップ）
    │   ├── test_s1_basic_lifecycle.py
    │   ├── test_s1b_limit_buy.py
    │   ...（67 スクリプト分）
    └── test_env.py          # 既存 SDK ユニットテスト（変更なし）
```

### FlowsurfaceClient の設計

`FlowsurfaceEnv`（RL 用 Gymnasium 環境）とは別に、E2E テスト専用の軽量 HTTP クライアントを作る。  
Gymnasium 空間（observation_space/action_space）は不要なため分離する。

```python
class FlowsurfaceClient:
    """E2E テスト用 HTTP API クライアント。プロセス管理＋全エンドポイントラッパー。"""

    def __init__(
        self,
        *,
        binary_path: str | None = None,
        api_port: int = 9876,
        headless: bool = True,
        ticker: str = "BinanceLinear:BTCUSDT",
        timeframe: str = "M1",
        env_vars: dict[str, str] | None = None,
    ): ...

    # プロセス管理
    def start(self) -> None: ...     # バイナリ起動 + API 待機
    def stop(self) -> None: ...      # 終了・クリーンアップ
    def __enter__ / __exit__: ...    # context manager 対応

    # Replay 制御
    def status(self) -> dict: ...
    def play(self, start: str, end: str) -> None: ...
    def pause(self) -> None: ...
    def resume(self) -> None: ...
    def step_forward(self) -> None: ...
    def step_backward(self) -> None: ...
    def cycle_speed(self) -> None: ...
    def toggle(self) -> None: ...

    # 待機ヘルパー（bash wait_* の相当）
    def wait_status(self, want: str, timeout_s: float = 10.0) -> dict: ...
    def wait_time_advance(self, ref_ms: int, timeout_s: float = 30.0) -> int: ...
    def wait_streams_ready(self, pane_id: str, timeout_s: float = 30.0) -> None: ...
    def wait_pane_count(self, n: int, timeout_s: float = 10.0) -> list: ...

    # ペイン
    def list_panes(self) -> list[dict]: ...
    def split_pane(self, pane_id: str, axis: str) -> dict: ...
    def close_pane(self, pane_id: str) -> None: ...
    def set_ticker(self, pane_id: str, ticker: str) -> None: ...
    def set_timeframe(self, pane_id: str, timeframe: str) -> None: ...
    def chart_snapshot(self, pane_id: str) -> dict: ...

    # 仮想約定エンジン
    def place_order(self, ticker: str, side: str, qty: float, order_type: str, limit_price: float | None = None) -> dict: ...
    def portfolio(self) -> dict: ...
    def replay_state(self) -> dict: ...
    def pending_orders(self) -> list[dict]: ...

    # Sidebar
    def sidebar_select_ticker(self, pane_id: str, ticker: str, kind: str | None = None) -> None: ...
    def open_order_pane(self, kind: str) -> None: ...

    # 通知
    def notifications(self) -> list[dict]: ...

    # 立花証券
    def buying_power(self) -> dict: ...
    def tachibana_orders(self, eig_day: str | None = None) -> list[dict]: ...
    def tachibana_order_detail(self, order_num: str, eig_day: str | None = None) -> dict: ...
    def tachibana_new_order(self, **kwargs) -> dict: ...
    def tachibana_correct_order(self, **kwargs) -> dict: ...
    def tachibana_cancel_order(self, **kwargs) -> dict: ...
    def tachibana_holdings(self, issue_code: str) -> dict: ...
    def tachibana_session_status(self) -> str: ...
```

---

## 移行フェーズ

### Phase 1 — 基盤構築（先に完成させる）

| タスク | 内容 |
|--------|------|
| `flowsurface_e2e/client.py` | FlowsurfaceClient 実装（全エンドポイント） |
| `flowsurface_e2e/helpers.py` | `utc_offset()`, `is_bar_boundary()`, `ct_in_range()` 等 |
| `tests/e2e/conftest.py` | pytest fixture（client 起動・クリーンアップ・saved-state 管理） |
| `pyproject.toml` | `flowsurface_e2e` パッケージ追加、pytest 設定 |

conftest.py の主要 fixture:

```python
@pytest.fixture(scope="function")
def client(tmp_path, request):
    """E2E テスト用クライアント。各テスト関数ごとに起動・終了。"""
    ticker = os.getenv("E2E_TICKER", "BinanceLinear:BTCUSDT")
    c = FlowsurfaceClient(
        ticker=ticker,
        headless=True,
        env_vars={"DEV_IS_DEMO": "true"},
    )
    c.start()
    yield c
    c.stop()

@pytest.fixture
def with_replay(client):
    """再生中状態の client を返す。"""
    client.play(start="2026-04-10 20:00", end="2026-04-10 22:00")
    client.wait_status("Playing", timeout_s=60)
    return client

@pytest.fixture
def saved_state(tmp_path):
    """テスト用 saved-state.json を APPDATA に配置し、終了後復元する。"""
    ...
```

### Phase 2 — S1 系（基本ライフサイクル） ✅ 基準点

| スクリプト | 移行先 | 主要テスト |
|-----------|--------|-----------|
| `s1_basic_lifecycle.sh` | `test_s1_basic_lifecycle.py` | replay mode/status 遷移, speed cycle, step |
| `s1b_limit_buy.sh` | `test_s1b_limit_buy.py` | 指値買い・約定確認 |
| `s1c_market_sell.sh` | `test_s1c_market_sell.py` | 成行売り |
| `s1d_limit_sell.sh` | `test_s1d_limit_sell.py` | 指値売り |

S1 系を先に完成させ、Python テスト基盤が正しく動くことを確認してから他に進む。

### Phase 3 — 仮想約定エンジン系（S34-S43）

bash の `node -e "BigInt(...)"` は Python int で自然に置換できる。  
S34/S35/S39-S43 を移行。

### Phase 4 — チャート・ペイン系（S2-S18、S23-S28、S33）

pane_id 管理（UUID）と streams_ready ポーリングが中心。  
`wait_streams_ready()` を活用。

### Phase 5 — 立花証券系（S5、S19-S22、S29、S32、S44-S49）

環境変数 `DEV_IS_DEMO` なしのテストが多い。  
Tachibana セッション確立の待機（`wait_tachibana_session()`）を実装する。

### Phase 6 — CI 更新

`e2e.yml` を bash 実行から Python 実行に切り替え:

```yaml
# 変更前
- run: bash tests/${{ matrix.test.script }} 2>&1 | tail -50

# 変更後
- run: uv run --project python --extra dev pytest python/tests/e2e/test_${{ matrix.test.module }}.py -v
```

---

## 技術的判断

### bash との対応表

| bash パターン | Python 実装 |
|--------------|------------|
| `jqn "..." "d.mode"` | `response.json()["mode"]` |
| `bigt_gt "$A" "$B"` | `int(a) > int(b)` |
| `wait_status "Playing" 60` | `client.wait_status("Playing", 60)` |
| `utc_offset -3` | `(datetime.utcnow() - timedelta(hours=3)).strftime(...)` |
| `api_post /api/replay/play "{...}"` | `client.play(start=..., end=...)` |
| `pass() / fail()` | `assert` + pytest report |
| `trap ... EXIT ERR` | `pytest fixture` の teardown |
| saved-state テンプレート | `conftest.py` の `saved_state` fixture |

### 並列化について

初期は `scope="function"`（各テストでプロセス起動）で安全に直列実行。  
ポート競合を避けるため並列化は Phase 6 以降に検討。

### 立花証券テスト（DEV_IS_DEMO なし）

実認証が必要なテストは `@pytest.mark.tachibana` でマーキングし、  
CI では `--ignore-glob="*tachibana*"` または mark filter で分離する。

---

## 完了基準

- [ ] Phase 1: FlowsurfaceClient + conftest.py 実装完了
- [ ] Phase 2: S1 系 4 スクリプト → Python 移行・全テスト GREEN
- [ ] Phase 3: S34-S43 → Python 移行・全テスト GREEN
- [ ] Phase 4: S2-S18、S23-S28、S33 → Python 移行・全テスト GREEN
- [ ] Phase 5: 立花証券系 → Python 移行・全テスト GREEN
- [ ] Phase 6: `e2e.yml` を Python 実行に切り替え・CI GREEN
- [ ] bash スクリプト（`tests/*.sh`）をアーカイブまたは削除

---

## 備考

- `flowsurface_sdk` (RL 用 Gymnasium 環境) は変更しない
- `FlowsurfaceClient` は `flowsurface_sdk` と独立したパッケージ（`flowsurface_e2e`）に置く
- bash の `common_helpers.sh` は `flowsurface_e2e/helpers.py` に相当する
- テスト名は `test_{スクリプト名}_{TC番号}_{説明}` の形式に統一する
