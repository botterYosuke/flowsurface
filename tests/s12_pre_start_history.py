#!/usr/bin/env python3
"""s12_pre_start_history.py — Suite S12: Start 以前の履歴バー表示

検証シナリオ:
  TC-S12-01: StepBackward 後 current_time >= start_time（下限クランプ）
  TC-S12-02-1〜5: StepBackward 連打 5 回でも start_time クランプ維持
  TC-S12-03: resume 後 current_time 正常前進（10x でポーリング）
  TC-S12-04: PEND — chart-snapshot API 実装後に bar_count 直接検証

仕様根拠:
  docs/replay_header.md §6.3 — PRE_START_HISTORY_BARS=300・start_time 下限クランプ

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
    wait_for_time_advance,
    get_status,
    api_post,
    utc_offset,
    reset_counters,
    pass_,
    fail,
    pend,
    print_summary,
)

BTC_TICKER = "BinanceLinear:BTCUSDT"


def run_s12() -> None:
    mode_label = "headless" if IS_HEADLESS else "GUI"
    print(f"=== S12: Start 以前の履歴バー表示 (ticker={TICKER} {mode_label}) ===")

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
            fail("TC-S12-precond", "Playing 到達せず")
            return

        api_post("/api/replay/pause")
        if not wait_status("Paused", 10):
            fail("TC-S12-precond", "Paused に遷移せず")
            return

        start_ms = int(get_status().get("start_time") or 0)
        print(f"  start_time={start_ms}")

        # TC-S12-01: 1 回 StepBackward → current_time >= start_time
        api_post("/api/replay/step-backward")
        time.sleep(1)
        wait_status("Paused", 10)
        ct = int(get_status().get("current_time") or 0)
        if ct >= start_ms:
            pass_(f"TC-S12-01: StepBackward 後 current_time({ct}) >= start_time({start_ms})")
        else:
            fail("TC-S12-01", f"current_time={ct} < start_time={start_ms}")

        # TC-S12-02: StepBackward 連打（5 回）でも start_time クランプ
        for i in range(1, 6):
            api_post("/api/replay/step-backward")
            time.sleep(0.5)
            wait_status("Paused", 10)
            ct = int(get_status().get("current_time") or 0)
            if ct >= start_ms:
                pass_(f"TC-S12-02-{i}: StepBackward #{i} current_time({ct}) >= start_time({start_ms})")
            else:
                fail(f"TC-S12-02-{i}", f"current_time={ct} < start_time={start_ms}")

        # TC-S12-03: resume 後に current_time が正常前進（10x でポーリング）
        api_post("/api/replay/resume")
        if not wait_status("Playing", 10):
            fail("TC-S12-03-pre", "Playing に遷移せず")
        else:
            speed_to_10x()
            ct_base = int(get_status().get("current_time") or 0)
            ct_after = wait_for_time_advance(ct_base, 15)
            if ct_after is not None:
                pass_(f"TC-S12-03: resume 後 current_time 前進 ({ct_base} → {ct_after})")
            else:
                fail("TC-S12-03", "15 秒待機しても current_time が前進しなかった")

        # TC-S12-04: バー本数直接検証（chart-snapshot API 未実装のため PEND）
        pend("TC-S12-04", "GET /api/pane/chart-snapshot 未実装 → 実装後に追加")

    finally:
        env.close()


def test_s12_pre_start_history() -> None:
    """pytest から呼ばれる場合のエントリポイント。"""
    reset_counters()
    backup_state()
    try:
        run_s12()
    finally:
        restore_state()
        print_summary()
    import helpers as _h
    assert _h._FAIL == 0, f"{_h._FAIL} TC(s) failed — see output above"


def main() -> None:
    reset_counters()
    backup_state()
    try:
        run_s12()
    finally:
        restore_state()
        print_summary()
        import helpers as _h
        if _h._FAIL > 0:
            sys.exit(1)


if __name__ == "__main__":
    main()
