#!/usr/bin/env python3
"""s7_mid_replay_pane.py — Suite S7: Mid-replay ペイン CRUD

検証シナリオ:
  TC-S7-01: Playing 中に split (Vertical) → ペイン数 2
  TC-S7-02: 新ペインに ETHUSDT 設定 → streams_ready=true
  TC-S7-02b: split 後 PANE0 の streams_ready 維持
  TC-S7-03: split 後も Playing 継続
  TC-S7-04: 新ペインで set-timeframe M5 → streams_ready=true
  TC-S7-05: 新ペインを close → ペイン数 1
  TC-S7-06: close 後も Playing 継続
  TC-S7-07: range end 到達後に split してもクラッシュなし

仕様根拠:
  docs/replay_header.md §9 — 再生中の動的ペイン追加・削除

フィクスチャ: BinanceLinear:BTCUSDT M1, auto-play (UTC[-3h, -1h])
"""

from __future__ import annotations

import sys
import time
from pathlib import Path

sys.path.insert(0, str(Path(__file__).parent))
import requests

from helpers import (
    API_BASE,
    FlowsurfaceEnv,
    IS_HEADLESS,
    TICKER,
    api_get,
    api_post,
    api_post_code,
    backup_state,
    fail,
    find_other_pane_id,
    get_pane_id,
    headless_play,
    pass_,
    pend,
    primary_ticker,
    print_summary,
    restore_state,
    secondary_ticker,
    setup_single_pane,
    speed_to_10x,
    utc_offset,
    wait_for_pane_count,
    wait_for_pane_streams_ready,
    wait_playing,
    wait_status,
)


