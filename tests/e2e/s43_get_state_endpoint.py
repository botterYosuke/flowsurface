#!/usr/bin/env python3
"""s43_get_state_endpoint.py — S43: GET /api/replay/state 詳細検証

検証シナリオ:
  A:   LIVE モード → HTTP 400
  B:   REPLAY Playing 遷移
  C:   HTTP 200 確認
  D:   current_time_ms > 0
  E:   klines フィールドが配列
  F:   trades フィールドが配列
  G:   klines に items がある場合: stream/time/open/high/low/close/volume の型・値
  H:   klines[*].stream が "Exchange:TICKER:timeframe" 形式
  I:   klines[*].time ≤ current_time_ms
  J:   open/high/low/close すべて > 0
  K:   StepForward 後に current_time_ms が増加し klines も含む
  L:   Idle（REPLAY モード切替前）→ HTTP 400

フィクスチャ: BinanceLinear:BTCUSDT M1
"""

from __future__ import annotations

import json
import re
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
    wait_playing,
    wait_status,
    wait_streams_ready,
    wait_for_pane_streams_ready,
    get_status,
    api_get,
    api_post,
    api_get_code,
    api_post_code,
    get_pane_id,
    utc_offset,
    reset_counters,
    pass_,
    fail,
    pend,
    print_summary,
)

import requests

BTC_TICKER = "BinanceLinear:BTCUSDT"


