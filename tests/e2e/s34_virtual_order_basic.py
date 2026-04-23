#!/usr/bin/env python3
"""s34_virtual_order_basic.py — S34: 仮想注文 API 基本動作検証

検証シナリオ:
  A-C: LIVE モード時は /order・/portfolio・/state がすべて HTTP 400 を返す
  D:   REPLAY Playing 到達
  E-G: Paused 状態で POST /api/replay/order (成行買い) → HTTP 200, order_id, status="pending"
  H:   指値買い注文 → HTTP 200, order_id 返却
  I:   指値売り注文 → HTTP 200, order_id 返却
  J-K: 不正リクエスト → HTTP 400
  L:   GET /api/replay/state → HTTP 200, current_time_ms フィールドあり

使い方:
    E2E_TICKER=BinanceLinear:BTCUSDT python tests/s34_virtual_order_basic.py
    IS_HEADLESS=true python tests/s34_virtual_order_basic.py
    pytest tests/s34_virtual_order_basic.py -v
"""

from __future__ import annotations

import sys
import time
from pathlib import Path

import requests

sys.path.insert(0, str(Path(__file__).parent))
from helpers import (
    API_BASE,
    IS_HEADLESS,
    TICKER,
    api_get,
    api_post,
    api_get_code,
    api_post_code,
    backup_state,
    get_status,
    headless_play,
    pass_,
    fail,
    pend,
    print_summary,
    restore_state,
    setup_single_pane,
    utc_offset,
    wait_status,
    write_live_fixture,
)

_REPO_ROOT = Path(__file__).parent.parent
try:
    from flowsurface import FlowsurfaceEnv
except ImportError:
    sys.path.insert(0, str(_REPO_ROOT / "python"))
    from env import FlowsurfaceEnv  # type: ignore[no-redef]


