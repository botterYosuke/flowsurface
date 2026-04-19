#!/usr/bin/env python3
"""s46_wrong_password.py — Phase 4-2: 発注パスワード誤り E2E テスト

検証シナリオ:
  1a: Live モード確認
  1b: Tachibana デモセッション確立
  2:  誤パスワード (WRONG_PASSWORD_PHASE4TEST) で TOYOTA 成行買い
  3:  レスポンスに order_number が含まれないことを確認
  4:  レスポンスに "error" フィールドがあることを確認

前提: DEV_IS_DEMO=true / DEV_USER_ID / DEV_PASSWORD が設定済み
NOTE: 意図的に DEV_SECOND_PASSWORD に依存しない（誤パスワードを使用する）
"""
from __future__ import annotations

import os
import re
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


def _extract_error_code(error_str: str) -> str:
    m = re.search(r"code=([^,}]+)", error_str)
    return m.group(1).strip() if m else "unknown"


def run_s46(env) -> None:
    print("=== Phase 4-2: 発注パスワード誤り E2E テスト ===")

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

    # ── Step 2: 誤パスワードで成行買い ──────────────────────────────────────
    print()
    print("── Step 2: 誤パスワード (WRONG_PASSWORD_PHASE4TEST) で TOYOTA 成行買い")
    order_body = {
        "issue_code": "7203",
        "qty": "100",
        "side": "3",
        "price": "0",
        "account_type": "1",
        "market_code": "00",
        "condition": "0",
        "cash_margin": "0",
        "expire_day": "0",
        "second_password": "WRONG_PASSWORD_PHASE4TEST",
    }
    order_resp: dict = {}
    try:
        r = requests.post(
            f"{API_BASE}/api/tachibana/order",
            json=order_body,
            timeout=5,
        )
        order_resp = r.json()
        print(f"  response: {order_resp}")
    except requests.RequestException as e:
        fail("Step 2", f"リクエスト失敗: {e}")
        return

    # ── Step 3: order_number が含まれないことを確認 ───────────────────────────
    print()
    print("── Step 3: 誤パスワードで order_number が返らないことを確認")
    order_num = order_resp.get("order_number")
    if order_num:
        fail("Step 3", f"誤パスワードなのに order_number が返った（セキュリティ上の問題）: {order_resp}")
    else:
        pass_("Step 3: 誤パスワードで order_number なし（期待どおり）")

    # ── Step 4: エラーフィールドの確認 ──────────────────────────────────────
    print()
    print("── Step 4: エラーコード抽出")
    error_val = order_resp.get("error")
    if error_val:
        err_code = _extract_error_code(str(error_val))
        print(f"  エラーコード: {err_code}")
        print(f"  エラーメッセージ: {error_val}")
        pass_(f"Step 4: エラーレスポンス取得（code={err_code}）")
    else:
        fail("Step 4", f"エラーフィールドなし: {order_resp}")


def test_s46_wrong_password() -> None:
    import helpers as _h
    _h._PASS = _h._FAIL = _h._PEND = 0
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
        run_s46(env)
    finally:
        env.close()
        restore_state()
    print_summary()
    assert _h._FAIL == 0, f"{_h._FAIL} TC(s) failed — see output above"


def main() -> None:
    guard = _check_env()
    if guard == "DEV_IS_DEMO=true を設定してください（本番誤発注防止）":
        print(f"ERROR: {guard}")
        sys.exit(1)
    if guard == "creds_missing":
        pend("全TC", "DEV_USER_ID / DEV_PASSWORD が未設定")
        print_summary()
        sys.exit(0)
    test_s46_wrong_password()
    print_summary()
    import helpers as _h
    sys.exit(0 if _h._FAIL == 0 else 1)


if __name__ == "__main__":
    main()
