#!/usr/bin/env python3
"""s2_persistence.py — スイート S2: 永続化往復テスト

検証シナリオ:
  TC-S2-01:   replay フィールドなし → mode=Live・range_start 空（後方互換）
  TC-S2-02a〜e: Replay モード保存 → 再起動で mode/range_start/range_end 復元
  TC-S2-03:   Play 後保存 → 再起動で range 維持
  TC-S2-04:   Live 保存 → 再起動で mode=Live

使い方:
    E2E_TICKER=BinanceLinear:BTCUSDT python tests/s2_persistence.py
    IS_HEADLESS=true python tests/s2_persistence.py
    pytest tests/s2_persistence.py -v
"""

from __future__ import annotations

import json
import sys
import time
from pathlib import Path

sys.path.insert(0, str(Path(__file__).parent))
from helpers import (
    DATA_DIR,
    IS_HEADLESS,
    STATE_FILE,
    TICKER,
    api_post,
    backup_state,
    get_status,
    pass_,
    fail,
    pend,
    print_summary,
    restore_state,
    utc_offset,
    utc_to_ms,
    wait_status,
)

_REPO_ROOT = Path(__file__).parent.parent
try:
    from flowsurface import FlowsurfaceEnv
except ImportError:
    sys.path.insert(0, str(_REPO_ROOT / "python"))
    from env import FlowsurfaceEnv  # type: ignore[no-redef]


# ── フィクスチャライター ──────────────────────────────────────────────────────

def _write_fixture_no_replay() -> None:
    """replay フィールドなし（Live モード起動）。"""
    DATA_DIR.mkdir(parents=True, exist_ok=True)
    fixture = {
        "layout_manager": {
            "layouts": [
                {
                    "name": "S2",
                    "dashboard": {
                        "pane": {
                            "KlineChart": {
                                "layout": {"splits": [0.78], "autoscale": "FitToVisible"},
                                "kind": "Candles",
                                "stream_type": [{"Kline": {"ticker": TICKER, "timeframe": "M1"}}],
                                "settings": {
                                    "tick_multiply": None,
                                    "visual_config": None,
                                    "selected_basis": {"Time": "M1"},
                                },
                                "indicators": ["Volume"],
                                "link_group": "A",
                            }
                        },
                        "popout": [],
                    },
                }
            ],
            "active_layout": "S2",
        },
        "timezone": "UTC",
        "trade_fetch_enabled": False,
        "size_in_quote_ccy": "Base",
    }
    STATE_FILE.write_text(json.dumps(fixture, indent=2))


def _write_fixture_with_replay(start: str, end: str) -> None:
    """replay フィールドあり（Replay モード起動）。"""
    DATA_DIR.mkdir(parents=True, exist_ok=True)
    fixture = {
        "layout_manager": {
            "layouts": [
                {
                    "name": "S2",
                    "dashboard": {
                        "pane": {
                            "KlineChart": {
                                "layout": {"splits": [0.78], "autoscale": "FitToVisible"},
                                "kind": "Candles",
                                "stream_type": [{"Kline": {"ticker": TICKER, "timeframe": "M1"}}],
                                "settings": {
                                    "tick_multiply": None,
                                    "visual_config": None,
                                    "selected_basis": {"Time": "M1"},
                                },
                                "indicators": ["Volume"],
                                "link_group": "A",
                            }
                        },
                        "popout": [],
                    },
                }
            ],
            "active_layout": "S2",
        },
        "timezone": "UTC",
        "trade_fetch_enabled": False,
        "size_in_quote_ccy": "Base",
        "replay": {"mode": "replay", "range_start": start, "range_end": end},
    }
    STATE_FILE.write_text(json.dumps(fixture, indent=2))


# ── テスト本体 ────────────────────────────────────────────────────────────────

