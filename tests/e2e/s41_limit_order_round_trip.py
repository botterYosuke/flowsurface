#!/usr/bin/env python3
"""s41_limit_order_round_trip.py — S41: 指値注文ラウンドトリップ

検証シナリオ:
  A-B: Playing 到達 → Pause
  C-H: 指値買い @9,999,999（必ず約定）→ step-forward → cash 減算 → 指値売り @1 → クローズ
  I-K: 指値買い @1（絶対約定しない）→ step-forward × 3 → pending のまま残ることを確認

使い方:
    E2E_TICKER=BinanceLinear:BTCUSDT python tests/s41_limit_order_round_trip.py
    IS_HEADLESS=true python tests/s41_limit_order_round_trip.py
    pytest tests/s41_limit_order_round_trip.py -v
"""

from __future__ import annotations

import os
import sys
import time
from pathlib import Path

sys.path.insert(0, str(Path(__file__).parent))
from helpers import (
    FlowsurfaceEnv,
    IS_HEADLESS,
    TICKER,
    api_get,
    api_post,
    backup_state,
    headless_play,
    order_symbol,
    pass_,
    fail,
    pend,
    print_summary,
    restore_state,
    setup_single_pane,
    utc_offset,
    wait_status,
    wait_tachibana_session,
    _PASS,
    _FAIL,
    _PEND,
)


def run_s41() -> None:
    print("=== S41: 指値注文ラウンドトリップ ===")

    # ── TC-A: REPLAY Playing 到達 ────────────────────────────────────────────
    print()
    print("── TC-A: REPLAY Playing 到達")

    if not wait_status("Playing", 60):
        fail("TC-A", "auto-play で Playing に到達せず（60s タイムアウト）")
        print_summary()
        sys.exit(1)
    pass_("TC-A: REPLAY Playing 到達")

    # ── TC-B: Pause ──────────────────────────────────────────────────────────
    print()
    print("── TC-B: Pause")

    api_post("/api/replay/pause")
    if not wait_status("Paused", 10):
        fail("TC-B", "Pause 遷移せず（10s タイムアウト）")
        print_summary()
        sys.exit(1)
    pass_("TC-B: Pause 遷移（step-forward 有効化）")

    # ── TC-C〜H: 指値ラウンドトリップ（必ず約定するトリック価格） ─────────
    print()
    print("── TC-C〜H: 指値ラウンドトリップ（必ず約定するトリック価格）")

    buy_resp = api_post(
        "/api/replay/order",
        {"ticker": order_symbol(), "side": "buy", "qty": 1.0, "order_type": {"limit": 9999999.0}},
    )
    print(f"  limit buy response: {buy_resp}")
    buy_id = buy_resp.get("order_id")
    if buy_id and buy_id != "null":
        pass_(f"TC-C: 指値買い @9,999,999 → order_id={buy_id}")
    else:
        fail("TC-C", f"order_id が返らない (resp={buy_resp})")

    open_count = 0
    portfolio: dict = {}
    for i in range(1, 11):
        api_post("/api/replay/step-forward")
        time.sleep(0.3)
        portfolio = api_get("/api/replay/portfolio")
        open_count = len(portfolio.get("open_positions", []))
        print(f"  step {i}: open_positions={open_count}")
        if open_count >= 1:
            break

    if open_count >= 1:
        pass_(f"TC-D: 指値買い約定 → open_positions={open_count} (A-2 指値パス)")
    else:
        fail("TC-D", f"10 回 step-forward しても open_positions が増えない (={open_count})")

    cash = float(portfolio.get("cash") or 0)
    print(f"  cash after limit buy: {cash}")
    if cash < 1_000_000:
        pass_(f"TC-E: cash < 1,000,000 (={cash}) — 指値 fill でも cash deduct される (A-1)")
    else:
        fail("TC-E", f"cash={cash} (expected < 1000000)")

    sell_resp = api_post(
        "/api/replay/order",
        {"ticker": order_symbol(), "side": "sell", "qty": 1.0, "order_type": {"limit": 1.0}},
    )
    print(f"  limit sell response: {sell_resp}")
    sell_id = sell_resp.get("order_id")
    if sell_id and sell_id != "null":
        pass_(f"TC-F: 指値売り @1 → order_id={sell_id}")
    else:
        fail("TC-F", f"order_id が返らない (resp={sell_resp})")

    open_count = 1
    for i in range(1, 11):
        api_post("/api/replay/step-forward")
        time.sleep(0.3)
        portfolio = api_get("/api/replay/portfolio")
        open_count = len(portfolio.get("open_positions", []))
        print(f"  step {i}: open_positions={open_count}")
        if open_count == 0:
            break

    if open_count == 0:
        pass_("TC-G: 指値売り → Long クローズ → open_positions=0 (A-2 指値クローズ)")
    else:
        fail("TC-G", f"10 回 step-forward しても Long がクローズされない (={open_count})")

    realized_pnl = float(portfolio.get("realized_pnl") or 0)
    cash = float(portfolio.get("cash") or 0)
    diff = abs(cash - 1_000_000 - realized_pnl)
    if diff < 1.0:
        pass_(f"TC-H: cash ({cash}) = 1,000,000 + realized_pnl ({realized_pnl}) — 指値 close の A-0 パス確認")
    else:
        fail("TC-H", f"cash={cash}, realized={realized_pnl} — cash=initial+realized が成立しない")

    # ── TC-I〜K: 未達指値（buy @1）→ pending 維持 ───────────────────────────
    print()
    print("── TC-I〜K: 未達指値（buy @1）→ pending 維持")

    unmatched_resp = api_post(
        "/api/replay/order",
        {"ticker": order_symbol(), "side": "buy", "qty": 0.1, "order_type": {"limit": 1.0}},
    )
    print(f"  unmatched limit buy response: {unmatched_resp}")
    unmatched_id = unmatched_resp.get("order_id")
    if unmatched_id and unmatched_id != "null":
        pass_(f"TC-I: 未達指値注文 → order_id={unmatched_id} (pending に追加)")
    else:
        fail("TC-I", f"order_id が返らない (resp={unmatched_resp})")

    for _ in range(3):
        api_post("/api/replay/step-forward")
        time.sleep(0.3)

    portfolio = api_get("/api/replay/portfolio")
    open_count = len(portfolio.get("open_positions", []))
    if open_count == 0:
        pass_("TC-J: step-forward × 3 後も open_positions=0 — 未達指値は約定しない")
    else:
        fail("TC-J", f"open_positions={open_count} (expected 0 — 指値 @1 は約定しないはず)")

    orders_resp = api_get("/api/replay/orders")
    print(f"  orders: {orders_resp}")
    pending_count = len(orders_resp.get("orders") or [])
    if pending_count >= 1:
        pass_(f"TC-K: GET /api/replay/orders → orders.length={pending_count} — pending 残存確認")
    else:
        fail("TC-K", f"orders.length={pending_count} (expected >= 1 — 未達指値が pending に残るはず)")


