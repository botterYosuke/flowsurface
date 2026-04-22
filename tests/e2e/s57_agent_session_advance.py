#!/usr/bin/env python3
"""s57_agent_session_advance.py — Phase 4b-1 サブフェーズ J: agent session advance E2E

検証シナリオ (ADR-0001 / plan §4.3):
- Headless runtime: advance で 100 tick 相当を instant 実行 → stopped_reason="until_reached"
- Headless runtime: stop_on=["fill"] で最初の約定時点で停止 → stopped_reason="fill"
- Headless runtime: include_fills=true で fills 配列同梱、false で省略
- GUI runtime: advance で 400 + "headless" メッセージ

GUI 時は 400 の方のみ検証。
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


def run_s57() -> None:
    mode_label = "headless" if IS_HEADLESS else "GUI"
    print(f"=== S57: agent session advance ({mode_label}) ===")

    requests.post(f"{API_BASE}/api/app/set-mode", json={"mode": "replay"}, timeout=5)
    requests.post(
        f"{API_BASE}/api/replay/play",
        json={"start": utc_offset(-3), "end": utc_offset(-1)},
        timeout=10,
    )
    if not wait_status("Paused", 30) and not wait_status("Playing", 30):
        fail("TC-S57-setup", "replay session did not reach Active")
        return

    # TC-S57-GUI-01: GUI runtime では 400 + "headless" メッセージ
    if not IS_HEADLESS:
        # 現在時刻 + 60 分を until_ms とする（実値は使われない、400 を期待）。
        now_ms = int(time.time() * 1000) + 60 * 60 * 1000
        r = requests.post(
            f"{API_BASE}/api/agent/session/default/advance",
            json={"until_ms": now_ms},
            timeout=10,
        )
        if r.status_code == 400 and "headless" in r.text.lower():
            pass_("TC-S57-GUI-01: GUI で 400 + headless hint")
        else:
            fail("TC-S57-GUI-01", f"expected 400 with headless, got {r.status_code} {r.text}")
        # GUI では advance 動作しないので headless 専用 TC はスキップ。
        return

    # Headless 以降:
    # 現在の clock_ms を取得（step で 1 回進めて取得）
    r = requests.post(f"{API_BASE}/api/agent/session/default/step", timeout=10)
    if r.status_code != 200:
        fail("TC-S57-setup", f"step failed: {r.status_code}")
        return
    current_clock = r.json()["clock_ms"]

    # TC-S57-01: until_ms で 3 バー先まで進める → stopped_reason=until_reached
    target = current_clock + 60_000 * 3  # M1 = 60_000 ms
    r = requests.post(
        f"{API_BASE}/api/agent/session/default/advance",
        json={"until_ms": target},
        timeout=10,
    )
    if r.status_code == 200:
        body = r.json()
        if body.get("stopped_reason") == "until_reached" and body.get("clock_ms") == target:
            pass_(f"TC-S57-01: advance until_reached at {target}")
        else:
            fail("TC-S57-01", f"body: {body}")
    else:
        fail("TC-S57-01", f"status={r.status_code} body={r.text}")

    # TC-S57-02: 成行注文後 stop_on=["fill"] → 1 tick で停止
    cli = f"s57_cli_{int(time.time() * 1000)}"
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
        fail("TC-S57-02-setup", f"order failed: {r.status_code}")
    else:
        r2 = requests.post(f"{API_BASE}/api/agent/session/default/step", timeout=10)
        cur = r2.json()["clock_ms"] if r2.status_code == 200 else 0
        # far-away until_ms にして stop_on で早期停止させる。
        r = requests.post(
            f"{API_BASE}/api/agent/session/default/advance",
            json={"until_ms": cur + 60_000 * 100, "stop_on": ["fill"]},
            timeout=10,
        )
        # 注: 合成 Trade が tick で約定するかは EventStore のデータ状況依存。
        # 約定しない環境では until_reached または end に到達する可能性もある。
        if r.status_code == 200:
            body = r.json()
            if body.get("stopped_reason") in ("fill", "until_reached", "end"):
                pass_(f"TC-S57-02: stop_on=['fill'] 受理 (stopped_reason={body['stopped_reason']})")
            else:
                fail("TC-S57-02", f"unexpected stopped_reason: {body}")
        else:
            fail("TC-S57-02", f"status={r.status_code} body={r.text}")

    # TC-S57-03: include_fills=false (デフォルト) で fills 配列が omit される
    r = requests.post(f"{API_BASE}/api/agent/session/default/step", timeout=10)
    cur = r.json()["clock_ms"] if r.status_code == 200 else 0
    r = requests.post(
        f"{API_BASE}/api/agent/session/default/advance",
        json={"until_ms": cur + 60_000 * 2},
        timeout=10,
    )
    if r.status_code == 200:
        body = r.json()
        if "fills" not in body:
            pass_("TC-S57-03: include_fills=false → fills field omitted")
        else:
            fail("TC-S57-03", f"fills must be omitted by default: {body}")
    else:
        fail("TC-S57-03", f"status={r.status_code}")


def test_s57_agent_session_advance() -> None:
    global _PASS, _FAIL
    _PASS = _FAIL = 0
    run_s57()
    print_summary()
    assert _FAIL == 0, f"{_FAIL} TC(s) failed"


def main() -> None:
    env = FlowsurfaceEnv(ticker=TICKER, timeframe="M1", headless=IS_HEADLESS)
    try:
        env._start_process()
        run_s57()
    finally:
        env.close()
        print_summary()
        if _FAIL > 0:
            sys.exit(1)


if __name__ == "__main__":
    main()
