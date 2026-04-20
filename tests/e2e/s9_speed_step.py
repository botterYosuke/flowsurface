#!/usr/bin/env python3
"""s9_speed_step.py — スイート S9: 再生速度・Step 精度

検証シナリオ:
  TC-S9-01a〜b: Speed サイクル順序（1x→2x→5x→10x→1x）
  TC-S9-02: 5x 速度で 1〜500 bar 前進
  TC-S9-03a〜b: Playing 中 StepForward → Paused・End 近傍到達
  TC-S9-04: StepBackward 連続 5 回 → 単調減少

使い方:
    E2E_TICKER=BinanceLinear:BTCUSDT python tests/s9_speed_step.py
    IS_HEADLESS=true python tests/s9_speed_step.py
    pytest tests/s9_speed_step.py -v
"""

from __future__ import annotations

import sys
import time
from pathlib import Path

sys.path.insert(0, str(Path(__file__).parent))
from helpers import (
    IS_HEADLESS,
    STEP_M1,
    TICKER,
    api_get,
    api_post,
    backup_state,
    get_status,
    headless_play,
    pass_,
    fail,
    pend,
    print_summary,
    restore_state,
    setup_single_pane,
    utc_offset,
    utc_to_ms,
    wait_for_time_advance,
    wait_status,
)

_REPO_ROOT = Path(__file__).parent.parent
try:
    from flowsurface import FlowsurfaceEnv
except ImportError:
    sys.path.insert(0, str(_REPO_ROOT / "python"))
    from env import FlowsurfaceEnv  # type: ignore[no-redef]


def run_s9(end: str) -> None:
    print("=== S9: 再生速度・Step 精度 ===")

    # ── TC-S9-01: Speed サイクルの順序 (1x→2x→5x→10x→1x) ─────────────────
    init_status = get_status()
    init_speed = init_status.get("speed")
    if init_speed == "1x":
        pass_("TC-S9-01a: 初期 speed=1x")
    else:
        fail("TC-S9-01a", f"speed={init_speed}")

    for expected in ("2x", "5x", "10x", "1x"):
        speed_res = api_post("/api/replay/speed")
        speed = speed_res.get("speed")
        if speed == expected:
            pass_(f"TC-S9-01b: speed cycle → {speed}")
        else:
            fail("TC-S9-01b", f"expected={expected} got={speed}")

    # ── TC-S9-02: 5x 速度で wall delay が概ね 200ms/bar ────────────────────
    api_post("/api/replay/pause")
    # 現在 1x → 2x → 5x
    api_post("/api/replay/speed")  # 2x
    api_post("/api/replay/speed")  # 5x
    sp = get_status().get("speed")
    if sp != "5x":
        fail("TC-S9-02-precond", f"speed={sp} (expected 5x)")

    ct_init = int(get_status().get("current_time") or 0)
    api_post("/api/replay/resume")
    ct_tick = wait_for_time_advance(ct_init, 30)
    if ct_tick is not None:
        api_post("/api/replay/pause")
        wait_status("Paused", 10)
        ct_end = int(get_status().get("current_time") or 0)
        delta = ct_end - ct_init
        bars = delta // STEP_M1
        if 1 <= bars <= 500:
            pass_(f"TC-S9-02: 5x で {bars} bar 前進")
        else:
            fail("TC-S9-02", f"{bars} bar (expected 1-500, delta={delta})")
    else:
        api_post("/api/replay/pause")
        fail("TC-S9-02", f"30 秒待機しても current_time が前進しなかった (CT_INIT={ct_init})")

    # ── TC-S9-03: Playing 中の StepForward は End まで一気に進んで Paused ───
    api_post("/api/replay/resume")
    time.sleep(0.3)
    api_post("/api/replay/step-forward")
    time.sleep(0.3)
    status_after = get_status().get("status")
    ct_after = int(get_status().get("current_time") or 0)
    end_time_ms = utc_to_ms(end)

    if status_after == "Paused":
        pass_("TC-S9-03a: Playing 中 StepForward → Paused")
    else:
        fail("TC-S9-03a", f"status={status_after} (expected Paused)")

    # End 近傍 = end_time_ms - 120000 以上
    if ct_after >= end_time_ms - 120_000:
        pass_(f"TC-S9-03b: Playing 中 StepForward → End 近傍到達 (ct={ct_after})")
    else:
        fail("TC-S9-03b", f"ct={ct_after} not near end={end_time_ms}")

    # ── TC-S9-04: StepBackward を連続 5 回 → 単調減少 ───────────────────────
    api_post("/api/replay/pause")
    for _ in range(5):
        api_post("/api/replay/step-forward")
        time.sleep(0.3)

    times: list[int] = []
    for _ in range(5):
        t = int(get_status().get("current_time") or 0)
        times.append(t)
        api_post("/api/replay/step-backward")
        time.sleep(0.3)

    monotone = all(times[i] > times[i + 1] for i in range(len(times) - 1))
    if monotone:
        pass_("TC-S9-04: StepBackward 連続 5 回 単調減少")
    else:
        fail("TC-S9-04", f"単調減少でない times={times}")


def test_s9_speed_step() -> None:
    """pytest エントリポイント。プロセス起動は外部で行うこと。"""
    import helpers
    helpers._PASS = helpers._FAIL = helpers._PEND = 0
    end = utc_offset(-1)
    run_s9(end)
    print_summary()
    assert helpers._FAIL == 0, f"{helpers._FAIL} TC(s) failed — see output above"


def main() -> None:
    start = utc_offset(-3)
    end = utc_offset(-1)

    backup_state()
    setup_single_pane(TICKER, "M1", start, end)

    env = FlowsurfaceEnv(ticker=TICKER, timeframe="M1", headless=IS_HEADLESS)
    try:
        env._start_process()
        headless_play()

        if not wait_status("Playing", 30):
            fail("TC-S9-precond", "auto-play で Playing に到達せず")
            restore_state()
            print_summary()
            sys.exit(1)

        run_s9(end)
    finally:
        env.close()
        restore_state()
        print_summary()

    import helpers
    if helpers._FAIL > 0:
        sys.exit(1)


if __name__ == "__main__":
    main()
