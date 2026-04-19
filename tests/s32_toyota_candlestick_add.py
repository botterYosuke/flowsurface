#!/usr/bin/env python3
"""s32_toyota_candlestick_add.py — S32: TOYOTA candlestick チャート追加テスト

検証シナリオ:
  TC-S32-01: auto-play → Playing 到達
  TC-S32-02: ペイン split → pane count = 2
  TC-S32-03: 新ペインに set-ticker TachibanaSpot:7203 → HTTP 200
  TC-S32-04: 新ペインに set-timeframe D1 → HTTP 200
  TC-S32-05: current_time == start_time（clock.seek(range.start) が発火）
  TC-S32-06: status = Paused（自動再生しない）
  TC-S32-07: 新ペインの ticker に 7203、timeframe == D1
  TC-S32-08: 新ペイン streams_ready = true（Tachibana セッションあり時のみ）
  TC-S32-09: Resume → Playing（Tachibana セッションあり時のみ）
  TC-S32-10: current_time 前進（Tachibana セッションあり時のみ）

仕様根拠:
  docs/replay_header.md §6.6 — 銘柄変更による初期状態リセット（seek(range.start) 発火）

使い方:
    python tests/s32_toyota_candlestick_add.py
    pytest tests/s32_toyota_candlestick_add.py -v
"""

from __future__ import annotations

import json
import sys
import time
from pathlib import Path

sys.path.insert(0, str(Path(__file__).parent))
from helpers import (
    IS_HEADLESS, FlowsurfaceEnv,
    pass_, fail, pend, print_summary,
    backup_state, restore_state,
    get_status, wait_status, wait_playing, wait_for_pane_streams_ready,
    wait_for_pane_count, wait_for_time_advance,
    api_get, api_post, api_get_code,
    get_pane_id, find_other_pane_id,
    headless_play,
    primary_ticker,
    utc_offset,
    DATA_DIR, STATE_FILE, API_BASE,
)

import requests
import helpers as _h


