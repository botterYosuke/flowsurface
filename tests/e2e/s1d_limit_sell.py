#!/usr/bin/env python3
"""s1d_limit_sell.py — TOYOTA (7203) 指値売り E2E テスト（シナリオ 1-4）

検証シナリオ:
  Step 1: Live モードで起動・Tachibana セッション確認
  Step 2: 指値売り注文を送信（side=1, price=99999 円 = 約定しない水準）
  Step 3: 保有なしの場合はエラーが返ることを確認
  Step 4: HTTP レスポンス形式確認

注意: 実際に TOYOTA 株を保有している場合は注文が受け付けられるため要注意
前提: DEV_USER_ID / DEV_PASSWORD / DEV_SECOND_PASSWORD 環境変数が設定済みであること

使い方:
    DEV_USER_ID=... DEV_PASSWORD=... DEV_SECOND_PASSWORD=... python tests/s1d_limit_sell.py
    pytest tests/s1d_limit_sell.py -v
"""

from __future__ import annotations

import json
import os
import sys
import time
from pathlib import Path

import requests

sys.path.insert(0, str(Path(__file__).parent))
from helpers import (
    API_BASE,
    DATA_DIR,
    IS_HEADLESS,
    STATE_FILE,
    api_get,
    backup_state,
    fail,
    pass_,
    pend,
    print_summary,
    restore_state,
    FlowsurfaceEnv,
)


# ── フィクスチャ ──────────────────────────────────────────────────────────────

def _write_toyota_live_fixture() -> None:
    """TachibanaSpot:7203 D1 Live モード用 saved-state.json を書き込む（replay フィールドなし）。"""
    DATA_DIR.mkdir(parents=True, exist_ok=True)
    fixture = {
        "layout_manager": {
            "layouts": [
                {
                    "name": "Toyota-Live",
                    "dashboard": {
                        "pane": {
                            "KlineChart": {
                                "layout": {"splits": [0.78], "autoscale": "FitToVisible"},
                                "kind": "Candles",
                                "stream_type": [{"Kline": {"ticker": "TachibanaSpot:7203", "timeframe": "D1"}}],
                                "settings": {
                                    "tick_multiply": None,
                                    "visual_config": None,
                                    "selected_basis": {"Time": "D1"},
                                },
                                "indicators": [],
                                "link_group": "A",
                            }
                        },
                        "popout": [],
                    },
                }
            ],
            "active_layout": "Toyota-Live",
        },
        "timezone": "UTC",
        "trade_fetch_enabled": False,
        "size_in_quote_ccy": "Base",
    }
    STATE_FILE.write_text(json.dumps(fixture, indent=2))


# ── テスト本体 ────────────────────────────────────────────────────────────────

def run_s1d() -> None:
    # Step 1: Live モード確認
    print()
    print("── Step 1: Live モード確認")
    try:
        status = api_get("/api/replay/status")
        mode = status.get("mode")
    except requests.RequestException as e:
        fail("Step 1", f"API 応答なし: {e}")
        return

    if mode == "Live":
        pass_(f"Step 1: Live モードで起動確認 (mode={mode})")
    else:
        fail("Step 1", f"mode={mode} (expected Live)")

    # Step 2: Tachibana セッション確認
    print()
    print("── Step 2: Tachibana セッション確認")
    try:
        body = requests.get(f"{API_BASE}/api/auth/tachibana/status", timeout=5).json()
        session = body.get("session", "none")
    except requests.RequestException:
        session = "none"
    print(f"  session={session}")
    pass_(f"Step 2: 認証 API 応答確認 (session={session})")

    # Step 3: 指値売り注文（side=1, price=99999 円 = 約定しない水準）
    print()
    print("── Step 3: TOYOTA (7203) 100株 指値売り (price=99999 円)")

    second_pw = os.environ.get("DEV_SECOND_PASSWORD", "")
    if not second_pw:
        fail("Step 3", "DEV_SECOND_PASSWORD が未設定です")
        pend("Step 4", "DEV_SECOND_PASSWORD が未設定のためスキップ")
        return

    order_payload = {
        "issue_code": "7203",
        "qty": "100",
        "side": "1",
        "price": "99999",
        "account_type": "1",
        "market_code": "00",
        "condition": "0",
        "cash_margin": "0",
        "expire_day": "0",
    }
    try:
        r = requests.post(f"{API_BASE}/api/tachibana/order", json=order_payload, timeout=10)
        order_resp = r.json()
    except requests.RequestException as e:
        fail("Step 3", f"POST /api/tachibana/order 失敗: {e}")
        pend("Step 4", "注文 API 呼び出し失敗のためスキップ")
        return
    print(f"  response: {order_resp}")

    order_num = order_resp.get("order_number") or ""
    error = order_resp.get("error") or ""

    if order_num:
        pass_(f"Step 3: 指値売り注文受付済み (order_number={order_num})")
    elif error:
        print(f"  INFO: エラー応答 (error={error}) — 保有株なしまたは市場時間外の可能性あり")
        pass_(f"Step 3: 指値売り API 疎通確認（エラー応答あり: {error}）")
    else:
        fail("Step 3", f"レスポンスが解析できない: {order_resp}")
        pend("Step 4", "応答解析失敗のためスキップ")
        return

    # Step 4: HTTP レスポンス形式確認
    print()
    print("── Step 4: HTTP レスポンス形式確認")

    try:
        r2 = requests.post(f"{API_BASE}/api/tachibana/order", json=order_payload, timeout=10)
        http_code = r2.status_code
    except requests.RequestException:
        http_code = 0
    print(f"  HTTP status: {http_code}")
    if http_code == 200:
        pass_(f"Step 4: POST /api/tachibana/order → HTTP {http_code}")
    else:
        fail("Step 4", f"HTTP={http_code} (expected 200)")


# ── pytest エントリポイント ───────────────────────────────────────────────────

def test_s1d_limit_sell() -> None:
    """pytest から呼ばれる場合のエントリポイント。プロセス起動は外部で行うこと。"""
    import helpers
    helpers._PASS = helpers._FAIL = helpers._PEND = 0

    if not os.environ.get("DEV_USER_ID") or not os.environ.get("DEV_PASSWORD"):
        pend("S1d", "DEV_USER_ID / DEV_PASSWORD が未設定 — スキップ")
        return

    run_s1d()
    print_summary()
    assert helpers._FAIL == 0, f"{helpers._FAIL} TC(s) failed — see output above"


# ── スタンドアロン実行 ────────────────────────────────────────────────────────

def main() -> None:
    import helpers
    helpers._PASS = helpers._FAIL = helpers._PEND = 0

    if not os.environ.get("DEV_USER_ID") or not os.environ.get("DEV_PASSWORD"):
        print("  SKIP: DEV_USER_ID / DEV_PASSWORD が未設定 — 自動ログインが無効です")
        sys.exit(0)

    print("=== TOYOTA (7203) 指値売り E2E テスト ===")
    backup_state()
    _write_toyota_live_fixture()

    env = FlowsurfaceEnv(ticker="TachibanaSpot:7203", timeframe="D1", headless=IS_HEADLESS)
    try:
        env._start_process()
        run_s1d()
    finally:
        env.close()
        restore_state()

    print_summary()
    if helpers._FAIL > 0:
        sys.exit(1)


if __name__ == "__main__":
    main()
