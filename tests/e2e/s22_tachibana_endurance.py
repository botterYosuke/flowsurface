#!/usr/bin/env python3
"""s22_tachibana_endurance.py — スイート S22: 耐久テスト（TachibanaSpot）

検証シナリオ:
  TC-S22-01: 長期 range (≈42 trading bars) を 10x 速度で完走 → Paused 到達
  TC-S22-02-fwd/bwd: StepForward × 50 + StepBackward × 50 → crash なし（D1 版）
  TC-S22-03: Playing 中 split→close × 20 サイクル → Playing 維持

仕様根拠:
  TachibanaSpot D1 での長時間再生・高速操作でのメモリリーク・デッドロック検証

警告: 完走に 15〜30 分かかる

前提条件: DEV_USER_ID / DEV_PASSWORD 環境変数設定済み

使い方:
    DEV_USER_ID=xxx DEV_PASSWORD=yyy python tests/s22_tachibana_endurance.py
    pytest tests/s22_tachibana_endurance.py -v
"""

from __future__ import annotations

import os
import sys
import time
from pathlib import Path

sys.path.insert(0, str(Path(__file__).parent))
from helpers import (
    FlowsurfaceEnv,
    pass_, fail, pend, print_summary,
    backup_state, restore_state,
    get_status, wait_status, wait_playing, wait_paused,
    wait_tachibana_session, wait_for_pane_streams_ready, wait_for_pane_count,
    api_get, api_post, api_get_code,
    get_pane_id, find_other_pane_id,
    tachibana_replay_setup,
    speed_to_10x,
    utc_offset,
    API_BASE,
)

import requests
import helpers as _h


def _tachibana_start(start: str, end: str) -> FlowsurfaceEnv:
    """saved-state を書き込み、FlowsurfaceEnv を起動。セッション確立・streams_ready・toggle+play まで行う。"""
    tachibana_replay_setup(start, end)
    env = FlowsurfaceEnv(ticker="TachibanaSpot:7203", timeframe="D1", headless=False)
    env._start_process()

    print("  waiting for Tachibana session (DEV AUTO-LOGIN)...")
    if not wait_tachibana_session(120):
        raise RuntimeError("Tachibana session not established after 120s")
    print("  Tachibana session established")

    pane_id = get_pane_id(0)
    if pane_id:
        print("  waiting for D1 klines (streams_ready)...")
        if not wait_for_pane_streams_ready(pane_id, 120):
            print("  WARN: streams_ready timeout (continuing)")

    api_post("/api/replay/toggle")
    api_post("/api/replay/toggle", {"start": start, "end": end})
    return env


