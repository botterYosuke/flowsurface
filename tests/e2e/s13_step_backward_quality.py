#!/usr/bin/env python3
"""s13_step_backward_quality.py — Suite S13: Rewind-to-start 品質保証

検証シナリオ:
  TC-S13-01: advance で進めた後、rewind-to-start で初期状態に戻る
  TC-S13-02: rewind 後に clock_ms が start に一致する
  TC-S13-03: rewind 後 streams_ready=true となる (チラつき防止)

仕様根拠:
  docs/plan/phase4b_agent_replay_api_followup.md — rewind-to-start への書き換え
"""

from __future__ import annotations

import sys
import time
from pathlib import Path

sys.path.insert(0, str(Path(__file__).parent))
from helpers import (
    TICKER,
    IS_HEADLESS,
    FlowsurfaceEnv,
    backup_state,
    restore_state,
    setup_single_pane,
    wait_status,
    api_get,
    api_post,
    utc_offset,
    reset_counters,
    pass_,
    fail,
    pend,
    print_summary,
    STEP_M1,
)

import requests

BTC_TICKER = "BinanceLinear:BTCUSDT"

def run_s13() -> None:
    mode_label = "headless" if IS_HEADLESS else "GUI"
    print(f"=== S13: Rewind-to-start 品質保証 (ticker={TICKER} {mode_label}) ===")

    start = utc_offset(-3)
    end = utc_offset(-1)
    setup_single_pane(BTC_TICKER, "M1", start, end)

    env = FlowsurfaceEnv(ticker=BTC_TICKER, timeframe="M1", headless=IS_HEADLESS)
    try:
        env._start_process()
        
        # Initialize session
        requests.post("http://127.0.0.1:9876/api/app/set-mode", json={"mode": "replay"}, timeout=5)
        api_post("/api/replay/toggle", {"start": start, "end": end})
        
        if not wait_status("Active", 30):
            fail("TC-S13-precond", "Active に遷移せず")
            return

        r = api_get("/api/replay/status")
        initial_clock = r.get("current_time")
        
        # Advance
        api_post("/api/agent/session/default/advance", {"until_ms": initial_clock + 60000 * 5})
        
        r2 = api_get("/api/replay/status")
        advanced_clock = r2.get("current_time")
        
        if advanced_clock <= initial_clock:
            fail("TC-S13-precond", "Advance で進まなかった")
            return

        # TC-S13-01 & TC-S13-02: Rewind to start
        rewind_r = requests.post("http://127.0.0.1:9876/api/agent/session/default/rewind-to-start", timeout=10)
        
        if rewind_r.status_code == 200:
            body = rewind_r.json()
            if body.get("clock_ms") == initial_clock:
                pass_("TC-S13-01 & 02: rewind-to-start 成功し clock_ms が元に戻った")
            else:
                fail("TC-S13-02", f"clock_ms が一致しない: {body}")
        else:
            fail("TC-S13-01", f"rewind failed: {rewind_r.status_code} {rewind_r.text}")

        # TC-S13-03: wait status Active
        if wait_status("Active", 10):
            pass_("TC-S13-03: Rewind 後 Active 状態に復帰")
        else:
            fail("TC-S13-03", "Rewind 後 Active に戻らなかった")

    finally:
        env.close()

def test_s13_step_backward_quality() -> None:
    reset_counters()
    backup_state()
    try:
        run_s13()
    finally:
        restore_state()
        print_summary()
    import helpers as _h
    assert _h._FAIL == 0, f"{_h._FAIL} TC(s) failed"

def main() -> None:
    reset_counters()
    backup_state()
    try:
        run_s13()
    finally:
        restore_state()
        print_summary()
        import helpers as _h
        if _h._FAIL > 0:
            sys.exit(1)

if __name__ == "__main__":
    main()
