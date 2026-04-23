#!/usr/bin/env python3
"""s30_mixed_sample_loading.py — S30: Tachibana D1 + ETHUSDT M1 混在起動時の Loading 解消

検証シナリオ:
  TC-A: Tachibana D1 + ETHUSDT M1 の 2 ペイン構成で Play → Playing に遷移すること
        （修正前: D1 ストリームが load_range 不正により Loading に固定されていた）
  TC-B: Playing 後 current_time が前進すること（再生が正常動作している）
  TC-C: 両ペインの streams_ready=true になること

仕様根拠:
  修正された不具合 (2 件):
  (1) D1 ストリームの load_range が M1 の step_size で計算されていた
      → compute_load_range を各 TF で計算するよう修正
  (2) KlinesLoadCompleted で空 klines が返ったとき on_klines_loaded を呼ばずにいた
      → status が "Loading" に固定されていた

前提条件: DEV_USER_ID / DEV_PASSWORD 環境変数設定済み（未設定時はスキップ）

使い方:
    DEV_USER_ID=... DEV_PASSWORD=... python tests/s30_mixed_sample_loading.py
    pytest tests/s30_mixed_sample_loading.py -v
"""

from __future__ import annotations

import json
import os
import sys
import time
from pathlib import Path

import requests

sys.path.insert(0, str(Path(__file__).parent))
from helpers import (
    DATA_DIR,
    IS_HEADLESS,
    STATE_FILE,
    api_get,
    api_post,
    backup_state,
    fail,
    pass_,
    pend,
    print_summary,
    restore_state,
    utc_offset,
    wait_playing,
    wait_for_time_advance,
    wait_tachibana_session,
    FlowsurfaceEnv,
)


# ── フィクスチャ ──────────────────────────────────────────────────────────────

