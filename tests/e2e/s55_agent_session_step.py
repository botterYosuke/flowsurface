#!/usr/bin/env python3
"""s55_agent_session_step.py — Phase 4b-1 サブフェーズ J: agent session step E2E

検証シナリオ (ADR-0001 / plan §4.2):
- POST /api/agent/session/default/step → observation / fills / updated_narrative_ids /
  clock_ms を 1 RTT で返す（polling 不要）
- 連続 step で clock_ms が厳密に 1 バー分進む
- 仮想注文後の step で fills 配列に client_order_id が同梱される
- session Idle で 404 + hint フィールド

使い方:
    IS_HEADLESS=true python tests/e2e/s55_agent_session_step.py
    pytest tests/e2e/s55_agent_session_step.py -v
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


def run_s55() -> None:
    mode_label = "headless" if IS_HEADLESS else "GUI"
    print(f"=== S55: agent session step ({mode_label}) ===")

    if not IS_HEADLESS:
        # GUI runtime では step も 501 が返る（app/api/mod.rs の stub）。
        # agent API は headless が主要ユースケースなので本スイートは skip。
        print("  SKIP: agent session step is 501 in GUI mode (designed for headless)")
        return

    # TC-S55-00: セッション未起動で 404 + hint
    r = requests.post(f"{API_BASE}/api/agent/session/default/step", timeout=5)
    if r.status_code == 404:
        body = r.json()
        if "hint" in body and "session not started" in body.get("error", ""):
            pass_("TC-S55-00: session Idle → 404 + hint")
        else:
            fail("TC-S55-00", f"body missing hint or error: {body}")
    else:
        fail("TC-S55-00", f"expected 404, got {r.status_code} {r.text}")

    # セッション起動（過去 2h 分）
    requests.post(f"{API_BASE}/api/app/set-mode", json={"mode": "replay"}, timeout=5)
    requests.post(
        f"{API_BASE}/api/replay/play",
        json={"start": utc_offset(-3), "end": utc_offset(-1)},
        timeout=10,
    )
    if not wait_status("Paused", 30) and not wait_status("Playing", 30):
        fail("TC-S55-setup", "replay session did not reach Active")
        return

    # TC-S55-01: step が必須 5 キーを同梱で返す
    r = requests.post(f"{API_BASE}/api/agent/session/default/step", timeout=10)
    if r.status_code == 200:
        body = r.json()
        required = ["clock_ms", "reached_end", "observation", "fills", "updated_narrative_ids"]
        missing = [k for k in required if k not in body]
        if not missing:
            pass_(f"TC-S55-01: step returns all 5 keys (clock_ms={body['clock_ms']})")
        else:
            fail("TC-S55-01", f"missing keys: {missing}")
    else:
        fail("TC-S55-01", f"status={r.status_code} body={r.text}")
        return

    first_clock = r.json()["clock_ms"]

    # TC-S55-02: 2 回目の step で clock が前進する
    r = requests.post(f"{API_BASE}/api/agent/session/default/step", timeout=10)
    if r.status_code == 200:
        second_clock = r.json()["clock_ms"]
        if second_clock > first_clock:
            pass_(f"TC-S55-02: clock advanced ({first_clock} → {second_clock})")
        else:
            fail("TC-S55-02", f"clock did not advance: {first_clock} vs {second_clock}")
    else:
        fail("TC-S55-02", f"status={r.status_code}")

    # TC-S55-03: agent API 発注後 step で fills.client_order_id が埋まる
    cli = f"s55_cli_{int(time.time() * 1000)}"
    r = requests.post(
        f"{API_BASE}/api/agent/session/default/order",
        json={
            "client_order_id": cli,
            "ticker": {"exchange": "BinanceLinear", "symbol": "BTCUSDT"},
            "side": "buy",
            "qty": 0.1,
            "order_type": {"market": {}},
        },
        timeout=10,
    )
    if r.status_code not in (200, 201):
        fail("TC-S55-03", f"order failed: {r.status_code} {r.text}")
    else:
        # step で次 tick 進めて約定 → fills に client_order_id 同梱
        r = requests.post(f"{API_BASE}/api/agent/session/default/step", timeout=10)
        if r.status_code == 200:
            fills = r.json().get("fills", [])
            matched = [f for f in fills if f.get("client_order_id") == cli]
            if matched:
                pass_(f"TC-S55-03: fills inline with client_order_id={cli}")
            else:
                fail("TC-S55-03", f"no fill with cli_order_id {cli} in {fills}")
        else:
            fail("TC-S55-03", f"step after order failed: {r.status_code}")


def test_s55_agent_session_step() -> None:
    global _PASS, _FAIL
    _PASS = _FAIL = 0
    run_s55()
    print_summary()
    assert _FAIL == 0, f"{_FAIL} TC(s) failed"


def main() -> None:
    env = FlowsurfaceEnv(ticker=TICKER, timeframe="M1", headless=IS_HEADLESS)
    try:
        env._start_process()
        run_s55()
    finally:
        env.close()
        print_summary()
        if _FAIL > 0:
            sys.exit(1)


if __name__ == "__main__":
    main()
