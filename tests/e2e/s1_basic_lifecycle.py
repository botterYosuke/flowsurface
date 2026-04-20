#!/usr/bin/env python3
"""s1_basic_lifecycle.py — Suite S1: 基本ライフサイクル (Python / flowsurface-sdk 版)

tests/archive/s1_basic_lifecycle.sh の検証ロジックを flowsurface-sdk の
FlowsurfaceEnv でプロセス管理しながら再実装したもの。
IS_HEADLESS=true で headless モード、未設定/false で GUI モード（ウィンドウあり）。

使い方:
    # GUI モード
    E2E_TICKER=HyperliquidLinear:BTC python tests/s1_basic_lifecycle.py
    # headless モード
    IS_HEADLESS=true E2E_TICKER=HyperliquidLinear:BTC python tests/s1_basic_lifecycle.py
    # pytest
    pytest tests/s1_basic_lifecycle.py -v
"""

from __future__ import annotations

import json
import os
import shutil
import sys
import time
from datetime import datetime, timezone, timedelta
from pathlib import Path

import requests

_REPO_ROOT = Path(__file__).parent.parent
try:
    from flowsurface import FlowsurfaceEnv
except ImportError:
    sys.path.insert(0, str(_REPO_ROOT / "python"))
    from env import FlowsurfaceEnv  # type: ignore[no-redef]

# ── 定数 ──────────────────────────────────────────────────────────────────────

API_BASE = "http://127.0.0.1:9876"
TICKER = os.environ.get("E2E_TICKER", "BinanceLinear:BTCUSDT")
IS_HEADLESS = os.environ.get("IS_HEADLESS", "").lower() == "true"
STEP_M1 = 60_000
DATA_DIR = Path(os.environ.get("APPDATA", "")) / "flowsurface"
STATE_FILE = DATA_DIR / "saved-state.json"
STATE_BACKUP = DATA_DIR / "saved-state.json.bak"

_PASS = 0
_FAIL = 0
_PEND = 0

# ── レポートヘルパー ──────────────────────────────────────────────────────────


def pass_(label: str) -> None:
    global _PASS
    print(f"  PASS: {label}")
    _PASS += 1


def fail(label: str, detail: str) -> None:
    global _FAIL
    print(f"  FAIL: {label} - {detail}")
    _FAIL += 1


def pend(label: str, reason: str) -> None:
    global _PEND
    print(f"  PEND: {label} - {reason}")
    _PEND += 1


def print_summary() -> None:
    print()
    print("=============================")
    print(f"  PASS: {_PASS}  FAIL: {_FAIL}  PEND: {_PEND}")
    print("=============================")


# ── 日時ユーティリティ ────────────────────────────────────────────────────────


def utc_offset(hours: float) -> str:
    """UTC 基準で hours 時間オフセットした時刻を 'YYYY-MM-DD HH:MM' 形式で返す。"""
    dt = datetime.now(timezone.utc) + timedelta(hours=hours)
    return dt.strftime("%Y-%m-%d %H:%M")


# ── saved-state バックアップ ──────────────────────────────────────────────────


def backup_state() -> None:
    if STATE_FILE.exists():
        shutil.copy2(STATE_FILE, STATE_BACKUP)


def restore_state() -> None:
    STATE_FILE.unlink(missing_ok=True)
    if STATE_BACKUP.exists():
        STATE_BACKUP.rename(STATE_FILE)


# ── GUI フィクスチャ ──────────────────────────────────────────────────────────


