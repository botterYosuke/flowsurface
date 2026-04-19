#!/usr/bin/env python3
"""s39_buying_power_portfolio.py — S39: BuyingPower パネル × portfolio.cash 整合性テスト

検証シナリオ:
  TC-A: BuyingPower パネルを開く → ペイン数 2
  TC-B: 新ペインの type = "Buying Power"
  TC-C: Pause → 初期 cash = 1000000
  TC-D: 成行買い 3件 → HTTP 200
  TC-E: status = "pending"
  TC-F: Paused 中 cash 不変
  TC-G: Paused 中 open_positions 空
  TC-H: BuyingPower パネル共存中のエラー通知 0 件
  TC-I: Live → Replay 再遷移 → mode=Replay
  TC-J: リセット後 cash = 1000000
  TC-K: リセット後 open_positions 空

仕様根拠:
  docs/plan/e2e_order_panels_replay.md §S39
  docs/order_windows.md §仮想約定エンジン §既知制限（Trades EventStore 未統合）

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


def _get_portfolio() -> dict:
    try:
        r = requests.get(f"{API_BASE}/api/replay/portfolio", timeout=5)
        return r.json() if r.status_code == 200 else {}
    except requests.RequestException:
        return {}


def run_s39() -> None:
    print("=== S39: BuyingPower × portfolio.cash 整合性テスト ===")

    if IS_HEADLESS:
        for label in [
            "TC-A: BuyingPower パネルを開く → ペイン数 2",
            'TC-B: 新ペイン type = "Buying Power"',
            "TC-C: 初期 portfolio.cash = 1000000",
            "TC-D: 成行買い 3件 → すべて HTTP 200",
            'TC-E: 成行買い status = "pending"',
            "TC-F: Paused 中 cash 不変",
            "TC-G: Paused 中 open_positions 空",
            "TC-H: BuyingPower パネル共存中のエラー通知 0 件",
            "TC-I: Live → Replay 再遷移成功",
            "TC-J: リセット後 cash = 1000000",
            "TC-K: リセット後 open_positions 空",
        ]:
            pend(label, "sidebar API returns 501 in headless")
        return

    sym = order_symbol()

    if not wait_status("Playing", 60):
        fail("precond", "REPLAY Playing に到達せず（60s タイムアウト）")
        return
    print("  REPLAY Playing 到達")

    pane0_id = get_pane_id(0)
    print(f"  PANE0={pane0_id}")

    # ── TC-A〜B: BuyingPower パネルを開く ─────────────────────────────────────
    print()
    print("── TC-A〜B: BuyingPower パネルを開く")

    try:
        api_post("/api/sidebar/open-order-pane", {"kind": "BuyingPower"})
    except Exception:
        pass

    if wait_for_pane_count(2, 15):
        pass_("TC-A: BuyingPower パネルを開く → ペイン数 2")
    else:
        body = api_get("/api/pane/list")
        actual = len(body.get("panes", []))
        fail("TC-A", f"15s 以内にペイン数 2 にならなかった (actual={actual})")
        return

    body_a = api_get("/api/pane/list")
    panes_a = body_a.get("panes", [])
    bp_pane = next((p for p in panes_a if p["id"] != pane0_id), None)
    bp_type = (bp_pane or {}).get("type", "not_found")
    print(f"  BuyingPower pane type={bp_type}")
    if bp_type == "Buying Power":
        pass_('TC-B: 新ペイン type = "Buying Power"')
    else:
        fail("TC-B", f'type="{bp_type}" (expected "Buying Power")')

    # ── TC-C: Paused にして初期 portfolio.cash を確認 ─────────────────────────
    print()
    print("── TC-C: 初期 portfolio.cash 確認")

    try:
        api_post("/api/replay/pause")
    except Exception:
        pass
    wait_status("Paused", 10)

    portfolio_init = _get_portfolio()
    print(f"  portfolio: {portfolio_init}")

    cash_init = portfolio_init.get("cash")
    if cash_init == 1000000 or cash_init == 1000000.0:
        pass_("TC-C: 初期 portfolio.cash = 1000000")
    else:
        fail("TC-C", f"cash={cash_init} (expected 1000000)")

    # ── TC-D〜E: Paused で成行買い × 3件 ─────────────────────────────────────
    print()
    print("── TC-D〜E: Paused で成行買い × 3件 place")

    code_1 = api_post_code(
        "/api/replay/order",
        {"ticker": sym, "side": "buy", "qty": 0.1, "order_type": "market"},
    )
    code_2 = api_post_code(
        "/api/replay/order",
        {"ticker": sym, "side": "buy", "qty": 0.2, "order_type": "market"},
    )
    code_3 = api_post_code(
        "/api/replay/order",
        {"ticker": sym, "side": "buy", "qty": 0.05, "order_type": "market"},
    )

    if code_1 == 200 and code_2 == 200 and code_3 == 200:
        pass_("TC-D: 成行買い 3件 → すべて HTTP 200")
    else:
        fail("TC-D", f"HTTP コード: {code_1} / {code_2} / {code_3} (expected 200/200/200)")

    # Paused のまま少し待ってからポートフォリオを確認（tick は来ない）
    time.sleep(1)
    portfolio_after = _get_portfolio()
    print(f"  portfolio after orders: {portfolio_after}")

    resp_status = api_post(
        "/api/replay/order",
        {"ticker": sym, "side": "buy", "qty": 0.03, "order_type": "market"},
    )
    order_status = resp_status.get("status")
    if order_status == "pending":
        pass_('TC-E: 成行買い status = "pending"')
    else:
        fail("TC-E", f"status={order_status} (expected pending)")

    # ── TC-F〜G: Paused 中 cash 不変・open_positions 空 ───────────────────────
    print()
    print("── TC-F〜G: Paused 中 portfolio 確認")

    cash_after = portfolio_after.get("cash")
    if cash_after == 1000000 or cash_after == 1000000.0:
        pass_(f"TC-F: Paused 中 cash 不変 (cash={cash_after})")
    else:
        fail("TC-F", f"cash={cash_after} (expected 1000000 — Paused なので約定しないはず)")

    open_after = portfolio_after.get("open_positions", None)
    open_len = len(open_after) if isinstance(open_after, list) else None
    if open_len == 0:
        pass_("TC-G: Paused 中 open_positions 空 (length=0)")
    else:
        fail("TC-G", f"open_positions.length={open_len} (expected 0)")

    # ── TC-H: BuyingPower パネル共存中のエラー通知 0 件 ──────────────────────
    print()
    print("── TC-H: エラー通知なし確認")
    err_count = count_error_notifications()
    if err_count == 0:
        pass_("TC-H: BuyingPower パネル共存中のエラー通知 0 件")
    else:
        fail("TC-H", f"エラー通知 {err_count} 件発生")

    # ── TC-I〜K: Live → Replay 再遷移でエンジンリセット → cash = 1000000 ──────
    print()
    print("── TC-I〜K: Live → Replay 再遷移でリセット確認")

    try:
        api_post("/api/replay/toggle")  # → Live
    except Exception:
        pass
    live_resp = api_get("/api/replay/status")
    live_mode = live_resp.get("mode")
    print(f"  toggle 後のモード: {live_mode}")

    try:
        api_post("/api/replay/toggle")  # → Replay (engine.reset() が呼ばれる)
    except Exception:
        pass
    replay_resp = api_get("/api/replay/status")
    replay_mode = replay_resp.get("mode")
    print(f"  再 toggle 後のモード: {replay_mode}")

    if replay_mode == "Replay":
        pass_(f"TC-I: Live → Replay 再遷移成功 (mode={replay_mode})")
    else:
        fail("TC-I", f"mode={replay_mode} (expected Replay)")

    portfolio_reset = _get_portfolio()
    print(f"  portfolio after reset: {portfolio_reset}")

    cash_reset = portfolio_reset.get("cash")
    if cash_reset == 1000000 or cash_reset == 1000000.0:
        pass_("TC-J: リセット後 cash = 1000000")
    else:
        fail("TC-J", f"cash={cash_reset} (expected 1000000)")

    open_reset = portfolio_reset.get("open_positions", None)
    open_reset_len = len(open_reset) if isinstance(open_reset, list) else None
    if open_reset_len == 0:
        pass_("TC-K: リセット後 open_positions 空 (length=0)")
    else:
        fail("TC-K", f"open_positions.length={open_reset_len} (expected 0)")


def test_s39_buying_power_portfolio() -> None:
    backup_state()
    start = utc_offset(-3)
    end = utc_offset(-1)
    setup_single_pane(primary_ticker(), "M1", start, end)
    env = FlowsurfaceEnv(ticker=primary_ticker(), timeframe="M1", headless=IS_HEADLESS)
    try:
        env._start_process()
        headless_play(start, end)
        run_s39()
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
        run_s39()
    finally:
        env.close()
        restore_state()
    print_summary()
    import helpers as _h
    sys.exit(0 if _h._FAIL == 0 else 1)


if __name__ == "__main__":
    main()
