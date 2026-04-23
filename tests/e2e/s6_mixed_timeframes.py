#!/usr/bin/env python3
"""s6_mixed_timeframes.py — S6: 異なる時間軸混在

検証シナリオ:
  TC-S6-01: BTCUSDT M1+M5+H1 混在（3ペイン）で 60s 以内 Playing
  TC-S6-02: step_size = min tf = M1 = 60000ms
  TC-S6-03: M5/H1 疎 step でもクラッシュなし・status=Paused
  TC-S6-04: M5 単独構成 → step_size = M5 = 300000ms

仕様根拠:
  docs/replay_header.md §7.3 — min_step_size = min(timeframes)

使い方:
    python tests/s6_mixed_timeframes.py
    IS_HEADLESS=true python tests/s6_mixed_timeframes.py
    pytest tests/s6_mixed_timeframes.py -v
"""

from __future__ import annotations

import json
import sys
import time
from pathlib import Path

import requests

sys.path.insert(0, str(Path(__file__).parent))
from helpers import (
    DATA_DIR,
    IS_HEADLESS,
    STATE_FILE,
    STEP_M1,
    STEP_M5,
    TICKER,
    api_get,
    api_get_code,
    api_post,
    backup_state,
    fail,
    pass_,
    pend,
    print_summary,
    restore_state,
    utc_offset,
    wait_playing,
    wait_status,
    FlowsurfaceEnv,
)

# BinanceLinear:BTCUSDT を固定で使う（S6 は BTCUSDT 専用シナリオ）
S6_TICKER = "BinanceLinear:BTCUSDT"


# ── フィクスチャ ──────────────────────────────────────────────────────────────

def _write_s6_3pane_fixture(start: str, end: str) -> None:
    """M1 + M5 + H1 の 3ペイン Split レイアウトを saved-state.json に書き込む。"""
    DATA_DIR.mkdir(parents=True, exist_ok=True)
    fixture = {
        "layout_manager": {
            "layouts": [
                {
                    "name": "S6-MixedTF",
                    "dashboard": {
                        "pane": {
                            "Split": {
                                "axis": "Vertical",
                                "ratio": 0.33,
                                "a": {
                                    "KlineChart": {
                                        "layout": {"splits": [0.78], "autoscale": "FitToVisible"},
                                        "kind": "Candles",
                                        "stream_type": [{"Kline": {"ticker": S6_TICKER, "timeframe": "M1"}}],
                                        "settings": {
                                            "tick_multiply": None,
                                            "visual_config": None,
                                            "selected_basis": {"Time": "M1"},
                                        },
                                        "indicators": ["Volume"],
                                        "link_group": "A",
                                    }
                                },
                                "b": {
                                    "Split": {
                                        "axis": "Vertical",
                                        "ratio": 0.5,
                                        "a": {
                                            "KlineChart": {
                                                "layout": {"splits": [0.78], "autoscale": "FitToVisible"},
                                                "kind": "Candles",
                                                "stream_type": [{"Kline": {"ticker": S6_TICKER, "timeframe": "M5"}}],
                                                "settings": {
                                                    "tick_multiply": None,
                                                    "visual_config": None,
                                                    "selected_basis": {"Time": "M5"},
                                                },
                                                "indicators": ["Volume"],
                                                "link_group": "A",
                                            }
                                        },
                                        "b": {
                                            "KlineChart": {
                                                "layout": {"splits": [0.78], "autoscale": "FitToVisible"},
                                                "kind": "Candles",
                                                "stream_type": [{"Kline": {"ticker": S6_TICKER, "timeframe": "H1"}}],
                                                "settings": {
                                                    "tick_multiply": None,
                                                    "visual_config": None,
                                                    "selected_basis": {"Time": "H1"},
                                                },
                                                "indicators": ["Volume"],
                                                "link_group": "A",
                                            }
                                        },
                                    }
                                },
                            }
                        },
                        "popout": [],
                    },
                }
            ],
            "active_layout": "S6-MixedTF",
        },
        "timezone": "UTC",
        "trade_fetch_enabled": False,
        "size_in_quote_ccy": "Base",
        "replay": {"mode": "replay", "range_start": start, "range_end": end},
    }
    STATE_FILE.write_text(json.dumps(fixture, indent=2))