def run_s2() -> None:
    print("=== S2: 永続化往復テスト ===")

    start = utc_offset(-4)
    end = utc_offset(-1)

    # ── TC-S2-01: replay フィールドなしで起動（後方互換） ────────────────────
    _write_fixture_no_replay()
    env1 = FlowsurfaceEnv(ticker=TICKER, timeframe="M1", headless=IS_HEADLESS)
    try:
        env1._start_process()
        status = get_status()
        mode = status.get("mode")
        rs = status.get("range_start") or ""
        if mode == "Live":
            pass_("TC-S2-01: replay なし → mode=Live")
        else:
            fail("TC-S2-01", f"mode={mode}")
        if rs == "":
            pass_("TC-S2-01b: range_start 空")
        else:
            fail("TC-S2-01b", f"range_start={rs}")
    finally:
        env1.close()

    # ── TC-S2-02: Replay モードで保存 → 再起動で復元 ─────────────────────────
    _write_fixture_with_replay(start, end)
    env2 = FlowsurfaceEnv(ticker=TICKER, timeframe="M1", headless=IS_HEADLESS)
    try:
        env2._start_process()
        # Playing または null になるまで最大 30s 待機
        st: dict = {}
        for _ in range(30):
            st = get_status()
            pstatus = st.get("status")
            if pstatus in ("Playing", None):
                break
            time.sleep(1)

        mode2 = st.get("mode")
        rs2 = st.get("range_start") or ""
        re2 = st.get("range_end") or ""
        st_t = st.get("start_time")
        et_t = st.get("end_time")

        if mode2 == "Replay":
            pass_("TC-S2-02: 再起動後 mode=Replay")
        else:
            fail("TC-S2-02", f"mode={mode2}")

        if rs2 == start:
            pass_("TC-S2-02b: range_start 復元")
        else:
            fail("TC-S2-02b", f"got={rs2} expected={start}")

        if re2 == end:
            pass_("TC-S2-02c: range_end 復元")
        else:
            fail("TC-S2-02c", f"got={re2} expected={end}")

        # TC-S2-02d/e: start_time / end_time ms 整合
        if st_t is None:
            pend("TC-S2-02d", "clock 未起動のため start_time=null（auto-play 前で計測不可）")
            pend("TC-S2-02e", "clock 未起動のため end_time=null")
        else:
            expected_st = utc_to_ms(start)
            expected_et = utc_to_ms(end)
            if int(st_t) == expected_st:
                pass_("TC-S2-02d: start_time ms 整合")
            else:
                fail("TC-S2-02d", f"got={st_t} expected={expected_st}")
            if et_t is not None and int(et_t) == expected_et:
                pass_("TC-S2-02e: end_time ms 整合")
            else:
                fail("TC-S2-02e", f"got={et_t} expected={expected_et}")
    finally:
        env2.close()

    # ── TC-S2-03: Play 実行後に保存 → 再起動で range_input 維持 ─────────────
    # fixture はすでに replay フィールドあり（上の _write_fixture_with_replay 済み）
    env3 = FlowsurfaceEnv(ticker=TICKER, timeframe="M1", headless=IS_HEADLESS)
    try:
        env3._start_process()
        # Playing まで最大 60s 待機（失敗してもテスト継続）
        wait_status("Playing", 60)
        api_post("/api/app/save")
    finally:
        env3.close()

    env4 = FlowsurfaceEnv(ticker=TICKER, timeframe="M1", headless=IS_HEADLESS)
    try:
        env4._start_process()
        st3 = get_status()
        rs3 = st3.get("range_start") or ""
        re3 = st3.get("range_end") or ""
        if rs3 == start:
            pass_("TC-S2-03: 保存→復元で range_start 維持")
        else:
            fail("TC-S2-03", f"got={rs3}")
        if re3 == end:
            pass_("TC-S2-03b: 保存→復元で range_end 維持")
        else:
            fail("TC-S2-03b", f"got={re3}")
    finally:
        env4.close()

    # ── TC-S2-04: toggle → Live に戻してから保存 → 再起動で Live ────────────
    # fixture はまだ replay フィールドあり → Replay モードで起動してから Live に切替
    env5 = FlowsurfaceEnv(ticker=TICKER, timeframe="M1", headless=IS_HEADLESS)
    try:
        env5._start_process()
        api_post("/api/replay/toggle")  # Replay → Live
        time.sleep(1)
        api_post("/api/app/save")
    finally:
        env5.close()

    env6 = FlowsurfaceEnv(ticker=TICKER, timeframe="M1", headless=IS_HEADLESS)
    try:
        env6._start_process()
        st4 = get_status()
        mode4 = st4.get("mode")
        if mode4 == "Live":
            pass_("TC-S2-04: Live 保存→復元で mode=Live")
        else:
            fail("TC-S2-04", f"mode={mode4}")
    finally:
        env6.close()


def test_s2_persistence() -> None:
    """pytest エントリポイント。プロセス起動はこの関数内で行う。"""
    import helpers
    helpers._PASS = helpers._FAIL = helpers._PEND = 0
    backup_state()
    try:
        run_s2()
    finally:
        restore_state()
        print_summary()
    assert helpers._FAIL == 0, f"{helpers._FAIL} TC(s) failed — see output above"


def main() -> None:
    backup_state()
    try:
        run_s2()
    finally:
        restore_state()
        print_summary()

    import helpers
    if helpers._FAIL > 0:
        sys.exit(1)


if __name__ == "__main__":
    main()
