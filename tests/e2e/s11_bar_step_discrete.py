#!/usr/bin/env python3
"""s11_bar_step_discrete.py — Suite S11: バーステップ離散化

検証シナリオ:
  TC-S11-01: M1 10x 再生中 delta が 60000ms の倍数
  TC-S11-02-1〜3: M1 StepForward × 3、各 delta = 60000ms
  TC-S11-03: M5 StepForward delta = 300000ms
  TC-S11-04: H1 StepForward delta = 3600000ms
  TC-S11-05: M1+M5 混在 StepForward → min TF (M1=60000ms) が優先
  TC-S11-06: M1 StepForward 10 連続 → 毎回 delta が厳密に 60000ms

仕様根拠:
  docs/replay_header.md §6.2 — バーステップ離散化（step_size = min timeframe）

フィクスチャ: BinanceLinear:BTCUSDT M1 / M5 / H1 / M1+ETHUSDT M5 混在（4パターン）
"""

from __future__ import annotations

import json
import sys
import time
from pathlib import Path

sys.path.insert(0, str(Path(__file__).parent))
from helpers import (
    TICKER,
    IS_HEADLESS,
    DATA_DIR,
    STATE_FILE,
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
    api_get,
    api_post,
    get_pane_id,
    find_other_pane_id,
    utc_offset,
    reset_counters,
    pass_,
    fail,
    print_summary,
    STEP_M1,
    STEP_M5,
    STEP_H1,
)

import requests

# Fixed ticker for this suite
BTC_TICKER = "BinanceLinear:BTCUSDT"
ETH_TICKER = "BinanceLinear:ETHUSDT"


def _step_forward_delta() -> int:
    """Pause → StepForward → return delta in ms."""
    tb = int(get_status().get("current_time") or 0)
    api_post("/api/replay/step-forward")
    time.sleep(1)
    wait_status("Paused", 10)
    ta = int(get_status().get("current_time") or 0)
    return ta - tb


