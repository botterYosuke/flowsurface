#!/usr/bin/env python3
"""s44_order_list.py — 注文一覧取得 E2E テスト（シナリオ 2-1）

検証シナリオ:
  1a: Live モード確認
  1b: Tachibana デモセッション確立
  2:  GET /api/tachibana/orders → HTTP 200
  3:  レスポンスが {"orders":[...]} 形式の JSON
  3b: 注文件数ログ
  4:  GET /api/tachibana/orders?eig_day=TODAY → HTTP 200
  5:  GET /api/tachibana/order/00000000 → HTTP 200（クラッシュなし）

前提: DEV_IS_DEMO=true / DEV_USER_ID / DEV_PASSWORD が設定済み
"""
from __future__ import annotations

import os
import sys
import time
from datetime import datetime
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


def run_s44(env) -> None:
    print("=== 注文一覧取得 E2E テスト ===")

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

    # ── Step 2: GET /api/tachibana/orders → HTTP 200 ──────────────────────────
    print()
    print("── Step 2: GET /api/tachibana/orders (全件)")
    try:
        r = requests.get(f"{API_BASE}/api/tachibana/orders", timeout=5)
        if r.status_code == 200:
            pass_("Step 2: GET /api/tachibana/orders → HTTP 200")
        else:
            fail("Step 2", f"HTTP={r.status_code} (expected 200)")
    except requests.RequestException as e:
        fail("Step 2", f"リクエスト失敗: {e}")

    # ── Step 3: レスポンス形式確認（セッションエラー時はリトライ）────────────
    print()
    print("── Step 3: レスポンス JSON 形式確認")
    resp: dict = {}
    for i in range(5):
        try:
            r = requests.get(f"{API_BASE}/api/tachibana/orders", timeout=5)
            resp = r.json()
            if not resp.get("error"):
                break
        except requests.RequestException:
            pass
        print(f"  API エラー (セッション切断など) — 3s 待機後リトライ ({i + 1}/5)...")
        time.sleep(3)

    print(f"  response: {resp}")
    if isinstance(resp.get("orders"), list):
        pass_("Step 3: orders フィールドが配列であることを確認")
    else:
        fail("Step 3", f"orders フィールドが配列でない: {resp}")

    order_count = len(resp.get("orders", []))
    print(f"  注文件数: {order_count}")
    pass_(f"Step 3b: 注文件数確認 ({order_count} 件)")

    # ── Step 4: eig_day クエリパラメータ付き ─────────────────────────────────
    print()
    print("── Step 4: GET /api/tachibana/orders?eig_day=今日")
    today = datetime.now().strftime("%Y%m%d")
    try:
        r = requests.get(
            f"{API_BASE}/api/tachibana/orders",
            params={"eig_day": today},
            timeout=5,
        )
        if r.status_code == 200:
            pass_(f"Step 4: GET /api/tachibana/orders?eig_day={today} → HTTP 200")
        else:
            fail("Step 4", f"HTTP={r.status_code} (expected 200)")
    except requests.RequestException as e:
        fail("Step 4", f"リクエスト失敗: {e}")

    # ── Step 5: 存在しない注文番号の明細取得 ─────────────────────────────────
    print()
    print("── Step 5: GET /api/tachibana/order/00000000 (存在しない注文番号)")
    try:
        r = requests.get(f"{API_BASE}/api/tachibana/order/00000000", timeout=5)
        print(f"  response: {r.text}")
        if r.status_code == 200:
            pass_(f"Step 5: GET /api/tachibana/order/00000000 → HTTP {r.status_code}（正常応答）")
        else:
            fail("Step 5", f"HTTP={r.status_code} (expected 200)")
    except requests.RequestException as e:
        fail("Step 5", f"リクエスト失敗: {e}")


def test_s44_order_list() -> None:
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
        run_s44(env)
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
    test_s44_order_list()
    import helpers as _h
    sys.exit(0 if _h._FAIL == 0 else 1)


if __name__ == "__main__":
    main()