def run_s32() -> None:
    print("=== S32: TOYOTA candlestick チャート追加テスト ===")

    start = utc_offset(-5)
    end = utc_offset(-1)
    primary = primary_ticker()

    # ── フィクスチャ: PRIMARY M1 with replay ──────────────────────────────────
    DATA_DIR.mkdir(parents=True, exist_ok=True)
    fixture = {
        "layout_manager": {"layouts": [{"name": "Test-M1", "dashboard": {"pane": {
            "KlineChart": {
                "layout": {"splits": [0.78], "autoscale": "FitToVisible"}, "kind": "Candles",
                "stream_type": [{"Kline": {"ticker": primary, "timeframe": "M1"}}],
                "settings": {"tick_multiply": None, "visual_config": None, "selected_basis": {"Time": "M1"}},
                "indicators": ["Volume"], "link_group": "A"
            }
        }, "popout": []}}], "active_layout": "Test-M1"},
        "timezone": "UTC", "trade_fetch_enabled": False, "size_in_quote_ccy": "Base",
        "replay": {"mode": "replay", "range_start": start, "range_end": end}
    }
    STATE_FILE.write_text(json.dumps(fixture, indent=2))
    print(f"  fixture: {primary} M1, replay {start} → {end}")

    env = FlowsurfaceEnv(ticker=primary, timeframe="M1", headless=IS_HEADLESS)
    env._start_process()
    try:
        headless_play(start, end)

        # ── TC-S32-01: auto-play → Playing 到達 ──────────────────────────────
        print("\n── TC-S32-01: auto-play → Playing 到達")
        if wait_playing(120):
            pass_("TC-S32-01: auto-play → Playing 到達")
        else:
            try:
                status_str = get_status().get("status")
            except requests.RequestException:
                status_str = "unknown"
            fail("TC-S32-01", f"Playing 未到達（120 秒タイムアウト）: status={status_str}")
            return

        # start_time を API から取得（基準値として使用）
        try:
            status_resp = get_status()
            start_time_ms = status_resp.get("start_time")
            print(f"  start_time_ms={start_time_ms}  (= {start} UTC)")
        except requests.RequestException:
            start_time_ms = None

        # 初期ペイン ID 取得
        pane0 = get_pane_id(0)
        if not pane0:
            fail("TC-S32-precond", "初期ペイン ID 取得失敗")
            return
        print(f"  PANE0={pane0}")

        # ── Tachibana セッション確認（inject-session 試行 → keyring フォールバック）──
        print("\n── Tachibana セッション確認")
        tach_session = "none"
        try:
            r = requests.post(f"{API_BASE}/api/test/tachibana/inject-session", timeout=5)
            if r.status_code == 200:
                try:
                    tach_status = api_get("/api/auth/tachibana/status")
                    tach_session = tach_status.get("session", "none")
                except requests.RequestException:
                    tach_session = "none"
                print(f"  inject-session 成功 → session={tach_session}")
            else:
                try:
                    tach_status = api_get("/api/auth/tachibana/status")
                    tach_session = tach_status.get("session", "none")
                except requests.RequestException:
                    tach_session = "none"
                print(f"  inject-session 利用不可 (HTTP={r.status_code}) → session={tach_session}")
        except requests.RequestException:
            try:
                tach_status = api_get("/api/auth/tachibana/status")
                tach_session = tach_status.get("session", "none")
            except requests.RequestException:
                tach_session = "none"

        if tach_session == "none":
            print("  INFO: Tachibana セッションなし — TC-S32-03 以降を全て PEND として早期終了")
            for tc in ["TC-S32-03", "TC-S32-04", "TC-S32-05", "TC-S32-06",
                       "TC-S32-07", "TC-S32-08", "TC-S32-09", "TC-S32-10"]:
                pend(tc, "Tachibana セッション不在（TachibanaSpot:7203 set-ticker 不可）")
            return

        # ── TC-S32-02: ペイン split → pane count = 2 ─────────────────────────
        print("\n── TC-S32-02: ペイン split → pane count = 2")
        try:
            api_post("/api/pane/split", {"pane_id": pane0, "axis": "Vertical"})
        except requests.RequestException as e:
            fail("TC-S32-02", f"split API error: {e}")
            return

        if wait_for_pane_count(2, 10):
            pass_("TC-S32-02: split 後 pane count = 2")
        else:
            fail("TC-S32-02", "10 秒以内に pane count が 2 にならなかった")
            return

        new_pane = find_other_pane_id(pane0)
        print(f"  NEW_PANE={new_pane}")
        if not new_pane:
            fail("TC-S32-02b", "新ペイン ID 取得失敗")
            return

        # ── TC-S32-03: 新ペインに set-ticker TachibanaSpot:7203 ──────────────
        # Tachibana マスタのダウンロードが set-ticker より先行しない場合に 404 が返ることがある。
        # 最大 60 秒リトライして、メタデータロード完了後に 200 になることを確認する。
        print("\n── TC-S32-03: 新ペインに set-ticker TachibanaSpot:7203")
        set_ticker_code = 0
        end_time = time.monotonic() + 60
        while time.monotonic() < end_time:
            try:
                r = requests.post(
                    f"{API_BASE}/api/pane/set-ticker",
                    json={"pane_id": new_pane, "ticker": "TachibanaSpot:7203"},
                    timeout=5
                )
                set_ticker_code = r.status_code
                if set_ticker_code == 200:
                    break
            except requests.RequestException:
                set_ticker_code = 0
            time.sleep(1)

        if set_ticker_code == 200:
            pass_("TC-S32-03: set-ticker TachibanaSpot:7203 → HTTP 200")
        else:
            fail("TC-S32-03", f"HTTP={set_ticker_code} (expected 200)")

        # ── TC-S32-04: 新ペインに set-timeframe D1 ─────────────────────────
        print("\n── TC-S32-04: 新ペインに set-timeframe D1")
        try:
            r = requests.post(
                f"{API_BASE}/api/pane/set-timeframe",
                json={"pane_id": new_pane, "timeframe": "D1"},
                timeout=5
            )
            set_tf_code = r.status_code
        except requests.RequestException:
            set_tf_code = 0

        if set_tf_code == 200:
            pass_("TC-S32-04: set-timeframe D1 → HTTP 200")
        else:
            fail("TC-S32-04", f"HTTP={set_tf_code} (expected 200)")

        # ticker/timeframe 変更が反映されるまで少し待機
        time.sleep(1)

        # ── TC-S32-05: current_time == start_time（clock.seek が発火）──────────
        print("\n── TC-S32-05: current_time == start_time（Replay が start に戻る）")
        try:
            status_after = get_status()
            ct_val = status_after.get("current_time")
            st_val = status_after.get("start_time")
            ct = int(ct_val) if ct_val not in (None, "null", "") else None
            st = int(st_val) if st_val not in (None, "null", "") else None
        except (requests.RequestException, TypeError, ValueError):
            ct = None
            st = None
        print(f"  current_time={ct}  start_time={st}")

        if ct is not None and st is not None:
            if ct == st:
                pass_("TC-S32-05: current_time == start_time (clock.seek が正しく発火)")
            else:
                fail("TC-S32-05", f"current_time={ct} != start_time={st} (expected clock.seek(range.start))")
        else:
            fail("TC-S32-05", f"current_time または start_time が null (CT={ct}, ST={st})")

        # ── TC-S32-06: status = Paused（自動再生しない）──────────────────────
        print("\n── TC-S32-06: ticker 変更後 status = Paused")
        try:
            status_str = status_after.get("status")
        except Exception:
            status_str = None

        if status_str == "Paused":
            pass_("TC-S32-06: status = Paused（自動再生なし）")
        else:
            fail("TC-S32-06", f"status={status_str} (expected Paused)")

        # ── TC-S32-07: 新ペインの ticker/timeframe が正しく設定されている ─────
        print("\n── TC-S32-07: 新ペインの ticker/timeframe 確認")
        try:
            panes_after = api_get("/api/pane/list").get("panes", [])
            new_p = next((p for p in panes_after if p.get("id") == new_pane), None)
            new_ticker = new_p.get("ticker", "null") if new_p else "not_found"
            new_tf = new_p.get("timeframe", "null") if new_p else "not_found"
        except requests.RequestException:
            new_ticker = "error"
            new_tf = "error"
        print(f"  new pane ticker={new_ticker}  timeframe={new_tf}")

        # pane/list は ticker を正規化して返す（"Tachibana:7203" 形式）
        if "7203" in str(new_ticker):
            pass_(f"TC-S32-07a: 新ペイン ticker に 7203 が含まれる (={new_ticker})")
        else:
            fail("TC-S32-07a", f"ticker={new_ticker} (expected to contain '7203')")

        if new_tf == "D1":
            pass_("TC-S32-07b: 新ペイン timeframe = D1")
        else:
            fail("TC-S32-07b", f"timeframe={new_tf} (expected D1)")

        # ── TC-S32-08〜10: Tachibana セッションあり時のみ実行 ─────────────────
        print()
        # TC-S32-08: streams_ready = true（TOYOTA D1 データロード完了）
        print("── TC-S32-08: 新ペイン streams_ready = true を待機（Tachibana D1）")
        if wait_for_pane_streams_ready(new_pane, 120):
            pass_("TC-S32-08: TachibanaSpot:7203 D1 streams_ready = true")
        else:
            fail("TC-S32-08", "streams_ready タイムアウト（120 秒）— Tachibana D1 データロード失敗")

        # TC-S32-09: Resume → Playing
        print("\n── TC-S32-09: Resume → Playing")
        try:
            api_post("/api/replay/resume")
        except requests.RequestException:
            pass
        if wait_status("Playing", 30):
            pass_("TC-S32-09: Resume → Playing 到達")
        else:
            try:
                status_str2 = get_status().get("status")
            except requests.RequestException:
                status_str2 = "unknown"
            fail("TC-S32-09", f"status={status_str2} (expected Playing)")

        # TC-S32-10: current_time が前進（再生が正常動作）
        print("\n── TC-S32-10: current_time が前進")
        try:
            t1_raw = get_status().get("current_time")
            t1 = int(t1_raw) if t1_raw not in (None, "null", "") else 0
        except (requests.RequestException, TypeError, ValueError):
            t1 = 0

        t2 = wait_for_time_advance(t1, 15)
        if t2 is not None:
            pass_(f"TC-S32-10: current_time 前進 ({t1} → {t2})")
        else:
            fail("TC-S32-10", "15 秒待機しても current_time が変化しなかった")

    finally:
        env.close()


def test_s32_toyota_candlestick_add() -> None:
    """pytest エントリポイント。"""
    _h._PASS = _h._FAIL = _h._PEND = 0
    backup_state()
    try:
        run_s32()
    finally:
        restore_state()
    print_summary()
    assert _h._FAIL == 0, f"{_h._FAIL} TC(s) failed — see output above"


def main() -> None:
    backup_state()
    try:
        run_s32()
    finally:
        restore_state()
        print_summary()
        if _h._FAIL > 0:
            sys.exit(1)


if __name__ == "__main__":
    main()
