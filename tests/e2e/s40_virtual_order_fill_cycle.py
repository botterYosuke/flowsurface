#!/usr/bin/env python3
"""s40_virtual_order_fill_cycle.py — S40: 仮想取引フルサイクル（成行ラウンドトリップ）

検証シナリオ:
  A-B: Playing 到達 → Pause（step-forward は Paused 時のみ 1 bar 前進）
  C-E: 成行買い → step-forward 約定 → cash 減算確認 (A-1: record_open)
  F-K: 成行売り → step-forward Long クローズ → PnL/cash 確定 (A-0/A-2)

約定メカニズム:
  step-forward が synthetic_trades_at_current_time() で kline close 価格の
  合成トレードを生成し on_tick() へ渡す。成行注文は 1 回目で約定する。

フィクスチャ: BinanceLinear:BTCUSDT M1, replay auto-play (UTC[-3h, -1h])
"""

from __future__ import annotations

import math
import os
import sys
import time
from pathlib import Path

import requests

sys.path.insert(0, str(Path(__file__).parent))
from helpers import (
    API_BASE,
    FlowsurfaceEnv,
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
    wait_tachibana_session,
)


def _get_portfolio() -> dict:
    try:
        r = requests.get(f"{API_BASE}/api/replay/portfolio", timeout=5)
        return r.json() if r.status_code == 200 else {}
    except requests.RequestException:
        return {}