def run_s7() -> None:
    mode_label = "headless" if IS_HEADLESS else "GUI"
    print(f"=== S7: Mid-replay ペイン CRUD ({mode_label}) ===")

    start = utc_offset(-3)
    end = utc_offset(-1)
    primary = primary_ticker()
    secondary = secondary_ticker()

    # フィクスチャ書き込み（第 1 ラウンド）
    setup_single_pane(primary, "M1", start, end)

    env1 = FlowsurfaceEnv(ticker=TICKER, timeframe="M1", headless=IS_HEADLESS)
    try:
        env1._start_process()
        headless_play(start, end)

        if not wait_playing(60):
            fail("TC-S7-precond", "Playing 到達せず（60s タイムアウト）")
            return

        # 初期ペイン ID 取得
        pane0 = get_pane_id(0)
        if not pane0:
            fail("TC-S7-precond", "初期ペイン ID 取得失敗")
            return
        print(f"  PANE0={pane0}")

        # TC-S7-01: Playing 中に split（Vertical）→ ペイン数 2
        api_post("/api/pane/split", {"pane_id": pane0, "axis": "Vertical"})
        if wait_for_pane_count(2, 10):
            pass_("TC-S7-01: split 後ペイン数=2")
        else:
            fail("TC-S7-01", "10 秒以内にペイン数が 2 にならなかった")

        # TC-S7-02: 新ペインで set-ticker → streams_ready=true
        new_pane = find_other_pane_id(pane0)
        print(f"  NEW_PANE={new_pane}")
        if not new_pane:
            fail("TC-S7-02", "新ペイン ID 取得失敗")
        else:
            api_post("/api/pane/set-ticker", {"pane_id": new_pane, "ticker": secondary})
            if wait_for_pane_streams_ready(new_pane, 30):
                pass_(f"TC-S7-02: 新ペイン {secondary} streams_ready=true")
            else:
                fail("TC-S7-02", "streams_ready タイムアウト（30s）")

            # TC-S7-02b: 元ペインの streams_ready も維持されているか
            body = api_get("/api/pane/list")
            panes = body.get("panes", [])
            p0_entry = next((x for x in panes if x.get("id") == pane0), None)
            pane0_ready = p0_entry is not None and p0_entry.get("streams_ready") is True
            if pane0_ready:
                pass_("TC-S7-02b: split 後 PANE0 streams_ready 維持")
            else:
                fail("TC-S7-02b", f"PANE0 streams_ready={pane0_ready}")

        # TC-S7-03: Replay 継続確認
        status = api_get("/api/replay/status").get("status")
        if status == "Playing":
            pass_("TC-S7-03: split 後も Playing 継続")
        else:
            fail("TC-S7-03", f"status={status}")

        # TC-S7-04: 新ペインで set-timeframe M5 → streams_ready=true
        if new_pane:
            api_post("/api/pane/set-timeframe", {"pane_id": new_pane, "timeframe": "M5"})
            if wait_for_pane_streams_ready(new_pane, 30):
                pass_("TC-S7-04: M5 set-timeframe → streams_ready=true")
            else:
                fail("TC-S7-04", "streams_ready タイムアウト（30s）")
            # set-timeframe は ReloadKlineStream を発生させ clock.pause() を呼ぶ（新仕様）。
            # TC-S7-06 でペイン close 後も Playing 継続を確認するため、ここで Resume する。
            try:
            except Exception:
                pass
            wait_status("Playing", 10)

        # TC-S7-05: 新ペインを close → ペイン数 1
        if new_pane:
            api_post("/api/pane/close", {"pane_id": new_pane})
            if wait_for_pane_count(1, 10):
                pass_("TC-S7-05: close 後ペイン数=1")
            else:
                fail("TC-S7-05", "10 秒以内にペイン数が 1 にならなかった")

        # TC-S7-06: Replay 継続確認
        status2 = api_get("/api/replay/status").get("status")
        if status2 == "Playing":
            pass_("TC-S7-06: close 後も Playing 継続")
        else:
            fail("TC-S7-06", f"status={status2}")

    finally:
        env1.close()

    # TC-S7-07: range end 到達後に split してもクラッシュしない
    # 短い range（1 時間）+ 10x 速度 → 約 6 分で end に到達
    start_short = utc_offset(-2)
    end_short = utc_offset(-1)
    setup_single_pane(primary, "M1", start_short, end_short)

    env2 = FlowsurfaceEnv(ticker=TICKER, timeframe="M1", headless=IS_HEADLESS)
    try:
        env2._start_process()
        headless_play(start_short, end_short)

        if not wait_playing(60):
            fail("TC-S7-07-pre", "Playing 到達せず（S7b, 60s タイムアウト）")
            return

        print("  10x 速度で range end を待機（最大 480 秒）...")
        speed_to_10x()
        if not wait_status("Paused", 480):
            fail("TC-S7-07-pre", "range end 到達せず（480 秒タイムアウト）")
            return

        # range end 到達後に split
        last_pane = get_pane_id(0)
        http_code = api_post_code("/api/pane/split", {"pane_id": last_pane, "axis": "Vertical"})

        # エラー通知チェック
        try:
            notifs = requests.get(f"{API_BASE}/api/notification/list", timeout=5).json()
        except Exception:
            notifs = {"notifications": []}
        has_err = any(
            n.get("level") == "error"
            for n in notifs.get("notifications", [])
        )

        if http_code == 200 and not has_err:
            pass_(f"TC-S7-07: range end 後 split → crash なし (HTTP={http_code}, error_toast={has_err})")
        else:
            fail("TC-S7-07", f"HTTP={http_code}, error_toast={has_err}")

    finally:
        env2.close()


def test_s7_mid_replay_pane() -> None:
    """pytest エントリポイント。"""
    import helpers as _h
    _h._PASS = _h._FAIL = _h._PEND = 0
    backup_state()
    try:
        run_s7()
    finally:
        restore_state()
    print_summary()
    assert _h._FAIL == 0, f"{_h._FAIL} TC(s) failed — see output above"


def main() -> None:
    backup_state()
    try:
        run_s7()
    finally:
        restore_state()
        print_summary()
        import helpers as _h
        if _h._FAIL > 0:
            sys.exit(1)


if __name__ == "__main__":
    main()
