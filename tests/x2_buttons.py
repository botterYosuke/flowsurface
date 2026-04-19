#!/usr/bin/env python3
"""x2_buttons.py — 横断スイート X2: ボタンの厳密挙動

tests/archive/x2_buttons.sh の検証ロジックを flowsurface-sdk の
FlowsurfaceEnv でプロセス管理しながら再実装したもの。

使い方:
    uv run tests/x2_buttons.py
    IS_HEADLESS=true uv run tests/x2_buttons.py
    pytest tests/x2_buttons.py -v
"""

from __future__ import annotations

import sys
import time
from pathlib import Path

sys.path.insert(0, str(Path(__file__).parent))
from helpers import *


def run_x2() -> None:
    start = utc_offset(-3)
    end = utc_offset(-1)
    mode_label = "headless" if IS_HEADLESS else "GUI"

    print(f"=== X2: ボタンの厳密挙動 (ticker={TICKER} {mode_label}) ===")

    setup_single_pane(TICKER, "M1", start, end)
    headless_play()

    if not wait_playing(60):
        fail("X2-precond", "Playing 到達せず")
        print_summary()
        raise SystemExit(1)

    api_post("/api/replay/pause")
    wait_paused(5)

    # --- TC-X2-01: StepForward x5 = +300000ms ---
    pre_val = get_status().get("current_time")
    pre = int(pre_val) if pre_val is not None else 0
    for _ in range(5):
        api_post("/api/replay/step-forward")
        time.sleep(0.2)
    post_val = get_status().get("current_time")
    post = int(post_val) if post_val is not None else 0
    diff = post - pre
    if diff == 300000:
        pass_("TC-X2-01: StepForward x5 = +300000ms")
    else:
        fail("TC-X2-01", f"diff={diff} (expected 300000)")

    # --- TC-X2-02: StepBackward x5 で完全可逆 ---
    for _ in range(5):
        api_post("/api/replay/step-backward")
        time.sleep(0.2)
    back_val = get_status().get("current_time")
    back = int(back_val) if back_val is not None else 0
    if back == pre:
        pass_(f"TC-X2-02: 可逆 (back={back})")
    else:
        fail("TC-X2-02", f"back={back} pre={pre}")

    # --- TC-X2-03: start 端での StepBackward は no-op ---
    st_t_val = get_status().get("start_time")
    st_t = int(st_t_val) if st_t_val is not None else 0
    for _ in range(200):
        ct_val = get_status().get("current_time")
        ct = int(ct_val) if ct_val is not None else 0
        if ct == st_t:
            break
        api_post("/api/replay/step-backward")
        time.sleep(0.05)

    at_start_val = get_status().get("current_time")
    at_start = int(at_start_val) if at_start_val is not None else 0
    api_post("/api/replay/step-backward")
    time.sleep(0.5)
    beyond_val = get_status().get("current_time")
    beyond = int(beyond_val) if beyond_val is not None else 0
    if at_start == beyond:
        pass_("TC-X2-03: start 端 StepBackward は no-op")
    else:
        fail("TC-X2-03", f"AT_START={at_start} BEYOND={beyond}")

    # --- TC-X2-04: Pause 冪等性 ---
    api_post("/api/replay/pause")
    s1 = get_status()
    st1 = s1.get("status")
    ct1_val = s1.get("current_time")
    ct1 = int(ct1_val) if ct1_val is not None else 0
    api_post("/api/replay/pause")
    s2 = get_status()
    st2 = s2.get("status")
    ct2_val = s2.get("current_time")
    ct2 = int(ct2_val) if ct2_val is not None else 0
    if st1 == st2 and ct1 == ct2:
        pass_("TC-X2-04: Pause 冪等")
    else:
        fail("TC-X2-04", f"ST={st1}→{st2} CT={ct1}→{ct2}")

    # --- TC-X2-05: Resume → Pause → Resume の往復で current_time の継続性 ---
    api_post("/api/replay/resume")
    time.sleep(1)
    pre_r_val = get_status().get("current_time")
    pre_r = int(pre_r_val) if pre_r_val is not None else 0
    api_post("/api/replay/pause")
    time.sleep(1)
    paused_at_val = get_status().get("current_time")
    paused_at = int(paused_at_val) if paused_at_val is not None else 0
    if paused_at >= pre_r:
        pass_("TC-X2-05a: Pause 後の時刻 >= Pause 前")
    else:
        fail("TC-X2-05a", f"PAUSED_AT={paused_at} PRE_R={pre_r}")
    api_post("/api/replay/resume")
    time.sleep(1)
    resumed_val = get_status().get("current_time")
    resumed = int(resumed_val) if resumed_val is not None else 0
    if resumed >= paused_at:
        pass_("TC-X2-05b: Resume 後 >= Pause 時刻")
    else:
        fail("TC-X2-05b", f"RESUMED={resumed} PAUSED_AT={paused_at}")

    # --- TC-X2-06: Speed サイクル一周 + speed 値の永続 ---
    api_post("/api/replay/pause")
    # 1x にリセット
    for _ in range(5):
        sp = get_status().get("speed")
        if sp == "1x":
            break
        api_post("/api/replay/speed")
    expected_speeds = ["2x", "5x", "10x", "1x"]
    all_ok = True
    for e in expected_speeds:
        res = api_post("/api/replay/speed")
        got = res.get("speed")
        if got != e:
            all_ok = False
            print(f"  cycle break: expected={e} got={got}")
    if all_ok:
        pass_("TC-X2-06: Speed cycle 1→2→5→10→1")
    else:
        fail("TC-X2-06", "cycle 異常")

    # --- TC-X2-07: CycleSpeed は current_time を変更しない ---
    wait_paused(5)
    api_post("/api/replay/step-forward")
    time.sleep(0.3)
    api_post("/api/replay/step-forward")
    time.sleep(0.3)
    st_now = get_status()
    pre_ct_val = st_now.get("current_time")
    pre_ct = int(pre_ct_val) if pre_ct_val is not None else 0
    start_t_val = st_now.get("start_time")
    start_t = int(start_t_val) if start_t_val is not None else 0
    if pre_ct <= start_t:
        fail("TC-X2-07-pre", f"pre-condition: not ahead of start (pre={pre_ct} start={start_t})")
    api_post("/api/replay/speed")
    post_ct_val = get_status().get("current_time")
    post_ct = int(post_ct_val) if post_ct_val is not None else 0
    if post_ct == pre_ct:
        pass_(f"TC-X2-07: CycleSpeed 後も current_time は不変 (pre={pre_ct} post={post_ct})")
    else:
        fail("TC-X2-07", f"current_time が変化した (pre={pre_ct} → post={post_ct})")

    # --- TC-X2-08: Live 中はボタンが意味を持たない ---
    if IS_HEADLESS:
        pend("TC-X2-08", "headless は Live モードなし")
    else:
        api_post("/api/replay/toggle")  # → Live
        live_before = get_status()
        api_post("/api/replay/step-forward")
        api_post("/api/replay/pause")
        api_post("/api/replay/resume")
        live_after = get_status()
        b_mode = live_before.get("mode")
        a_mode = live_after.get("mode")
        b_ct = live_before.get("current_time")
        a_ct = live_after.get("current_time")
        if a_mode == "Live" and b_mode == "Live" and b_ct is None and a_ct is None:
            pass_("TC-X2-08: Live 中ボタン操作は no-op")
        else:
            fail("TC-X2-08", f"mode={b_mode}→{a_mode} ct={b_ct}→{a_ct}")


def test_x2_buttons() -> None:
    """pytest から呼ばれる場合のエントリポイント。プロセス起動は外部で行うこと。"""
    import helpers as _h
    _h._PASS = _h._FAIL = _h._PEND = 0
    run_x2()
    print_summary()
    assert _h._FAIL == 0, f"{_h._FAIL} TC(s) failed — see output above"


def main() -> None:
    backup_state()
    env = FlowsurfaceEnv(ticker=TICKER, timeframe="M1", headless=IS_HEADLESS)
    try:
        env._start_process()
        run_x2()
    finally:
        env.close()
        restore_state()
        print_summary()
        import helpers as _h
        if _h._FAIL > 0:
            sys.exit(1)


if __name__ == "__main__":
    main()
