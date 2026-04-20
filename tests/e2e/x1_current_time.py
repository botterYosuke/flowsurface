#!/usr/bin/env python3
"""x1_current_time.py — 横断スイート X1: current_time 表示の不変条件

tests/archive/x1_current_time.sh の検証ロジックを flowsurface-sdk の
FlowsurfaceEnv でプロセス管理しながら再実装したもの。

使い方:
    uv run tests/x1_current_time.py
    IS_HEADLESS=true uv run tests/x1_current_time.py
    pytest tests/x1_current_time.py -v
"""

from __future__ import annotations

import sys
import time
from datetime import datetime, timezone
from pathlib import Path

sys.path.insert(0, str(Path(__file__).parent))
from helpers import *


def run_x1() -> None:
    start = utc_offset(-3)
    end = utc_offset(-1)
    mode_label = "headless" if IS_HEADLESS else "GUI"

    print(f"=== X1: current_time 表示の不変条件 (ticker={TICKER} {mode_label}) ===")

    setup_single_pane(TICKER, "M1", start, end)
    headless_play()

    if not wait_playing(60):
        fail("X1-precond", "Playing 到達せず")
        print_summary()
        raise SystemExit(1)

    # --- TC-X1-01: バー境界スナップ不変条件（10 サンプル）---
    all_on_bar = True
    for i in range(1, 11):
        ct_val = get_status().get("current_time")
        ct = int(ct_val) if ct_val is not None else 0
        on = is_bar_boundary(ct, STEP_M1)
        if not on:
            all_on_bar = False
            print(f"  off-bar at i={i} ct={ct}")
        time.sleep(0.5)

    if all_on_bar:
        pass_("TC-X1-01: 10 サンプル全てバー境界")
    else:
        fail("TC-X1-01", "バー境界違反あり")

    # --- TC-X1-02: current_time の単調非減少 ---
    prev = 0
    mono = True
    for i in range(1, 9):
        ct_val = get_status().get("current_time")
        ct = int(ct_val) if ct_val is not None else 0
        if ct < prev:
            mono = False
        prev = ct
        time.sleep(0.4)

    if mono:
        pass_("TC-X1-02: current_time 単調非減少")
    else:
        fail("TC-X1-02", "逆行あり")

    # --- TC-X1-03: range 内不変条件（連続サンプル）---
    st = get_status()
    st_t = int(st.get("start_time") or 0)
    et_t = int(st.get("end_time") or 0)
    all_in = True
    for i in range(1, 7):
        ct_val = get_status().get("current_time")
        ct = int(ct_val) if ct_val is not None else 0
        if not ct_in_range(ct, st_t, et_t):
            all_in = False
        time.sleep(0.5)

    if all_in:
        pass_("TC-X1-03: range 内不変")
    else:
        fail("TC-X1-03", "range 外")

    # --- TC-X1-04: [要 API 拡張] current_time_display と current_time の整合 ---
    status_now = get_status()
    display = status_now.get("current_time_display")
    if display is None:
        pend("TC-X1-04", "ReplayStatus.current_time_display 未実装")
    else:
        ct_val = status_now.get("current_time")
        ct = int(ct_val) if ct_val is not None else 0
        dt = datetime.utcfromtimestamp(ct / 1000)
        expect = dt.strftime("%Y-%m-%d %H:%M:%S")
        if display == expect:
            pass_(f"TC-X1-04: display={display} と current_time 整合")
        else:
            fail("TC-X1-04", f"display={display} expected={expect}")

    # --- TC-X1-05: [要 API 拡張] display も連続して進む ---
    d1 = get_status().get("current_time_display")
    if d1 is None:
        pend("TC-X1-05", "current_time_display 未実装")
    else:
        time.sleep(3)
        d2 = get_status().get("current_time_display")
        if d1 != d2:
            pass_(f"TC-X1-05: display が前進 ({d1} → {d2})")
        else:
            fail("TC-X1-05", f"display 固定 ({d1})")

    # --- TC-X1-06: Live モードで current_time / display が null（GUI のみ）---
    if IS_HEADLESS:
        pend("TC-X1-06a", "headless は Live モードなし")
        pend("TC-X1-06b", "headless は Live モードなし")
    else:
        api_post("/api/replay/toggle")  # → Live
        time.sleep(1)
        st_live = get_status()
        ct_live = st_live.get("current_time")
        sp_live = st_live.get("speed")
        if ct_live is None:
            pass_("TC-X1-06a: Live current_time=null")
        else:
            fail("TC-X1-06a", f"ct={ct_live}")
        if sp_live is None:
            pass_("TC-X1-06b: Live speed=null")
        else:
            fail("TC-X1-06b", f"speed={sp_live}")


def test_x1_current_time() -> None:
    """pytest から呼ばれる場合のエントリポイント。プロセス起動は外部で行うこと。"""
    import helpers as _h
    _h._PASS = _h._FAIL = _h._PEND = 0
    run_x1()
    print_summary()
    assert _h._FAIL == 0, f"{_h._FAIL} TC(s) failed — see output above"


def main() -> None:
    backup_state()
    env = FlowsurfaceEnv(ticker=TICKER, timeframe="M1", headless=IS_HEADLESS)
    try:
        env._start_process()
        run_x1()
    finally:
        env.close()
        restore_state()
        print_summary()
        import helpers as _h
        if _h._FAIL > 0:
            sys.exit(1)


if __name__ == "__main__":
    main()
