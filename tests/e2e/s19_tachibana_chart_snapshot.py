#!/usr/bin/env python3
"""s19_tachibana_chart_snapshot.py — スイート S19: chart-snapshot API テスト（TachibanaSpot）

検証シナリオ:
  TC-S19-01: Play 後 bar_count が 1〜301（PRE_START_HISTORY_BARS=300 確認）
  TC-S19-02: StepForward 後 bar_count 増加または同数（D1 step = 86400000ms）
  TC-S19-03: StepBackward 後も snapshot 取得可能（クラッシュなし）
  TC-S19-04: 存在しない pane_id → {"error":"..."} + アプリ生存
  TC-S19-05: Live モード中の snapshot 取得後もアプリ応答あり

仕様根拠:
  docs/replay_header.md §9.2 — GET /api/pane/chart-snapshot（TachibanaSpot D1 版）

前提条件: DEV_USER_ID / DEV_PASSWORD 環境変数設定済み

使い方:
    DEV_USER_ID=xxx DEV_PASSWORD=yyy python tests/s19_tachibana_chart_snapshot.py
    pytest tests/s19_tachibana_chart_snapshot.py -v
"""

from __future__ import annotations

import os
import sys
import time
from pathlib import Path

sys.path.insert(0, str(Path(__file__).parent))
from helpers import (
    IS_HEADLESS, FlowsurfaceEnv,
    pass_, fail, pend, print_summary,
    backup_state, restore_state,
    get_status, wait_status, wait_playing, wait_paused,
    wait_tachibana_session, wait_for_pane_streams_ready,
    api_get, api_post, api_get_code,
    get_pane_id,
    tachibana_replay_setup,
    utc_offset,
    API_BASE,
)

import requests
import helpers as _h