def run_s11() -> None:
    mode_label = "headless" if IS_HEADLESS else "GUI"
    print(f"=== S11: バーステップ離散化 (ticker={TICKER} {mode_label}) ===")

    # ── TC-S11-01: M1 10x 再生中 delta が 60000ms の倍数 ──────────────────────
    setup_single_pane(BTC_TICKER, "M1", utc_offset(-3), utc_offset(-1))
    env = FlowsurfaceEnv(ticker=BTC_TICKER, timeframe="M1", headless=IS_HEADLESS)
    try:
        env._start_process()
        if not IS_HEADLESS:
            wait_streams_ready(30)
        headless_play()
        if not wait_playing(60):   # was 30 — CI で不安定なため延長
            fail("TC-S11-01-pre", "Playing 到達せず")
        else:
            speed_to_10x()
            t1 = int(get_status().get("current_time") or 0)
            t2 = wait_for_time_advance(t1, 15)
            if t2 is not None:
                delta = t2 - t1
                if delta % STEP_M1 == 0:
                    pass_(f"TC-S11-01: M1 10x delta={delta} ms（60000ms の倍数）")
                else:
                    fail("TC-S11-01", f"delta={delta}, mod={delta % STEP_M1} (60000ms の倍数でない)")
            else:
                fail("TC-S11-01", "15 秒待機しても current_time が変化しなかった")

            # ── TC-S11-02: M1 Pause → StepForward × 3、各 delta = 60000ms ─────
            api_post("/api/replay/pause")
            if not wait_status("Paused", 10):
                fail("TC-S11-02-pre", "Paused に遷移せず")
            else:
                for i in range(1, 4):
                    delta = _step_forward_delta()
                    if delta == STEP_M1:
                        pass_(f"TC-S11-02-{i}: StepForward #{i} delta=60000ms")
                    else:
                        fail(f"TC-S11-02-{i}", f"delta={delta} (expected 60000)")
    finally:
        env.close()

    # ── TC-S11-03: M5 ペイン StepForward delta = 300000ms ─────────────────────
    setup_single_pane(BTC_TICKER, "M5", utc_offset(-6), utc_offset(-1))
    env3 = FlowsurfaceEnv(ticker=BTC_TICKER, timeframe="M5", headless=IS_HEADLESS)
    try:
        env3._start_process()
        if not IS_HEADLESS:
            wait_streams_ready(30)
        headless_play()
        if not wait_playing(60):   # was 30 — CI で不安定なため延長
            fail("TC-S11-03-pre", "Playing 到達せず")
        else:
            api_post("/api/replay/pause")
            wait_status("Paused", 10)
            delta = _step_forward_delta()
            if delta == STEP_M5:
                pass_("TC-S11-03: M5 StepForward delta=300000ms")
            else:
                fail("TC-S11-03", f"delta={delta} (expected 300000)")
    finally:
        env3.close()

    # ── TC-S11-04: H1 ペイン StepForward delta = 3600000ms ────────────────────
    setup_single_pane(BTC_TICKER, "H1", utc_offset(-24), utc_offset(-1))
    env4 = FlowsurfaceEnv(ticker=BTC_TICKER, timeframe="H1", headless=IS_HEADLESS)
    try:
        env4._start_process()
        if not IS_HEADLESS:
            wait_streams_ready(30)
        headless_play()
        if not wait_playing(60):
            fail("TC-S11-04-pre", "Playing 到達せず")
        else:
            api_post("/api/replay/pause")
            wait_status("Paused", 10)
            delta = _step_forward_delta()
            if delta == STEP_H1:
                pass_("TC-S11-04: H1 StepForward delta=3600000ms")
            else:
                fail("TC-S11-04", f"delta={delta} (expected 3600000)")
    finally:
        env4.close()

    # ── TC-S11-05: M1+M5 混在 → 最小 TF (M1=60000ms) が優先 ─────────────────
    if IS_HEADLESS:
        # headless: pane/split + pane/set-timeframe で M1+M5 混在を再現
        setup_single_pane(BTC_TICKER, "M1", utc_offset(-3), utc_offset(-1))
        env5 = FlowsurfaceEnv(ticker=BTC_TICKER, timeframe="M1", headless=IS_HEADLESS)
        try:
            env5._start_process()
            headless_play()
            if not wait_playing(60):   # was 30 — CI で不安定なため延長
                fail("TC-S11-05-pre", "Playing 到達せず")
            else:
                api_post("/api/replay/pause")
                wait_status("Paused", 10)

                pane0 = get_pane_id(0)
                api_post("/api/pane/split", {"pane_id": pane0, "axis": "Vertical"})
                time.sleep(0.3)

                pane1 = find_other_pane_id(pane0)
                api_post("/api/pane/set-timeframe", {"pane_id": pane1, "timeframe": "M5"})

                delta = _step_forward_delta()
                if delta == STEP_M1:
                    pass_("TC-S11-05: M1+M5 混在 StepForward delta=60000ms（M1 優先）")
                else:
                    fail("TC-S11-05", f"delta={delta} (expected 60000, M1 優先のはず)")
        finally:
            env5.close()
    else:
        # GUI: saved-state.json に Split レイアウトを直接書き込む
        start5 = utc_offset(-3)
        end5 = utc_offset(-1)
        DATA_DIR.mkdir(parents=True, exist_ok=True)
        fixture = {
            "layout_manager": {
                "layouts": [
                    {
                        "name": "S11-mix",
                        "dashboard": {
                            "pane": {
                                "Split": {
                                    "axis": "Vertical",
                                    "ratio": 0.5,
                                    "a": {
                                        "KlineChart": {
                                            "layout": {"splits": [0.78], "autoscale": "FitToVisible"},
                                            "kind": "Candles",
                                            "stream_type": [{"Kline": {"ticker": BTC_TICKER, "timeframe": "M1"}}],
                                            "settings": {
                                                "tick_multiply": None,
                                                "visual_config": None,
                                                "selected_basis": {"Time": "M1"},
                                            },
                                            "indicators": [],
                                            "link_group": "A",
                                        }
                                    },
                                    "b": {
                                        "KlineChart": {
                                            "layout": {"splits": [0.78], "autoscale": "FitToVisible"},
                                            "kind": "Candles",
                                            "stream_type": [{"Kline": {"ticker": ETH_TICKER, "timeframe": "M5"}}],
                                            "settings": {
                                                "tick_multiply": None,
                                                "visual_config": None,
                                                "selected_basis": {"Time": "M5"},
                                            },
                                            "indicators": [],
                                            "link_group": "A",
                                        }
                                    },
                                }
                            },
                            "popout": [],
                        },
                    }
                ],
                "active_layout": "S11-mix",
            },
            "timezone": "UTC",
            "trade_fetch_enabled": False,
            "size_in_quote_ccy": "Base",
            "replay": {"mode": "replay", "range_start": start5, "range_end": end5},
        }
        STATE_FILE.write_text(json.dumps(fixture, indent=2))

        env5 = FlowsurfaceEnv(ticker=BTC_TICKER, timeframe="M1", headless=IS_HEADLESS)
        try:
            env5._start_process()
            wait_streams_ready(30)
            if not wait_playing(60):   # was 30 — CI で不安定なため延長
                fail("TC-S11-05-pre", "Playing 到達せず")
            else:
                api_post("/api/replay/pause")
                wait_status("Paused", 10)
                delta = _step_forward_delta()
                if delta == STEP_M1:
                    pass_("TC-S11-05: M1+M5 混在 StepForward delta=60000ms（M1 優先）")
                else:
                    fail("TC-S11-05", f"delta={delta} (expected 60000, M1 優先のはず)")
        finally:
            env5.close()

    # ── TC-S11-06: M1 StepForward 10 連続 → 毎回 delta が厳密に 60000ms ─────────
    setup_single_pane(BTC_TICKER, "M1", utc_offset(-3), utc_offset(-1))
    env6 = FlowsurfaceEnv(ticker=BTC_TICKER, timeframe="M1", headless=IS_HEADLESS)
    try:
        env6._start_process()
        if not IS_HEADLESS:
            wait_streams_ready(30)
        headless_play()
        if not wait_playing(60):   # was 30 — CI で不安定なため延長
            fail("TC-S11-06-pre", "Playing 到達せず")
        else:
            api_post("/api/replay/pause")
            if not wait_status("Paused", 10):
                fail("TC-S11-06-pre", "Paused に遷移せず")
            else:
                for i in range(1, 11):
                    tb = int(get_status().get("current_time") or 0)
                    api_post("/api/replay/step-forward")
                    time.sleep(0.5)
                    wait_status("Paused", 10)
                    ta = int(get_status().get("current_time") or 0)
                    delta = ta - tb
                    if delta == STEP_M1:
                        pass_(f"TC-S11-06-{i}: StepForward #{i} delta=60000ms（exact）")
                    else:
                        fail(f"TC-S11-06-{i}", f"delta={delta} (expected exactly 60000 — 複数バー同時前進の疑い)")
    finally:
        env6.close()


def test_s11_bar_step_discrete() -> None:
    """pytest から呼ばれる場合のエントリポイント。"""
    reset_counters()
    backup_state()
    try:
        run_s11()
    finally:
        restore_state()
        print_summary()
    import helpers as _h
    assert _h._FAIL == 0, f"{_h._FAIL} TC(s) failed — see output above"


def main() -> None:
    reset_counters()
    backup_state()
    try:
        run_s11()
    finally:
        restore_state()
        print_summary()
        import helpers as _h
        if _h._FAIL > 0:
            sys.exit(1)


if __name__ == "__main__":
    main()
