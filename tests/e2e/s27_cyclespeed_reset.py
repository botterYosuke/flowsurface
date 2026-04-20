#!/usr/bin/env python3
"""s27_cyclespeed_reset.py — S27: CycleSpeed は速度のみ変更する（停止・シーク副作用なし）

検証シナリオ（仕様 R4-3-2「CycleSpeed 副作用除去」）:
  TC-A: Playing 中に CycleSpeed (1x→2x) → status=Playing のまま・speed=2x・current_time 前進維持
  TC-B: Playing 中に CycleSpeed (2x→5x) → status=Playing のまま・speed=5x
  TC-C: Playing 中に CycleSpeed (5x→10x) → status=Playing のまま・speed=10x
  TC-D: Playing 中に CycleSpeed (10x→1x ラップ) → status=Playing のまま・speed=1x
  TC-E: Pause 後に CycleSpeed (1x→2x) → status=Paused のまま・speed=2x
  TC-F: Paused 状態から Resume → Playing 到達

仕様根拠:
  docs/replay_header.md §8.1 — R4-3-2「CycleSpeed 副作用除去」
  旧仕様: CycleSpeed は pause + seek(range.start) を伴っていた
  新仕様: CycleSpeed は speed ラベルのサイクルのみ。status・current_time に影響しない

tests/archive/s27_cyclespeed_reset.sh の Python 版。

使い方:
    uv run tests/s27_cyclespeed_reset.py
    IS_HEADLESS=true uv run tests/s27_cyclespeed_reset.py
    pytest tests/s27_cyclespeed_reset.py -v
"""

from __future__ import annotations

import sys
import time
from pathlib import Path

sys.path.insert(0, str(Path(__file__).parent))
from helpers import *