def _write_m5_only_fixture(start: str, end: str) -> None:
    """M5 単独ペインの saved-state.json を書き込む。"""
    DATA_DIR.mkdir(parents=True, exist_ok=True)
    fixture = {
        "layout_manager": {
            "layouts": [
                {
                    "name": "S6-M5Only",
                    "dashboard": {
                        "pane": {
                            "KlineChart": {
                                "layout": {"splits": [0.78], "autoscale": "FitToVisible"},
                                "kind": "Candles",
                                "stream_type": [{"Kline": {"ticker": S6_TICKER, "timeframe": "M5"}}],
                                "settings": {
                                    "tick_multiply": None,
                                    "visual_config": None,
                                    "selected_basis": {"Time": "M5"},
                                },
                                "indicators": ["Volume"],
                                "link_group": "A",
                            }
                        },
                        "popout": [],
                    },
                }
            ],
            "active_layout": "S6-M5Only",
        },
        "timezone": "UTC",
        "trade_fetch_enabled": False,
        "size_in_quote_ccy": "Base",
        "replay": {"mode": "replay", "range_start": start, "range_end": end},
    }
    STATE_FILE.write_text(json.dumps(fixture, indent=2))


# ── all streams ready ポーリング ──────────────────────────────────────────────

def _wait_all_streams_ready(timeout: int = 30) -> bool:
    """全ペインの streams_ready=true になるまで待つ。"""
    deadline = time.monotonic() + timeout
    while time.monotonic() < deadline:
        try:
            body = api_get("/api/pane/list")
            panes = body.get("panes", [])
            if panes and all(p.get("streams_ready") is True for p in panes):
                print(f"  all streams ready ({len(panes)} panes)")
                return True
        except requests.RequestException:
            pass
        time.sleep(1)
    return False


# ── テスト本体 ────────────────────────────────────────────────────────────────

def run_s6_3pane(start: str, end: str) -> bool:
    """3ペイン（M1+M5+H1）サイクルのテスト。失敗したら False を返す。"""

    # Binance の到達可能性チェック
    try:
        r = requests.get("https://fapi.binance.com/fapi/v1/ping", timeout=10)
        if r.status_code != 200:
            pend("TC-S6-01〜03", f"Binance unreachable (HTTP {r.status_code}) — S6 は IP ブロック環境では実行不可")
            return True
    except requests.RequestException as e:
        pend("TC-S6-01〜03", f"Binance unreachable ({e}) — S6 は IP ブロック環境では実行不可")
        return True

    # streams ready 確認 + GUI モードはトグル
    _wait_all_streams_ready(30)
    if not IS_HEADLESS:
        try:
            api_post("/api/replay/toggle")
        except requests.RequestException:
            pass

    # TC-S6-01: Play → Playing
    try:
        api_post("/api/replay/toggle", {"start": start, "end": end})
    except requests.RequestException:
        pass

    if wait_playing(60):
        pass_("TC-S6-01: M1+M5+H1 混在 → Playing")
    else:
        fail("TC-S6-01", "60s 以内に Playing にならなかった")
        return False

    # TC-S6-02: step_size = M1 = 60000ms
    try:
    except requests.RequestException:
        pass
    time.sleep(0.5)
    pre = int(api_get("/api/replay/status").get("current_time") or 0)
    try:
        api_post("/api/replay/step-forward")
    except requests.RequestException:
        pass
    time.sleep(1)
    post_sf = int(api_get("/api/replay/status").get("current_time") or 0)
    diff = post_sf - pre
    if diff == STEP_M1:
        pass_("TC-S6-02: step_size=60000ms (M1 が最小 tf)")
    else:
        fail("TC-S6-02", f"diff={diff} (expected {STEP_M1})")

    # TC-S6-03: 5x StepForward でクラッシュなし、status=Paused
    for _ in range(5):
        try:
            api_post("/api/replay/step-forward")
        except requests.RequestException:
            pass
        time.sleep(0.5)
    st = api_get("/api/replay/status").get("status")
    if st == "Paused":
        pass_("TC-S6-03: M5/H1 疎 step でもクラッシュなし (status=Paused)")
    else:
        fail("TC-S6-03", f"status={st} (expected Paused)")

    return True


