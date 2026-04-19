#!/usr/bin/env python3
"""s13_step_backward_quality.py — Suite S13: StepBackward 品質保証

検証シナリオ:
  TC-S13-01: StepBackward 後 2s 以内に Loading 解消（チラつき防止）
  TC-S13-02-1〜10: 10 回 StepBackward、各ステップ後 streams_ready=true
  TC-S13-03: resume 後 delta が 60000ms 倍数（live data 非混入）
  TC-S13-04-1〜5: StepForward ↔ StepBackward 交互 × 5 → status=Paused 維持

仕様根拠:
  docs/replay_header.md §6.3 — StepBackward 品質保証（チラつき防止・live data 非混入）

フィクスチャ: BinanceLinear:BTCUSDT M1, auto-play (UTC[-3h, -1h])
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
    headless_play,
    speed_to_10x,
    wait_playing,
    wait_status,
    wait_streams_ready,
    wait_for_pane_streams_ready,
    wait_for_time_advance,
    get_status,
    api_get,
    api_post,
    get_pane_id,
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
    print(f"=== S13: StepBackward 品質保証 (ticker={TICKER} {mode_label}) ===")

    start = utc_offset(-3)
    end = utc_offset(-1)
    setup_single_pane(BTC_TICKER, "M1", start, end)

    env = FlowsurfaceEnv(ticker=BTC_TICKER, timeframe="M1", headless=IS_HEADLESS)
    try:
        env._start_process()
        if not IS_HEADLESS:
            wait_streams_ready(30)
        headless_play()

        if not wait_playing(30):
            fail("TC-S13-precond", "Playing 到達せず")
            return

        api_post("/api/replay/pause")
        if not wait_status("Paused", 10):
            fail("TC-S13-precond", "Paused に遷移せず")
            return

        # GUI モードのみ pane_id を取得
        if not IS_HEADLESS:
            pane_id = get_pane_id(0)
            print(f"  PANE_ID={pane_id}")
        else:
            pane_id = ""

        # 少し前進させてから StepBackward のテストを行う（start_time 境界を避ける）
        for _ in range(5):
            api_post("/api/replay/step-forward")
            time.sleep(0.3)
        wait_status("Paused", 10)

        # TC-S13-01: StepBackward 後 2 秒以内に Loading が解消される
        api_post("/api/replay/step-backward")
        t_start = time.monotonic()
        resolved = False
        final_status = "unknown"
        while time.monotonic() - t_start <= 2.0:
            try:
                final_status = get_status().get("status") or "unknown"
                if final_status in ("Paused", "Playing"):
                    resolved = True
                    break
            except requests.RequestException:
                pass
            time.sleep(0.2)

        elapsed = time.monotonic() - t_start
        if resolved:
            pass_(f"TC-S13-01: StepBackward 後 {elapsed:.1f}s 以内に status={final_status}（Loading 解消）")
        else:
            try:
                final_status = get_status().get("status") or "unknown"
            except requests.RequestException:
                pass
            fail("TC-S13-01", f"2 秒経過後も status={final_status}（Loading 継続の疑い）")

        wait_status("Paused", 10)

        # TC-S13-02: 10 回 StepBackward — 各ステップ後に streams_ready=true を個別確認
        if IS_HEADLESS:
            pend("TC-S13-02", "headless は pane/list API 非対応（501）— streams_ready 検証不可")
        else:
            for i in range(1, 11):
                api_post("/api/replay/step-backward")
                wait_status("Paused", 10)
                time.sleep(0.3)
                try:
                    body = api_get("/api/pane/list")
                    panes = body.get("panes", [])
                    p = next((x for x in panes if x.get("id") == pane_id), None)
                    ready = p is not None and p.get("streams_ready") is True
                except requests.RequestException:
                    ready = False

                if ready:
                    pass_(f"TC-S13-02-{i}: StepBackward #{i} 後 streams_ready=true")
                else:
                    fail(f"TC-S13-02-{i}", f"streams_ready={ready}（チラつき発生の疑い）")

        # TC-S13-03: resume 後の delta がバー境界に揃う（live data 非混入確認）
        api_post("/api/replay/resume")
        if not wait_status("Playing", 10):
            fail("TC-S13-03-pre", "Playing に遷移せず")
        else:
            speed_to_10x()
            t1 = int(get_status().get("current_time") or 0)
            t2 = wait_for_time_advance(t1, 15)
            if t2 is not None:
                delta = t2 - t1
                mod = delta % STEP_M1
                if mod == 0:
                    pass_(f"TC-S13-03: resume 後 delta={delta} ms（60000ms 倍数、live data 非混入）")
                else:
                    fail("TC-S13-03", f"delta={delta}, mod={mod}（live data 混入の疑い）")
            else:
                fail("TC-S13-03", "15 秒待機しても current_time が変化しなかった")

        # TC-S13-04: StepForward ↔ StepBackward 交互 × 5 でも status=Paused 維持
        api_post("/api/replay/pause")
        wait_status("Paused", 10)
        for i in range(1, 6):
            api_post("/api/replay/step-forward")
            wait_status("Paused", 10)
            api_post("/api/replay/step-backward")
            wait_status("Paused", 10)
            status = get_status().get("status")
            if status == "Paused":
                pass_(f"TC-S13-04-{i}: 交互 Step #{i} 後 status=Paused")
            else:
                fail(f"TC-S13-04-{i}", f"status={status}")

    finally:
        env.close()


def test_s13_step_backward_quality() -> None:
    """pytest から呼ばれる場合のエントリポイント。"""
    reset_counters()
    backup_state()
    try:
        run_s13()
    finally:
        restore_state()
        print_summary()
    import helpers as _h
    assert _h._FAIL == 0, f"{_h._FAIL} TC(s) failed — see output above"


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
