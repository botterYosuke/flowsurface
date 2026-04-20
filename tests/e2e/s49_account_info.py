#!/usr/bin/env python3
"""s49_account_info.py — Phase 3-1/3-2/3-3: 口座情報系 HTTP API E2E テスト

検証シナリオ:
  1a: Live モード確認
  1b: Tachibana デモセッション確立
  2 (3-1): GET /api/buying-power → cash_buying_power フィールドあり
  2b:       エラーフィールドなし
  3 (3-2): GET /api/buying-power → margin_new_order_power フィールドあり
  4 (3-3): GET /api/tachibana/holdings?issue_code=7203 → holdings_qty フィールドあり
  5:        GET /api/tachibana/holdings (issue_code なし) → HTTP 400

前提: DEV_IS_DEMO=true / DEV_USER_ID / DEV_PASSWORD が設定済み
"""
from __future__ import annotations

import os
import sys
from pathlib import Path

import requests

sys.path.insert(0, str(Path(__file__).parent))
from helpers import (
    API_BASE,
    FlowsurfaceEnv,
    backup_state,
    restore_state,
    write_live_fixture,
    get_status,
    wait_tachibana_session,
    api_get_code,
    pass_,
    fail,
    pend,
    print_summary,
)

def _check_env() -> str | None:
    """環境ガード。問題があればエラーメッセージを返す。None なら OK。"""
    if os.environ.get("DEV_IS_DEMO", "") != "true":
        return "DEV_IS_DEMO=true を設定してください（本番誤発注防止）"
    if not os.environ.get("DEV_USER_ID") or not os.environ.get("DEV_PASSWORD"):
        return "creds_missing"
    return None


def run_s49(env) -> None:
    print("=== Phase 3-1/3-2/3-3: 口座情報系 HTTP API E2E テスト ===")

    # ── Step 1a: Live モード確認 ─────────────────────────────────────────────
    print()
    print("── Step 1a: Live モード確認")
    try:
        status = get_status()
        mode = status.get("mode", "null")
        if mode == "Live":
            pass_("Step 1a: Live モード確認 (mode=Live)")
        else:
            fail("Step 1a", f"mode={mode} (expected Live)")
    except requests.RequestException as e:
        fail("Step 1a", f"ステータス取得失敗: {e}")

    # ── Step 1b: Tachibana デモセッション待機 ─────────────────────────────────
    print()
    print("── Step 1b: Tachibana デモセッション待機")
    if wait_tachibana_session(120):
        pass_("Step 1b: デモセッション確立")
    else:
        fail("Step 1b", "セッション未確立")

    # ── Step 2 (3-1): GET /api/buying-power → cash_buying_power 確認 ──────────
    print()
    print("── Step 2 (3-1): GET /api/buying-power → cash_buying_power 確認")
    bp_resp: dict = {}
    try:
        r = requests.get(f"{API_BASE}/api/buying-power", timeout=5)
        bp_resp = r.json()
        print(f"  response: {bp_resp}")
    except requests.RequestException as e:
        fail("Step 2 (3-1)", f"リクエスト失敗: {e}")

    if "cash_buying_power" in bp_resp:
        cash_val = bp_resp["cash_buying_power"]
        print(f"  cash_buying_power={cash_val}")
        pass_("Step 2 (3-1): cash_buying_power フィールドあり")
    else:
        fail("Step 2 (3-1)", f"cash_buying_power フィールドなし: {bp_resp}")

    # ── Step 2b: エラーフィールドなし ─────────────────────────────────────────
    if not bp_resp.get("error"):
        pass_("Step 2b: エラーなし")
    else:
        fail("Step 2b", f"エラーあり: {bp_resp}")

    # ── Step 3 (3-2): GET /api/buying-power → margin_new_order_power 確認 ─────
    print()
    print("── Step 3 (3-2): GET /api/buying-power → margin_new_order_power 確認")
    if "margin_new_order_power" in bp_resp:
        margin_val = bp_resp["margin_new_order_power"]
        print(f"  margin_new_order_power={margin_val}")
        pass_("Step 3 (3-2): margin_new_order_power フィールドあり")
    else:
        fail("Step 3 (3-2)", f"margin_new_order_power フィールドなし: {bp_resp}")

    # ── Step 4 (3-3): GET /api/tachibana/holdings?issue_code=7203 ────────────
    print()
    print("── Step 4 (3-3): GET /api/tachibana/holdings?issue_code=7203 → holdings_qty 確認")
    holdings_resp: dict = {}
    try:
        r = requests.get(
            f"{API_BASE}/api/tachibana/holdings",
            params={"issue_code": "7203"},
            timeout=5,
        )
        holdings_resp = r.json()
        print(f"  response: {holdings_resp}")
    except requests.RequestException as e:
        fail("Step 4 (3-3)", f"リクエスト失敗: {e}")

    if "holdings_qty" in holdings_resp:
        holdings_qty = holdings_resp["holdings_qty"]
        print(f"  holdings_qty={holdings_qty} (TOYOTA 7203 の保有株数)")
        pass_("Step 4 (3-3): holdings_qty フィールドあり")
    else:
        fail("Step 4 (3-3)", f"holdings_qty フィールドなし: {holdings_resp}")

    # ── Step 5: issue_code なし → HTTP 400 ───────────────────────────────────
    print()
    print("── Step 5: issue_code なし → BadRequest")
    code = api_get_code("/api/tachibana/holdings")
    if code == 400:
        pass_("Step 5: issue_code なし → HTTP 400")
    else:
        fail("Step 5", f"HTTP {code} (expected 400)")


def test_s49_account_info() -> None:
    guard = _check_env()
    if guard == "DEV_IS_DEMO=true を設定してください（本番誤発注防止）":
        import pytest
        pytest.skip(guard)
    if guard == "creds_missing":
        pend("全TC", "DEV_USER_ID / DEV_PASSWORD が未設定")
        return
    backup_state()
    write_live_fixture("TachibanaSpot:7203", "D1", "Toyota-Live")
    env = FlowsurfaceEnv(ticker="TachibanaSpot:7203", timeframe="D1", headless=False)
    try:
        env._start_process()
        run_s49(env)
    finally:
        env.close()
        restore_state()


def main() -> None:
    guard = _check_env()
    if guard == "DEV_IS_DEMO=true を設定してください（本番誤発注防止）":
        print(f"ERROR: {guard}")
        sys.exit(1)
    if guard == "creds_missing":
        pend("全TC", "DEV_USER_ID / DEV_PASSWORD が未設定")
        print_summary()
        sys.exit(0)
    test_s49_account_info()
    print_summary()
    import helpers as _h
    sys.exit(0 if _h._FAIL == 0 else 1)


if __name__ == "__main__":
    main()
