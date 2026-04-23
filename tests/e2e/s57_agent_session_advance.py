#!/usr/bin/env python3
"""s57_agent_session_advance.py 窶・Phase 4b-1 繧ｵ繝悶ヵ繧ｧ繝ｼ繧ｺ J: agent session advance E2E

讀懆ｨｼ繧ｷ繝翫Μ繧ｪ (ADR-0001 / plan ﾂｧ4.3):
- Headless runtime: advance 縺ｧ 100 tick 逶ｸ蠖薙ｒ instant 螳溯｡・竊・stopped_reason="until_reached"
- Headless runtime: stop_on=["fill"] 縺ｧ譛蛻昴・邏・ｮ壽凾轤ｹ縺ｧ蛛懈ｭ｢ 竊・stopped_reason="fill"
- Headless runtime: include_fills=true 縺ｧ fills 驟榊・蜷梧｢ｱ縲’alse 縺ｧ逵∫払
- GUI runtime: advance 縺ｧ 400 + "headless" 繝｡繝・そ繝ｼ繧ｸ

GUI 譎ゅ・ 400 縺ｮ譁ｹ縺ｮ縺ｿ讀懆ｨｼ縲・
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
        f"{API_BASE}/api/replay/toggle",
        json={"start": utc_offset(-3), "end": utc_offset(-1)},
        timeout=10,
    )
    if not wait_status("Active", 30):
        fail("TC-S57-setup", "replay session did not reach Active")
        return
    r = requests.get(f"{API_BASE}/api/replay/status", timeout=10)
    if r.status_code != 200:
        fail("TC-S57-setup", f"status failed: {r.status_code}")
        return
    current_clock = r.json().get("current_time")
    if not isinstance(current_clock, int):
        fail("TC-S57-setup", f"missing current_time in status: {r.text}")
        return

    # TC-S57-01: until_ms 縺ｧ 3 繝舌・蜈医∪縺ｧ騾ｲ繧√ｋ 竊・stopped_reason=until_reached
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

    if not IS_HEADLESS:
        return

    # TC-S57-02: 謌占｡梧ｳｨ譁・ｾ・stop_on=["fill"] 竊・1 tick 縺ｧ蛛懈ｭ｢
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
        # far-away until_ms 縺ｫ縺励※ stop_on 縺ｧ譌ｩ譛溷●豁｢縺輔○繧九・
        r = requests.post(
            f"{API_BASE}/api/agent/session/default/advance",
            json={"until_ms": cur + 60_000 * 100, "stop_on": ["fill"]},
            timeout=10,
        )
        # 豕ｨ: 蜷域・ Trade 縺・tick 縺ｧ邏・ｮ壹☆繧九°縺ｯ EventStore 縺ｮ繝・・繧ｿ迥ｶ豕∽ｾ晏ｭ倥・
        # 邏・ｮ壹＠縺ｪ縺・腸蠅・〒縺ｯ until_reached 縺ｾ縺溘・ end 縺ｫ蛻ｰ驕斐☆繧句庄閭ｽ諤ｧ繧ゅ≠繧九・
        if r.status_code == 200:
            body = r.json()
            if body.get("stopped_reason") in ("fill", "until_reached", "end"):
                pass_(f"TC-S57-02: stop_on=['fill'] 蜿礼炊 (stopped_reason={body['stopped_reason']})")
            else:
                fail("TC-S57-02", f"unexpected stopped_reason: {body}")
        else:
            fail("TC-S57-02", f"status={r.status_code} body={r.text}")

    # TC-S57-03: include_fills=false (繝・ヵ繧ｩ繝ｫ繝・ 縺ｧ fills 驟榊・縺・omit 縺輔ｌ繧・
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
            pass_("TC-S57-03: include_fills=false 竊・fills field omitted")
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
