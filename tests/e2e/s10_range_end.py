#!/usr/bin/env python3
"""s10_range_end.py — Suite S10: 範囲端・終端到達

検証シナリオ:
  TC-S10-01: 10x 速度で終端到達 → 自動 Paused（最大 300s 待機）
  TC-S10-02: 終端到達後 StepForward は no-op
  TC-S10-03: 終端から StepBackward で戻れる
  TC-S10-04: 終端付近から Resume → Playing
  TC-S10-05: 2 分幅の最小 range で Playing/終端 Paused 到達

仕様根拠:
  docs/replay_header.md §6.4 — range end 到達時の自動 Pause・終端クランプ

フィクスチャ: BinanceLinear:BTCUSDT M1, auto-play (UTC[-3h, -1h]) + 2 分 range パターン
"""

from __future__ import annotations

import sys
import time
from datetime import datetime, timezone, timedelta
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
    wait_paused,
    wait_status,
    wait_streams_ready,
    get_status,
    api_post,
    utc_offset,
    utc_to_ms,
    reset_counters,
    pass_,
    fail,
    print_summary,
    _PASS,
    _FAIL,
    _PEND,
)

import requests


def run_s10() -> None:
    start = utc_offset(-3)
    end = utc_offset(-1)
    end_ms = utc_to_ms(end)

    mode_label = "headless" if IS_HEADLESS else "GUI"
    print(f"=== S10: 範囲端・終端到達 (ticker={TICKER} {mode_label}) ===")

    setup_single_pane(TICKER, "M1", start, end)

    env = FlowsurfaceEnv(ticker=TICKER, timeframe="M1", headless=IS_HEADLESS)
    try:
        env._start_process()
        if not IS_HEADLESS:
            wait_streams_ready(30)
        headless_play()

        if not wait_playing(30):
            fail("TC-S10-precond", "auto-play で Playing に到達せず")
            return

        # --- TC-S10-01: 速度を 10x にして終端まで再生 ---
        # CycleSpeed は pause + seek(range.start) を伴う。速度変更後に Resume が必要。
        speed_to_10x()
        api_post("/api/replay/resume")
        wait_status("Playing", 10)
        print("  10x 速度で終端まで待機（最大 300s）...")

        reached_end = False
        ct_at_end = 0
        for _ in range(300):
            try:
                status = get_status()
                st = status.get("status")
                ct = int(status.get("current_time") or 0)
                if st == "Paused":
                    # end_ms の 2 分前（120000ms）以上に到達していれば終端とみなす
                    if ct >= end_ms - 120_000:
                        reached_end = True
                        ct_at_end = ct
                    break
            except requests.RequestException:
                pass
            time.sleep(1)

        if reached_end:
            pass_("TC-S10-01: 終端到達で自動 Paused")
        else:
            fail("TC-S10-01", "終端到達しなかった or Paused にならなかった")
            ct_at_end = int(get_status().get("current_time") or 0)

        # --- TC-S10-02: 終端到達後 StepForward は完全 no-op ---
        ct_at_end = int(get_status().get("current_time") or 0)
        api_post("/api/replay/step-forward")
        time.sleep(1)
        ct_after_sf = int(get_status().get("current_time") or 0)
        if ct_at_end == ct_after_sf:
            pass_("TC-S10-02: 終端後 StepForward は no-op")
        else:
            fail("TC-S10-02", f"終端後 StepForward が前進 (before={ct_at_end} after={ct_after_sf})")

        # --- TC-S10-03: 終端から StepBackward で戻れる ---
        api_post("/api/replay/step-backward")
        time.sleep(1)
        ct_back = int(get_status().get("current_time") or 0)
        if ct_at_end > ct_back:
            pass_("TC-S10-03: 終端から StepBackward 可能")
        else:
            fail("TC-S10-03", f"後退しない (end={ct_at_end} back={ct_back})")

        # --- TC-S10-04: Resume で再び Playing になる ---
        # BASE_STEP_DELAY_MS=100ms / 10x = 10ms/bar。
        # 60 バー後退 (600ms) して Resume し、即座に status を確認。
        for _ in range(59):
            api_post("/api/replay/step-backward")
        api_post("/api/replay/resume")
        # 10ms/bar × 60 bars = 600ms の余裕。すぐにチェック
        time.sleep(0.1)
        st = get_status().get("status")
        if st == "Playing":
            pass_("TC-S10-04: StepBackward 後に Resume → Playing")
        else:
            fail("TC-S10-04", f"status={st}")

    finally:
        env.close()

    # --- TC-S10-05: 2 分幅のレンジ（最小動作確認） ---
    # 別プロセスとして起動し直す
    tiny_start = utc_offset(-2)
    # tiny_start + 2 分
    tiny_start_dt = datetime.strptime(tiny_start, "%Y-%m-%d %H:%M").replace(tzinfo=timezone.utc)
    tiny_end_dt = tiny_start_dt + timedelta(minutes=2)
    tiny_end = tiny_end_dt.strftime("%Y-%m-%d %H:%M")
    tiny_end_ms = int(tiny_end_dt.timestamp() * 1000)

    setup_single_pane(TICKER, "M1", tiny_start, tiny_end)

    env2 = FlowsurfaceEnv(ticker=TICKER, timeframe="M1", headless=IS_HEADLESS)
    try:
        env2._start_process()
        if not IS_HEADLESS:
            wait_streams_ready(30)
        headless_play(start=tiny_start, end=tiny_end)

        # 2 分 range (2 bars) は BASE_STEP_DELAY_MS=100ms/1x では 200ms で完走する。
        # wait_playing (1s ポーリング) では捕捉できないため、Paused 終端も合格条件とする。
        tc05_ok = False
        st05 = "unknown"
        ct05 = 0
        for _ in range(30):
            try:
                status = get_status()
                st05 = status.get("status") or "null"
                ct05 = int(status.get("current_time") or 0)
                if st05 == "Playing":
                    tc05_ok = True
                    break
                # Paused かつ終端近く（高速完走）も合格
                if st05 == "Paused" and ct05 >= tiny_end_ms - 120_000:
                    tc05_ok = True
                    break
            except requests.RequestException:
                pass
            time.sleep(1)

        if tc05_ok:
            pass_(f"TC-S10-05: 2 分 range で Playing/終端 Paused 到達 (status={st05})")
            # Playing だった場合は Paused 終端も確認
            if st05 == "Playing":
                if wait_paused(60):
                    pass_("TC-S10-05b: 小 range で終端到達 → Paused")
                else:
                    fail("TC-S10-05b", "終端到達しなかった")
        else:
            fail("TC-S10-05", "2 分 range で Playing/終端 Paused にならなかった")
    finally:
        env2.close()


def test_s10_range_end() -> None:
    """pytest から呼ばれる場合のエントリポイント。"""
    reset_counters()
    run_s10()
    print_summary()
    import helpers as _h
    assert _h._FAIL == 0, f"{_h._FAIL} TC(s) failed — see output above"


def main() -> None:
    backup_state()
    try:
        run_s10()
    finally:
        restore_state()
        print_summary()
        import helpers as _h
        if _h._FAIL > 0:
            sys.exit(1)


if __name__ == "__main__":
    reset_counters()
    main()
