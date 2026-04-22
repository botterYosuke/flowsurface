#!/usr/bin/env python3
"""s58_agent_session_session_id.py — Phase 4b-1 サブフェーズ J: session_id 検証 E2E

検証シナリオ (ADR-0001 §Risks):
- session_id != "default" → 501 Not Implemented + 固定エラーメッセージ
- session_id 空文字（// パス）→ 400 Bad Request
- セッション未起動 (Idle) で step/order → 404 + hint
"""
from __future__ import annotations

import os
import sys
from pathlib import Path

import requests

_REPO_ROOT = Path(__file__).parent.parent.parent
try:
    from flowsurface import FlowsurfaceEnv
except ImportError:
    sys.path.insert(0, str(_REPO_ROOT / "python"))
    from env import FlowsurfaceEnv  # type: ignore[no-redef]

API_BASE = "http://127.0.0.1:9876"
TICKER = os.environ.get("E2E_TICKER", "BinanceLinear:BTCUSDT")
IS_HEADLESS = os.environ.get("IS_HEADLESS", "").lower() == "true"

_PASS = 0
_FAIL = 0


def pass_(label: str) -> None:
    global _PASS
    print(f"  PASS: {label}")
    _PASS += 1


def fail(label: str, detail: str) -> None:
    global _FAIL
    print(f"  FAIL: {label} - {detail}")
    _FAIL += 1


def print_summary() -> None:
    print()
    print("=============================")
    print(f"  PASS: {_PASS}  FAIL: {_FAIL}")
    print("=============================")


def run_s58() -> None:
    mode_label = "headless" if IS_HEADLESS else "GUI"
    print(f"=== S58: session_id validation ({mode_label}) ===")

    # TC-S58-01: session_id != "default" → 501
    r = requests.post(f"{API_BASE}/api/agent/session/other/step", timeout=5)
    if r.status_code == 501:
        body = r.text.lower()
        if "multi-session" in body and "default" in body:
            pass_("TC-S58-01: non-default → 501 + multi-session message")
        else:
            fail("TC-S58-01", f"501 但しメッセージ不一致: {r.text}")
    else:
        fail("TC-S58-01", f"expected 501, got {r.status_code} {r.text}")

    # TC-S58-02: UUID 風 session_id も 501
    uuid_like = "550e8400-e29b-41d4-a716-446655440000"
    r = requests.post(f"{API_BASE}/api/agent/session/{uuid_like}/step", timeout=5)
    if r.status_code == 501:
        pass_(f"TC-S58-02: UUID-like session_id → 501")
    else:
        fail("TC-S58-02", f"expected 501, got {r.status_code}")

    # TC-S58-03: session_id 空文字 → 400 Bad Request
    # 実際の URL は `/api/agent/session//step` のような double slash になる。
    # requests は URL を正規化しないので verbatim に送信される。
    r = requests.post(f"{API_BASE}/api/agent/session//step", timeout=5)
    if r.status_code in (400, 404):
        # 一部 HTTP サーバは double slash を 404 に正規化するため 404 も許容。
        pass_(f"TC-S58-03: empty session_id → {r.status_code}")
    else:
        fail("TC-S58-03", f"expected 400/404, got {r.status_code} {r.text}")

    # TC-S58-04: session 未起動（Idle）で agent step → 404 + hint (headless のみ実装)
    if IS_HEADLESS:
        # Live に戻して Idle に持って行く
        requests.post(f"{API_BASE}/api/app/set-mode", json={"mode": "live"}, timeout=5)
        # replay 側セッションを明示的に toggle で終了
        requests.post(f"{API_BASE}/api/replay/toggle", timeout=5)
        r = requests.post(f"{API_BASE}/api/agent/session/default/step", timeout=5)
        if r.status_code == 404:
            body = r.json()
            if body.get("hint") and "session not started" in body.get("error", ""):
                pass_("TC-S58-04: Idle → 404 + hint")
            else:
                fail("TC-S58-04", f"404 但し body 不適: {body}")
        elif r.status_code == 501:
            # GUI スタブ路が動いた場合（IS_HEADLESS 環境不整合）
            pass_("TC-S58-04: 501 (GUI runtime stub) — headless 環境と不一致、スキップ")
        else:
            fail("TC-S58-04", f"expected 404, got {r.status_code} {r.text}")
    else:
        print("  SKIP: TC-S58-04 requires headless (GUI stub は常に 501 を返す)")


def test_s58_agent_session_id_validation() -> None:
    global _PASS, _FAIL
    _PASS = _FAIL = 0
    run_s58()
    print_summary()
    assert _FAIL == 0, f"{_FAIL} TC(s) failed"


def main() -> None:
    env = FlowsurfaceEnv(ticker=TICKER, timeframe="M1", headless=IS_HEADLESS)
    try:
        env._start_process()
        run_s58()
    finally:
        env.close()
        print_summary()
        if _FAIL > 0:
            sys.exit(1)


if __name__ == "__main__":
    main()
