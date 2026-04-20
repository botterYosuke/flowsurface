#!/usr/bin/env python3
"""s42_naked_short_cycle.py — S42: 裸ショートフルサイクル

検証シナリオ:
  A-B: Playing 到達 → Pause
  C:   Long ポジションなしを確認（裸ショートの前提）
  D-F: 成行売り（Long なし）→ step-forward → Short open → cash 増加 (A-1/A-2)
  G-K: 成行買い → step-forward → Short クローズ → PnL 確定 (A-0/A-2 対称拡張)

使い方:
    E2E_TICKER=BinanceLinear:BTCUSDT python tests/s42_naked_short_cycle.py
    IS_HEADLESS=true python tests/s42_naked_short_cycle.py
    pytest tests/s42_naked_short_cycle.py -v
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
)


def run_s42() -> None:
    print("=== S42: 裸ショートフルサイクル ===")

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

    # ── TC-C: 裸ショート前提確認（open_positions == 0） ──────────────────────
    print()
    print("── TC-C: 裸ショート前提確認（open_positions == 0）")

    portfolio = api_get("/api/replay/portfolio")
    open_count = len(portfolio.get("open_positions", []))
    if open_count == 0:
        pass_("TC-C: open_positions=0 — 裸ショートの前提を満たす")
    else:
        fail("TC-C", f"open_positions={open_count} (expected 0 — Long ポジションがあると裸ショートにならない)")

    # ── TC-D〜F: 裸ショート open → cash 増加 (A-1/A-2) ───────────────────────
    print()
    print("── TC-D〜F: 裸ショート open → cash 増加 (A-1/A-2)")

    short_resp = api_post(
        "/api/replay/order",
        {"ticker": order_symbol(), "side": "sell", "qty": 1.0, "order_type": "market"},
    )
    print(f"  naked short response: {short_resp}")
    short_id = short_resp.get("order_id")
    if short_id and short_id != "null":
        pass_(f"TC-D: 成行売り（Long なし）→ order_id={short_id}")
    else:
        fail("TC-D", f"order_id が返らない (resp={short_resp})")

    open_count = 0
    for i in range(1, 11):
        api_post("/api/replay/step-forward")
        time.sleep(0.3)
        portfolio = api_get("/api/replay/portfolio")
        open_count = len(portfolio.get("open_positions", []))
        print(f"  step {i}: open_positions={open_count}")
        if open_count >= 1:
            break

    if open_count >= 1:
        pass_(f"TC-D-check: step-forward で Short open → open_positions={open_count} (A-2: 裸ショート)")
    else:
        fail("TC-D-check", f"10 回 step-forward しても open_positions が増えない (={open_count})")

    positions = portfolio.get("open_positions", [])
    side = positions[0].get("side") if positions else None
    print(f"  open_positions[0].side: {side}")
    if side == "Short":
        pass_("TC-E: open_positions[0].side=Short — 裸ショートが Short として記録 (A-2)")
    else:
        fail("TC-E", f"side={side} (expected Short)")

    cash = float(portfolio.get("cash") or 0)
    print(f"  cash after short open: {cash}")
    if cash > 1_000_000:
        pass_(f"TC-F: cash > 1,000,000 (={cash}) — Short open で売り代金を受け取った (A-1)")
    else:
        fail("TC-F", f"cash={cash} (expected > 1000000 — Short open では cash が増加するはず)")

    # ── TC-G〜K: 買い注文で Short クローズ → PnL 確定 (A-0/A-2 対称拡張) ────
    print()
    print("── TC-G〜K: 買い注文で Short クローズ → PnL 確定 (A-0/A-2 対称拡張)")

    buy_resp = api_post(
        "/api/replay/order",
        {"ticker": order_symbol(), "side": "buy", "qty": 1.0, "order_type": "market"},
    )
    print(f"  buy to close response: {buy_resp}")
    buy_id = buy_resp.get("order_id")
    if buy_id and buy_id != "null":
        pass_(f"TC-G: 成行買い（Short クローズ用）→ order_id={buy_id}")
    else:
        fail("TC-G", f"order_id が返らない (resp={buy_resp})")

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
        pass_("TC-H: Short クローズ → open_positions=0 (A-2 対称拡張: buy closes Short)")
    else:
        fail("TC-H", f"10 回 step-forward しても Short がクローズされない (={open_count})")

    closed_count = len(portfolio.get("closed_positions", []))
    if closed_count == 1:
        pass_("TC-I: closed_positions.length=1 — Short の record_close() 呼び出し確認")
    else:
        fail("TC-I", f"closed_positions.length={closed_count} (expected 1)")

    # VirtualOrderFilled が iced メッセージループで処理されるまで最大 5s ポーリング
    realized_pnl = float(portfolio.get("realized_pnl") or 0)
    for _ in range(25):
        if realized_pnl != 0:
            break
        time.sleep(0.2)
        portfolio = api_get("/api/replay/portfolio")
        realized_pnl = float(portfolio.get("realized_pnl") or 0)

    print(f"  realized_pnl: {realized_pnl}")
    if realized_pnl != 0:
        pass_(f"TC-J: realized_pnl != 0 (={realized_pnl}) — Short の PnL 計算確認 (A-0)")
    else:
        fail("TC-J", "realized_pnl=0 (PnL が確定していない)")

    cash = float(portfolio.get("cash") or 0)
    diff = abs(cash - 1_000_000 - realized_pnl)
    if diff < 1.0:
        pass_(f"TC-K: cash ({cash}) = 1,000,000 + realized_pnl ({realized_pnl}) — Short close の A-0 パス確認")
    else:
        fail("TC-K", f"cash={cash}, realized={realized_pnl} — cash=initial+realized が成立しない")


def test_s42_naked_short_cycle() -> None:
    """pytest エントリポイント。"""
    import helpers as _h
    _h._PASS = _h._FAIL = _h._PEND = 0
    backup_state()
    try:
        run_s42()
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

        run_s42()
    finally:
        env.close()
        restore_state()
        print_summary()

    import helpers
    if helpers._FAIL > 0:
        sys.exit(1)


if __name__ == "__main__":
    main()