def run_s6_m5only(start: str, end: str) -> None:
    """M5 単独サイクルのテスト (TC-S6-04)。"""

    _wait_all_streams_ready(30)
    if not IS_HEADLESS:
        try:
            api_post("/api/replay/toggle")
        except requests.RequestException:
            pass

    try:
        api_post("/api/replay/toggle", {"start": start, "end": end})
    except requests.RequestException:
        pass

    if not wait_playing(60):
        fail("TC-S6-04-precond", "M5 単独構成で Playing 到達せず")
        return

    try:
    except requests.RequestException:
        pass
    time.sleep(0.5)
    pre2 = int(api_get("/api/replay/status").get("current_time") or 0)
    try:
        api_post("/api/replay/step-forward")
    except requests.RequestException:
        pass
    wait_status("Paused", 10)                                       # Paused 確定後に読む（was: time.sleep(1)）
    post2 = int(api_get("/api/replay/status").get("current_time") or 0)
    diff2 = post2 - pre2
    if diff2 == STEP_M5:
        pass_("TC-S6-04: M5 単独 → step=300000ms")
    else:
        fail("TC-S6-04", f"diff={diff2} (expected {STEP_M5})")


# ── pytest エントリポイント ───────────────────────────────────────────────────

def test_s6_mixed_timeframes() -> None:
    """pytest から呼ばれる場合のエントリポイント。プロセス起動は外部で行うこと。"""
    from helpers import _FAIL, _PASS, _PEND
    import helpers
    helpers._PASS = helpers._FAIL = helpers._PEND = 0

    start = utc_offset(-6)
    end = utc_offset(-1)
    run_s6_3pane(start, end)
    print_summary()
    assert helpers._FAIL == 0, f"{helpers._FAIL} TC(s) failed — see output above"


# ── スタンドアロン実行 ────────────────────────────────────────────────────────

def main() -> None:
    import helpers
    helpers._PASS = helpers._FAIL = helpers._PEND = 0

    start = utc_offset(-6)
    end = utc_offset(-1)

    print(f"=== S6: 異なる時間軸混在 (ticker={S6_TICKER}) ===")
    backup_state()

    # ── サイクル 1: 3ペイン（M1+M5+H1）────────────────────────────────────────
    _write_s6_3pane_fixture(start, end)

    env1 = FlowsurfaceEnv(ticker=S6_TICKER, timeframe="M1", headless=IS_HEADLESS)
    try:
        env1._start_process()
        ok = run_s6_3pane(start, end)
    finally:
        env1.close()

    if not ok:
        restore_state()
        print_summary()
        if helpers._FAIL > 0:
            sys.exit(1)
        return

    # ── TC-S6-04: M5 単独構成 ────────────────────────────────────────────────
    _write_m5_only_fixture(start, end)

    env2 = FlowsurfaceEnv(ticker=S6_TICKER, timeframe="M5", headless=IS_HEADLESS)
    try:
        env2._start_process()
        run_s6_m5only(start, end)
    finally:
        env2.close()

    restore_state()
    print_summary()
    if helpers._FAIL > 0:
        sys.exit(1)


if __name__ == "__main__":
    main()
