#!/usr/bin/env python3
"""s48_invalid_issue.py — Phase 4-4: 存在しない銘柄コードの注文 E2E テスト

検証シナリオ:
  1a: Live モード確認
  1b: Tachibana デモセッション確立
  2:  銘柄コード '0000' で成行買い注文
  3:  order_number が返らないことを確認
  4a: レスポンスが有効な JSON であることを確認
  4b: レスポンスに "error" フィールドがあることを確認

前提: DEV_IS_DEMO=true / DEV_USER_ID / DEV_PASSWORD / DEV_SECOND_PASSWORD が設定済み
"""
from __future__ import annotations

import json as _json
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
    if not os.environ.get("DEV_SECOND_PASSWORD"):
        return "second_password_missing"
    return None


def _extract_error_code(error_str: str) -> str:
    m = re.search(r"code=([^,}]+)", error_str)
    return m.group(1).strip() if m else "unknown"


def run_s48(env) -> None:
    print("=== Phase 4-4: 存在しない銘柄コード E2E テスト ===")

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

    # ── Step 2: 銘柄コード '0000' で成行買い注文 ──────────────────────────────
    print()
    print("── Step 2: 銘柄コード '0000' で成行買い注文")
    order_body = {
        "issue_code": "0000",
        "qty": "1",
        "side": "3",
        "price": "0",
        "account_type": "1",
        "market_code": "00",
        "condition": "0",
        "cash_margin": "0",
        "expire_day": "0",
    }
    order_resp_text = ""
    order_resp: dict = {}
    try:
        r = requests.post(
            f"{API_BASE}/api/tachibana/order",
            json=order_body,
            timeout=5,
        )
        order_resp_text = r.text
        order_resp = r.json()
        print(f"  response: {order_resp}")
    except requests.RequestException as e:
        fail("Step 2", f"リクエスト失敗: {e}")
        return

    # ── Step 3: order_number が返らないことを確認 ────────────────────────────
    print()
    print("── Step 3: 無効銘柄コードで order_number が返らないことを確認")
    order_num = order_resp.get("order_number")
    if order_num:
        fail("Step 3", f"無効銘柄コードなのに order_number が返った: {order_resp}")
    else:
        pass_("Step 3: 無効銘柄コードで order_number なし（期待どおり）")

    # ── Step 4a: レスポンスが有効な JSON ────────────────────────────────────
    print()
    print("── Step 4: エラーコード抽出")
    try:
        _json.loads(order_resp_text)
        pass_("Step 4a: レスポンスが有効な JSON（クラッシュなし）")
    except (_json.JSONDecodeError, ValueError):
        fail("Step 4a", f"レスポンスが JSON でない（クラッシュの可能性）: {order_resp_text}")

    # ── Step 4b: error フィールドの確認 ─────────────────────────────────────
    error_val = order_resp.get("error")
    if error_val:
        err_code = _extract_error_code(str(error_val))
        print(f"  エラーコード: {err_code}")
        print(f"  エラーメッセージ: {error_val}")
        pass_(f"Step 4b: エラーレスポンス取得（code={err_code}）")
    else:
        fail("Step 4b", f"無効銘柄コードに対して error フィールドが返らなかった: {order_resp}")


def test_s48_invalid_issue() -> None:
    import helpers as _h
    _h._PASS = _h._FAIL = _h._PEND = 0
    guard = _check_env()
    if guard == "DEV_IS_DEMO=true を設定してください（本番誤発注防止）":
        import pytest
        pytest.skip(guard)
    if guard in ("creds_missing", "second_password_missing"):
        pend("全TC", f"環境変数未設定: {guard}")
        return
    backup_state()
    write_live_fixture("TachibanaSpot:7203", "D1", "Toyota-Live")
    env = FlowsurfaceEnv(ticker="TachibanaSpot:7203", timeframe="D1", headless=False)
    try:
        env._start_process()
        run_s48(env)
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
    if guard in ("creds_missing", "second_password_missing"):
        pend("全TC", f"環境変数未設定: {guard}")
        print_summary()
        sys.exit(0)
    test_s48_invalid_issue()
    print_summary()
    import helpers as _h
    sys.exit(0 if _h._FAIL == 0 else 1)


if __name__ == "__main__":
    main()
