#!/usr/bin/env python3
"""s5_tachibana_mixed.py — スイート S5: 立花証券 + Binance 混在 Replay

検証シナリオ:
  TC-S5-01: inject-session でモックセッション注入 → session=present
  TC-S5-02: inject-master で銘柄マスター注入 (ok=true)
  TC-S5-03: toggle + play → Playing 到達（Binance M1 + Tachibana D1 混在）
  TC-S5-04/05: 両ペインの streams_ready=true 確認
  TC-S5-06: 10x 速度で current_time 前進
  TC-S5-07: M1+D1 混在での StepForward delta=60000ms（M1 が最小 TF）

仕様根拠:
  docs/replay_header.md §7 — マルチストリーム同期
  e2e-mock feature — inject エンドポイント（inject-master / inject-daily-history）

使い方:
    python tests/s5_tachibana_mixed.py
    pytest tests/s5_tachibana_mixed.py -v
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
    get_status, wait_status, wait_playing, wait_paused,
    api_get, api_post, api_get_code, api_post_code,
    speed_to_10x,
    utc_offset,
    DATA_DIR, STATE_FILE, API_BASE,
)

import requests
import helpers as _h


def run_s5() -> None:
    print("=== S5: 立花証券 + Binance 混在 Replay ===")

    mid_ms = int(time.time() * 1000) - 3 * 3600 * 1000
    start = utc_offset(-4)
    end = utc_offset(-2)

    # ── フィクスチャ: 2ペイン（BinanceLinear:BTCUSDT M1 + TachibanaSpot:7203 D1）──
    # Live モードで起動（auto-play 問題を回避するため replay 設定なし）
    DATA_DIR.mkdir(parents=True, exist_ok=True)
    fixture = {
        "layout_manager": {"layouts": [{"name": "S5", "dashboard": {"pane": {
            "Split": {"axis": "Vertical", "ratio": 0.5,
                "a": {"KlineChart": {
                    "layout": {"splits": [0.78], "autoscale": "FitToVisible"}, "kind": "Candles",
                    "stream_type": [{"Kline": {"ticker": "BinanceLinear:BTCUSDT", "timeframe": "M1"}}],
                    "settings": {"tick_multiply": None, "visual_config": None, "selected_basis": {"Time": "M1"}},
                    "indicators": [], "link_group": "A"
                }},
                "b": {"KlineChart": {
                    "layout": {"splits": [0.78], "autoscale": "FitToVisible"}, "kind": "Candles",
                    "stream_type": [{"Kline": {"ticker": "TachibanaSpot:7203", "timeframe": "D1"}}],
                    "settings": {"tick_multiply": None, "visual_config": None, "selected_basis": {"Time": "D1"}},
                    "indicators": [], "link_group": "A"
                }}
            }
        }, "popout": []}}], "active_layout": "S5"},
        "timezone": "UTC", "trade_fetch_enabled": False, "size_in_quote_ccy": "Base"
    }
    STATE_FILE.write_text(json.dumps(fixture, indent=2))

    env = FlowsurfaceEnv(ticker="BinanceLinear:BTCUSDT", timeframe="M1", headless=IS_HEADLESS)
    env._start_process()
    try:
        # TC-S5-01: inject-session プローブ — 404 なら e2e-mock feature なし → 全 TC を PEND
        try:
            r = requests.post(f"{API_BASE}/api/test/tachibana/inject-session", timeout=5)
            probe_code = r.status_code
        except requests.RequestException:
            probe_code = 0

        if probe_code == 404:
            pend("TC-S5-01", "inject-session 404 — e2e-mock feature 不要（--features e2e-mock が必要）")
            pend("TC-S5-02", "inject-session 404 — e2e-mock feature 不要")
            pend("TC-S5-03", "inject-session 404 — e2e-mock feature 不要")
            pend("TC-S5-04", "inject-session 404 — e2e-mock feature 不要")
            pend("TC-S5-05", "inject-session 404 — e2e-mock feature 不要")
            pend("TC-S5-06", "inject-session 404 — e2e-mock feature 不要")
            pend("TC-S5-07", "inject-session 404 — e2e-mock feature 不要")
            return

        # inject-session 成功: /api/auth/tachibana/status → session=present を確認
        try:
            tach_status = api_get("/api/auth/tachibana/status")
            session = tach_status.get("session", "none")
        except requests.RequestException as e:
            fail("TC-S5-01", f"tachibana/status API error: {e}")
            return

        if session == "present":
            pass_("TC-S5-01: Tachibana セッション注入成功 (session=present)")
        else:
            fail("TC-S5-01", f"session={session} (expected present)")
            return

        # TC-S5-02: inject-master（銘柄マスター注入）
        master_body = {"records": [{"sIssueCode": "7203", "sIssueNameEizi": "Toyota Motor", "sCLMID": "CLMIssueMstKabu"}]}
        try:
            r = requests.post(f"{API_BASE}/api/test/tachibana/inject-master", json=master_body, timeout=5)
            resp = r.json()
            if resp.get("ok"):
                pass_("TC-S5-02: inject-master 成功 (ok=true)")
            elif "Not Found" in resp.get("error", ""):
                pend("TC-S5-02", "inject-master 404 — e2e-mock feature 不要")
            else:
                fail("TC-S5-02", f"inject-master 失敗: {resp}")
        except requests.RequestException as e:
            fail("TC-S5-02", f"inject-master API error: {e}")

        # inject-daily-history: replay 範囲内のモック D1 kline を注入
        daily_body = {
            "issue_code": "7203",
            "klines": [
                {"time": mid_ms - 86400000, "open": 3000, "high": 3100, "low": 2900, "close": 3050, "volume": 500000},
                {"time": mid_ms,            "open": 3050, "high": 3150, "low": 2950, "close": 3100, "volume": 600000},
            ]
        }
        try:
            r = requests.post(f"{API_BASE}/api/test/tachibana/inject-daily-history", json=daily_body, timeout=5)
            dh_resp = r.json()
            if dh_resp.get("ok"):
                print(f"  inject-daily-history OK (count={dh_resp.get('count')})")
            else:
                print(f"  WARN: inject-daily-history: {dh_resp}")
        except requests.RequestException as e:
            print(f"  WARN: inject-daily-history failed: {e}")

        # Binance ストリームが Ready になるまで待つ（prepare_replay が M1 stream を登録できるように）
        print("  waiting for BTC stream ready...")
        for i in range(30):
            try:
                body = api_get("/api/pane/list")
                panes = body.get("panes", [])
                btc_pane = next((p for p in panes if p.get("ticker", "").find("BTCUSDT") >= 0), None)
                if btc_pane and btc_pane.get("streams_ready") is True:
                    print(f"  BTC stream ready ({i + 1}s)")
                    break
            except requests.RequestException:
                pass
            time.sleep(1)

        # TC-S5-03: Replay に切替 + Manual Play → Playing 到達
        api_post("/api/replay/toggle")
        api_post("/api/replay/toggle", {"start": start, "end": end})

        if wait_playing(60):
            pass_("TC-S5-03: Replay Playing 到達（Binance M1 + Tachibana D1 混在）")
        else:
            fail("TC-S5-03", "Playing に到達せず（60 秒タイムアウト）")
            return

        # TC-S5-04/05: 両ペインの streams_ready 確認（最大 20 秒）
        btc_ready = False
        tach_ready = False
        for _ in range(20):
            try:
                panes = api_get("/api/pane/list").get("panes", [])
                btc_p = next((p for p in panes if p.get("ticker", "").find("BTCUSDT") >= 0), None)
                tach_p = next((p for p in panes if p.get("ticker", "").find("7203") >= 0), None)
                btc_ready = bool(btc_p and btc_p.get("streams_ready"))
                tach_ready = bool(tach_p and tach_p.get("streams_ready"))
                if btc_ready and tach_ready:
                    break
            except requests.RequestException:
                pass
            time.sleep(1)

        if btc_ready:
            pass_("TC-S5-04: Binance BTCUSDT streams_ready=true")
        else:
            fail("TC-S5-04", f"Binance streams_ready={btc_ready}")

        if tach_ready:
            pass_("TC-S5-05: Tachibana 7203 streams_ready=true")
        else:
            fail("TC-S5-05", f"Tachibana streams_ready={tach_ready}")

        # TC-S5-06: 10x 速度で current_time 前進をポーリング確認
        speed_to_10x()
        try:
            ct_raw = get_status().get("current_time")
            t1 = int(ct_raw) if ct_raw is not None else 0
        except (requests.RequestException, TypeError, ValueError):
            t1 = 0

        t2 = None
        deadline = time.monotonic() + 15
        while time.monotonic() < deadline:
            try:
                ct = get_status().get("current_time")
                if ct is not None and int(ct) > t1:
                    t2 = int(ct)
                    break
            except (requests.RequestException, TypeError, ValueError):
                pass
            time.sleep(0.5)

        if t2 is not None:
            pass_(f"TC-S5-06: current_time 前進 ({t1} → {t2})")
        else:
            fail("TC-S5-06", "15 秒待機しても current_time が変化しなかった")

        # TC-S5-07: M1+D1 混在での StepForward — delta = 60000ms（M1 が最小 TF）
        try:
        except requests.RequestException:
            pass

        if not wait_status("Paused", 10):
            fail("TC-S5-07-pre", "Paused に遷移せず")
        else:
            try:
                t_before_raw = get_status().get("current_time")
                t_before = int(t_before_raw) if t_before_raw not in (None, "null", "") else None
            except (requests.RequestException, TypeError, ValueError):
                t_before = None

            try:
                api_post("/api/replay/step-forward")
            except requests.RequestException:
                pass
            time.sleep(1)
            wait_status("Paused", 10)

            try:
                t_after_raw = get_status().get("current_time")
                t_after = int(t_after_raw) if t_after_raw not in (None, "null", "") else None
            except (requests.RequestException, TypeError, ValueError):
                t_after = None

            if t_before is None or t_after is None:
                fail("TC-S5-07", f"current_time 取得失敗 (before={t_before} after={t_after})")
            else:
                delta = t_after - t_before
                if delta == 60000:
                    pass_("TC-S5-07: M1+D1 混在 StepForward delta=60000ms（M1 が最小 TF）")
                else:
                    fail("TC-S5-07", f"delta={delta} (expected 60000ms: M1 is min TF in M1+D1 mix)")

    finally:
        env.close()


def test_s5_tachibana_mixed() -> None:
    """pytest エントリポイント。"""
    _h._PASS = _h._FAIL = _h._PEND = 0
    backup_state()
    try:
        run_s5()
    finally:
        restore_state()
    print_summary()
    assert _h._FAIL == 0, f"{_h._FAIL} TC(s) failed — see output above"


def main() -> None:
    backup_state()
    try:
        run_s5()
    finally:
        restore_state()
        print_summary()
        if _h._FAIL > 0:
            sys.exit(1)


if __name__ == "__main__":
    main()