def run_s27() -> None:
    start = utc_offset(-9)
    end = utc_offset(-1)
    start_ms = utc_to_ms(start)
    mode_label = "headless" if IS_HEADLESS else "GUI"

    print(f"=== S27: CycleSpeed は速度のみ変更する（停止・シーク副作用なし）({mode_label}) ===")
    print(f"  range: {start} → {end} (start_ms={start_ms})")

    setup_single_pane(TICKER, "M1", start, end)
    headless_play()

    # Playing に到達するまで待機（最大 60 秒）
    if not wait_status("Playing", 60):
        fail("precond", "auto-play で Playing に到達せず")
        print_summary()
        raise SystemExit(1)
    print("  Playing 到達")

    # 前準備: current_time が start_time より十分進んでいることを確認する
    print()
    print("── 前準備: current_time が start_time より前進するまで待機 (最大 15s)")
    ct_advanced = False
    for _ in range(15):
        ct_val = get_status().get("current_time")
        if ct_val is not None:
            ct = int(ct_val)
            if ct > start_ms + 60000:
                ct_advanced = True
                print(f"  current_time 前進確認: {ct} (start_ms={start_ms})")
                break
        time.sleep(1)

    if not ct_advanced:
        print("  WARN: current_time が 15s で十分に前進しなかった")

    # ── TC-A: Playing 中に CycleSpeed (1x→2x) → Playing のまま ─────────────────
    print()
    print("── TC-A: Playing 中に CycleSpeed (1x→2x) → Playing のまま")

    ct_before_a_val = get_status().get("current_time")
    ct_before_a = int(ct_before_a_val) if ct_before_a_val is not None else 0
    print(f"  CycleSpeed 前 current_time={ct_before_a} status=Playing")

    resp_a = api_post("/api/replay/speed")
    speed_a = resp_a.get("speed")
    status_a = resp_a.get("status")
    ct_a_val = resp_a.get("current_time")
    ct_a = int(ct_a_val) if ct_a_val is not None else 0
    print(f"  CycleSpeed 後: status={status_a} speed={speed_a} current_time={ct_a}")

    # TC-A1: status=Playing のまま（停止しない）
    if status_a == "Playing":
        pass_("TC-A1: CycleSpeed 後 status=Playing（停止なし）")
    else:
        fail("TC-A1", f"status={status_a} (expected Playing — CycleSpeed が意図せず停止)")

    # TC-A2: speed=2x
    if speed_a == "2x":
        pass_("TC-A2: CycleSpeed 後 speed=2x")
    else:
        fail("TC-A2", f"speed={speed_a} (expected 2x)")

    # TC-A3: current_time が start_time より前進したまま（range.start にリセットされない）
    if ct_a_val is not None:
        if ct_a > start_ms + 60000:
            pass_(f"TC-A3: current_time={ct_a} は start_time にリセットされていない")
        else:
            fail("TC-A3", f"current_time={ct_a} は start_time={start_ms} から 1 bar 以内（不正リセット）")
    else:
        fail("TC-A3", "current_time が null")

    # ── TC-B: Playing 中に CycleSpeed (2x→5x) → Playing のまま ─────────────────
    print()
    print("── TC-B: CycleSpeed (2x→5x) → Playing のまま")

    resp_b = api_post("/api/replay/speed")
    speed_b = resp_b.get("speed")
    status_b = resp_b.get("status")
    print(f"  CycleSpeed 後: status={status_b} speed={speed_b}")

    if status_b == "Playing":
        pass_("TC-B1: CycleSpeed (2x→5x) 後 status=Playing")
    else:
        fail("TC-B1", f"status={status_b} (expected Playing)")

    if speed_b == "5x":
        pass_("TC-B2: speed=5x")
    else:
        fail("TC-B2", f"speed={speed_b} (expected 5x)")

    # ── TC-C: Playing 中に CycleSpeed (5x→10x) → Playing のまま ────────────────
    print()
    print("── TC-C: CycleSpeed (5x→10x) → Playing のまま")

    resp_c = api_post("/api/replay/speed")
    speed_c = resp_c.get("speed")
    status_c = resp_c.get("status")
    print(f"  CycleSpeed 後: status={status_c} speed={speed_c}")

    if status_c == "Playing":
        pass_("TC-C1: CycleSpeed (5x→10x) 後 status=Playing")
    else:
        fail("TC-C1", f"status={status_c} (expected Playing)")

    if speed_c == "10x":
        pass_("TC-C2: speed=10x")
    else:
        fail("TC-C2", f"speed={speed_c} (expected 10x)")

    # ── TC-D: Playing 中に CycleSpeed (10x→1x ラップ) → Playing のまま ──────────
    print()
    print("── TC-D: CycleSpeed (10x→1x ラップ) → Playing のまま")

    resp_d = api_post("/api/replay/speed")
    speed_d = resp_d.get("speed")
    status_d = resp_d.get("status")
    print(f"  CycleSpeed 後: status={status_d} speed={speed_d}")

    if status_d == "Playing":
        pass_("TC-D1: CycleSpeed (10x→1x) 後 status=Playing")
    else:
        fail("TC-D1", f"status={status_d} (expected Playing)")

    if speed_d == "1x":
        pass_("TC-D2: speed=1x（ラップ確認）")
    else:
        fail("TC-D2", f"speed={speed_d} (expected 1x)")

    # ── TC-E: Paused 中に CycleSpeed → Paused のまま ────────────────────────────
    print()
    print("── TC-E: Paused 中に CycleSpeed → Paused のまま")

    api_post("/api/replay/pause")
    if not wait_status("Paused", 10):
        fail("TC-E-precond", "Paused に遷移せず")
    else:
        resp_e = api_post("/api/replay/speed")
        speed_e = resp_e.get("speed")
        status_e = resp_e.get("status")
        print(f"  CycleSpeed 後: status={status_e} speed={speed_e}")

        if status_e == "Paused":
            pass_("TC-E1: Paused 中 CycleSpeed 後 status=Paused のまま")
        else:
            fail("TC-E1", f"status={status_e} (expected Paused)")

        if speed_e == "2x":
            pass_("TC-E2: Paused 中 CycleSpeed 後 speed=2x")
        else:
            fail("TC-E2", f"speed={speed_e} (expected 2x)")

    # ── TC-F: Paused → Resume → Playing ─────────────────────────────────────────
    print()
    print("── TC-F: Paused → Resume → Playing")

    api_post("/api/replay/resume")
    if wait_status("Playing", 30):
        pass_("TC-F: Resume 後 status=Playing")
    else:
        current_st = get_status().get("status")
        fail("TC-F", f"status={current_st} (expected Playing)")


def test_s27_cyclespeed_reset() -> None:
    """pytest から呼ばれる場合のエントリポイント。プロセス起動は外部で行うこと。"""
    import helpers as _h
    _h._PASS = _h._FAIL = _h._PEND = 0
    run_s27()
    print_summary()
    assert _h._FAIL == 0, f"{_h._FAIL} TC(s) failed — see output above"


def main() -> None:
    backup_state()

    start = utc_offset(-9)
    end = utc_offset(-1)
    setup_single_pane(TICKER, "M1", start, end)

    env = FlowsurfaceEnv(ticker=TICKER, timeframe="M1", headless=IS_HEADLESS)
    try:
        env._start_process()
        run_s27()
    finally:
        env.close()
        restore_state()
        print_summary()
        import helpers as _h
        if _h._FAIL > 0:
            sys.exit(1)


if __name__ == "__main__":
    main()