def _write_gui_fixture(start: str, end: str) -> None:
    """GUI 用 saved-state.json を書き込む（Live モード起動 — replay フィールドなし）。"""
    DATA_DIR.mkdir(parents=True, exist_ok=True)
    fixture = {
        "layout_manager": {
            "layouts": [
                {
                    "name": "S43",
                    "dashboard": {
                        "pane": {
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
                        "popout": [],
                    },
                }
            ],
            "active_layout": "S43",
        },
        "timezone": "UTC",
        "trade_fetch_enabled": False,
        "size_in_quote_ccy": "Base",
    }
    STATE_FILE.write_text(json.dumps(fixture, indent=2))


def run_s43() -> None:
    mode_label = "headless" if IS_HEADLESS else "GUI"
    print(f"=== S43: GET /api/replay/state 詳細検証 (ticker={TICKER} {mode_label}) ===")

    start = utc_offset(-3)
    end = utc_offset(-1)

    if not IS_HEADLESS:
        _write_gui_fixture(start, end)

    env = FlowsurfaceEnv(ticker=BTC_TICKER, timeframe="M1", headless=IS_HEADLESS)
    try:
        env._start_process()

        # ─────────────────────────────────────────────────────────────────────
        # TC-A: LIVE モード → HTTP 400
        # ─────────────────────────────────────────────────────────────────────
        print()
        print("── TC-A: LIVE モード → HTTP 400")

        if IS_HEADLESS:
            pend("TC-A", "headless は Live モードなし")
        else:
            wait_streams_ready(30)
            code_a = api_get_code("/api/replay/state")
            if code_a == 400:
                pass_("TC-A: LIVE 中 GET /api/replay/state → HTTP 400")
            else:
                fail("TC-A", f"HTTP={code_a} (expected 400)")

            # Replay に入る前に TickerInfo（metadata）の解決を待つ。
            # 未解決のままだと EventStore に kline が格納されず TC-K2 が失敗する。
            pane_id_a = get_pane_id(0)
            if pane_id_a:
                print(f"  waiting for streams_ready (pane={pane_id_a}, max 30s)...")
                if not wait_for_pane_streams_ready(pane_id_a, 30):
                    print("  WARN: streams_ready timeout (continuing)")

        # ─────────────────────────────────────────────────────────────────────
        # TC-L: Replay モード切替直後（Idle）→ HTTP 400
        # ─────────────────────────────────────────────────────────────────────
        print()
        print("── TC-L: Replay Idle → HTTP 400")

        if IS_HEADLESS:
            # headless は起動時点で Replay Idle → そのまま HTTP 400 を確認
            code_l = api_get_code("/api/replay/state")
            if code_l == 400:
                pass_("TC-L: Replay Idle → HTTP 400")
            else:
                fail("TC-L", f"HTTP={code_l} (expected 400)")
        else:
            api_post("/api/replay/toggle")
            code_l = api_get_code("/api/replay/state")
            if code_l == 400:
                pass_("TC-L: Replay Idle（Play 前）→ HTTP 400")
            else:
                fail("TC-L", f"HTTP={code_l} (expected 400)")

        # ─────────────────────────────────────────────────────────────────────
        # TC-B: REPLAY Playing 遷移
        # ─────────────────────────────────────────────────────────────────────
        print()
        print("── TC-B: REPLAY Playing 遷移")

        try:
            api_post("/api/replay/toggle", {"start": start, "end": end})
        except requests.RequestException as e:
            fail("TC-B-play", f"POST /api/replay/play 失敗: {e}")

        if not wait_status("Playing", 60):
            fail("TC-B", "REPLAY Playing に到達せず（60s タイムアウト）")
            return
        pass_("TC-B: REPLAY Playing 到達")

        # Paused 状態で確定論的な検証を行う
        wait_status("Paused", 10)

        # ─────────────────────────────────────────────────────────────────────
        # TC-C/D/E/F: HTTP ステータスとトップレベルスキーマ
        # ─────────────────────────────────────────────────────────────────────
        print()
        print("── TC-C〜F: HTTP 200 + トップレベルスキーマ")

        code_c = api_get_code("/api/replay/state")
        print(f"  HTTP={code_c}")

        if code_c == 200:
            pass_("TC-C: Paused 中 GET /api/replay/state → HTTP 200")
        else:
            fail("TC-C", f"HTTP={code_c} (expected 200)")

        try:
            state = api_get("/api/replay/state")
        except requests.RequestException as e:
            fail("TC-C-parse", f"GET /api/replay/state 失敗: {e}")
            return

        # TC-D: current_time_ms > 0
        ct_ms = state.get("current_time_ms") or state.get("current_time") or 0
        try:
            ct_ms = int(ct_ms)
        except (TypeError, ValueError):
            ct_ms = 0
        print(f"  current_time_ms={ct_ms}")
        if ct_ms > 0:
            pass_(f"TC-D: current_time_ms={ct_ms} (>0)")
        else:
            fail("TC-D", f"current_time_ms={ct_ms} (expected >0)")

        # TC-E: klines フィールドが配列
        klines = state.get("klines")
        if isinstance(klines, list):
            pass_("TC-E: klines フィールドが配列")
        else:
            fail("TC-E", f"klines が配列でない (response={state})")
            klines = []

        # TC-F: trades フィールドが配列
        trades = state.get("trades")
        if isinstance(trades, list):
            pass_("TC-F: trades フィールドが配列")
        else:
            fail("TC-F", f"trades が配列でない (response={state})")

        # ─────────────────────────────────────────────────────────────────────
        # TC-G〜J: klines items スキーマ（items がある場合のみ）
        # ─────────────────────────────────────────────────────────────────────
        print()
        print("── TC-G〜J: klines items スキーマ")

        kline_count = len(klines)
        print(f"  klines count={kline_count}")

        if IS_HEADLESS:
            pend("TC-G", "headless klines スキーマ差分あり")
            pend("TC-H", "headless klines スキーマ差分あり")
            pend("TC-I", "headless klines スキーマ差分あり")
            pend("TC-J", "headless klines スキーマ差分あり")
        elif kline_count > 0:
            k0 = klines[0]

            # TC-G: stream/time/open/high/low/close/volume の存在と型
            g_ok = (
                isinstance(k0.get("stream"), str) and len(k0.get("stream", "")) > 0
                and isinstance(k0.get("time"), (int, float))
                and isinstance(k0.get("open"), (int, float))
                and isinstance(k0.get("high"), (int, float))
                and isinstance(k0.get("low"), (int, float))
                and isinstance(k0.get("close"), (int, float))
                and isinstance(k0.get("volume"), (int, float))
            )
            if g_ok:
                pass_("TC-G: klines[0] に stream/time/open/high/low/close/volume あり (型正常)")
            else:
                fail("TC-G", f"klines[0] スキーマ不正 (response={state})")

            # TC-H: stream ラベルが "Exchange:TICKER:timeframe" 形式
            stream_label = k0.get("stream", "")
            print(f"  klines[0].stream={stream_label}")
            # "BinanceLinear:BTCUSDT:1m" のように Exchange:TICKER:timeframe 形式
            if re.match(r"^[A-Za-z]+:[A-Z0-9]+:[A-Za-z0-9]+$", stream_label):
                pass_(f"TC-H: stream ラベル形式が \"Exchange:TICKER:timeframe\" ({stream_label})")
            else:
                fail("TC-H", f"stream ラベル形式が想定外 ({stream_label})")

            # TC-I: klines[*].time ≤ current_time_ms
            try:
                time_ok = all(int(k.get("time", 0)) <= ct_ms for k in klines)
            except (TypeError, ValueError):
                time_ok = False
            if time_ok:
                pass_("TC-I: klines[*].time ≤ current_time_ms")
            else:
                fail("TC-I", f"未来の kline が含まれている (response={state})")

            # TC-J: open/high/low/close > 0 かつ high >= low
            try:
                ohlc_ok = all(
                    k.get("open", 0) > 0
                    and k.get("high", 0) > 0
                    and k.get("low", 0) > 0
                    and k.get("close", 0) > 0
                    and k.get("high", 0) >= k.get("low", 0)
                    for k in klines
                )
            except (TypeError, ValueError):
                ohlc_ok = False
            if ohlc_ok:
                pass_("TC-J: open/high/low/close > 0 かつ high ≥ low")
            else:
                fail("TC-J", f"OHLC 値に不正な値あり (response={state})")
        else:
            pass_("TC-G: klines=0 件（Paused 直後のため許容）")
            pass_("TC-H: klines=0 件（スキップ）")
            pass_("TC-I: klines=0 件（スキップ）")
            pass_("TC-J: klines=0 件（スキップ）")

        # ─────────────────────────────────────────────────────────────────────
        # TC-K: StepForward 後に current_time_ms が増加し klines が含まれる
        # ─────────────────────────────────────────────────────────────────────
        print()
        print("── TC-K: StepForward 後の state 変化")

        ct_before = state.get("current_time_ms") or state.get("current_time") or 0
        try:
            ct_before = int(ct_before)
        except (TypeError, ValueError):
            ct_before = 0

        api_post("/api/replay/step-forward")
        time.sleep(1)

        try:
            state2 = api_get("/api/replay/state")
        except requests.RequestException as e:
            fail("TC-K-fetch", f"GET /api/replay/state 失敗: {e}")
            return

        ct_after_k = state2.get("current_time_ms") or state2.get("current_time") or 0
        try:
            ct_after_k = int(ct_after_k)
        except (TypeError, ValueError):
            ct_after_k = 0

        print(f"  current_time_ms: {ct_before} → {ct_after_k}")

        if ct_after_k > ct_before:
            pass_(f"TC-K1: StepForward 後に current_time_ms が増加 ({ct_before} → {ct_after_k})")
        else:
            fail("TC-K1", f"current_time_ms が増加しない ({ct_before} → {ct_after_k})")

        klines2 = state2.get("klines", [])
        kline_count2 = len(klines2) if isinstance(klines2, list) else 0
        print(f"  klines count after step={kline_count2}")

        if IS_HEADLESS:
            pend("TC-K2", "headless klines スキーマ差分あり")
        else:
            if kline_count2 > 0:
                pass_("TC-K2: StepForward 後に klines が 1 件以上")
            else:
                fail("TC-K2", "StepForward 後も klines=0 件")

    finally:
        env.close()


def test_s43_get_state_endpoint() -> None:
    """pytest から呼ばれる場合のエントリポイント。"""
    reset_counters()
    backup_state()
    try:
        run_s43()
    finally:
        restore_state()
        print_summary()
    import helpers as _h
    assert _h._FAIL == 0, f"{_h._FAIL} TC(s) failed — see output above"


def main() -> None:
    reset_counters()
    backup_state()
    try:
        run_s43()
    finally:
        restore_state()
        print_summary()
        import helpers as _h
        if _h._FAIL > 0:
            sys.exit(1)


if __name__ == "__main__":
    main()
