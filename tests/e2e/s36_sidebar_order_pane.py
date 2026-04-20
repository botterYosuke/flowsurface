#!/usr/bin/env python3
"""s36_sidebar_order_pane.py — S36: /api/sidebar/open-order-pane による注文ペイン分割テスト

検証シナリオ:
  TC-A: OrderEntry → ペイン数 2、新ペインの type = "Order Entry"
  TC-B: OrderList → ペイン数 3、新ペインの type = "Order List"
  TC-C: BuyingPower → ペイン数 4、新ペインの type = "Buying Power"
  TC-D: エラー通知 0 件
  TC-E: 元ペイン (pane0) の type = "Candlestick Chart" のまま

仕様根拠:
  docs/order_windows.md §サイドバー注文ボタン — open-order-pane API

フィクスチャ: primary_ticker() M1, auto-play (UTC[-3h, -1h])
"""

from __future__ import annotations

import sys
from pathlib import Path

sys.path.insert(0, str(Path(__file__).parent))
from helpers import (
    IS_HEADLESS,
    api_get,
    api_post,
    backup_state,
    count_error_notifications,
    fail,
    get_pane_id,
    headless_play,
    pass_,
    pend,
    primary_ticker,
    print_summary,
    restore_state,
    setup_single_pane,
    utc_offset,
    wait_for_pane_count,
    wait_status,
)

_REPO_ROOT = Path(__file__).parent.parent
try:
    from flowsurface import FlowsurfaceEnv
except ImportError:
    sys.path.insert(0, str(_REPO_ROOT / "python"))
    from env import FlowsurfaceEnv  # type: ignore[no-redef]