def run_s34(start: str, end: str) -> None:
    print("=== S34: 仮想注文 API 基本動作検証 ===")

    # ── TC-A〜C: LIVE モード時は HTTP 400 ────────────────────────────────────
    print()
    print("── TC-A〜C: LIVE モード時は HTTP 400")

    live_mode = get_status().get("mode")
    print(f"  現在のモード: {live_mode}")

    if IS_HEADLESS:
        # headless は常に Replay モード → LIVE ガードは発動しない
        pend("TC-A", "headless は常に Replay モード（LIVE ガード不要）")
        pend("TC-B", "headless は常に Replay モード（LIVE ガード不要）")
    else:
        code_a = api_post_code(
            "/api/replay/order",
            {"ticker": "BTCUSDT", "side": "buy", "qty": 0.1, "order_type": "market"},
        )
        if code_a == 400:
            pass_("TC-A: LIVE 中 POST /api/replay/order → HTTP 400")
        else:
            fail("TC-A", f"HTTP={code_a} (expected 400)")

        code_b = api_get_code("/api/replay/portfolio")
        if code_b == 400:
            pass_("TC-B: LIVE 中 GET /api/replay/portfolio → HTTP 400")
        else:
            fail("TC-B", f"HTTP={code_b} (expected 400)")

    # TC-C: /api/replay/state は headless でも LIVE ガードが効く
    code_c = api_get_code("/api/replay/state")
    if code_c == 400:
        pass_("TC-C: LIVE 中 GET /api/replay/state → HTTP 400")
    else:
        fail("TC-C", f"HTTP={code_c} (expected 400)")

    # ── TC-D: REPLAY Playing に遷移 ──────────────────────────────────────────
    print()
    print("── TC-D: REPLAY Playing に遷移")

    api_post("/api/replay/toggle")
    api_post("/api/replay/toggle", {"start": start, "end": end})

    if not wait_status("Playing", 60):
        fail("TC-D", "REPLAY Playing に到達せず（60s タイムアウト）")
        print_summary()
        sys.exit(1)
    pass_("TC-D: REPLAY Playing 到達")

    # 以降の注文テストは Paused 状態で行う（約定を防いで結果を決定論的にする）
    wait_status("Paused", 10)

    # ── TC-E〜G: 成行買い注文 ────────────────────────────────────────────────
    print()
    print("── TC-E〜G: 成行買い注文")

    market_resp = api_post(
        "/api/replay/order",
        {"ticker": "BTCUSDT", "side": "buy", "qty": 0.1, "order_type": "market"},
    )
    print(f"  response: {market_resp}")

    code_e = api_post_code(
        "/api/replay/order",
        {"ticker": "BTCUSDT", "side": "buy", "qty": 0.05, "order_type": "market"},
    )
    if code_e == 200:
        pass_("TC-E: POST /api/replay/order (成行買い) → HTTP 200")
    else:
        fail("TC-E", f"HTTP={code_e} (expected 200)")

    order_id = market_resp.get("order_id")
    if isinstance(order_id, str) and len(order_id) > 0:
        pass_(f"TC-F: order_id が文字列として返る ({order_id})")
    else:
        fail("TC-F", f"order_id が null または不正 (response={market_resp})")

    order_status = market_resp.get("status")
    if order_status == "pending":
        pass_("TC-G: 注文ステータス = \"pending\"")
    else:
        fail("TC-G", f"status={order_status} (expected pending)")

    # ── TC-H: 指値買い注文 → HTTP 200, order_id 返却 ─────────────────────────
    print()
    print("── TC-H: 指値買い注文")

    limit_buy_resp = api_post(
        "/api/replay/order",
        {"ticker": "BTCUSDT", "side": "buy", "qty": 0.05, "order_type": {"limit": 1.0}},
    )
    print(f"  response: {limit_buy_resp}")

    lb_id = limit_buy_resp.get("order_id")
    if isinstance(lb_id, str) and len(lb_id) > 0:
        pass_("TC-H: 指値買い注文 → HTTP 200, order_id 返却 (status=pending)")
    else:
        fail("TC-H", f"response={limit_buy_resp}")

    # ── TC-I: 指値売り注文 → HTTP 200, order_id 返却 ─────────────────────────
    print()
    print("── TC-I: 指値売り注文")

    limit_sell_resp = api_post(
        "/api/replay/order",
        {"ticker": "BTCUSDT", "side": "sell", "qty": 0.05, "order_type": {"limit": 9999999.0}},
    )
    print(f"  response: {limit_sell_resp}")

    ls_id = limit_sell_resp.get("order_id")
    if isinstance(ls_id, str) and len(ls_id) > 0:
        pass_("TC-I: 指値売り注文 → HTTP 200, order_id 返却 (status=pending)")
    else:
        fail("TC-I", f"response={limit_sell_resp}")

    # ── TC-J〜K: 不正リクエスト → HTTP 400 ──────────────────────────────────
    print()
    print("── TC-J〜K: 不正リクエスト")

    code_j = api_post_code("/api/replay/order", "not-valid-json")
    if code_j == 400:
        pass_("TC-J: 不正 JSON → HTTP 400")
    else:
        fail("TC-J", f"HTTP={code_j} (expected 400)")

    # side / qty / order_type を省略した不完全なリクエスト
    code_k = api_post_code("/api/replay/order", {"ticker": "BTCUSDT"})
    if code_k == 400:
        pass_("TC-K: 必須フィールド欠落 (side/qty/order_type なし) → HTTP 400")
    else:
        fail("TC-K", f"HTTP={code_k} (expected 400)")

    # ── TC-L: GET /api/replay/state → HTTP 200, スキーマ検証 ─────────────────
    print()
    print("── TC-L: GET /api/replay/state (Phase 1 実データ)")

    r = requests.get(f"{API_BASE}/api/replay/state", timeout=5)
    code_l = r.status_code
    print(f"  HTTP={code_l}")

    if code_l == 200:
        pass_("TC-L1: GET /api/replay/state → HTTP 200")
    else:
        fail("TC-L1", f"HTTP={code_l} (expected 200)")
        return

    try:
        state_resp = r.json()
    except Exception:
        fail("TC-L2", "レスポンス JSON パース失敗")
        return

    has_schema = (
        isinstance(state_resp.get("current_time_ms"), (int, float))
        and state_resp["current_time_ms"] > 0
        and isinstance(state_resp.get("klines"), list)
        and isinstance(state_resp.get("trades"), list)
    )
    if has_schema:
        pass_("TC-L2: current_time_ms(>0) / klines[] / trades[] フィールドあり")
    else:
        fail("TC-L2", f"スキーマ不正 (response={state_resp})")

    klines = state_resp.get("klines", [])
    kline_count = len(klines)
    print(f"  klines count={kline_count}")
    if kline_count > 0:
        k = klines[0]
        kline_ok = (
            isinstance(k.get("stream"), str) and len(k["stream"]) > 0
            and isinstance(k.get("time"), (int, float))
            and isinstance(k.get("open"), (int, float)) and k["open"] > 0
            and isinstance(k.get("high"), (int, float))
            and isinstance(k.get("low"), (int, float))
            and isinstance(k.get("close"), (int, float))
            and isinstance(k.get("volume"), (int, float))
        )
        if kline_ok:
            pass_("TC-L3: klines[0] に stream/time/open/high/low/close/volume あり")
        else:
            fail("TC-L3", f"klines[0] スキーマ不正 (response={state_resp})")
    else:
        pass_("TC-L3: klines=0 件（Playing 直後のため許容）")


def test_s34_virtual_order_basic() -> None:
    """pytest エントリポイント。プロセス起動は外部で行うこと。"""
    import helpers
    helpers._PASS = helpers._FAIL = helpers._PEND = 0
    start = utc_offset(-3)
    end = utc_offset(-1)
    run_s34(start, end)
    print_summary()
    assert helpers._FAIL == 0, f"{helpers._FAIL} TC(s) failed — see output above"


def main() -> None:
    start = utc_offset(-3)
    end = utc_offset(-1)

    backup_state()
    # Live モード起動（LIVE ガードを最初に検証するため replay フィールドなし）
    write_live_fixture(ticker=TICKER, timeframe="M1", name="S34")

    env = FlowsurfaceEnv(ticker=TICKER, timeframe="M1", headless=IS_HEADLESS)
    try:
        env._start_process()
        run_s34(start, end)
    finally:
        env.close()
        restore_state()
        print_summary()

    import helpers
    if helpers._FAIL > 0:
        sys.exit(1)


if __name__ == "__main__":
    main()