def _write_s30_fixture() -> None:
    """BinanceLinear:ETHUSDT M1 + TachibanaSpot:7203 D1 の 2ペイン Split レイアウトを書き込む。"""
    DATA_DIR.mkdir(parents=True, exist_ok=True)
    fixture = {
        "layout_manager": {
            "layouts": [
                {
                    "name": "S30",
                    "dashboard": {
                        "pane": {
                            "Split": {
                                "axis": "Vertical",
                                "ratio": 0.5,
                                "a": {
                                    "KlineChart": {
                                        "layout": {"splits": [0.78], "autoscale": "FitToVisible"},
                                        "kind": "Candles",
                                        "stream_type": [{"Kline": {"ticker": "BinanceLinear:ETHUSDT", "timeframe": "M1"}}],
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
                                        "stream_type": [{"Kline": {"ticker": "TachibanaSpot:7203", "timeframe": "D1"}}],
                                        "settings": {
                                            "tick_multiply": None,
                                            "visual_config": None,
                                            "selected_basis": {"Time": "D1"},
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
            "active_layout": "S30",
        },
        "timezone": "UTC",
        "trade_fetch_enabled": False,
        "size_in_quote_ccy": "Base",
    }
    STATE_FILE.write_text(json.dumps(fixture, indent=2))


# ── ポーリング ────────────────────────────────────────────────────────────────

def _wait_eth_stream_ready(timeout: int = 30) -> bool:
    """ETHUSDT ペインの streams_ready=true になるまで待つ。"""
    deadline = time.monotonic() + timeout
    i = 0
    while time.monotonic() < deadline:
        i += 1
        try:
            panes = api_get("/api/pane/list").get("panes", [])
            p = next((x for x in panes if "ETHUSDT" in (x.get("ticker") or "")), None)
            if p and p.get("streams_ready") is True:
                print(f"  ETHUSDT M1 stream ready ({i}s)")
                return True
        except requests.RequestException:
            pass
        time.sleep(1)
    return False


def _wait_both_streams_ready(timeout: int = 20) -> tuple[bool, bool]:
    """7203 と ETHUSDT 両ペインの streams_ready を待ち (tach_ready, eth_ready) を返す。"""
    deadline = time.monotonic() + timeout
    tach_ready = False
    eth_ready = False
    while time.monotonic() < deadline:
        try:
            panes = api_get("/api/pane/list").get("panes", [])
            tach = next((x for x in panes if "7203" in (x.get("ticker") or "")), None)
            eth = next((x for x in panes if "ETHUSDT" in (x.get("ticker") or "")), None)
            tach_ready = bool(tach and tach.get("streams_ready") is True)
            eth_ready = bool(eth and eth.get("streams_ready") is True)
            if tach_ready and eth_ready:
                break
        except requests.RequestException:
            pass
        time.sleep(1)
    return tach_ready, eth_ready


# ── テスト本体 ────────────────────────────────────────────────────────────────

def run_s30(start: str, end: str) -> None:
    print()
    print("── TC-A: Tachibana D1 + ETHUSDT M1 混在 Play → Playing に遷移")

    # ETHUSDT M1 stream ready 待機
    print("  ETHUSDT M1 stream ready 待機（最大 30 秒）...")
    eth_ok = _wait_eth_stream_ready(30)
    if not eth_ok:
        print("  WARN: ETHUSDT M1 stream が 30 秒で ready にならなかった — Live 接続待ちのまま続行")

    # Replay に切替 + Play
    if not IS_HEADLESS:
        try:
            api_post("/api/replay/toggle")
        except requests.RequestException:
            pass
    try:
        api_post("/api/replay/toggle", {"start": start, "end": end})
    except requests.RequestException:
        pass

    print("  Play 開始、Loading → Playing を待機（最大 120 秒）...")
    if wait_playing(120):
        pass_("TC-A: Loading が解消され Playing に遷移（D1 load_range 修正が有効）")
    else:
        try:
            last_st = api_get("/api/replay/status").get("status")
        except requests.RequestException:
            last_st = "unknown"
        fail(
            "TC-A: Playing に到達しなかった（120 秒タイムアウト）",
            f"status={last_st} — 修正前: D1 stream が load_range 不正により Loading に固定される",
        )
        return

    # TC-B: current_time が前進すること
    print()
    print("── TC-B: current_time が前進すること（再生が正常動作）")
    try:
        t1 = int(api_get("/api/replay/status").get("current_time") or 0)
    except requests.RequestException:
        t1 = 0
    t2 = wait_for_time_advance(t1, 15)
    if t2 is not None:
        pass_(f"TC-B: current_time が前進 ({t1} → {t2})")
    else:
        fail("TC-B", "15 秒待機しても current_time が変化しなかった")

    # TC-C: 両ペインの streams_ready 確認
    print()
    print("── TC-C: 両ペインの streams_ready 確認")
    tach_ready, eth_ready = _wait_both_streams_ready(20)
    if tach_ready:
        pass_("TC-C1: Tachibana D1 (7203) streams_ready=true")
    else:
        fail("TC-C1", f"Tachibana streams_ready={tach_ready}")
    if eth_ready:
        pass_("TC-C2: BinanceLinear ETHUSDT M1 streams_ready=true")
    else:
        fail("TC-C2", f"ETHUSDT streams_ready={eth_ready}")


# ── pytest エントリポイント ───────────────────────────────────────────────────

def test_s30_mixed_sample_loading() -> None:
    """pytest から呼ばれる場合のエントリポイント。プロセス起動は外部で行うこと。"""
    import helpers
    helpers._PASS = helpers._FAIL = helpers._PEND = 0

    if not os.environ.get("DEV_USER_ID") or not os.environ.get("DEV_PASSWORD"):
        pend("S30", "DEV_USER_ID / DEV_PASSWORD が未設定 — スキップ")
        return

    start = utc_offset(-4)
    end = utc_offset(-2)
    run_s30(start, end)
    print_summary()
    assert helpers._FAIL == 0, f"{helpers._FAIL} TC(s) failed — see output above"


# ── スタンドアロン実行 ────────────────────────────────────────────────────────

def main() -> None:
    import helpers
    helpers._PASS = helpers._FAIL = helpers._PEND = 0

    print("=== S30: Tachibana D1 + ETHUSDT M1 混在起動時の Loading 解消 ===")

    if not os.environ.get("DEV_USER_ID") or not os.environ.get("DEV_PASSWORD"):
        print("  SKIP: DEV_USER_ID / DEV_PASSWORD が未設定 — Tachibana セッション不要環境ではスキップ")
        sys.exit(0)

    start = utc_offset(-4)
    end = utc_offset(-2)
    print(f"  range: {start} → {end}")

    backup_state()
    _write_s30_fixture()

    env = FlowsurfaceEnv(ticker="BinanceLinear:ETHUSDT", timeframe="M1", headless=IS_HEADLESS)
    try:
        env._start_process()

        # Tachibana セッション確認
        try:
            body = requests.get("http://127.0.0.1:9876/api/auth/tachibana/status", timeout=5).json()
            session = body.get("session", "none")
        except requests.RequestException:
            session = "none"
        print(f"  Tachibana session: {session}")

        if session == "none":
            print("  SKIP: Tachibana セッションなし — このテストはキーリングのセッションが必要です")
            print("  (inject-session が利用できない環境では Tachibana ストリームはテストできません)")
            print()
            print_summary()
            sys.exit(0)

        run_s30(start, end)
    finally:
        env.close()
        restore_state()

    print_summary()
    if helpers._FAIL > 0:
        sys.exit(1)


if __name__ == "__main__":
    main()
