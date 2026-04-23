#!/usr/bin/env python3
"""s56_agent_session_order.py — Phase 4b-1 サブフェーズ J: agent session order E2E

検証シナリオ (ADR-0001 / plan §3.3, §4.4, §4.5):
- 新規 place_order → 200/201 + order_id + idempotent_replay=false
- 同一 client_order_id + 同一 body 再送 → 200 + idempotent_replay=true + 既存 order_id
- 同一 client_order_id + body 相違 → 409 Conflict
- string ticker → 400（silent normalization 再発防止）
- order_type 省略 → 400（silent market default 再発防止）
- client_order_id 不正文字 → 400
"""
from __future__ import annotations

import os
import sys
import time
from datetime import datetime, timedelta, timezone
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


def utc_offset(hours: int) -> str:
    dt = datetime.now(timezone.utc) + timedelta(hours=hours)
    return dt.strftime("%Y-%m-%d %H:%M")


def wait_status(want: str, timeout: int = 60) -> bool:
    deadline = time.monotonic() + timeout
    while time.monotonic() < deadline:
        try:
            r = requests.get(f"{API_BASE}/api/replay/status", timeout=5)
            if r.status_code == 200 and r.json().get("status") == want:
                return True
        except requests.RequestException:
            pass
        time.sleep(0.5)
    return False


def _valid_body(cli: str, qty: float = 0.1) -> dict:
    return {
        "client_order_id": cli,
        "ticker": {"exchange": "BinanceLinear", "symbol": "BTCUSDT"},
        "side": "buy",
        "qty": qty,
        "order_type": {"market": {}},
    }


def run_s56() -> None:
    mode_label = "headless" if IS_HEADLESS else "GUI"
    print(f"=== S56: agent session order ({mode_label}) ===")

    if not IS_HEADLESS:
        print("  SKIP: agent session order is 501 in GUI mode")
        return

    requests.post(f"{API_BASE}/api/app/set-mode", json={"mode": "replay"}, timeout=5)
    requests.post(
        f"{API_BASE}/api/replay/toggle",
        json={"start": utc_offset(-3), "end": utc_offset(-1)},
        timeout=10,
    )
    if not wait_status("Active", 30):
        fail("TC-S56-setup", "replay session did not reach Active")
        return

    # TC-S56-01: 新規 order → 2xx + idempotent_replay=false
    cli = f"s56_cli_new_{int(time.time() * 1000)}"
    r = requests.post(
        f"{API_BASE}/api/agent/session/default/order",
        json=_valid_body(cli),
        timeout=10,
    )
    if r.status_code in (200, 201):
        body = r.json()
        if not body.get("idempotent_replay") and body.get("order_id"):
            pass_(f"TC-S56-01: new order accepted (order_id={body['order_id'][:8]}...)")
        else:
            fail("TC-S56-01", f"unexpected body: {body}")
    else:
        fail("TC-S56-01", f"status={r.status_code} body={r.text}")
        return

    first_order_id = r.json()["order_id"]

    # TC-S56-02: 同一 cli + 同一 body → 200 + idempotent_replay=true + 既存 order_id
    r = requests.post(
        f"{API_BASE}/api/agent/session/default/order",
        json=_valid_body(cli),
        timeout=10,
    )
    if r.status_code == 200:
        body = r.json()
        if body.get("idempotent_replay") is True and body.get("order_id") == first_order_id:
            pass_("TC-S56-02: idempotent_replay=true で既存 order_id を返す")
        else:
            fail("TC-S56-02", f"replay={body.get('idempotent_replay')} order_id={body.get('order_id')}")
    else:
        fail("TC-S56-02", f"status={r.status_code}")

    # TC-S56-03: 同一 cli + body 相違 (qty=0.2) → 409 Conflict
    r = requests.post(
        f"{API_BASE}/api/agent/session/default/order",
        json=_valid_body(cli, qty=0.2),
        timeout=10,
    )
    if r.status_code == 409:
        if "conflict" in r.text.lower():
            pass_("TC-S56-03: 409 Conflict on different body")
        else:
            fail("TC-S56-03", f"409 但し body not conflict: {r.text}")
    else:
        fail("TC-S56-03", f"expected 409, got {r.status_code}")

    # TC-S56-04: string ticker → 400（silent normalization 防止）
    r = requests.post(
        f"{API_BASE}/api/agent/session/default/order",
        json={
            "client_order_id": f"s56_cli_str_{int(time.time() * 1000)}",
            "ticker": "BinanceLinear:BTCUSDT",
            "side": "buy",
            "qty": 0.1,
            "order_type": {"market": {}},
        },
        timeout=10,
    )
    if r.status_code == 400 and "ticker" in r.text.lower():
        pass_("TC-S56-04: string ticker → 400 with ticker keyword")
    else:
        fail("TC-S56-04", f"expected 400 with 'ticker' keyword, got {r.status_code} {r.text}")

    # TC-S56-05: order_type 省略 → 400（silent market default 防止）
    r = requests.post(
        f"{API_BASE}/api/agent/session/default/order",
        json={
            "client_order_id": f"s56_cli_ot_{int(time.time() * 1000)}",
            "ticker": {"exchange": "BinanceLinear", "symbol": "BTCUSDT"},
            "side": "buy",
            "qty": 0.1,
        },
        timeout=10,
    )
    if r.status_code == 400 and "order_type" in r.text.lower():
        pass_("TC-S56-05: missing order_type → 400 with order_type keyword")
    else:
        fail("TC-S56-05", f"expected 400 with 'order_type' keyword, got {r.status_code} {r.text}")

    # TC-S56-06: client_order_id 不正文字 (space) → 400
    r = requests.post(
        f"{API_BASE}/api/agent/session/default/order",
        json={
            "client_order_id": "cli with space",
            "ticker": {"exchange": "BinanceLinear", "symbol": "BTCUSDT"},
            "side": "buy",
            "qty": 0.1,
            "order_type": {"market": {}},
        },
        timeout=10,
    )
    if r.status_code == 400 and "client_order_id" in r.text.lower():
        pass_("TC-S56-06: invalid client_order_id charset → 400")
    else:
        fail("TC-S56-06", f"expected 400 with 'client_order_id' keyword, got {r.status_code} {r.text}")


def test_s56_agent_session_order() -> None:
    global _PASS, _FAIL
    _PASS = _FAIL = 0
    run_s56()
    print_summary()
    assert _FAIL == 0, f"{_FAIL} TC(s) failed"


def main() -> None:
    env = FlowsurfaceEnv(ticker=TICKER, timeframe="M1", headless=IS_HEADLESS)
    try:
        env._start_process()
        run_s56()
    finally:
        env.close()
        print_summary()
        if _FAIL > 0:
            sys.exit(1)


if __name__ == "__main__":
    main()
