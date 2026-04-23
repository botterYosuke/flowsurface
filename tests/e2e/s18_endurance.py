#!/usr/bin/env python3
"""s18_endurance.py — スイート S18: 耐久テスト

検証シナリオ:
  TC-S18-01: 2h range を 10x 速度で完走 → Paused 到達（最大 900s 待機）
  TC-S18-02-fwd/bwd: StepForward × 500 + StepBackward × 500 → crash なし・status=Paused
  TC-S18-03: Playing 中 split→close × 20 サイクル → Playing 維持

警告: 完走に 15〜30 分かかる

使い方:
    E2E_TICKER=BinanceLinear:BTCUSDT python tests/s18_endurance.py
    IS_HEADLESS=true python tests/s18_endurance.py
    pytest tests/s18_endurance.py -v
"""

from __future__ import annotations

import sys
import time
from pathlib import Path

sys.path.insert(0, str(Path(__file__).parent))
from helpers import (
    TICKER, IS_HEADLESS, FlowsurfaceEnv,
    pass_, fail, pend, print_summary,
    backup_state, restore_state,
    setup_single_pane, headless_play, speed_to_10x,
    get_status, wait_status, wait_playing,
    wait_for_pane_count,
    api_post, api_post_code, api_get, api_get_code,
    utc_offset,
    get_pane_id, find_other_pane_id,
)

import requests
import helpers as _h


def is_alive() -> bool:
    return api_get_code("/api/replay/status") not in (0,)