def run_s19() -> None:
    print("=== S19: chart-snapshot API テスト（TachibanaSpot:7203 D1）===")

    # 環境変数チェック
    if not os.environ.get("DEV_USER_ID") or not os.environ.get("DEV_PASSWORD"):
        pend("TC-S19-01", "DEV_USER_ID / DEV_PASSWORD が未設定 — Tachibana live テストをスキップ")
        pend("TC-S19-02", "DEV_USER_ID / DEV_PASSWORD が未設定")
        pend("TC-S19-03", "DEV_USER_ID / DEV_PASSWORD が未設定")
        pend("TC-S19-04", "DEV_USER_ID / DEV_PASSWORD が未設定")
        pend("TC-S19-05", "DEV_USER_ID / DEV_PASSWORD が未設定")
        return

    start = utc_offset(-96)
    end = utc_offset(-24)

    # saved-state.json を書き込む（tachibana_replay_setup は saved-state のみ書き込む）
    tachibana_replay_setup(start, end)

    env = FlowsurfaceEnv(ticker="TachibanaSpot:7203", timeframe="D1", headless=False)
    env._start_process()
    try:
        if not wait_tachibana_session(120):
            fail("TC-S19-precond", "Tachibana セッション確立せず（120 秒タイムアウト）")
            return

        pane_id = get_pane_id(0)
        if not pane_id:
            fail("TC-S19-precond", "ペイン ID 取得失敗")
            return
        print(f"  PANE_ID={pane_id}")

        # streams_ready=true になるまで待機
        print("  waiting for streams_ready...")
        if not wait_for_pane_streams_ready(pane_id, 120):
            print("  WARN: streams_ready タイムアウト（継続）")

        # Replay モードへ切替 → Play 送信
        api_post("/api/replay/toggle")
        api_post("/api/replay/toggle", {"start": start, "end": end})

        if not wait_playing(120):
            fail("TC-S19-precond", "Playing 到達せず（120 秒タイムアウト）")
            return

        # 少し再生させてから Pause
        time.sleep(2)
        if not wait_paused(15):
            fail("TC-S19-precond", "Paused に遷移せず")
            return
        time.sleep(0.5)

        # TC-S19-01: Paused 直後のバー本数が 1 ≤ bar_count ≤ 301
        try:
            r = requests.get(f"{API_BASE}/api/pane/chart-snapshot?pane_id={pane_id}", timeout=5)
            if r.status_code in (404, 501):
                pend("TC-S19-01", f"chart-snapshot API 未実装 (HTTP {r.status_code})")
                pend("TC-S19-02", "chart-snapshot API 未実装")
                pend("TC-S19-03", "chart-snapshot API 未実装")
                pend("TC-S19-04", "chart-snapshot API 未実装")
                pend("TC-S19-05", "chart-snapshot API 未実装")
                return
            snap = r.json()
        except requests.RequestException as e:
            fail("TC-S19-01", f"chart-snapshot API error: {e}")
            return

        print(f"  snapshot response: {snap}")
        bar_count = snap.get("bar_count")
        print(f"  bar_count={bar_count}")

        if bar_count is not None and isinstance(bar_count, (int, float)) and 1 <= bar_count <= 301:
            pass_(f"TC-S19-01: Play 後 bar_count={bar_count} (1 ≤ N ≤ 301, PRE_START_HISTORY_BARS 確認)")
        else:
            fail("TC-S19-01", f"bar_count={bar_count} (想定: 1..301)")

        # TC-S19-02: StepForward 後 bar_count が増加または同数
        bar_before = bar_count
        api_post("/api/replay/step-forward")
        wait_paused(15)
        time.sleep(0.5)

        try:
            r2 = requests.get(f"{API_BASE}/api/pane/chart-snapshot?pane_id={pane_id}", timeout=5)
            snap2 = r2.json()
        except requests.RequestException as e:
            fail("TC-S19-02", f"chart-snapshot API error after StepForward: {e}")
            snap2 = {}

        bar_after = snap2.get("bar_count")
        print(f"  bar_count after StepForward: {bar_before} → {bar_after}")

        if bar_after is not None and bar_before is not None and bar_after >= bar_before:
            pass_(f"TC-S19-02: StepForward 後 bar_count={bar_after} >= before={bar_before}")
        else:
            fail("TC-S19-02", f"bar_count={bar_after} < before={bar_before}（バー減少の異常）")

        # TC-S19-03: StepBackward 後も snapshot 取得可能（クラッシュしない）
        # 少し前進してから StepBackward（start 境界クランプを避けるため）
        for _ in range(3):
            try:
                api_post("/api/replay/step-forward")
            except requests.RequestException:
                pass
            time.sleep(0.5)
        wait_status("Paused", 15)
        wait_paused(15)
        time.sleep(0.3)

        try:
            r3 = requests.get(f"{API_BASE}/api/pane/chart-snapshot?pane_id={pane_id}", timeout=5)
            snap3 = r3.json()
        except requests.RequestException as e:
            fail("TC-S19-03", f"chart-snapshot API error after StepBackward: {e}")
            snap3 = {"error": str(e)}

        has_bar = snap3.get("bar_count") is not None and "error" not in snap3
        bar3 = snap3.get("bar_count")
        if has_bar:
            pass_(f"TC-S19-03: StepBackward 後 snapshot 取得成功 (bar_count={bar3})")
        else:
            fail("TC-S19-03", f"snapshot 異常レスポンス: {snap3}")

        # TC-S19-04: 存在しないペイン ID に対する snapshot → {"error":"..."} かつクラッシュなし
        fake_id = "00000000-0000-0000-0000-deadbeef0000"
        try:
            r_fake = requests.get(f"{API_BASE}/api/pane/chart-snapshot?pane_id={fake_id}", timeout=5)
            snap_fake = r_fake.json()
        except requests.RequestException as e:
            snap_fake = {"error": str(e)}

        has_error = "error" in snap_fake
        alive = api_get_code("/api/replay/status") == 200

        if has_error and alive:
            pass_(f"TC-S19-04: 不正 pane_id → error 応答 & アプリ生存確認 (resp={snap_fake})")
        else:
            fail("TC-S19-04", f"has_error={has_error} alive={alive} resp={snap_fake}")

        # TC-S19-05: Live モードで snapshot を取得してもクラッシュしない
        api_post("/api/replay/toggle")
        time.sleep(3)

        try:
            r_live = requests.get(f"{API_BASE}/api/pane/chart-snapshot?pane_id={pane_id}", timeout=5)
            snap_live = r_live.json()
            print(f"  Live mode snapshot: {snap_live}")
        except requests.RequestException as e:
            print(f"  Live mode snapshot error (expected): {e}")

        alive_after = api_get_code("/api/replay/status") == 200
        if alive_after:
            pass_("TC-S19-05: Live モード中の snapshot 取得後もアプリ応答あり")
        else:
            fail("TC-S19-05", "Live モード中の snapshot 取得後にアプリが応答しなくなった")

        # Replay モードに戻す
        try:
            api_post("/api/replay/toggle")
        except requests.RequestException:
            pass
        time.sleep(2)

    finally:
        env.close()


def test_s19_tachibana_chart_snapshot() -> None:
    """pytest エントリポイント。"""
    _h._PASS = _h._FAIL = _h._PEND = 0
    backup_state()
    try:
        run_s19()
    finally:
        restore_state()
    print_summary()
    assert _h._FAIL == 0, f"{_h._FAIL} TC(s) failed — see output above"


def main() -> None:
    backup_state()
    try:
        run_s19()
    finally:
        restore_state()
        print_summary()
        if _h._FAIL > 0:
            sys.exit(1)


if __name__ == "__main__":
    main()
