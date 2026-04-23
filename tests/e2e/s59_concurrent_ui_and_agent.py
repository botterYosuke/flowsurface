#!/usr/bin/env python3
"""s59_concurrent_ui_and_agent.py — Phase 4b-1 サブフェーズ J: UI + agent 混在 E2E

ADR-0001 §Risks の「2 系統 API の同一 VirtualExchange への同時アクセス」回帰検知。

検証シナリオ:
- TC-S59-01: UI `/api/replay/play` → agent `/step` → agent `/api/agent/session/default/observation`
  または `/step` 直後に UI `GET /api/replay/state` の clock が一致
  (本 Phase では agent /observation エンドポイントは未実装のため step レスポンスで代用)
- TC-S59-02: UI `/api/replay/play` で session が再起動されると agent の client_order_id map が
  自動クリアされる（ADR-0001 SessionLifecycleEvent 購読の検証）
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


def run_s59() -> None:
    mode_label = "headless" if IS_HEADLESS else "GUI"
    print(f"=== S59: concurrent UI + agent ({mode_label}) ===")

    if not IS_HEADLESS:
        print("  SKIP: agent API is 501 in GUI mode, concurrent test needs headless")
        return

    requests.post(f"{API_BASE}/api/app/set-mode", json={"mode": "replay"}, timeout=5)
    r = requests.post(
        f"{API_BASE}/api/replay/toggle",
        json={"start": utc_offset(-3), "end": utc_offset(-1)},
        timeout=10,
    )
    if not wait_status("Active", 30):
        fail("TC-S59-setup", "replay session did not reach Active")
        return

    # TC-S59-01: agent /step 直後に UI /api/replay/state を取得 → clock 一致
    r = requests.post(f"{API_BASE}/api/agent/session/default/step", timeout=10)
    if r.status_code != 200:
        fail("TC-S59-01", f"step failed: {r.status_code}")
    else:
        agent_clock = r.json()["clock_ms"]
        r = requests.get(f"{API_BASE}/api/replay/state", timeout=5)
        if r.status_code == 200:
            ui_clock = r.json().get("current_time_ms")
            if ui_clock == agent_clock:
                pass_(f"TC-S59-01: UI と agent の clock 一致 ({ui_clock})")
            else:
                fail("TC-S59-01", f"clock mismatch: agent={agent_clock} UI={ui_clock}")
        else:
            fail("TC-S59-01", f"UI state failed: {r.status_code}")

    # TC-S59-02: UI /play 再実行で agent client_order_id map がクリアされる
    # （同じ cli で qty 違いが 201 / 200 で通る = conflict にならない ことを検証）。
    cli = f"s59_cli_{int(time.time() * 1000)}"
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
        fail("TC-S59-02-setup", f"first order failed: {r.status_code}")
        return

    # UI 経由で session を再起動 → SessionLifecycleEvent::Started 発火
    r = requests.post(
        f"{API_BASE}/api/replay/toggle",
        json={"start": utc_offset(-3), "end": utc_offset(-1)},
        timeout=10,
    )
    if r.status_code != 200:
        fail("TC-S59-02-setup", f"replay play failed: {r.status_code}")
        return
    if not wait_status("Active", 30):
        fail("TC-S59-02-setup", "session did not re-activate")
        return

    # 同じ cli + qty 違い → 新セッションなのでマップは空、Created (201 or 200) を期待。
    r = requests.post(
        f"{API_BASE}/api/agent/session/default/order",
        json={
            "client_order_id": cli,
            "ticker": {"exchange": "BinanceLinear", "symbol": "BTCUSDT"},
            "side": "buy",
            "qty": 0.2,  # 異なる
            "order_type": {"market": {}},
        },
        timeout=10,
    )
    if r.status_code in (200, 201) and not r.json().get("idempotent_replay"):
        pass_("TC-S59-02: UI /play が agent map をクリア（同 cli + 異 qty が 201）")
    elif r.status_code == 409:
        fail(
            "TC-S59-02",
            "UI /play 後に agent map がクリアされず 409 conflict（SessionLifecycleEvent 未発火）",
        )
    else:
        fail("TC-S59-02", f"unexpected: {r.status_code} {r.text}")


def test_s59_concurrent_ui_and_agent() -> None:
    global _PASS, _FAIL
    _PASS = _FAIL = 0
    run_s59()
    print_summary()
    assert _FAIL == 0, f"{_FAIL} TC(s) failed"


def main() -> None:
    env = FlowsurfaceEnv(ticker=TICKER, timeframe="M1", headless=IS_HEADLESS)
    try:
        env._start_process()
        run_s59()
    finally:
        env.close()
        print_summary()
        if _FAIL > 0:
            sys.exit(1)


if __name__ == "__main__":
    main()
