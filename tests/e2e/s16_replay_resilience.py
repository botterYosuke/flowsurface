#!/usr/bin/env python3
"""s16_replay_resilience.py — スイート S16: UI操作中の Replay 耐性テスト

検証シナリオ:
  TC-S16-01: speed API 20 連打 → Resume 後 status=Playing
  TC-S16-02: UTC 0:00 をまたぐ range で StepForward/StepBackward → クラッシュなし
  TC-S16-03: Live ↔ Replay toggle 10 連打 → アプリ応答維持
  TC-S16-04: Playing 中の toggle → アプリ生存
  TC-S16-05a〜b: Paused 中の toggle（Live → Replay）→ アプリ生存

使い方:
    E2E_TICKER=BinanceLinear:BTCUSDT python tests/s16_replay_resilience.py
    IS_HEADLESS=true python tests/s16_replay_resilience.py
    pytest tests/s16_replay_resilience.py -v
"""

from __future__ import annotations

import sys
import time
from datetime import datetime, timezone, timedelta
from pathlib import Path

sys.path.insert(0, str(Path(__file__).parent))
from helpers import (
    TICKER, IS_HEADLESS, FlowsurfaceEnv,
    pass_, fail, pend, print_summary, reset_counters,
    _PASS, _FAIL, _PEND,
    backup_state, restore_state,
    setup_single_pane, headless_play, speed_to_10x,
    get_status, wait_status, wait_playing,
    api_post, api_post_code, api_get_code,
    utc_offset, utc_to_ms,
    STEP_M1,
)

import requests

# Access module-level counters for assertions
import helpers as _h