def run_s18() -> None:
    print(f"=== S18: 耐久テスト (ticker={TICKER}) ===")
    print("  警告: このスクリプトは完走に 15〜30 分かかる")

    # ── TC-S18-01: 2 時間 range を 10x 速度で完走 → Paused ─────────────────
    print("  [TC-S18-01] 10x 速度 2h range 完走テスト...")
    start_long = utc_offset(-4)
    end_long = utc_offset(-2)
    setup_single_pane(TICKER, "M1", start_long, end_long)

    env1 = FlowsurfaceEnv(ticker=TICKER, timeframe="M1", headless=IS_HEADLESS)
    try:
        env1._start_process()
        headless_play()

        if not wait_playing(30):
            fail("TC-S18-01-pre", "Playing 到達せず")
        else:
            speed_to_10x()
            print("  10x 速度で再生中（最大 900 秒待機）...")
            if wait_status("Paused", 900):
                pass_("TC-S18-01: 2h range 10x 完走 → Paused 到達")
            else:
                try:
                    status = get_status().get("status")
                except requests.RequestException:
                    status = "unknown"
                fail("TC-S18-01", f"900 秒経過後も status={status}（Paused 未到達）")
    finally:
        env1.close()

    # ── TC-S18-02: Step 1000 回（各方向 500 回）→ crash なし ───────────────
    print("  [TC-S18-02] Step 1000 回耐久テスト...")
    # 広い range を使って StepForward 500 回分の空間を確保
    start_wide = utc_offset(-12)
    end_wide = utc_offset(-1)
    setup_single_pane(TICKER, "M1", start_wide, end_wide)

    env2 = FlowsurfaceEnv(ticker=TICKER, timeframe="M1", headless=IS_HEADLESS)
    try:
        env2._start_process()
        headless_play()

        if not wait_playing(30):
            fail("TC-S18-02-pre", "Playing 到達せず")
        else:
            try:
            except requests.RequestException:
                pass

            if not wait_status("Paused", 10):
                fail("TC-S18-02-pre", "Paused に遷移せず")
            else:
                print("  StepForward × 500...")
                fwd_crash = False
                for i in range(1, 501):
                    try:
                        api_post("/api/replay/step-forward")
                    except requests.RequestException:
                        pass
                    time.sleep(0.3)
                    if api_get_code("/api/replay/status") == 0:
                        fwd_crash = True
                        print(f"  CRASH detected at forward step #{i}")
                        break
                    if i % 100 == 0:
                        print(f"    forward step {i}/500...")

                if fwd_crash:
                    fail("TC-S18-02-fwd", "StepForward 連打中にアプリがクラッシュした")
                else:
                    wait_status("Paused", 15)
                    try:
                        status = get_status().get("status")
                    except requests.RequestException:
                        status = "unknown"
                    if status == "Paused":
                        pass_("TC-S18-02-fwd: StepForward 500 回完了 → status=Paused")
                    else:
                        fail("TC-S18-02-fwd", f"status={status} (Paused 期待)")

                print("  StepBackward × 500...")
                bwd_crash = False
                for i in range(1, 501):
                    try:
                    except requests.RequestException:
                        pass
                    time.sleep(0.3)
                    if api_get_code("/api/replay/status") == 0:
                        bwd_crash = True
                        print(f"  CRASH detected at backward step #{i}")
                        break
                    if i % 100 == 0:
                        print(f"    backward step {i}/500...")

                if bwd_crash:
                    fail("TC-S18-02-bwd", "StepBackward 連打中にアプリがクラッシュした")
                else:
                    wait_status("Paused", 15)
                    try:
                        status = get_status().get("status")
                    except requests.RequestException:
                        status = "unknown"
                    if status == "Paused":
                        pass_("TC-S18-02-bwd: StepBackward 500 回完了 → status=Paused")
                    else:
                        fail("TC-S18-02-bwd", f"status={status} (Paused 期待)")
    finally:
        env2.close()

    # ── TC-S18-03: Playing 中 split→close × 20 サイクル ───────────────────
    print("  [TC-S18-03] Playing 中 split→close × 20 サイクル...")
    # 6h range (360 bars) を使用して終端到達前に 20 サイクル完了できる範囲を確保
    setup_single_pane(TICKER, "M1", utc_offset(-7), utc_offset(-1))

    env3 = FlowsurfaceEnv(ticker=TICKER, timeframe="M1", headless=IS_HEADLESS)
    try:
        env3._start_process()
        headless_play()

        if not wait_playing(30):
            fail("TC-S18-03-pre", "Playing 到達せず")
        else:
            crud_fail = False
            for i in range(1, 21):
                # 初期ペイン ID 取得
                pane0 = get_pane_id(0)
                if not pane0:
                    fail(f"TC-S18-03-{i}", "ペイン ID 取得失敗")
                    crud_fail = True
                    break

                # split
                api_post_code("/api/pane/split", {"pane_id": pane0, "axis": "Vertical"})
                if not wait_for_pane_count(2, 10):
                    fail(f"TC-S18-03-{i}", "split 後ペイン数が 2 にならなかった")
                    crud_fail = True
                    break

                # 新ペイン ID 取得
                new_pane = find_other_pane_id(pane0)
                if not new_pane:
                    fail(f"TC-S18-03-{i}", "新ペイン ID 取得失敗")
                    crud_fail = True
                    break

                # close
                api_post_code("/api/pane/close", {"pane_id": new_pane})
                if not wait_for_pane_count(1, 10):
                    fail(f"TC-S18-03-{i}", "close 後ペイン数が 1 にならなかった")
                    crud_fail = True
                    break

                # Playing 維持確認（5 サイクルごと）
                if i % 5 == 0:
                    try:
                        status = get_status().get("status")
                    except requests.RequestException:
                        status = "unknown"
                    if status != "Playing":
                        fail(f"TC-S18-03-{i}", f"CRUD サイクル {i} 回後 status={status} (Playing 期待)")
                        crud_fail = True
                        break
                    print(f"    cycle {i}/20: status=Playing OK")

            if not crud_fail:
                try:
                    status = get_status().get("status")
                except requests.RequestException:
                    status = "unknown"
                if status == "Playing":
                    pass_("TC-S18-03: CRUD 20 サイクル完了 → status=Playing 維持")
                else:
                    fail("TC-S18-03", f"20 サイクル後 status={status} (Playing 期待)")
    finally:
        env3.close()


def test_s18_endurance() -> None:
    """pytest エントリポイント。プロセス起動は外部で行うこと。"""
    _h._PASS = _h._FAIL = _h._PEND = 0
    run_s18()
    print_summary()
    assert _h._FAIL == 0, f"{_h._FAIL} TC(s) failed — see output above"


def main() -> None:
    backup_state()
    try:
        run_s18()
    finally:
        restore_state()
        print_summary()
        if _h._FAIL > 0:
            sys.exit(1)


if __name__ == "__main__":
    main()
