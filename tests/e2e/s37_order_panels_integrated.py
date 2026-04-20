#!/usr/bin/env python3
"""s37_order_panels_integrated.py — S37: 3パネル統合テスト（OrderEntry / OrderList / BuyingPower）

検証シナリオ:
  TC-A: 3パネルを順に開いてペイン数 4
  TC-B: パネル開閉中のエラー通知 0 件
  TC-C: 成行買い 3件 → HTTP 200
  TC-D: order_id 返却
  TC-E: Pause → portfolio.cash = 1000000（約定なし）
  TC-F: open_positions 空
  TC-G: 指値買い → HTTP 200
  TC-H: 指値売り → HTTP 200
  TC-I: 指値注文 4件 すべて status="pending"
  TC-J: 注文後エラー通知 0 件
  TC-K: 元ペインの type = "Candlestick Chart" のまま

仕様根拠:
  docs/plan/e2e_order_panels_replay.md §S37
  docs/order_windows.md §仮想約定エンジン §既知制限

フィクスチャ: primary_ticker() M1, replay auto-play (UTC[-3h, -1h])
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
    api_get,
    api_post,
    api_post_code,
    backup_state,
    count_error_notifications,
    fail,
    get_pane_id,
    headless_play,
    order_symbol,
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


def run_s37() -> None:
    print("=== S37: 3パネル統合テスト（OrderEntry / OrderList / BuyingPower） ===")

    if IS_HEADLESS:
        for label in [
            "TC-A: 3パネルを順に開いてペイン数 4",
            "TC-B: パネル開閉中のエラー通知 0 件",
            "TC-C: 成行買い 3件 → すべて HTTP 200",
            "TC-D: 成行買い 3件 → すべて order_id 返却",
            "TC-E: Paused 中 cash = 1000000（約定なし）",
            "TC-F: Paused 中 open_positions 空",
            "TC-G: 指値買い → HTTP 200",
            "TC-H: 指値売り → HTTP 200",
            'TC-I: 指値注文 4件 すべて status="pending"',
            "TC-J: 注文後エラー通知 0 件",
            'TC-K: 元ペイン type = "Candlestick Chart" のまま',
        ]:
            pend(label, "sidebar API returns 501 in headless")
        return

    sym = order_symbol()

    # autoplay で Playing に到達するまで待機
    if not wait_status("Playing", 60):
        fail("precond", "REPLAY Playing に到達せず（60s タイムアウト）")
        return
    print("  REPLAY Playing 到達")

    pane0_id = get_pane_id(0)
    print(f"  PANE0={pane0_id}")
    if not pane0_id:
        fail("precond", "初期ペイン ID 取得失敗")
        return

    # ── TC-A〜B: Playing 中に 3パネルを順に開く ───────────────────────────────
    print()
    print("── TC-A〜B: Playing 中に 3パネルを順に開く")

    try:
        api_post("/api/sidebar/open-order-pane", {"kind": "OrderEntry"})
    except Exception:
        pass
    wait_for_pane_count(2, 15)

    try:
        api_post("/api/sidebar/open-order-pane", {"kind": "OrderList"})
    except Exception:
        pass
    wait_for_pane_count(3, 15)

    try:
        api_post("/api/sidebar/open-order-pane", {"kind": "BuyingPower"})
    except Exception:
        pass

    if wait_for_pane_count(4, 15):
        pass_("TC-A: 3パネルを順に開いてペイン数 4 に到達")
    else:
        body = api_get("/api/pane/list")
        actual = len(body.get("panes", []))
        fail("TC-A", f"15s 以内にペイン数 4 にならなかった (actual={actual})")
        return

    err_a = count_error_notifications()
    if err_a == 0:
        pass_("TC-B: パネル開閉中のエラー通知 0 件")
    else:
        fail("TC-B", f"エラー通知 {err_a} 件発生")

    # ── TC-C〜D: Playing 中に成行買い × 3件 place ─────────────────────────────
    print()
    print("── TC-C〜D: Playing 中に成行買い × 3件")

    all_200 = True
    all_ids = True
    for _ in range(3):
        resp = api_post(
            "/api/replay/order",
            {"ticker": sym, "side": "buy", "qty": 0.05, "order_type": "market"},
        )
        code = api_post_code(
            "/api/replay/order",
            {"ticker": sym, "side": "buy", "qty": 0.02, "order_type": "market"},
        )
        order_id = resp.get("order_id")
        if code != 200:
            all_200 = False
        if not (isinstance(order_id, str) and len(order_id) > 0):
            all_ids = False

    if all_200:
        pass_("TC-C: 成行買い 3件 → すべて HTTP 200")
    else:
        fail("TC-C", "HTTP 200 でない注文あり")

    if all_ids:
        pass_("TC-D: 成行買い 3件 → すべて order_id 返却")
    else:
        fail("TC-D", "order_id が null の注文あり")

    # ── TC-E〜F: Pause → portfolio 確認 ──────────────────────────────────────
    print()
    print("── TC-E〜F: Pause → portfolio 確認")

    try:
        api_post("/api/replay/pause")
    except Exception:
        pass
    wait_status("Paused", 10)

    try:
        portfolio = requests.get(f"{API_BASE}/api/replay/portfolio", timeout=5).json()
    except Exception:
        portfolio = {}
    print(f"  portfolio: {portfolio}")

    cash = portfolio.get("cash")
    if cash == 1000000 or cash == 1000000.0:
        pass_("TC-E: Paused 中 cash = 1000000（約定なし）")
    else:
        fail("TC-E", f"cash={cash} (expected 1000000)")

    open_positions = portfolio.get("open_positions", None)
    open_len = len(open_positions) if isinstance(open_positions, list) else None
    if open_len == 0:
        pass_("TC-F: Paused 中 open_positions 空（約定なし）")
    else:
        fail("TC-F", f"open_positions.length={open_len} (expected 0)")

    # ── TC-G〜I: Paused のまま指値注文 × 4件 ─────────────────────────────────
    print()
    print("── TC-G〜I: Paused 中に指値注文 × 4件")

    limit_resps = []
    limit_buy_1 = api_post(
        "/api/replay/order",
        {"ticker": sym, "side": "buy", "qty": 0.05, "order_type": {"limit": 1.0}},
    )
    limit_resps.append(limit_buy_1)
    limit_buy_2 = api_post(
        "/api/replay/order",
        {"ticker": sym, "side": "buy", "qty": 0.03, "order_type": {"limit": 1.0}},
    )
    limit_resps.append(limit_buy_2)
    limit_sell_1 = api_post(
        "/api/replay/order",
        {"ticker": sym, "side": "sell", "qty": 0.05, "order_type": {"limit": 9999999.0}},
    )
    limit_resps.append(limit_sell_1)
    limit_sell_2 = api_post(
        "/api/replay/order",
        {"ticker": sym, "side": "sell", "qty": 0.03, "order_type": {"limit": 9999999.0}},
    )
    limit_resps.append(limit_sell_2)

    code_lb = api_post_code(
        "/api/replay/order",
        {"ticker": sym, "side": "buy", "qty": 0.01, "order_type": {"limit": 1.0}},
    )
    code_ls = api_post_code(
        "/api/replay/order",
        {"ticker": sym, "side": "sell", "qty": 0.01, "order_type": {"limit": 9999999.0}},
    )

    if code_lb == 200:
        pass_("TC-G: 指値買い → HTTP 200")
    else:
        fail("TC-G", f"HTTP={code_lb} (expected 200)")

    if code_ls == 200:
        pass_("TC-H: 指値売り → HTTP 200")
    else:
        fail("TC-H", f"HTTP={code_ls} (expected 200)")

    all_pending = all(r.get("status") == "pending" for r in limit_resps)
    if all_pending:
        pass_('TC-I: 指値注文 4件 すべて status="pending"')
    else:
        fail("TC-I", "pending でない注文あり")

    # ── TC-J: 注文後もエラー通知 0 件 ────────────────────────────────────────
    print()
    print("── TC-J: 注文後エラー通知なし確認")
    err_j = count_error_notifications()
    if err_j == 0:
        pass_("TC-J: 注文後エラー通知 0 件")
    else:
        fail("TC-J", f"エラー通知 {err_j} 件発生")

    # ── TC-K: 元チャートペインの type が変わっていない ────────────────────────
    print()
    print("── TC-K: 元ペイン (PANE0) の type 確認")
    body_k = api_get("/api/pane/list")
    panes_k = body_k.get("panes", [])
    orig_pane = next((p for p in panes_k if p["id"] == pane0_id), None)
    orig_type = (orig_pane or {}).get("type", "not_found")
    print(f"  orig pane type={orig_type}")
    if orig_type == "Candlestick Chart":
        pass_('TC-K: 元ペイン type = "Candlestick Chart" のまま')
    else:
        fail("TC-K", f'元ペイン type="{orig_type}" (expected "Candlestick Chart")')


def test_s37_order_panels_integrated() -> None:
    backup_state()
    start = utc_offset(-3)
    end = utc_offset(-1)
    setup_single_pane(primary_ticker(), "M1", start, end)
    env = FlowsurfaceEnv(ticker=primary_ticker(), timeframe="M1", headless=IS_HEADLESS)
    try:
        env._start_process()
        headless_play(start, end)
        run_s37()
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
        run_s37()
    finally:
        env.close()
        restore_state()
    print_summary()
    import helpers as _h
    sys.exit(0 if _h._FAIL == 0 else 1)


if __name__ == "__main__":
    main()
