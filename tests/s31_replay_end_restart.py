#!/usr/bin/env python3
"""s31_replay_end_restart.py — S31: 混合データ（Tachibana D1 + ETHUSDT M1）終端到達後 Play で先頭から再スタート

検証シナリオ:
  TC-A: Play → 10x 加速 → 終端到達 (Paused @ end_time)
  TC-B: Play 再呼び出し → レスポンスが Loading または Playing かつ再スタート開始
  TC-C: Play レスポンスの current_time が start_time 付近であること（先頭からの再開）

仕様根拠:
  修正された不具合: Play ボタンが終端 Paused のとき Resume ではなく Play を送るべきだった
  → 終端到達後に Play を押しても current_time が end_time のまま動かなかった

前提条件: DEV_USER_ID / DEV_PASSWORD 環境変数設定済み（未設定時はスキップ）

使い方:
    DEV_USER_ID=... DEV_PASSWORD=... python tests/s31_replay_end_restart.py
    pytest tests/s31_replay_end_restart.py -v
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
    utc_to_ms,
    wait_status,
    FlowsurfaceEnv,
)


# ── フィクスチャ ──────────────────────────────────────────────────────────────

def _write_s31_fixture() -> None:
    """BinanceLinear:ETHUSDT M1 + TachibanaSpot:7203 D1 の 2ペイン Split レイアウトを書き込む。"""
    DATA_DIR.mkdir(parents=True, exist_ok=True)
    fixture = {
        "layout_manager": {
            "layouts": [
                {
                    "name": "S31",
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
            "active_layout": "S31",
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


def _wait_end_reached(end_ms: int, timeout: int = 120) -> tuple[bool, int]:
    """終端到達（Paused かつ current_time >= end_ms - 120000ms）を待つ。
    (reached, ct_at_end) を返す。"""
    deadline = time.monotonic() + timeout
    while time.monotonic() < deadline:
        try:
            s = api_get("/api/replay/status")
            ct_raw = s.get("current_time")
            st = s.get("status")
            if st == "Paused" and ct_raw is not None:
                ct = int(ct_raw)
                if ct >= end_ms - 120_000:
                    return True, ct
        except (requests.RequestException, TypeError, ValueError):
            pass
        time.sleep(1)
    return False, 0


# ── テスト本体 ────────────────────────────────────────────────────────────────

def run_s31(start: str, end: str) -> None:
    start_ms = utc_to_ms(start)
    end_ms = utc_to_ms(end)
    print(f"  range: {start} → {end}")
    print(f"  start_ms={start_ms} end_ms={end_ms}")

    # ETHUSDT M1 stream ready 待機
    print("  ETHUSDT M1 stream ready 待機（最大 30 秒）...")
    _wait_eth_stream_ready(30)

    # Replay に切替 + Play
    if not IS_HEADLESS:
        try:
            api_post("/api/replay/toggle")
        except requests.RequestException:
            pass
    try:
        api_post("/api/replay/play", {"start": start, "end": end})
    except requests.RequestException:
        pass

    # Playing 到達待機（最大 60 秒）
    print("  Playing 待機...")
    if not wait_status("Playing", 60):
        fail("precond", "Playing に到達せず")
        return
    print("  Playing 到達")

    # ── TC-A: 10x 加速 → 終端到達 ─────────────────────────────────────────────
    print()
    print("── TC-A: 10x 加速 → 終端到達（Paused @ end_time）")

    # 1x→2x→5x→10x（3 回 CycleSpeed）
    for _ in range(3):
        try:
            api_post("/api/replay/speed")
        except requests.RequestException:
            pass

    print("  10x 加速完了、終端まで待機（最大 120 秒）...")
    reached_end, ct_at_end = _wait_end_reached(end_ms, 120)

    if reached_end:
        pass_(f"TC-A: 終端到達確認 (current_time={ct_at_end}, status=Paused)")
    else:
        try:
            s = api_get("/api/replay/status")
            last_st = s.get("status")
            last_ct = s.get("current_time")
        except requests.RequestException:
            last_st = "unknown"
            last_ct = "unknown"
        fail("TC-A: 終端到達しなかった", f"status={last_st} current_time={last_ct}")
        return

    # current_time が start_time とは異なることを確認
    if ct_at_end == start_ms:
        print("  [SKIP] current_time が既に start_time と一致 — レンジが小さすぎてテスト不成立")
        return

    # ── TC-B: Play 再呼び出し → 再スタート ────────────────────────────────────
    print()
    print("── TC-B: 終端到達後 Play 再呼び出し → レスポンスで再スタートを確認")

    try:
        play_resp = requests.post(
            "http://127.0.0.1:9876/api/replay/play",
            json={"start": start, "end": end},
            timeout=5,
        )
        play_data = play_resp.json()
    except requests.RequestException as e:
        fail("TC-B", f"play POST 失敗: {e}")
        return
    print(f"  play response: {play_data}")

    resp_status = play_data.get("status", "none")
    resp_ct = play_data.get("current_time")

    if resp_status in ("Loading", "Playing"):
        pass_(f"TC-B: 終端後 Play レスポンス status={resp_status}（再スタート開始）")
    else:
        fail("TC-B", f"play レスポンス status={resp_status} (expected Loading or Playing) — {play_data}")
        return

    # ── TC-C: Play レスポンスの current_time が start_time 付近か確認 ────────
    print()
    print("── TC-C: Play レスポンスの current_time が start_time 付近か確認")
    print(f"  resp.current_time={resp_ct} start_time={start_ms} end_time={end_ms}")

    if resp_ct is None:
        fail("TC-C", f"play レスポンスに current_time がない — {play_data}")
    else:
        ct = int(resp_ct)
        near_start = start_ms <= ct <= start_ms + 300_000
        far_from_end = end_ms - ct > 60_000
        if near_start and far_from_end:
            pass_(f"TC-C: 再スタート後 current_time が start_time 付近 (ct={ct} st={start_ms}) — 先頭から再開を確認")
        else:
            fail(
                "TC-C",
                f"current_time={ct} は start_time={start_ms} 付近でない — end_time={end_ms} 付近のまま？ (修正前の挙動)",
            )


# ── pytest エントリポイント ───────────────────────────────────────────────────

def test_s31_replay_end_restart() -> None:
    """pytest から呼ばれる場合のエントリポイント。プロセス起動は外部で行うこと。"""
    import helpers
    helpers._PASS = helpers._FAIL = helpers._PEND = 0

    if not os.environ.get("DEV_USER_ID") or not os.environ.get("DEV_PASSWORD"):
        pend("S31", "DEV_USER_ID / DEV_PASSWORD が未設定 — スキップ")
        return

    start = utc_offset(-4)
    end = utc_offset(-2)
    run_s31(start, end)
    print_summary()
    assert helpers._FAIL == 0, f"{helpers._FAIL} TC(s) failed — see output above"


# ── スタンドアロン実行 ────────────────────────────────────────────────────────

def main() -> None:
    import helpers
    helpers._PASS = helpers._FAIL = helpers._PEND = 0

    print("=== S31: 混合データ 終端到達後 Play で先頭から再スタート ===")

    if not os.environ.get("DEV_USER_ID") or not os.environ.get("DEV_PASSWORD"):
        print("  SKIP: DEV_USER_ID / DEV_PASSWORD が未設定 — Tachibana セッション不要環境ではスキップ")
        sys.exit(0)

    start = utc_offset(-4)
    end = utc_offset(-2)

    backup_state()
    _write_s31_fixture()

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
            print_summary()
            sys.exit(0)

        run_s31(start, end)
    finally:
        env.close()
        restore_state()

    print_summary()
    if helpers._FAIL > 0:
        sys.exit(1)


if __name__ == "__main__":
    main()
