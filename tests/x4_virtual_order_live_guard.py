#!/usr/bin/env python3
"""x4_virtual_order_live_guard.py — X4: 仮想注文 LIVE モードガード (クイック検証)

検証シナリオ:
  01-03: LIVE モード時は /order・/portfolio・/state がすべて HTTP 400
  04-05: Replay (Idle) モードに切替後は /order・/portfolio が HTTP 200
  06:    LIVE モードに戻すと /order が再び HTTP 400（ガード復元）

仕様根拠:
  docs/replay_header.md §11.2 — 「REPLAY モード専用。LIVE モード時は 400 を返す。」
  docs/order_windows.md §REPLAY モード Safety Guard

tests/archive/x4_virtual_order_live_guard.sh の Python 版。

使い方:
    uv run tests/x4_virtual_order_live_guard.py
    IS_HEADLESS=true uv run tests/x4_virtual_order_live_guard.py
    pytest tests/x4_virtual_order_live_guard.py -v
"""

from __future__ import annotations

import sys
from pathlib import Path

sys.path.insert(0, str(Path(__file__).parent))
from helpers import *


def run_x4() -> None:
    mode_label = "headless" if IS_HEADLESS else "GUI"
    print(f"=== X4: 仮想注文 LIVE モードガード ({mode_label}) ===")

    if IS_HEADLESS:
        # headless は常に Replay モードのため Live guard テストは非対応
        pend("TC-01", "headless は Live モードなし")
        pend("TC-02", "headless は Live モードなし")
        pend("TC-03", "headless は Live モードなし")

        # TC-04/05: headless は既に Replay Idle → HTTP 200
        code_04 = api_post_code(
            "/api/replay/order",
            {"ticker": "BTCUSDT", "side": "buy", "qty": 0.1, "order_type": "market"},
        )
        if code_04 == 200:
            pass_("TC-04: Replay 中 POST /api/replay/order → HTTP 200")
        else:
            fail("TC-04", f"HTTP={code_04} (expected 200)")

        code_05 = api_get_code("/api/replay/portfolio")
        if code_05 == 200:
            pass_("TC-05: Replay 中 GET /api/replay/portfolio → HTTP 200")
        else:
            fail("TC-05", f"HTTP={code_05} (expected 200)")

        pend("TC-06", "headless は Live モードなし")
        return

    # ── GUI モード ────────────────────────────────────────────────────────────

    print()
    print("── TC-01〜03: LIVE モード時は HTTP 400")

    live_mode = get_status().get("mode")
    if live_mode != "Live":
        fail("precond", f"LIVE モード起動失敗 (mode={live_mode})")
        print_summary()
        raise SystemExit(1)
    print(f"  起動モード確認: mode={live_mode}")

    code_01 = api_post_code(
        "/api/replay/order",
        {"ticker": "BTCUSDT", "side": "buy", "qty": 0.1, "order_type": "market"},
    )
    if code_01 == 400:
        pass_("TC-01: LIVE 中 POST /api/replay/order → HTTP 400")
    else:
        fail("TC-01", f"HTTP={code_01} (expected 400)")

    code_02 = api_get_code("/api/replay/portfolio")
    if code_02 == 400:
        pass_("TC-02: LIVE 中 GET /api/replay/portfolio → HTTP 400")
    else:
        fail("TC-02", f"HTTP={code_02} (expected 400)")

    code_03 = api_get_code("/api/replay/state")
    if code_03 == 400:
        pass_("TC-03: LIVE 中 GET /api/replay/state → HTTP 400")
    else:
        fail("TC-03", f"HTTP={code_03} (expected 400)")

    # ── TC-04〜05: Replay モード（Idle）に切替後は HTTP 200 ───────────────────
    print()
    print("── TC-04〜05: Replay (Idle) モードに切替 → HTTP 200")

    api_post("/api/replay/toggle")
    replay_mode = get_status().get("mode")
    print(f"  toggle 後のモード: {replay_mode}")
    if replay_mode != "Replay":
        fail("precond", f"Replay モード遷移失敗 (mode={replay_mode})")
        print_summary()
        raise SystemExit(1)

    code_04 = api_post_code(
        "/api/replay/order",
        {"ticker": "BTCUSDT", "side": "buy", "qty": 0.1, "order_type": "market"},
    )
    if code_04 == 200:
        pass_("TC-04: Replay (Idle) 中 POST /api/replay/order → HTTP 200")
    else:
        fail("TC-04", f"HTTP={code_04} (expected 200)")

    code_05 = api_get_code("/api/replay/portfolio")
    if code_05 == 200:
        pass_("TC-05: Replay (Idle) 中 GET /api/replay/portfolio → HTTP 200")
    else:
        fail("TC-05", f"HTTP={code_05} (expected 200)")

    # ── TC-06: LIVE モードに戻すと HTTP 400 が復元される ──────────────────────
    print()
    print("── TC-06: LIVE に戻すと HTTP 400 が復元される")

    api_post("/api/replay/toggle")
    live_mode_again = get_status().get("mode")
    print(f"  再 toggle 後のモード: {live_mode_again}")

    code_06 = api_post_code(
        "/api/replay/order",
        {"ticker": "BTCUSDT", "side": "buy", "qty": 0.1, "order_type": "market"},
    )
    if code_06 == 400:
        pass_("TC-06: LIVE 復帰後 POST /api/replay/order → HTTP 400（ガード復元）")
    else:
        fail("TC-06", f"HTTP={code_06} (expected 400)")


def test_x4_virtual_order_live_guard() -> None:
    """pytest から呼ばれる場合のエントリポイント。プロセス起動は外部で行うこと。"""
    import helpers as _h
    _h._PASS = _h._FAIL = _h._PEND = 0
    run_x4()
    print_summary()
    assert _h._FAIL == 0, f"{_h._FAIL} TC(s) failed — see output above"


def main() -> None:
    backup_state()

    if not IS_HEADLESS:
        write_live_fixture(ticker="BinanceLinear:BTCUSDT", timeframe="M1", name="X4")

    env = FlowsurfaceEnv(ticker=TICKER, timeframe="M1", headless=IS_HEADLESS)
    try:
        env._start_process()
        run_x4()
    finally:
        env.close()
        restore_state()
        print_summary()
        import helpers as _h
        if _h._FAIL > 0:
            sys.exit(1)


if __name__ == "__main__":
    main()