def run_s22() -> None:
    print("=== S22: 耐久テスト（TachibanaSpot:7203 D1）===")
    print("  警告: このスクリプトは完走に 15〜30 分かかる")

    # 環境変数チェック
    if not os.environ.get("DEV_USER_ID") or not os.environ.get("DEV_PASSWORD"):
        pend("TC-S22-01", "DEV_USER_ID / DEV_PASSWORD が未設定 — Tachibana live テストをスキップ")
        pend("TC-S22-02-fwd", "DEV_USER_ID / DEV_PASSWORD が未設定")
        pend("TC-S22-02-bwd", "DEV_USER_ID / DEV_PASSWORD が未設定")
        pend("TC-S22-03", "DEV_USER_ID / DEV_PASSWORD が未設定")
        return

    # ── TC-S22-01: 60 日 range を 10x 速度で再生し終了 → Paused ───────────────
    # -1440h(-24h) ≈ 59 calendar days ≈ 42 trading bars
    print("  [TC-S22-01] 10x 速度 60 日 range 完走テスト...")
    try:
        env1 = _tachibana_start(utc_offset(-1440), utc_offset(-24))
    except RuntimeError as e:
        fail("TC-S22-01-pre", str(e))
        env1 = None

    if env1 is not None:
        try:
            if not wait_playing(60):
                fail("TC-S22-01-pre", "Playing 到達せず")
            else:
                speed_to_10x()
                print("  10x 速度で再生中（最大 180 秒待機）...")
                if wait_paused(180):
                    pass_("TC-S22-01: 60 日 range 10x 完走 → Paused 到達")
                else:
                    try:
                        status = get_status().get("status")
                    except requests.RequestException:
                        status = "unknown"
                    fail("TC-S22-01", f"180 秒経過後も status={status}（Paused 未到達）")
        finally:
            env1.close()

    # ── TC-S22-02: D1 Step 100 回（各方向 50 回）→ crash なし ────────────────
    # StepForward 50 回 = 50 × 86400000ms ≒ 50 日分の range が必要
    print("  [TC-S22-02] D1 Step 100 回耐久テスト（forward × 50 + backward × 50）...")
    try:
        env2 = _tachibana_start(utc_offset(-1300), utc_offset(-24))
    except RuntimeError as e:
        fail("TC-S22-02-pre", str(e))
        env2 = None

    if env2 is not None:
        try:
            if not wait_playing(60):
                fail("TC-S22-02-pre", "Playing 到達せず")
            else:
                try:
                except requests.RequestException:
                    pass

                if not wait_paused(15):
                    fail("TC-S22-02-pre", "Paused に遷移せず")
                else:
                    print("  StepForward × 50...")
                    crash_fwd = False
                    for i in range(50):
                        try:
                            api_post("/api/replay/step-forward")
                        except requests.RequestException:
                            pass
                        time.sleep(0.3)
                        if api_get_code("/api/replay/status") == 0:
                            crash_fwd = True
                            print(f"  CRASH detected at forward step #{i + 1}")
                            break
                        if (i + 1) % 10 == 0:
                            print(f"    forward step {i + 1}/50...")

                    if crash_fwd:
                        fail("TC-S22-02-fwd", "StepForward 連打中にアプリがクラッシュした")
                    else:
                        wait_paused(15)
                        try:
                            status = get_status().get("status")
                        except requests.RequestException:
                            status = "unknown"
                        if status == "Paused":
                            pass_("TC-S22-02-fwd: StepForward 50 回完了 → status=Paused")
                        else:
                            fail("TC-S22-02-fwd", f"status={status} (Paused 期待)")

                    print("  StepBackward × 50...")
                    crash_bwd = False
                    for i in range(50):
                        try:
                        except requests.RequestException:
                            pass
                        time.sleep(0.3)
                        if api_get_code("/api/replay/status") == 0:
                            crash_bwd = True
                            print(f"  CRASH detected at backward step #{i + 1}")
                            break
                        if (i + 1) % 10 == 0:
                            print(f"    backward step {i + 1}/50...")

                    if crash_bwd:
                        fail("TC-S22-02-bwd", "StepBackward 連打中にアプリがクラッシュした")
                    else:
                        wait_paused(15)
                        try:
                            status = get_status().get("status")
                        except requests.RequestException:
                            status = "unknown"
                        if status == "Paused":
                            pass_("TC-S22-02-bwd: StepBackward 50 回完了 → status=Paused")
                        else:
                            fail("TC-S22-02-bwd", f"status={status} (Paused 期待)")
        finally:
            env2.close()

    # ── TC-S22-03: ペイン CRUD サイクル 20 回（Playing 中）→ Playing 維持 ───
    # 20 CRUD サイクル × ~3 秒/cycle ≒ 60 秒かかるため -18000h/-24h (750 bars ≒ 75 秒 at 1x) を使用
    print("  [TC-S22-03] Playing 中 split→close × 20 サイクル...")
    try:
        env3 = _tachibana_start(utc_offset(-18000), utc_offset(-24))
    except RuntimeError as e:
        fail("TC-S22-03-pre", str(e))
        env3 = None

    if env3 is not None:
        try:
            if not wait_playing(60):
                fail("TC-S22-03-pre", "Playing 到達せず")
            else:
                crud_fail = False
                for i in range(20):
                    pane0 = get_pane_id(0)
                    if not pane0:
                        fail(f"TC-S22-03-{i + 1}", "ペイン ID 取得失敗")
                        crud_fail = True
                        break

                    try:
                        api_post("/api/pane/split", {"pane_id": pane0, "axis": "Vertical"})
                    except requests.RequestException as e:
                        fail(f"TC-S22-03-{i + 1}", f"split API error: {e}")
                        crud_fail = True
                        break

                    if not wait_for_pane_count(2, 10):
                        fail(f"TC-S22-03-{i + 1}", "split 後ペイン数が 2 にならなかった")
                        crud_fail = True
                        break

                    new_pane = find_other_pane_id(pane0)
                    if not new_pane:
                        fail(f"TC-S22-03-{i + 1}", "新ペイン ID 取得失敗")
                        crud_fail = True
                        break

                    try:
                        api_post("/api/pane/close", {"pane_id": new_pane})
                    except requests.RequestException as e:
                        fail(f"TC-S22-03-{i + 1}", f"close API error: {e}")
                        crud_fail = True
                        break

                    if not wait_for_pane_count(1, 10):
                        fail(f"TC-S22-03-{i + 1}", "close 後ペイン数が 1 にならなかった")
                        crud_fail = True
                        break

                    if (i + 1) % 5 == 0:
                        try:
                            status = get_status().get("status")
                        except requests.RequestException:
                            status = "unknown"
                        if status != "Playing":
                            fail(f"TC-S22-03-{i + 1}", f"CRUD サイクル {i + 1} 回後 status={status} (Playing 期待)")
                            crud_fail = True
                            break
                        print(f"    cycle {i + 1}/20: status=Playing OK")

                if not crud_fail:
                    try:
                        status = get_status().get("status")
                    except requests.RequestException:
                        status = "unknown"
                    if status == "Playing":
                        pass_("TC-S22-03: CRUD 20 サイクル完了 → status=Playing 維持")
                    else:
                        fail("TC-S22-03", f"20 サイクル後 status={status} (Playing 期待)")
        finally:
            env3.close()


def test_s22_tachibana_endurance() -> None:
    """pytest エントリポイント。"""
    _h._PASS = _h._FAIL = _h._PEND = 0
    backup_state()
    try:
        run_s22()
    finally:
        restore_state()
    print_summary()
    assert _h._FAIL == 0, f"{_h._FAIL} TC(s) failed — see output above"


def main() -> None:
    backup_state()
    try:
        run_s22()
    finally:
        restore_state()
        print_summary()
        if _h._FAIL > 0:
            sys.exit(1)


if __name__ == "__main__":
    main()