def run_s36() -> None:
    print("=== S36: sidebar/open-order-pane による注文ペイン分割テスト ===")

    if IS_HEADLESS:
        for label in [
            "TC-A: OrderEntry → ペイン数 2",
            'TC-A: 新ペイン type = "Order Entry"',
            "TC-B: OrderList → ペイン数 3",
            'TC-B: 新ペイン type = "Order List"',
            "TC-C: BuyingPower → ペイン数 4",
            'TC-C: 新ペイン type = "Buying Power"',
            "TC-D: エラー通知 0 件",
            'TC-E: 元ペイン type = "Candlestick Chart" のまま',
        ]:
            pend(label, "sidebar API returns 501 in headless")
        return

    # autoplay で Playing に到達するまで待機
    if not wait_status("Playing", 60):
        fail("S36-precond", "Playing 到達せず（timeout）")
        return

    pane0_id = get_pane_id(0)
    print(f"  PANE0={pane0_id}")
    if not pane0_id:
        fail("S36-precond", "初期ペイン ID 取得失敗")
        return

    known_ids = {pane0_id}

    # ── TC-A: OrderEntry → ペイン数 2 ────────────────────────────────────────
    print()
    print("── TC-A: OrderEntry → ペイン数 2")
    try:
        api_post("/api/sidebar/open-order-pane", {"kind": "OrderEntry"})
    except Exception:
        pass

    if wait_for_pane_count(2, 15):
        pass_("TC-A: OrderEntry → ペイン数 2")
    else:
        body = api_get("/api/pane/list")
        actual = len(body.get("panes", []))
        fail("TC-A", f"15 秒以内に pane count が 2 にならなかった (actual={actual})")
        return

    body_a = api_get("/api/pane/list")
    panes_a = body_a.get("panes", [])
    pane_a = next((p for p in panes_a if p["id"] not in known_ids), None)
    pane_a_type = (pane_a or {}).get("type", "not_found")
    print(f"  new pane type={pane_a_type}")
    if pane_a_type == "Order Entry":
        pass_('TC-A: 新ペイン type = "Order Entry"')
    else:
        fail("TC-A", f'新ペイン type="{pane_a_type}" (expected "Order Entry")')
    if pane_a:
        known_ids.add(pane_a["id"])

    # ── TC-B: OrderList → ペイン数 3 ─────────────────────────────────────────
    print()
    print("── TC-B: OrderList → ペイン数 3")
    try:
        api_post("/api/sidebar/open-order-pane", {"kind": "OrderList"})
    except Exception:
        pass

    if wait_for_pane_count(3, 15):
        pass_("TC-B: OrderList → ペイン数 3")
    else:
        body = api_get("/api/pane/list")
        actual = len(body.get("panes", []))
        fail("TC-B", f"15 秒以内に pane count が 3 にならなかった (actual={actual})")
        return

    body_b = api_get("/api/pane/list")
    panes_b = body_b.get("panes", [])
    pane_b = next((p for p in panes_b if p["id"] not in known_ids), None)
    pane_b_type = (pane_b or {}).get("type", "not_found")
    print(f"  new pane type={pane_b_type}")
    if pane_b_type == "Order List":
        pass_('TC-B: 新ペイン type = "Order List"')
    else:
        fail("TC-B", f'新ペイン type="{pane_b_type}" (expected "Order List")')
    if pane_b:
        known_ids.add(pane_b["id"])

    # ── TC-C: BuyingPower → ペイン数 4 ───────────────────────────────────────
    print()
    print("── TC-C: BuyingPower → ペイン数 4")
    try:
        api_post("/api/sidebar/open-order-pane", {"kind": "BuyingPower"})
    except Exception:
        pass

    if wait_for_pane_count(4, 15):
        pass_("TC-C: BuyingPower → ペイン数 4")
    else:
        body = api_get("/api/pane/list")
        actual = len(body.get("panes", []))
        fail("TC-C", f"15 秒以内に pane count が 4 にならなかった (actual={actual})")
        return

    body_c = api_get("/api/pane/list")
    panes_c = body_c.get("panes", [])
    pane_c = next((p for p in panes_c if p["id"] not in known_ids), None)
    pane_c_type = (pane_c or {}).get("type", "not_found")
    print(f"  new pane type={pane_c_type}")
    if pane_c_type == "Buying Power":
        pass_('TC-C: 新ペイン type = "Buying Power"')
    else:
        fail("TC-C", f'新ペイン type="{pane_c_type}" (expected "Buying Power")')

    # ── TC-D: エラー通知が出ていない ─────────────────────────────────────────
    print()
    print("── TC-D: エラー通知なし確認")
    err_count = count_error_notifications()
    print(f"  error notification count={err_count}")
    if err_count == 0:
        pass_("TC-D: エラー通知 0 件")
    else:
        fail("TC-D", f"エラー通知が {err_count} 件発生")

    # ── TC-E: 元ペインの type が変わっていない ────────────────────────────────
    print()
    print("── TC-E: 元ペイン (PANE0) の type 確認")
    orig_pane = next((p for p in panes_c if p["id"] == pane0_id), None)
    orig_type = (orig_pane or {}).get("type", "not_found")
    print(f"  orig pane type={orig_type}")
    if orig_type == "Candlestick Chart":
        pass_('TC-E: 元ペイン type = "Candlestick Chart" のまま')
    else:
        fail("TC-E", f'元ペイン type="{orig_type}" (expected "Candlestick Chart")')


def test_s36_sidebar_order_pane() -> None:
    backup_state()
    start = utc_offset(-3)
    end = utc_offset(-1)
    setup_single_pane(primary_ticker(), "M1", start, end)
    env = FlowsurfaceEnv(ticker=primary_ticker(), timeframe="M1", headless=IS_HEADLESS)
    try:
        env._start_process()
        headless_play(start, end)
        run_s36()
    finally:
        env.close()
        restore_state()
    import helpers as _h
    assert _h._FAIL == 0, f"{_h._FAIL} TC(s) failed — see output above"


def main() -> None:
    backup_state()
    start = utc_offset(-3)
    end = utc_offset(-1)
    setup_single_pane(primary_ticker(), "M1", start, end)
    env = FlowsurfaceEnv(ticker=primary_ticker(), timeframe="M1", headless=IS_HEADLESS)
    try:
        env._start_process()
        headless_play(start, end)
        run_s36()
    finally:
        env.close()
        restore_state()
    print_summary()
    import helpers as _h
    sys.exit(0 if _h._FAIL == 0 else 1)


if __name__ == "__main__":
    main()
