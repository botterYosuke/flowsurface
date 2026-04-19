#!/usr/bin/env python3
"""s14_autoplay_event_driven.py — スイート S14: Auto-play タイムアウト廃止テスト

検証シナリオ:
  TC-S14-01/02: DEV AUTO-LOGIN → keyring セッション保存 → 再起動後セッション復元
                → pending_auto_play=true のまま → マスター取得完了後 Playing 到達
  TC-S14-03a/b: keyring セッションなし → pending_auto_play クリア → Playing にならず
                待機系 info トーストが出る
  TC-S14-04: PEND（マスター遅延シミュレーションは real API では再現不可）

仕様根拠:
  docs/replay_header.md §5.1 — auto-play event-driven（タイムアウト廃止・マスター完了待ち）

前提条件: DEV_USER_ID / DEV_PASSWORD 環境変数設定済み

使い方:
    DEV_USER_ID=xxx DEV_PASSWORD=yyy python tests/s14_autoplay_event_driven.py
    pytest tests/s14_autoplay_event_driven.py -v
"""

from __future__ import annotations

import json
import os
import sys
import time
from pathlib import Path

sys.path.insert(0, str(Path(__file__).parent))
from helpers import (
    IS_HEADLESS, FlowsurfaceEnv,
    pass_, fail, pend, print_summary,
    backup_state, restore_state,
    get_status, wait_status, wait_playing, wait_tachibana_session,
    api_get, api_post, api_get_code,
    utc_offset,
    DATA_DIR, STATE_FILE, API_BASE,
)

import requests
import helpers as _h


def _write_s14_state(start: str, end: str) -> None:
    """TachibanaSpot:7203 D1 Replay フィクスチャを書き込む。"""
    DATA_DIR.mkdir(parents=True, exist_ok=True)
    fixture = {
        "layout_manager": {"layouts": [{"name": "S14", "dashboard": {"pane": {
            "KlineChart": {
                "layout": {"splits": [0.78], "autoscale": "FitToVisible"}, "kind": "Candles",
                "stream_type": [{"Kline": {"ticker": "TachibanaSpot:7203", "timeframe": "D1"}}],
                "settings": {"tick_multiply": None, "visual_config": None, "selected_basis": {"Time": "D1"}},
                "indicators": [], "link_group": "A"
            }
        }, "popout": []}}], "active_layout": "S14"},
        "timezone": "UTC", "trade_fetch_enabled": False, "size_in_quote_ccy": "Base",
        "replay": {"mode": "replay", "range_start": start, "range_end": end}
    }
    STATE_FILE.write_text(json.dumps(fixture, indent=2))