def run_s40() -> None:
    mode_label = "headless" if IS_HEADLESS else "GUI"
    print(f"=== S40: 仮想取引フルサイクル（成行ラウンドトリップ）({mode_label}) ===")

    start = utc_offset(-3)
    end = utc_offset(-1)
    setup_single_pane(TICKER, "M1", start, end)

    # GUI パス: Tachibana セッションが必要
    if not IS_HEADLESS:
        dev_user_id = os.environ.get("DEV_USER_ID", "")
        dev_password = os.environ.get("DEV_PASSWORD", "")
        if not dev_user_id or not dev_password:
            print("  SKIP: DEV_USER_ID / DEV_PASSWORD が未設定")
            return

    env = FlowsurfaceEnv(ticker=TICKER, timeframe="M1", headless=IS_HEADLESS)
    try:
        env._start_process()
        headless_play(start, end)

        # GUI パス: Tachibana セッション確立待ち
        if not IS_HEADLESS:
            print("  Tachibana セッション確立待ち（最大 60 秒）...")
            if not wait_tachibana_session(60):
                fail("precond", "Tachibana セッションが確立されなかった（DEV_USER_ID でのログインに失敗）")
                return
            print("  Tachibana セッション確立")

        # ──────────────────────────────────────────────────────────────────────
        # TC-A: REPLAY Playing 到達
        # ──────────────────────────────────────────────────────────────────────
        print()
        print("── TC-A: REPLAY Playing 到達")

        if not wait_status("Playing", 60):
            fail("TC-A", "auto-play で Playing に到達せず（60s タイムアウト）")
            return
        pass_("TC-A: REPLAY Playing 到達")

        # ──────────────────────────────────────────────────────────────────────
        # TC-B: Pause（step-forward は Paused 時のみ 1 bar 前進する。Playing 中は range 末尾へジャンプ）
        # ──────────────────────────────────────────────────────────────────────
        print()
        print("── TC-B: Pause")

        if not wait_status("Paused", 10):
            fail("TC-B", "Pause 遷移せず（10s タイムアウト）")
            return
        pass_("TC-B: Pause 遷移（step-forward 有効化）")

        # ──────────────────────────────────────────────────────────────────────
        # TC-C〜E: 成行買い → step-forward 約定 → cash 確認 (A-1)
        # ──────────────────────────────────────────────────────────────────────
        print()
        print("── TC-C〜E: 成行買い → step-forward 約定 → cash 確認 (A-1)")

        sym = order_symbol()
        buy_resp = api_post(
            "/api/replay/order",
            {"ticker": sym, "side": "buy", "qty": 1.0, "order_type": "market"},
        )
        print(f"  buy response: {buy_resp}")
        buy_id = buy_resp.get("order_id")
        if buy_id is not None and buy_id != "null" and buy_id != "":
            pass_(f"TC-C: 成行買い → order_id={buy_id}")
        else:
            fail("TC-C", f"order_id が返らない (resp={buy_resp})")

        open_count = 0
        portfolio: dict = {}
        for i in range(1, 11):
            api_post("/api/replay/step-forward")
            time.sleep(0.3)
            portfolio = _get_portfolio()
            open_count = len(portfolio.get("open_positions", []))
            print(f"  step {i}: open_positions={open_count}")
            if open_count >= 1:
                break

        if open_count >= 1:
            pass_(f"TC-D: step-forward で約定 → open_positions={open_count} (A-2)")
        else:
            fail("TC-D", f"10 回 step-forward しても open_positions が増えない (={open_count})")

        cash_after_buy = portfolio.get("cash", 0)
        print(f"  cash after buy: {cash_after_buy}")
        if isinstance(cash_after_buy, (int, float)) and cash_after_buy < 1_000_000:
            pass_(f"TC-E: cash < 1,000,000 (={cash_after_buy}) — 購入コスト減算確認 (A-1)")
        else:
            fail("TC-E", f"cash={cash_after_buy} (expected < 1000000)")

        # ──────────────────────────────────────────────────────────────────────
        # TC-F〜K: 成行売り → step-forward Long クローズ → PnL 確定 (A-0/A-2)
        # ──────────────────────────────────────────────────────────────────────
        print()
        print("── TC-F〜K: 成行売り → Long クローズ → PnL 確定 (A-0/A-2)")

        sell_resp = api_post(
            "/api/replay/order",
            {"ticker": sym, "side": "sell", "qty": 1.0, "order_type": "market"},
        )
        print(f"  sell response: {sell_resp}")
        sell_id = sell_resp.get("order_id")
        if sell_id is not None and sell_id != "null" and sell_id != "":
            pass_(f"TC-F: 成行売り → order_id={sell_id}")
        else:
            fail("TC-F", f"order_id が返らない (resp={sell_resp})")

        open_count = 1
        for i in range(1, 11):
            api_post("/api/replay/step-forward")
            time.sleep(0.3)
            portfolio = _get_portfolio()
            open_count = len(portfolio.get("open_positions", [1]))
            print(f"  step {i}: open_positions={open_count}")
            if open_count == 0:
                break

        if open_count == 0:
            pass_("TC-G: Long クローズ → open_positions=0 (A-2)")
        else:
            fail("TC-G", f"10 回 step-forward しても open_positions が 0 にならない (={open_count})")

        closed_len = len(portfolio.get("closed_positions", []))
        if closed_len == 1:
            pass_("TC-H: closed_positions.length=1 — record_close() 呼び出し確認 (A-2)")
        else:
            fail("TC-H", f"closed_positions.length={closed_len} (expected 1)")

        realized_pnl = portfolio.get("realized_pnl")
        print(f"  realized_pnl: {realized_pnl}")
        if isinstance(realized_pnl, (int, float)) and math.isfinite(float(realized_pnl)):
            pass_(f"TC-I: realized_pnl={realized_pnl} (PnL が数値として確定)")
        else:
            fail("TC-I", f"realized_pnl が数値でない (={realized_pnl})")

        cash_final = portfolio.get("cash", 0)
        realized_float = float(realized_pnl) if realized_pnl is not None else 0.0
        cash_expected = 1_000_000 + realized_float
        if abs(float(cash_final) - cash_expected) < 1.0:
            pass_(
                f"TC-J: cash ({cash_final}) = 1,000,000 + realized_pnl ({realized_pnl})"
                " — 売却代金返還確認 (A-0)"
            )
        else:
            fail(
                "TC-J",
                f"cash={cash_final}, realized={realized_pnl} — cash=initial+realized が成立しない",
            )

        unrealized_pnl = portfolio.get("unrealized_pnl", 0)
        total_equity = portfolio.get("total_equity", 0)
        equity_expected = float(cash_final) + float(unrealized_pnl)
        if abs(float(total_equity) - equity_expected) < 1.0:
            pass_(
                f"TC-K: total_equity ({total_equity}) = cash ({cash_final})"
                f" + unrealized ({unrealized_pnl})"
            )
        else:
            fail(
                "TC-K",
                f"total_equity={total_equity} ≠ cash+unrealized — スキーマ不整合",
            )

    finally:
        env.close()


def test_s40_virtual_order_fill_cycle() -> None:
    """pytest エントリポイント。"""
    import helpers as _h
    _h._PASS = _h._FAIL = _h._PEND = 0
    backup_state()
    try:
        run_s40()
    finally:
        restore_state()
    print_summary()
    assert _h._FAIL == 0, f"{_h._FAIL} TC(s) failed — see output above"


def main() -> None:
    backup_state()
    try:
        run_s40()
    finally:
        restore_state()
        print_summary()
        import helpers as _h
        if _h._FAIL > 0:
            sys.exit(1)


if __name__ == "__main__":
    main()
