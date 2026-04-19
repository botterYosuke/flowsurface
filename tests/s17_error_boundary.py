#!/usr/bin/env python3
"""s17_error_boundary.py — スイート S17: クラッシュ・エラー境界テスト

検証シナリオ:
  TC-S17-01〜03: 存在しない pane_id（pane/split, pane/close, pane/set-ticker）→ HTTP 404 + アプリ生存
  TC-S17-04: 空 range (start == end) でもアプリ生存
  TC-S17-05: 未来の range でもアプリ生存
  TC-S17-06: StepForward 50 連打（Paused 状態）→ crash なし・status=Paused
  TC-S17-07: split 上限到達後もクラッシュなし

使い方:
    E2E_TICKER=BinanceLinear:BTCUSDT python tests/s17_error_boundary.py
    IS_HEADLESS=true python tests/s17_error_boundary.py
    pytest tests/s17_error_boundary.py -v
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
    setup_single_pane, headless_play,
    get_status, wait_status, wait_playing,
    api_post, api_post_code, api_get, api_get_code,
    utc_offset,
    primary_ticker, secondary_ticker,
)

import requests
import helpers as _h

FAKE_UUID = "ffffffff-ffff-ffff-ffff-ffffffffffff"


def is_alive() -> bool:
    return api_get_code("/api/replay/status") not in (0,)


def run_s17() -> None:
    print(f"=== S17: クラッシュ・エラー境界テスト (ticker={TICKER}) ===")

    # ── TC-S17-01〜03: 存在しない pane_id ─────────────────────────────────
    print("  [TC-S17-01/03] 不正 pane_id テスト...")
    start = utc_offset(-3)
    end = utc_offset(-1)
    setup_single_pane(primary_ticker(), "M1", start, end)

    env1 = FlowsurfaceEnv(ticker=TICKER, timeframe="M1", headless=IS_HEADLESS)
    try:
        env1._start_process()
        headless_play()

        if not wait_playing(60):
            fail("TC-S17-precond", "Playing 到達せず（60s タイムアウト）")
        else:
            # TC-S17-01: pane/split に存在しない UUID → HTTP 200 or 404 + error フィールド
            http_split = api_post_code(
                "/api/pane/split",
                {"pane_id": FAKE_UUID, "axis": "Vertical"},
            )
            alive = is_alive()
            if http_split in (200, 404) and alive:
                pass_(f"TC-S17-01a: pane/split 存在しない UUID → HTTP={http_split} & アプリ生存")
            else:
                fail("TC-S17-01a", f"HTTP={http_split} alive={alive}")

            try:
                body_split = api_post("/api/pane/split", {"pane_id": FAKE_UUID, "axis": "Vertical"})
                has_err_split = bool(body_split.get("error"))
            except requests.RequestException:
                # 404 raises for non-2xx
                has_err_split = True
            if has_err_split:
                pass_("TC-S17-01b: pane/split 不正 UUID → error フィールドあり")
            else:
                fail("TC-S17-01b", "error フィールドなし")

            # TC-S17-02: pane/close に存在しない UUID
            http_close = api_post_code("/api/pane/close", {"pane_id": FAKE_UUID})
            alive = is_alive()
            if http_close in (200, 404) and alive:
                pass_(f"TC-S17-02a: pane/close 存在しない UUID → HTTP={http_close} & アプリ生存")
            else:
                fail("TC-S17-02a", f"HTTP={http_close} alive={alive}")

            try:
                body_close = api_post("/api/pane/close", {"pane_id": FAKE_UUID})
                has_err_close = bool(body_close.get("error"))
            except requests.RequestException:
                has_err_close = True
            if has_err_close:
                pass_("TC-S17-02b: pane/close 不正 UUID → error フィールドあり")
            else:
                fail("TC-S17-02b", "error フィールドなし")

            # TC-S17-03: pane/set-ticker に存在しない UUID
            http_ticker = api_post_code(
                "/api/pane/set-ticker",
                {"pane_id": FAKE_UUID, "ticker": secondary_ticker()},
            )
            alive = is_alive()
            if http_ticker in (200, 404) and alive:
                pass_(f"TC-S17-03a: pane/set-ticker 存在しない UUID → HTTP={http_ticker} & アプリ生存")
            else:
                fail("TC-S17-03a", f"HTTP={http_ticker} alive={alive}")

            try:
                body_ticker = api_post(
                    "/api/pane/set-ticker",
                    {"pane_id": FAKE_UUID, "ticker": secondary_ticker()},
                )
                has_err_ticker = bool(body_ticker.get("error"))
            except requests.RequestException:
                has_err_ticker = True
            if has_err_ticker:
                pass_("TC-S17-03b: pane/set-ticker 不正 UUID → error フィールドあり")
            else:
                fail("TC-S17-03b", "error フィールドなし")

            # TC-S17-03c: pane 全削除後 最終ペイン 1 つ残存確認
            try:
                panes_body = api_get("/api/pane/list")
                panes = panes_body.get("panes", [])
                pane_id_0 = panes[0].get("id", "") if panes else ""
            except requests.RequestException:
                pane_id_0 = ""

            if pane_id_0:
                # split して 2 ペインにしてから両方 close
                api_post_code("/api/pane/split", {"pane_id": pane_id_0, "axis": "Vertical"})
                time.sleep(0.3)
                try:
                    panes2_body = api_get("/api/pane/list")
                    pane_ids = [p.get("id", "") for p in panes2_body.get("panes", [])]
                except requests.RequestException:
                    pane_ids = []

                for pid in pane_ids:
                    api_post_code("/api/pane/close", {"pane_id": pid})
                    time.sleep(0.3)

                time.sleep(0.5)
                try:
                    panes_after = api_get("/api/pane/list")
                    count = len(panes_after.get("panes", []))
                except requests.RequestException:
                    count = -1

                if count == 1:
                    pass_("TC-S17-03c: 全 pane close 後 最終ペイン1つ残存 (count=1, iced pane_grid 仕様)")
                else:
                    fail("TC-S17-03c", f"count={count} (expected 1)")
            else:
                fail("TC-S17-03c-pre", "ペイン ID 取得失敗")
    finally:
        env1.close()

    # ── TC-S17-04: 空 range (start == end) ─────────────────────────────
    print("  [TC-S17-04] 空 range (start == end)...")
    same_time = utc_offset(-1)
    setup_single_pane(primary_ticker(), "M1", same_time, same_time)

    env2 = FlowsurfaceEnv(ticker=TICKER, timeframe="M1", headless=IS_HEADLESS)
    try:
        env2._start_process()
        headless_play(same_time, same_time)

        time.sleep(5)
        alive = is_alive()
        if alive:
            try:
                status = get_status().get("status")
            except requests.RequestException:
                status = "unknown"
            pass_(f"TC-S17-04: 空 range でもアプリ生存 (status={status})")
        else:
            fail("TC-S17-04", "空 range でアプリがクラッシュした")
    finally:
        env2.close()

    # ── TC-S17-05: 未来の range (現在時刻 + 24h 先) ─────────────────────
    print("  [TC-S17-05] 未来 range テスト...")
    future_start = utc_offset(24)
    future_end = utc_offset(26)
    setup_single_pane(primary_ticker(), "M1", future_start, future_end)

    env3 = FlowsurfaceEnv(ticker=TICKER, timeframe="M1", headless=IS_HEADLESS)
    try:
        env3._start_process()
        headless_play(future_start, future_end)

        time.sleep(10)
        alive = is_alive()
        if alive:
            try:
                status = get_status().get("status")
            except requests.RequestException:
                status = "unknown"
            pass_(f"TC-S17-05: 未来 range でもアプリ生存 (status={status})")
        else:
            fail("TC-S17-05", "未来 range でアプリがクラッシュした")
    finally:
        env3.close()

    # ── TC-S17-06: StepForward 連打 50 回 ────────────────────────────────
    print("  [TC-S17-06] StepForward 連打 50 回...")
    setup_single_pane(primary_ticker(), "M1", utc_offset(-3), utc_offset(-1))

    env4 = FlowsurfaceEnv(ticker=TICKER, timeframe="M1", headless=IS_HEADLESS)
    try:
        env4._start_process()
        headless_play()

        if not wait_playing(60):
            fail("TC-S17-06-pre", "Playing 到達せず（60s タイムアウト）")
        else:
            try:
                api_post("/api/replay/pause")
            except requests.RequestException:
                pass

            if not wait_status("Paused", 10):
                fail("TC-S17-06-pre", "Paused に遷移せず")
            else:
                crashed = False
                for i in range(1, 51):
                    try:
                        api_post("/api/replay/step-forward")
                    except requests.RequestException:
                        pass
                    time.sleep(0.3)
                    if api_get_code("/api/replay/status") == 0:
                        crashed = True
                        break

                wait_status("Paused", 15)
                try:
                    status = get_status().get("status")
                except requests.RequestException:
                    status = "unknown"
                alive = is_alive()

                if not crashed and alive and status == "Paused":
                    pass_("TC-S17-06: StepForward 50 連打 → crash なし, status=Paused")
                else:
                    fail("TC-S17-06", f"crash={crashed} alive={alive} status={status}")
    finally:
        env4.close()

    # ── TC-S17-07: split 上限テスト ──────────────────────────────────────
    print("  [TC-S17-07] split 上限テスト...")
    setup_single_pane(primary_ticker(), "M1", utc_offset(-3), utc_offset(-1))

    env5 = FlowsurfaceEnv(ticker=TICKER, timeframe="M1", headless=IS_HEADLESS)
    try:
        env5._start_process()
        headless_play()

        if not wait_playing(60):
            fail("TC-S17-07-pre", "Playing 到達せず（60s タイムアウト）")
        else:
            split_count = 0
            last_http = 200

            for _ in range(10):
                try:
                    panes_body = api_get("/api/pane/list")
                    panes = panes_body.get("panes", [])
                    first_pane = panes[0].get("id", "") if panes else ""
                except requests.RequestException:
                    first_pane = ""

                if not first_pane:
                    break

                last_http = api_post_code(
                    "/api/pane/split",
                    {"pane_id": first_pane, "axis": "Vertical"},
                )
                split_count += 1
                time.sleep(0.5)

                if last_http != 200:
                    break

            alive = is_alive()
            try:
                panes_final = api_get("/api/pane/list")
                pane_count = len(panes_final.get("panes", []))
            except requests.RequestException:
                pane_count = -1

            if alive:
                pass_(f"TC-S17-07: split {split_count} 回後 (HTTP={last_http}) クラッシュなし (panes={pane_count})")
            else:
                fail("TC-S17-07", "split 繰り返し後にアプリがクラッシュした")
    finally:
        env5.close()


def test_s17_error_boundary() -> None:
    """pytest エントリポイント。"""
    _h._PASS = _h._FAIL = _h._PEND = 0
    backup_state()
    try:
        run_s17()
    finally:
        restore_state()
    print_summary()
    assert _h._FAIL == 0, f"{_h._FAIL} TC(s) failed — see output above"


def main() -> None:
    backup_state()
    try:
        run_s17()
    finally:
        restore_state()
        print_summary()
        if _h._FAIL > 0:
            sys.exit(1)


if __name__ == "__main__":
    main()