def write_gui_fixture() -> None:
    DATA_DIR.mkdir(parents=True, exist_ok=True)
    fixture = {
        "layout_manager": {
            "layouts": [
                {
                    "name": "S1-Basic",
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
            "active_layout": "S1-Basic",
        },
        "timezone": "UTC",
        "trade_fetch_enabled": False,
        "size_in_quote_ccy": "Base",
    }
    STATE_FILE.write_text(json.dumps(fixture, indent=2))


# ── API ラッパー ──────────────────────────────────────────────────────────────


def api_get(path: str) -> dict:
    r = requests.get(f"{API_BASE}{path}", timeout=5)
    r.raise_for_status()
    return r.json()


def api_post(path: str, body: dict | None = None) -> dict:
    r = requests.post(f"{API_BASE}{path}", json=body or {}, timeout=5)
    r.raise_for_status()
    return r.json()


def get_status() -> dict:
    return api_get("/api/replay/status")


# ── ポーリングヘルパー ────────────────────────────────────────────────────────


def wait_status(want: str, timeout: int = 10) -> bool:
    deadline = time.monotonic() + timeout
    while time.monotonic() < deadline:
        try:
            if get_status().get("status") == want:
                return True
        except requests.RequestException:
            pass
        time.sleep(0.5)
    return False


def wait_playing(timeout: int = 120) -> bool:
    return wait_status("Playing", timeout)


def wait_for_time_advance(ref: int, timeout: int = 30) -> int | None:
    """current_time > ref になるまでポーリング。新しい値を返す。タイムアウト時は None。"""
    deadline = time.monotonic() + timeout
    while time.monotonic() < deadline:
        try:
            ct = get_status().get("current_time")
            if ct is not None and int(ct) > ref:
                return int(ct)
        except (requests.RequestException, TypeError, ValueError):
            pass
        time.sleep(0.5)
    return None


def wait_streams_ready(timeout: int = 30) -> bool:
    """GUI 起動後、最初のペインの streams_ready が true になるまで待つ。"""
    deadline = time.monotonic() + timeout
    while time.monotonic() < deadline:
        try:
            body = api_get("/api/pane/list")
            panes = body.get("panes", [])
            if panes and panes[0].get("streams_ready") is True:
                ticker = panes[0].get("ticker", "")
                print(f"  streams ready (ticker={ticker})")
                return True
        except requests.RequestException:
            pass
        time.sleep(1)
    return False


# ── 検証ヘルパー ──────────────────────────────────────────────────────────────


def is_bar_boundary(ct: int, step: int) -> bool:
    return ct % step == 0


def advance_within(ct1: int, ct2: int, step: int, max_bars: int = 100) -> bool:
    diff = ct2 - ct1
    if diff < 0 or diff % step != 0:
        return False
    bars = diff // step
    return 1 <= bars <= max_bars


def ct_in_range(ct: int, st: int, et: int) -> bool:
    return st <= ct <= et


# ── テスト本体 ────────────────────────────────────────────────────────────────


def run_s1() -> None:
    start = utc_offset(-3)
    end = utc_offset(-1)
    mode_label = "headless" if IS_HEADLESS else "GUI"

    print(f"=== S1: 基本ライフサイクル (ticker={TICKER} {mode_label}) ===")

    # TC-S1-01: 起動時モード
    status = get_status()
    mode = status.get("mode")
    if IS_HEADLESS:
        if mode == "Replay":
            pass_("TC-S1-01: headless 起動時 mode=Replay")
        else:
            fail("TC-S1-01", f"mode={mode}")
    else:
        if mode == "Live":
            pass_("TC-S1-01: 起動時 mode=Live")
        else:
            fail("TC-S1-01", f"mode={mode}")

    # TC-S1-02: Replay モードへ
    toggle_res = api_post("/api/replay/toggle")
    mode2 = toggle_res.get("mode")
    if IS_HEADLESS:
        if mode2 == "Replay":
            pass_("TC-S1-02: headless toggle → mode=Replay (no-op)")
        else:
            fail("TC-S1-02", f"mode={mode2}")
    else:
        if mode2 == "Replay":
            pass_("TC-S1-02: toggle → mode=Replay")
        else:
            fail("TC-S1-02", f"mode={mode2}")

    # TC-S1-03: Play 開始 → Loading or Playing
    play_res = api_post("/api/replay/play", {"start": start, "end": end})
    play_st = play_res.get("status", "")
    if play_st in ("Loading", "loading", "Playing"):
        pass_("TC-S1-03: play → Loading or Playing")
    else:
        fail("TC-S1-03", f"status={play_st}")

    # TC-S1-04: Playing 到達（最大 120s）
    if wait_playing(120):
        pass_("TC-S1-04: Playing に到達")
    else:
        fail("TC-S1-04", "120秒以内に Playing にならなかった")

    # TC-S1-05: 1x で current_time が前進する
    ct1 = int(get_status().get("current_time") or 0)
    ct2_opt = wait_for_time_advance(ct1, 60)
    if ct2_opt is not None:
        ct2 = ct2_opt
        if advance_within(ct1, ct2, STEP_M1, 100):
            pass_(f"TC-S1-05: 1x で current_time 前進 ({ct1} → {ct2})")
        else:
            fail("TC-S1-05", f"想定外の前進 (CT1={ct1} CT2={ct2} step={STEP_M1})")
    else:
        fail("TC-S1-05", f"30秒待機しても current_time が前進しなかった (CT1={ct1})")
        ct2 = ct1

    # TC-S1-05b: current_time はバー境界値
    if is_bar_boundary(ct2, STEP_M1):
        pass_("TC-S1-05b: current_time バー境界スナップ")
    else:
        fail("TC-S1-05b", f"CT2={ct2} はバー境界ではない")

    # TC-S1-05c: current_time ∈ [start_time, end_time]
    st_now = get_status()
    start_t = int(st_now.get("start_time") or 0)
    end_t = int(st_now.get("end_time") or 0)
    if ct_in_range(ct2, start_t, end_t):
        pass_("TC-S1-05c: current_time ∈ [start,end]")
    else:
        fail("TC-S1-05c", f"CT2={ct2} range=[{start_t},{end_t}]")

    # TC-S1-06: Pause で current_time 固定
    api_post("/api/replay/pause")
    p2 = ct2
    if wait_status("Paused", 10):
        p1 = int(get_status().get("current_time") or 0)
        time.sleep(1)
        p2 = int(get_status().get("current_time") or 0)
        if p1 == p2:
            pass_("TC-S1-06: Pause 中は current_time 固定")
        else:
            fail("TC-S1-06", f"Pause 中に時刻が変化 ({p1} → {p2})")
    else:
        fail("TC-S1-06", "Pause に遷移しなかった")

    # TC-S1-07: status=Paused
    st_paused = get_status().get("status")
    if st_paused == "Paused":
        pass_("TC-S1-07: status=Paused")
    else:
        fail("TC-S1-07", f"status={st_paused}")

    # TC-S1-08: Resume 後に current_time 前進
    api_post("/api/replay/resume")
    r1 = wait_for_time_advance(p2, 30)
    if r1 is not None and r1 > p2:
        pass_("TC-S1-08: Resume 後に current_time 前進")
    else:
        fail("TC-S1-08", f"Resume 後に前進しない ({p2} → {r1})")

    # TC-S1-09〜12: Speed サイクル（1x→2x→5x→10x→1x）
    api_post("/api/replay/pause")
    for expected in ("2x", "5x", "10x", "1x"):
        speed_res = api_post("/api/replay/speed")
        speed = speed_res.get("speed")
        if speed == expected:
            pass_(f"TC-S1-speed: speed={speed}")
        else:
            fail("TC-S1-speed", f"expected={expected} got={speed}")

    # TC-S1-13: StepForward は M1 バー 1 本分（60000ms）進む
    pre = int(get_status().get("current_time") or 0)
    api_post("/api/replay/step-forward")
    time.sleep(1)
    post_sf = int(get_status().get("current_time") or 0)
    diff = post_sf - pre
    if diff == 60_000:
        pass_("TC-S1-13: StepForward +60000ms")
    else:
        fail("TC-S1-13", f"diff={diff} (expected 60000)")

    # TC-S1-13b: StepForward 後もバー境界
    if is_bar_boundary(post_sf, STEP_M1):
        pass_("TC-S1-13b: StepForward 後もバー境界")
    else:
        fail("TC-S1-13b", f"POST_SF={post_sf}")

    # TC-S1-14: StepBackward は 1 バー後退
    if IS_HEADLESS:
        pend("TC-S1-14", "StepBackward は headless 未実装")
    else:
        bef = int(get_status().get("current_time") or 0)
        api_post("/api/replay/step-backward")
        time.sleep(1)
        aft = int(get_status().get("current_time") or 0)
        diff_b = bef - aft
        if diff_b == 60_000:
            pass_("TC-S1-14: StepBackward -60000ms")
        else:
            fail("TC-S1-14", f"diff={diff_b} (expected 60000, before={bef} after={aft})")

    # TC-S1-15: Live 復帰（GUI モードのみ）
    if IS_HEADLESS:
        for sub in ("a", "b", "c", "d", "e", "f"):
            pend(f"TC-S1-15{sub}", "headless は Live モードなし")
    else:
        live_res = api_post("/api/replay/toggle")
        live_mode = live_res.get("mode")
        live_st = live_res.get("status")
        live_ct = live_res.get("current_time")
        live_sp = live_res.get("speed")
        live_rs = live_res.get("range_start")
        live_re = live_res.get("range_end")

        if live_mode == "Live":
            pass_("TC-S1-15a: mode=Live")
        else:
            fail("TC-S1-15a", f"mode={live_mode}")

        if live_st is None:
            pass_("TC-S1-15b: status=null")
        else:
            fail("TC-S1-15b", f"status={live_st}")

        if live_ct is None:
            pass_("TC-S1-15c: current_time=null")
        else:
            fail("TC-S1-15c", f"ct={live_ct}")

        if live_sp is None:
            pass_("TC-S1-15d: speed=null")
        else:
            fail("TC-S1-15d", f"speed={live_sp}")

        if live_rs:
            pass_(f"TC-S1-15e: range_start は最後の Replay 値を保持 ({live_rs})")
        else:
            fail("TC-S1-15e", "range_start が空")

        if live_re:
            pass_(f"TC-S1-15f: range_end は最後の Replay 値を保持 ({live_re})")
        else:
            fail("TC-S1-15f", "range_end が空")

    # TC-S1-H09: GET /api/pane/list → HTTP 200 + panes 配列
    resp = requests.get(f"{API_BASE}/api/pane/list", timeout=5)
    body: dict = resp.json() if resp.status_code == 200 else {}
    has_panes = isinstance(body.get("panes"), list)
    if resp.status_code == 200 and has_panes:
        pass_("TC-S1-H09: GET /api/pane/list → HTTP 200 + panes 配列")
    else:
        fail("TC-S1-H09", f"HTTP={resp.status_code} has_panes={has_panes}")


# ── pytest エントリポイント ───────────────────────────────────────────────────


def test_s1_basic_lifecycle() -> None:
    """pytest から呼ばれる場合のエントリポイント。プロセス起動は外部で行うこと。"""
    global _PASS, _FAIL, _PEND
    _PASS = _FAIL = _PEND = 0
    run_s1()
    print_summary()
    assert _FAIL == 0, f"{_FAIL} TC(s) failed — see output above"


# ── スタンドアロン実行 ────────────────────────────────────────────────────────


def main() -> None:
    backup_state()
    if not IS_HEADLESS:
        write_gui_fixture()

    env = FlowsurfaceEnv(
        ticker=TICKER,
        timeframe="M1",
        headless=IS_HEADLESS,
    )
    try:
        env._start_process()
        if not IS_HEADLESS:
            wait_streams_ready(30)
        run_s1()
    finally:
        env.close()
        restore_state()
        print_summary()
        if _FAIL > 0:
            sys.exit(1)


if __name__ == "__main__":
    main()