def test_s41_limit_order_round_trip() -> None:
    """pytest エントリポイント。"""
    import helpers as _h
    _h._PASS = _h._FAIL = _h._PEND = 0
    backup_state()
    try:
        run_s41()
    finally:
        restore_state()
    print_summary()
    assert _h._FAIL == 0, f"{_h._FAIL} TC(s) failed — see output above"


def main() -> None:
    start = utc_offset(-3)
    end = utc_offset(-1)

    backup_state()
    setup_single_pane(TICKER, "M1", start, end)

    if not IS_HEADLESS:
        dev_user_id = os.environ.get("DEV_USER_ID", "")
        dev_password = os.environ.get("DEV_PASSWORD", "")
        if not dev_user_id or not dev_password:
            print("  SKIP: DEV_USER_ID / DEV_PASSWORD が未設定 — 自動ログインが無効です")
            restore_state()
            sys.exit(0)

    env = FlowsurfaceEnv(ticker=TICKER, timeframe="M1", headless=IS_HEADLESS)
    try:
        env._start_process()
        headless_play()

        if not IS_HEADLESS:
            print("  Tachibana セッション確立待ち（最大 60 秒）...")
            if not wait_tachibana_session(60):
                fail("precond", "Tachibana セッションが確立されなかった（DEV_USER_ID でのログインに失敗）")
                print_summary()
                sys.exit(1)
            print("  Tachibana セッション確立")

        run_s41()
    finally:
        env.close()
        restore_state()
        print_summary()

    import helpers
    if helpers._FAIL > 0:
        sys.exit(1)


if __name__ == "__main__":
    main()
