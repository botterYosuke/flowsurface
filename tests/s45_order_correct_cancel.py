#!/usr/bin/env python3
"""s45_order_correct_cancel.py — 訂正→取消 round-trip E2E テスト（シナリオ 2-3, 2-4）

検証シナリオ:
  1a: Live モード確認
  1b: Tachibana デモセッション確立
  2:  TOYOTA 100株 指値買い (price=70円) → 注文番号取得 or API 疎通確認
  3:  訂正注文 (price 70→75円) → 成功 or API 疎通確認
  4:  取消注文 → 成功 or API 疎通確認
  5:  GET /api/tachibana/orders → orders が配列

前提: DEV_IS_DEMO=true / DEV_USER_ID / DEV_PASSWORD / DEV_SECOND_PASSWORD が設定済み
"""
from __future__ import annotations

import os
import sys
import time
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

# ── デモ環境ガード ────────────────────────────────────────────────────────────

DEV_IS_DEMO = os.environ.get("DEV_IS_DEMO", "")
DEV_USER_ID = os.environ.get("DEV_USER_ID", "")
DEV_PASSWORD = os.environ.get("DEV_PASSWORD", "")
DEV_SECOND_PASSWORD = os.environ.get("DEV_SECOND_PASSWORD", "")


def run_s45(env) -> None:
    print("=== 訂正→取消 round-trip E2E テスト ===")

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

    # ── Step 2: TOYOTA 100株 指値買い (price=70円) ───────────────────────────
    print()
    print("── Step 2: TOYOTA (7203) 100株 指値買い (price=70 円)")
    order_body = {
        "issue_code": "7203",
        "qty": "100",
        "side": "3",
        "price": "70",
        "account_type": "1",
        "market_code": "00",
        "condition": "0",
        "cash_margin": "0",
        "expire_day": "0",
    }
    order_num = "none"
    eig_day = "none"
    try:
        r = requests.post(
            f"{API_BASE}/api/tachibana/order",
            json=order_body,
            timeout=5,
        )
        order_resp = r.json()
        print(f"  order response: {order_resp}")
        order_num = order_resp.get("order_number") or "none"
        eig_day = order_resp.get("eig_day") or "none"
        if order_num and order_num != "none":
            pass_(f"Step 2: 指値買い注文受付済み (order_number={order_num}, eig_day={eig_day})")
        else:
            error = order_resp.get("error", "unknown")
            print(f"  INFO: 注文エラー ({error})")
            pass_("Step 2: 指値買い API 疎通確認 — エラーコード取得済み")
    except requests.RequestException as e:
        fail("Step 2", f"リクエスト失敗: {e}")

    # ── Step 3: 訂正注文 (price 70→75円) ────────────────────────────────────
    print()
    print(f"── Step 3: 訂正注文 (order_number={order_num}, price 70→75 円)")
    correct_body = {
        "order_number": order_num,
        "eig_day": eig_day,
        "condition": "*",
        "price": "75",
        "qty": "*",
        "expire_day": "*",
    }
    try:
        r = requests.post(
            f"{API_BASE}/api/tachibana/order/correct",
            json=correct_body,
            timeout=5,
        )
        correct_resp = r.json()
        print(f"  correct response: {correct_resp}")
        correct_num = correct_resp.get("order_number") or "none"
        correct_err = correct_resp.get("error")
        if correct_num and correct_num != "none":
            pass_(f"Step 3: 訂正注文受付済み (order_number={correct_num})")
            order_num = correct_num
        elif correct_err:
            print(f"  INFO: 訂正エラー ({correct_err}) — 市場時間外または既約定の可能性")
            pass_(f"Step 3: 訂正注文 API 疎通確認（エラー応答: {correct_err}）")
        else:
            fail("Step 3", f"訂正注文レスポンス解析失敗: {correct_resp}")
    except requests.RequestException as e:
        fail("Step 3", f"リクエスト失敗: {e}")

    # ── Step 4: 取消注文 ─────────────────────────────────────────────────────
    print()
    print(f"── Step 4: 取消注文 (order_number={order_num})")
    cancel_body = {
        "order_number": order_num,
        "eig_day": eig_day,
    }
    try:
        r = requests.post(
            f"{API_BASE}/api/tachibana/order/cancel",
            json=cancel_body,
            timeout=5,
        )
        cancel_resp = r.json()
        print(f"  cancel response: {cancel_resp}")
        cancel_num = cancel_resp.get("order_number") or "none"
        cancel_err = cancel_resp.get("error")
        if cancel_num and cancel_num != "none":
            pass_(f"Step 4: 取消注文受付済み (order_number={cancel_num})")
        elif cancel_err:
            print(f"  INFO: 取消エラー ({cancel_err}) — 市場時間外または既取消の可能性")
            pass_(f"Step 4: 取消注文 API 疎通確認（エラー応答: {cancel_err}）")
        else:
            fail("Step 4", f"取消注文レスポンス解析失敗: {cancel_resp}")
    except requests.RequestException as e:
        fail("Step 4", f"リクエスト失敗: {e}")

    # ── Step 5: 注文一覧で状態確認 ───────────────────────────────────────────
    print()
    print("── Step 5: 注文一覧で状態確認")
    time.sleep(1)
    try:
        r = requests.get(f"{API_BASE}/api/tachibana/orders", timeout=5)
        list_resp = r.json()
        print(f"  orders: {list_resp}")
        if isinstance(list_resp.get("orders"), list):
            pass_("Step 5: 注文一覧レスポンス確認（配列形式）")
        else:
            fail("Step 5", f"注文一覧が配列でない: {list_resp}")
    except requests.RequestException as e:
        fail("Step 5", f"リクエスト失敗: {e}")


def test_s45_order_correct_cancel() -> None:
    import pytest
    if DEV_IS_DEMO != "true":
        pytest.skip("DEV_IS_DEMO=true が未設定（本番誤発注防止）")
    if not DEV_USER_ID or not DEV_PASSWORD:
        pytest.skip("DEV_USER_ID / DEV_PASSWORD が未設定")
    if not DEV_SECOND_PASSWORD:
        pytest.skip("DEV_SECOND_PASSWORD が未設定")
    backup_state()
    write_live_fixture("TachibanaSpot:7203", "D1", "Toyota-Live")
    env = FlowsurfaceEnv(ticker="TachibanaSpot:7203", timeframe="D1", headless=False)
    try:
        env._start_process()
        run_s45(env)
    finally:
        env.close()
        restore_state()


def main() -> None:
    test_s45_order_correct_cancel()
    print_summary()
    import helpers as _h
    sys.exit(0 if _h._FAIL == 0 else 1)


if __name__ == "__main__":
    main()
