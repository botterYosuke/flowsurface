#!/usr/bin/env python3
"""s21_tachibana_error_boundary.py — スイート S21: クラッシュ・エラー境界テスト（TachibanaSpot）

検証シナリオ:
  TC-S21-01〜03: 不正 pane_id（pane/split, pane/close, pane/set-ticker）→ HTTP 404 + アプリ生存
  TC-S21-04: 空 range (start == end) でもアプリ生存（D1 版）
  TC-S21-05: 未来 range でもアプリ生存
  TC-S21-06: StepForward 連打 50 回 (Paused 状態) → crash なし, status=Paused
  TC-S21-07: split 上限テスト → アプリ生存

仕様根拠:
  docs/replay_header.md §10 — エラー境界・クラッシュ防止（TachibanaSpot D1 版）

前提条件: DEV_USER_ID / DEV_PASSWORD 環境変数設定済み

使い方:
    DEV_USER_ID=xxx DEV_PASSWORD=yyy python tests/s21_tachibana_error_boundary.py
    pytest tests/s21_tachibana_error_boundary.py -v
"""

from __future__ import annotations

import json
import os
import sys
import time
from pathlib import Path

sys.path.insert(0, str(Path(__file__).parent))
from helpers import (
    FlowsurfaceEnv,
    pass_, fail, pend, print_summary,
    backup_state, restore_state,
    get_status, wait_status, wait_playing, wait_paused,
    wait_tachibana_session, wait_for_pane_streams_ready,
    api_post, api_get_code,
    get_pane_id,
    tachibana_replay_setup,
    utc_offset,
    DATA_DIR, STATE_FILE, API_BASE,
)

import requests
import helpers as _h

FAKE_UUID = "ffffffff-ffff-ffff-ffff-ffffffffffff"


def _close_with_logout(env: FlowsurfaceEnv) -> None:
    """teardown ヘルパー: logout API を呢いてから env を閉じる。
    CI 環境で前ジョブのセッションが残留し次ジョブのログインが失敗する問題を防ぐ。
    """
    try:
        api_post("/api/auth/tachibana/logout")
        time.sleep(3)  # サーバー側のセッション切断を待つ
    except Exception:
        pass
    env.close()


def _write_live_tachibana_fixture() -> None:
    """TachibanaSpot:7203 D1 Live モードのフィクスチャを書き込む（replay フィールドなし）。"""
    DATA_DIR.mkdir(parents=True, exist_ok=True)
    fixture = {
        "layout_manager": {"layouts": [{"name": "Test-D1", "dashboard": {"pane": {
            "KlineChart": {
                "layout": {"splits": [0.78], "autoscale": "FitToVisible"}, "kind": "Candles",
                "stream_type": [{"Kline": {"ticker": "TachibanaSpot:7203", "timeframe": "D1"}}],
                "settings": {"tick_multiply": None, "visual_config": None, "selected_basis": {"Time": "D1"}},
                "indicators": ["Volume"], "link_group": "A"
            }
        }, "popout": []}}], "active_layout": "Test-D1"},
        "timezone": "UTC", "trade_fetch_enabled": False, "size_in_quote_ccy": "Base"
    }
    STATE_FILE.write_text(json.dumps(fixture, indent=2))


def _tachibana_start(start: str, end: str) -> FlowsurfaceEnv:
    """saved-state を書き込み、FlowsurfaceEnv を起動。セッション確立・streams_ready・toggle+play まで行う。"""
    tachibana_replay_setup(start, end)
    env = FlowsurfaceEnv(ticker="TachibanaSpot:7203", timeframe="D1", headless=False)
    env._start_process()

    print("  waiting for Tachibana session (DEV AUTO-LOGIN)...")
    if not wait_tachibana_session(120):
        raise RuntimeError("Tachibana session not established after 120s")
    print("  Tachibana session established")

    pane_id = get_pane_id(0)
    if pane_id:
        print("  waiting for D1 klines (streams_ready)...")
        if not wait_for_pane_streams_ready(pane_id, 120):
            print("  WARN: streams_ready timeout (continuing)")

    api_post("/api/replay/toggle")
    api_post("/api/replay/toggle", {"start": start, "end": end})
    return env