def run_s16() -> None:
    print(f"=== S16: UI操作中の Replay 耐性テスト (ticker={TICKER}) ===")

    # ── TC-S16-01: 速度ボタン連打 ────────────────────────────────────────────
    print("  [TC-S16-01] 速度ボタン連打...")
    start = utc_offset(-3)
    end = utc_offset(-1)
    setup_single_pane(TICKER, "M1", start, end)

    env1 = FlowsurfaceEnv(ticker=TICKER, timeframe="M1", headless=IS_HEADLESS)
    try:
        env1._start_process()
        headless_play()

        if not wait_playing(30):
            fail("TC-S16-01-pre", "Playing 到達せず")
        else:
            for _ in range(20):
                try:
                except requests.RequestException:
                    pass

            # CycleSpeed は Paused + range.start リセットを伴うため、連打後は Paused になる。
            # Resume して Playing に戻してから状態を確認する。
            try:
            except requests.RequestException:
                pass
            wait_status("Playing", 10)

            try:
                final_status = get_status().get("status")
            except requests.RequestException:
                final_status = None

            if final_status == "Playing":
                pass_("TC-S16-01: speed 20 連打後 Resume → status=Playing")
            else:
                fail("TC-S16-01", f"status={final_status} (Playing 期待)")
    finally:
        env1.close()

    # ── TC-S16-02: 日付境界（UTC 0:00 越え）────────────────────────────────
    print("  [TC-S16-02] 日付境界テスト（UTC 0:00 越え）...")

    # 前日 23:00 UTC ～ 当日 03:00 UTC の range を計算（Python datetime）
    now_utc = datetime.now(timezone.utc)
    yesterday = now_utc.replace(hour=0, minute=0, second=0, microsecond=0) - timedelta(days=1)
    midnight_minus_1 = (yesterday + timedelta(hours=23)).strftime("%Y-%m-%d %H:%M")
    today = now_utc.replace(hour=0, minute=0, second=0, microsecond=0)
    midnight_plus_1 = (today + timedelta(hours=3)).strftime("%Y-%m-%d %H:%M")
    t_midnight_ms = int(today.timestamp() * 1000)

    print(f"  range: {midnight_minus_1} → {midnight_plus_1}")
    setup_single_pane(TICKER, "M1", midnight_minus_1, midnight_plus_1)

    env2 = FlowsurfaceEnv(ticker=TICKER, timeframe="M1", headless=IS_HEADLESS)
    try:
        env2._start_process()
        headless_play(midnight_minus_1, midnight_plus_1)

        if not wait_playing(30):
            fail("TC-S16-02-pre", "Playing 到達せず（日付境界 range）")
            pend("TC-S16-02", "日付境界 range でデータなし / Playing 到達せず")
        else:
            try:
            except requests.RequestException:
                pass
            wait_status("Paused", 10)

            speed_to_10x()
            try:
            except requests.RequestException:
                pass
            wait_status("Playing", 10)

            # current_time が 0:00 UTC を超えるまで待機（最大 30 秒）
            crossed = False
            for _ in range(60):
                try:
                    ct = get_status().get("current_time")
                    if ct is not None and ct != "null" and int(ct) > t_midnight_ms:
                        crossed = True
                        break
                except (requests.RequestException, TypeError, ValueError):
                    pass
                time.sleep(0.5)

            try:
            except requests.RequestException:
                pass
            wait_status("Paused", 10)

            if crossed:
                # 0:00 越え後に StepForward → crash なし
                try:
                    ct_before = int(get_status().get("current_time") or 0)
                    api_post("/api/replay/step-forward")
                    wait_status("Paused", 10)
                    ct_after = int(get_status().get("current_time") or 0)
                    delta = ct_after - ct_before
                    if delta == 60000:
                        pass_("TC-S16-02a: UTC 0:00 越え後 StepForward delta=60000ms")
                    else:
                        fail("TC-S16-02a", f"delta={delta} (expected 60000)")

                    # StepBackward → crash なし
                    wait_status("Paused", 10)
                    status = get_status().get("status")
                    if status == "Paused":
                        pass_("TC-S16-02b: UTC 0:00 越え後 StepBackward → status=Paused")
                    else:
                        fail("TC-S16-02b", f"status={status}")
                except requests.RequestException as e:
                    fail("TC-S16-02", f"API error: {e}")
            else:
                pend("TC-S16-02", "UTC 0:00 境界を超えられなかった（データ不足 or 速度不足）")
    finally:
        env2.close()

    # ── TC-S16-03: Live ↔ Replay 高速切替 ────────────────────────────────
    print("  [TC-S16-03] Live ↔ Replay 高速切替...")
    if IS_HEADLESS:
        pend("TC-S16-03", "headless は Live/Replay toggle 非対応")
    else:
        setup_single_pane(TICKER, "M1", utc_offset(-3), utc_offset(-1))
        env3 = FlowsurfaceEnv(ticker=TICKER, timeframe="M1", headless=IS_HEADLESS)
        try:
            env3._start_process()

            if not wait_playing(30):
                fail("TC-S16-03-pre", "Playing 到達せず")
            else:
                for _ in range(10):
                    try:
                        api_post("/api/replay/toggle")
                    except requests.RequestException:
                        pass
                    time.sleep(0.2)

                # 最終状態が安定しているか（アプリが応答する）
                alive = api_get_code("/api/replay/status") != 0
                try:
                    final = get_status().get("status")
                except requests.RequestException:
                    final = "unknown"

                if alive:
                    pass_(f"TC-S16-03: toggle 10 連打後もアプリ応答あり (final_status={final})")
                else:
                    fail("TC-S16-03", "toggle 連打後にアプリが応答しなくなった")
        finally:
            env3.close()

    # ── TC-S16-04: Playing 中の toggle ───────────────────────────────────
    print("  [TC-S16-04] Playing 中の toggle...")
    if IS_HEADLESS:
        pend("TC-S16-04", "headless は Live/Replay toggle 非対応")
    else:
        setup_single_pane(TICKER, "M1", utc_offset(-3), utc_offset(-1))
        env4 = FlowsurfaceEnv(ticker=TICKER, timeframe="M1", headless=IS_HEADLESS)
        try:
            env4._start_process()

            if not wait_playing(30):
                fail("TC-S16-04-pre", "Playing 到達せず")
            else:
                try:
                    api_post("/api/replay/toggle")
                except requests.RequestException:
                    pass

                status_after = None
                for _ in range(10):
                    try:
                        s = get_status().get("status")
                        if s not in (None, "null", ""):
                            status_after = s
                            break
                    except requests.RequestException:
                        pass
                    time.sleep(0.2)

                alive = api_get_code("/api/replay/status") != 0
                if alive:
                    pass_(f"TC-S16-04: Playing 中の toggle → アプリ生存 (status={status_after})")
                else:
                    fail("TC-S16-04", "toggle 後にアプリが応答しなくなった")
        finally:
            env4.close()

    # ── TC-S16-05: Paused 中の toggle → Live → 再び Replay → Playing ────
    print("  [TC-S16-05] Paused 中の toggle...")
    if IS_HEADLESS:
        pend("TC-S16-05a", "headless は Live/Replay toggle 非対応")
        pend("TC-S16-05b", "headless は Live/Replay toggle 非対応")
    else:
        setup_single_pane(TICKER, "M1", utc_offset(-3), utc_offset(-1))
        env5 = FlowsurfaceEnv(ticker=TICKER, timeframe="M1", headless=IS_HEADLESS)
        try:
            env5._start_process()

            if not wait_playing(30):
                fail("TC-S16-05-pre", "Playing 到達せず")
            else:
                try:
                except requests.RequestException:
                    pass

                if not wait_status("Paused", 10):
                    fail("TC-S16-05-pre", "Paused に遷移せず")
                else:
                    # toggle → Live へ
                    try:
                        api_post("/api/replay/toggle")
                    except requests.RequestException:
                        pass

                    status_live = None
                    for _ in range(10):
                        try:
                            s = get_status().get("status")
                            if s not in (None, "null", ""):
                                status_live = s
                                break
                        except requests.RequestException:
                            pass
                        time.sleep(0.2)

                    alive = api_get_code("/api/replay/status") != 0
                    if alive:
                        pass_(f"TC-S16-05a: Paused → toggle → アプリ生存 (status={status_live})")
                    else:
                        fail("TC-S16-05a", "toggle 後にアプリが応答しなくなった")

                    # toggle → Replay に戻る
                    try:
                        api_post("/api/replay/toggle")
                    except requests.RequestException:
                        pass

                    for _ in range(10):
                        try:
                            get_status()
                            break
                        except requests.RequestException:
                            pass
                        time.sleep(0.2)

                    alive2 = api_get_code("/api/replay/status") != 0
                    try:
                        status_back = get_status().get("status")
                    except requests.RequestException:
                        status_back = "unknown"

                    if alive2:
                        pass_(f"TC-S16-05b: 2 回目 toggle 後もアプリ生存 (status={status_back})")
                    else:
                        fail("TC-S16-05b", "2 回目 toggle 後にアプリが応答しなくなった")
        finally:
            env5.close()


def test_s16_replay_resilience() -> None:
    """pytest エントリポイント。プロセス起動は外部で行うこと。"""
    _h._PASS = _h._FAIL = _h._PEND = 0
    run_s16()
    print_summary()
    assert _h._FAIL == 0, f"{_h._FAIL} TC(s) failed — see output above"


def main() -> None:
    backup_state()
    try:
        run_s16()
    finally:
        restore_state()
        print_summary()
        if _h._FAIL > 0:
            sys.exit(1)


if __name__ == "__main__":
    main()
