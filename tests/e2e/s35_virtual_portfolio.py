#!/usr/bin/env python3
"""s35_virtual_portfolio.py — S35: 仮想ポートフォリオのライフサイクル検証

検証シナリオ:
  A-G: 初期ポートフォリオスナップショットのスキーマと値を検証
       (cash=1000000, unrealized_pnl=0, realized_pnl=0,
        total_equity=cash, open_positions=[], closed_positions=[])
  H-I: 成行注文を 2 件 place 後もポートフォリオは変化しない
       (現状 Trades EventStore 未統合のため約定なし → cash/positions 不変)
  J:   PEND — StepBackward による仮想エンジンリセット（未実装）
       docs/order_windows.md §未実装: "SeekBackward 時のエンジンリセット"
  K-L: Live → Replay 遷移でエンジンが reset() される
       (toggle → toggle 後に portfolio が初期値に戻ることを確認)

仕様根拠:
  docs/replay_header.md §11.2 PortfolioSnapshot スキーマ
  docs/order_windows.md §仮想約定エンジン §main.rs の拡張

フィクスチャ: BinanceLinear:BTCUSDT M1, auto-play (UTC[-3h, -1h])
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
    backup_state,
    fail,
    headless_play,
    order_symbol,
    pass_,
    pend,
    print_summary,
    restore_state,
    setup_single_pane,
    utc_offset,
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


def run_s35() -> None:
    """アプリが起動済みかつ Replay Playing 状態で呼び出す pure test function。"""
    mode_label = "headless" if IS_HEADLESS else "GUI"
    print(f"=== S35: 仮想ポートフォリオ ライフサイクル検証 ({mode_label}) ===")

    if not wait_status("Playing", 60):
        fail("precond", "auto-play で Playing に到達せず（60s タイムアウト）")
        return
    print("  REPLAY Playing 到達")

    # Paused にして状態を安定させてから検証する
    wait_status("Paused", 10)

    # ──────────────────────────────────────────────────────────────────────
    # TC-A〜G: 初期ポートフォリオスナップショット
    # ──────────────────────────────────────────────────────────────────────
    print()
    print("── TC-A〜G: 初期ポートフォリオスナップショット")

    code_a = requests.get(f"{API_BASE}/api/replay/portfolio", timeout=5).status_code
    if code_a == 200:
        pass_("TC-A: GET /api/replay/portfolio → HTTP 200")
    else:
        fail("TC-A", f"HTTP={code_a} (expected 200)")

    portfolio = _get_portfolio()
    print(f"  portfolio: {portfolio}")

    cash = portfolio.get("cash")
    if cash == 1000000 or cash == 1000000.0:
        pass_("TC-B: 初期 cash = 1000000.0")
    else:
        fail("TC-B", f"cash={cash} (expected 1000000)")

    unrealized = portfolio.get("unrealized_pnl")
    if unrealized == 0 or unrealized == 0.0:
        pass_("TC-C: 初期 unrealized_pnl = 0")
    else:
        fail("TC-C", f"unrealized_pnl={unrealized} (expected 0)")

    realized = portfolio.get("realized_pnl")
    if realized == 0 or realized == 0.0:
        pass_("TC-D: 初期 realized_pnl = 0")
    else:
        fail("TC-D", f"realized_pnl={realized} (expected 0)")

    equity = portfolio.get("total_equity")
    if equity == 1000000 or equity == 1000000.0:
        pass_("TC-E: 初期 total_equity = cash (1000000)")
    else:
        fail("TC-E", f"total_equity={equity} (expected 1000000)")

    open_positions = portfolio.get("open_positions", None)
    if isinstance(open_positions, list) and len(open_positions) == 0:
        pass_("TC-F: 初期 open_positions = [] (length=0)")
    else:
        fail("TC-F", f"open_positions.length={len(open_positions) if isinstance(open_positions, list) else 'null'} (expected 0)")

    closed_positions = portfolio.get("closed_positions", None)
    if isinstance(closed_positions, list) and len(closed_positions) == 0:
        pass_("TC-G: 初期 closed_positions = [] (length=0)")
    else:
        fail("TC-G", f"closed_positions.length={len(closed_positions) if isinstance(closed_positions, list) else 'null'} (expected 0)")

    # ──────────────────────────────────────────────────────────────────────
    # TC-H〜I: 注文 place 後のポートフォリオ（約定なし確認）
    # ──────────────────────────────────────────────────────────────────────
    print()
    print("── TC-H〜I: 注文 place 後のポートフォリオ（約定なし確認）")

    sym = order_symbol()
    api_post("/api/replay/order", {"ticker": sym, "side": "buy", "qty": 0.1, "order_type": "market"})
    api_post("/api/replay/order", {"ticker": sym, "side": "sell", "qty": 0.05, "order_type": "market"})

    # Paused のまま少し待ってからポートフォリオを確認（tick は来ない）
    time.sleep(1)
    portfolio_after = _get_portfolio()
    print(f"  portfolio after orders: {portfolio_after}")

    cash_after = portfolio_after.get("cash")
    if cash_after == 1000000 or cash_after == 1000000.0:
        pass_(f"TC-H: Paused 中に成行注文を place しても cash は不変 ({cash_after})")
    else:
        fail("TC-H", f"cash={cash_after} (expected 1000000 — Paused なので約定しないはず)")

    open_after = portfolio_after.get("open_positions", [])
    if isinstance(open_after, list) and len(open_after) == 0:
        pass_("TC-I: Paused 中 open_positions は空のまま (length=0)")
    else:
        fail("TC-I", f"open_positions.length={len(open_after) if isinstance(open_after, list) else 'null'} (expected 0)")

    # ──────────────────────────────────────────────────────────────────────
    # TC-J: PEND — StepBackward によるエンジンリセット（未実装）
    # ──────────────────────────────────────────────────────────────────────
    print()
    print("── TC-J: StepBackward によるポートフォリオリセット（実装待ち）")
    pend("TC-J", "StepBackward 後のエンジンリセットは未実装 (docs/order_windows.md §未実装)")

    # ──────────────────────────────────────────────────────────────────────
    # TC-K〜L: Live → Replay 遷移でエンジンが reset() される
    # ──────────────────────────────────────────────────────────────────────
    print()
    print("── TC-K〜L: Live → Replay 遷移でエンジンリセット")

    if IS_HEADLESS:
        pend("TC-K", "headless は Live/Replay toggle 非対応")
        pend("TC-L", "headless は Live/Replay toggle 非対応")
    else:
        # Replay → Live → Replay とトグルし、エンジンが reset() されることを確認
        api_post("/api/replay/toggle")  # → Live
        live_resp = api_get("/api/replay/status")
        live_mode = live_resp.get("mode")
        print(f"  toggle 後のモード: {live_mode}")

        api_post("/api/replay/toggle")  # → Replay (engine.reset() が呼ばれる)
        replay_resp = api_get("/api/replay/status")
        replay_mode = replay_resp.get("mode")
        print(f"  再 toggle 後のモード: {replay_mode}")

        if replay_mode == "Replay":
            pass_(f"TC-K: Live → Replay 再遷移成功 (mode={replay_mode})")
        else:
            fail("TC-K", f"mode={replay_mode} (expected Replay)")

        portfolio_reset = _get_portfolio()
        print(f"  portfolio after reset: {portfolio_reset}")

        cash_reset = portfolio_reset.get("cash")
        open_reset = portfolio_reset.get("open_positions", [])
        cash_ok = cash_reset == 1000000 or cash_reset == 1000000.0
        open_ok = isinstance(open_reset, list) and len(open_reset) == 0

        if cash_ok and open_ok:
            pass_(
                f"TC-L: Live→Replay 遷移後 portfolio リセット (cash={cash_reset}, open_positions=[])"
            )
        else:
            fail(
                "TC-L",
                f"cash={cash_reset} open_positions={len(open_reset) if isinstance(open_reset, list) else 'null'}"
                " (expected cash=1000000, open=0)",
            )


def test_s35_virtual_portfolio() -> None:
    """pytest エントリポイント。プロセス起動は外部で行うこと。"""
    import helpers as _h
    _h._PASS = _h._FAIL = _h._PEND = 0
    run_s35()
    print_summary()
    assert _h._FAIL == 0, f"{_h._FAIL} TC(s) failed — see output above"


def main() -> None:
    start = utc_offset(-3)
    end = utc_offset(-1)

    backup_state()
    setup_single_pane(TICKER, "M1", start, end)

    env = FlowsurfaceEnv(ticker=TICKER, timeframe="M1", headless=IS_HEADLESS)
    try:
        env._start_process()
        headless_play(start, end)
        run_s35()
    finally:
        env.close()
        restore_state()
        print_summary()
        import helpers as _h
        if _h._FAIL > 0:
            sys.exit(1)


if __name__ == "__main__":
    main()