def run_s21() -> None:
    print("=== S21: クラッシュ・エラー境界テスト（TachibanaSpot:7203 D1）===")

    # 環境変数チェック
    if not os.environ.get("DEV_USER_ID") or not os.environ.get("DEV_PASSWORD"):
        for tc in ["TC-S21-01", "TC-S21-02", "TC-S21-03", "TC-S21-04", "TC-S21-05",
                   "TC-S21-06", "TC-S21-07"]:
            pend(tc, "DEV_USER_ID / DEV_PASSWORD が未設定 — Tachibana live テストをスキップ")
        return

    # ── TC-S21-01〜03: 不正 pane_id に対する各エンドポイント ──────────────────
    print("  [TC-S21-01/03] 不正 pane_id テスト...")
    try:
        env1 = _tachibana_start(utc_offset(-1440), utc_offset(-24))
    except RuntimeError as e:
        fail("TC-S21-precond", str(e))
        env1 = None

    if env1 is not None:
        try:
            if not wait_playing(120):
                fail("TC-S21-precond", "Playing 到達せず")
            else:
                # TC-S21-01: pane/split に存在しない UUID → HTTP 404
                try:
                    r = requests.post(
                        f"{API_BASE}/api/pane/split",
                        json={"pane_id": FAKE_UUID, "axis": "Vertical"},
                        timeout=5
                    )
                    http_split = r.status_code
                except requests.RequestException:
                    http_split = 0

                alive = api_get_code("/api/replay/status") == 200
                if http_split == 404 and alive:
                    pass_(f"TC-S21-01: pane/split 存在しない UUID → HTTP={http_split} & アプリ生存")
                else:
                    fail("TC-S21-01", f"HTTP={http_split} alive={alive}")

                # TC-S21-02: pane/close に存在しない UUID → HTTP 404
                try:
                    r = requests.post(
                        f"{API_BASE}/api/pane/close",
                        json={"pane_id": FAKE_UUID},
                        timeout=5
                    )
                    http_close = r.status_code
                except requests.RequestException:
                    http_close = 0

                alive = api_get_code("/api/replay/status") == 200
                if http_close == 404 and alive:
                    pass_(f"TC-S21-02: pane/close 存在しない UUID → HTTP={http_close} & アプリ生存")
                else:
                    fail("TC-S21-02", f"HTTP={http_close} alive={alive}")

                # TC-S21-03: pane/set-ticker に存在しない UUID → HTTP 404
                try:
                    r = requests.post(
                        f"{API_BASE}/api/pane/set-ticker",
                        json={"pane_id": FAKE_UUID, "ticker": "TachibanaSpot:6758"},
                        timeout=5
                    )
                    http_ticker = r.status_code
                except requests.RequestException:
                    http_ticker = 0

                alive = api_get_code("/api/replay/status") == 200
                if http_ticker == 404 and alive:
                    pass_(f"TC-S21-03: pane/set-ticker 存在しない UUID → HTTP={http_ticker} & アプリ生存")
                else:
                    fail("TC-S21-03", f"HTTP={http_ticker} alive={alive}")
        finally:
            _close_with_logout(env1)

    # ── TC-S21-04: 空 range (start == end) ─────────────────────────────────
    print("  [TC-S21-04] 空 range (start == end)...")
    same_time = utc_offset(-24)
    _write_live_tachibana_fixture()
    env4 = FlowsurfaceEnv(ticker="TachibanaSpot:7203", timeframe="D1", headless=False)
    env4._start_process()
    try:
        print("  waiting for Tachibana session (DEV AUTO-LOGIN)...")
        if not wait_tachibana_session(120):
            fail("TC-S21-04-pre", "Tachibana セッション確立せず（120 秒タイムアウト）")
        else:
            api_post("/api/replay/toggle")
            api_post("/api/replay/toggle", {"start": same_time, "end": same_time})
            time.sleep(5)

            alive = api_get_code("/api/replay/status") == 200
            try:
                status = get_status().get("status")
            except requests.RequestException:
                status = "unknown"

            if alive:
                pass_(f"TC-S21-04: 空 range でもアプリ生存 (status={status})")
            else:
                fail("TC-S21-04", "空 range でアプリがクラッシュした")
    finally:
        _close_with_logout(env4)

    # ── TC-S21-05: 未来の range (現在時刻 + 24h 先) ──────────────────────────
    print("  [TC-S21-05] 未来 range テスト...")
    future_start = utc_offset(24)
    future_end = utc_offset(48)
    _write_live_tachibana_fixture()
    env5 = FlowsurfaceEnv(ticker="TachibanaSpot:7203", timeframe="D1", headless=False)
    env5._start_process()
    try:
        print("  waiting for Tachibana session (DEV AUTO-LOGIN)...")
        if not wait_tachibana_session(120):
            fail("TC-S21-05-pre", "Tachibana セッション確立せず（120 秒タイムアウト）")
        else:
            api_post("/api/replay/toggle")
            api_post("/api/replay/toggle", {"start": future_start, "end": future_end})
            time.sleep(10)

            alive = api_get_code("/api/replay/status") == 200
            try:
                status = get_status().get("status")
            except requests.RequestException:
                status = "unknown"

            if alive:
                pass_(f"TC-S21-05: 未来 range でもアプリ生存 (status={status})")
            else:
                fail("TC-S21-05", "未来 range でアプリがクラッシュした")
    finally:
        _close_with_logout(env5)

    # ── TC-S21-06: StepForward 連打 50 回 (Paused 状態) ─────────────────────
    print("  [TC-S21-06] StepForward 連打 50 回...")
    try:
        env6 = _tachibana_start(utc_offset(-1440), utc_offset(-24))
    except RuntimeError as e:
        fail("TC-S21-06-pre", str(e))
        env6 = None

    if env6 is not None:
        try:
            if not wait_playing(120):
                fail("TC-S21-06-pre", "Playing 到達せず")
            else:
                try:
                except requests.RequestException:
                    pass

                if not wait_paused(15):
                    fail("TC-S21-06-pre", "Paused に遷移せず")
                else:
                    crash = False
                    for i in range(50):
                        try:
                            api_post("/api/replay/step-forward")
                        except requests.RequestException:
                            pass
                        time.sleep(0.3)
                        if api_get_code("/api/replay/status") == 0:
                            crash = True
                            print(f"  CRASH detected at forward step #{i + 1}")
                            break

                    wait_paused(15)
                    try:
                        status = get_status().get("status")
                    except requests.RequestException:
                        status = "unknown"
                    alive = api_get_code("/api/replay/status") == 200

                    if not crash and alive and status == "Paused":
                        pass_("TC-S21-06: StepForward 50 連打 → crash なし, status=Paused")
                    else:
                        fail("TC-S21-06", f"crash={crash} alive={alive} status={status}")

            # ── TC-S21-07: split 上限テスト（TC-S21-06 の same env で続行）──
            print("  [TC-S21-07] split 上限テスト...")
            split_count = 0
            last_http = 0
            for _ in range(10):
                pane0 = get_pane_id(0)
                if not pane0:
                    break
                try:
                    r = requests.post(
                        f"{API_BASE}/api/pane/split",
                        json={"pane_id": pane0, "axis": "Vertical"},
                        timeout=5
                    )
                    last_http = r.status_code
                except requests.RequestException:
                    last_http = 0
                split_count += 1
                time.sleep(0.5)
                if last_http != 200:
                    break

            alive = api_get_code("/api/replay/status") == 200
            try:
                pane_list = api_get_code("/api/pane/list")
            except Exception:
                pane_list = 0

            if alive:
                pass_(f"TC-S21-07: split {split_count} 回後 (HTTP={last_http}) クラッシュなし")
            else:
                fail("TC-S21-07", f"split 繰り返し後にアプリがクラッシュした")

        finally:
            _close_with_logout(env6)


def test_s21_tachibana_error_boundary() -> None:
    """pytest エントリポイント。"""
    _h._PASS = _h._FAIL = _h._PEND = 0
    backup_state()
    try:
        run_s21()
    finally:
        restore_state()
    print_summary()
    assert _h._FAIL == 0, f"{_h._FAIL} TC(s) failed — see output above"


def main() -> None:
    backup_state()
    try:
        run_s21()
    finally:
        restore_state()
        print_summary()
        if _h._FAIL > 0:
            sys.exit(1)


if __name__ == "__main__":
    main()