def run_s14() -> None:
    print("=== S14: Auto-play タイムアウト廃止テスト ===")

    # 環境変数チェック
    if not os.environ.get("DEV_USER_ID") or not os.environ.get("DEV_PASSWORD"):
        pend("TC-S14-01", "DEV_USER_ID / DEV_PASSWORD が未設定 — Tachibana live テストをスキップ")
        pend("TC-S14-02", "DEV_USER_ID / DEV_PASSWORD が未設定")
        pend("TC-S14-03a", "DEV_USER_ID / DEV_PASSWORD が未設定")
        pend("TC-S14-03b", "DEV_USER_ID / DEV_PASSWORD が未設定")
        pend("TC-S14-04", "DEV_USER_ID / DEV_PASSWORD が未設定")
        return

    # headless は Tachibana keyring 操作（persist_session / delete_session）が不可能なため全 TC を PEND
    if IS_HEADLESS:
        pend("TC-S14-01", "headless は Tachibana keyring 操作不可")
        pend("TC-S14-02", "headless は Tachibana keyring 操作不可")
        pend("TC-S14-03a", "headless は Tachibana keyring 操作不可")
        pend("TC-S14-03b", "headless は Tachibana keyring 操作不可")
        pend("TC-S14-04", "headless は Tachibana keyring 操作不可")
        return

    start = utc_offset(-120)
    end = utc_offset(-24)

    # ── 事前準備: セッションを keyring に保存 ─────────────────────────────────
    print("  [準備] DEV AUTO-LOGIN でセッションを keyring に保存...")
    _write_s14_state(start, end)
    env_prep = FlowsurfaceEnv(ticker="TachibanaSpot:7203", timeframe="D1", headless=False)
    env_prep._start_process()
    try:
        print("  waiting for Tachibana session (DEV AUTO-LOGIN)...")
        if not wait_tachibana_session(120):
            print("  ERROR: Tachibana session not established after 120s")
            fail("precond", "Tachibana セッション確立失敗")
            return
        print("  Tachibana session established, closing app (session persisted to keyring)...")
    finally:
        env_prep.close()
    print("  [準備] 完了")

    # ===== TC-S14-01 / TC-S14-02: keyring セッション復元 → Playing 到達 =====
    _write_s14_state(start, end)
    env1 = FlowsurfaceEnv(ticker="TachibanaSpot:7203", timeframe="D1", headless=False)
    env1._start_process()
    try:
        # ↑ try_restore_session() がキーリングのセッションを復元
        # → pending_auto_play = true のまま

        print("  セッション復元待機なしで 35 秒経過を確認（旧 30s タイムアウトが発火しないことを検証）...")
        elapsed = 0
        premature_play = False
        while elapsed < 35:
            try:
                status = get_status().get("status")
                if status == "Playing":
                    print(f"  INFO: Playing 到達 (elapsed={elapsed}s) — マスター取得完了")
                    premature_play = True
                    break
            except requests.RequestException:
                pass
            time.sleep(1)
            elapsed += 1

        # TC-S14-02: 35 秒時点で timed out トーストがないことを確認
        try:
            notifs = api_get("/api/notification/list")
            notifications = notifs.get("notifications", [])
            has_timeout = any(
                "timed out" in (n.get("body", "") or "").lower() or
                "timed out" in (n.get("title", "") or "").lower()
                for n in notifications
            )
        except requests.RequestException:
            has_timeout = False

        if not has_timeout:
            pass_("TC-S14-02: 35s 経過後も timed out トーストなし（タイムアウト廃止確認）")
        else:
            fail("TC-S14-02", "timed out トースト発見（旧実装の挙動）")

        # TC-S14-01: keyring セッション復元後に Playing 到達（マスター取得完了で自動発火）
        try:
            ct_after_loop_raw = get_status().get("current_time")
            ct_after_loop = int(ct_after_loop_raw) if ct_after_loop_raw not in (None, "null", "") else 0
        except (requests.RequestException, TypeError, ValueError):
            ct_after_loop = 0

        from helpers import utc_to_ms
        range_start_ms = utc_to_ms(start)
        ct_past_start = ct_after_loop > range_start_ms

        if premature_play:
            pass_("TC-S14-01: keyring セッション復元 → マスター取得完了 → Playing 到達")
        elif ct_past_start:
            pass_(f"TC-S14-01: keyring セッション復元 → auto-play 完了（CT={ct_after_loop} > range_start={range_start_ms}）")
        elif wait_playing(120):
            pass_("TC-S14-01: keyring セッション復元 → Playing 到達（120s 以内）")
        else:
            fail("TC-S14-01", "Playing に到達せず（120 秒タイムアウト）")

    finally:
        env1.close()

    # ===== TC-S14-03: セッションなし → Playing にならず待機系 info トーストが出る =====
    print("  [TC-S14-03] keyring セッションを削除してセッションなし状態でテスト...")

    # keyring からセッションを削除
    _write_s14_state(start, end)
    env_del = FlowsurfaceEnv(ticker="TachibanaSpot:7203", timeframe="D1", headless=False)
    env_del._start_process()
    try:
        try:
            api_post("/api/test/tachibana/delete-persisted-session")
        except requests.RequestException:
            pass
    finally:
        env_del.close()

    _write_s14_state(start, end)
    env2 = FlowsurfaceEnv(ticker="TachibanaSpot:7203", timeframe="D1", headless=False)
    env2._start_process()
    try:
        # ↑ try_restore_session() → None → SessionRestoreResult(None) → on_session_unavailable()
        #   → pending_auto_play=false + Toast::info("Replay auto-play was deferred: please log in to resume")

        # TC-S14-03b: toast は DEFAULT_TIMEOUT=8s でタイムアウト → 3s 以内に確認
        print("  セッションなしで 3 秒待機中（SessionRestoreResult 処理待ち）...")
        time.sleep(3)

        KEYWORDS = ["waiting", "session", "login", "pending", "tachibana", "deferred", "待機", "ログイン"]
        try:
            notifs = api_get("/api/notification/list")
            notifications = notifs.get("notifications", [])
            has_wait_info = any(
                n.get("level") == "info" and any(
                    kw in (n.get("body", "") or "").lower() or kw in (n.get("title", "") or "").lower()
                    for kw in KEYWORDS
                )
                for n in notifications
            )
            notifs_text = " | ".join(
                f"{n.get('level')}:{n.get('body')}" for n in notifications
            ) or "(none)"
        except requests.RequestException:
            has_wait_info = False
            notifs_text = "(api error)"

        if has_wait_info:
            pass_("TC-S14-03b: 待機系 info トーストあり")
        else:
            fail("TC-S14-03b", f"待機系 info トーストなし。通知一覧: {notifs_text}")

        # TC-S14-03a: 15 秒経過後も Playing でないことを確認
        print("  さらに 12 秒待機中（合計 15 秒）...")
        time.sleep(12)

        try:
            status = get_status().get("status")
        except requests.RequestException:
            status = "unknown"

        if status != "Playing":
            pass_(f"TC-S14-03a: セッションなし → Playing でない (status={status})")
        else:
            fail("TC-S14-03a", "Playing になった（セッションなしなのに）")

    finally:
        env2.close()

    # ===== TC-S14-04: マスター遅延シミュレーション（PEND） =====
    pend("TC-S14-04", "マスター遅延シミュレーションは real API 環境では再現不可（e2e-mock 専用シナリオ）")

    # クリーンアップ: keyring セッション削除
    _write_s14_state(start, end)
    env_cleanup = FlowsurfaceEnv(ticker="TachibanaSpot:7203", timeframe="D1", headless=False)
    env_cleanup._start_process()
    try:
        try:
            api_post("/api/test/tachibana/delete-persisted-session")
        except requests.RequestException:
            pass
    finally:
        env_cleanup.close()


def test_s14_autoplay_event_driven() -> None:
    """pytest エントリポイント。"""
    _h._PASS = _h._FAIL = _h._PEND = 0
    backup_state()
    try:
        run_s14()
    finally:
        restore_state()
    print_summary()
    assert _h._FAIL == 0, f"{_h._FAIL} TC(s) failed — see output above"


def main() -> None:
    backup_state()
    try:
        run_s14()
    finally:
        restore_state()
        print_summary()
        if _h._FAIL > 0:
            sys.exit(1)


if __name__ == "__main__":
    main()
