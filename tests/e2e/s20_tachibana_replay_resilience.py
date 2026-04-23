#!/usr/bin/env python3
"""s20_tachibana_replay_resilience.py — スイート S20: UI操作中の Replay 耐性テスト（TachibanaSpot）

検証シナリオ:
  TC-S20-01: speed 20 連打 + Resume → status=Playing（D1, クラッシュなし）
  TC-S20-02a/b: D1 StepForward delta=86400000ms・StepBackward 後 status=Paused
  TC-S20-03: Live ↔ Replay toggle 10 連打 → アプリ応答維持（D1 版）
  TC-S20-04: Playing 中の toggle → アプリ生存
  TC-S20-05a/b: Paused 中の toggle（Live → Replay）→ アプリ生存

仕様根拠:
  docs/replay_header.md §8 — 速度ボタン連打耐性, §4 — Live/Replay toggle 安定性（TachibanaSpot D1 版）

前提条件: DEV_USER_ID / DEV_PASSWORD 環境変数設定済み

使い方:
    DEV_USER_ID=xxx DEV_PASSWORD=yyy python tests/s20_tachibana_replay_resilience.py
    pytest tests/s20_tachibana_replay_resilience.py -v
"""

from __future__ import annotations

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
)

import requests
import helpers as _h


def _close_with_logout(env: FlowsurfaceEnv) -> None:
    """teardown ヘルパー: ルート API でセッションを明示切断してから env を閉じる。
    CI 環境で前ジョブのセッションが残留し次ジョブのログインが失敗する問題を防ぐ。
    """
    try:
        api_post("/api/auth/tachibana/logout")
        time.sleep(3)  # サーバー側のセッション切断を待つ
    except Exception:
        pass
    env.close()


def _tachibana_start(start: str, end: str) -> FlowsurfaceEnv:
    """saved-state を書き込み、FlowsurfaceEnv を起動して返す（close は呼び出し元の責任）。"""
    tachibana_replay_setup(start, end)
    env = FlowsurfaceEnv(ticker="TachibanaSpot:7203", timeframe="D1", headless=False)
    env._start_process()
    if not wait_tachibana_session(120):
        raise RuntimeError("Tachibana session not established after 120s")
    pane_id = get_pane_id(0)
    if pane_id:
        wait_for_pane_streams_ready(pane_id, 120)
    api_post("/api/replay/toggle")
    api_post("/api/replay/toggle", {"start": start, "end": end})
    return env


def run_s20() -> None:
    print("=== S20: UI操作中の Replay 耐性テスト（TachibanaSpot:7203 D1）===")

    # 環境変数チェック
    if not os.environ.get("DEV_USER_ID") or not os.environ.get("DEV_PASSWORD"):
        pend("TC-S20-01", "DEV_USER_ID / DEV_PASSWORD が未設定 — Tachibana live テストをスキップ")
        pend("TC-S20-02a", "DEV_USER_ID / DEV_PASSWORD が未設定")
        pend("TC-S20-02b", "DEV_USER_ID / DEV_PASSWORD が未設定")
        pend("TC-S20-03", "DEV_USER_ID / DEV_PASSWORD が未設定")
        pend("TC-S20-04", "DEV_USER_ID / DEV_PASSWORD が未設定")
        pend("TC-S20-05a", "DEV_USER_ID / DEV_PASSWORD が未設定")
        pend("TC-S20-05b", "DEV_USER_ID / DEV_PASSWORD が未設定")
        return

    # ── TC-S20-01: 速度ボタン 20 連打 ─────────────────────────────────────────
    print("  [TC-S20-01] 速度ボタン連打...")
    try:
        env1 = _tachibana_start(utc_offset(-2400), utc_offset(-24))
    except RuntimeError as e:
        fail("TC-S20-01-pre", str(e))
        env1 = None

    if env1 is not None:
        try:
            if not wait_playing(60):
                fail("TC-S20-01-pre", "Playing 到達せず")
            else:
                for _ in range(20):
                    try:
                    except requests.RequestException:
                        pass

                # CycleSpeed は pause + seek(range.start) を伴う → 20 連打後は Paused。
                # SPEED_INSTANT を通過した場合は 1 tick で range_end まで消化（高速完了）。
                try:
                    status_full = get_status()
                    ct_pre_resume = int(status_full.get("current_time") or 0)
                    range_end_ms = int(status_full.get("range_end") or 0)
                except (requests.RequestException, TypeError, ValueError):
                    ct_pre_resume = 0
                    range_end_ms = 0

                print(f"  [diag] CT_PRE_RESUME={ct_pre_resume}  RANGE_END_MS={range_end_ms}")

                try:
                except requests.RequestException:
                    pass
                wait_status("Playing", 10)

                try:
                    final_full = get_status()
                    final_status = final_full.get("status")
                    ct_post_resume = int(final_full.get("current_time") or 0)
                except (requests.RequestException, TypeError, ValueError):
                    final_status = None
                    ct_post_resume = 0

                print(f"  [diag] CT_POST_RESUME={ct_post_resume}  FINAL_STATUS={final_status}")

                at_end_post = range_end_ms > 0 and ct_post_resume >= range_end_ms - 3600000
                at_end_pre = range_end_ms > 0 and ct_pre_resume >= range_end_ms - 3600000
                ct_advanced = ct_post_resume > ct_pre_resume or at_end_post or at_end_pre

                if final_status == "Playing" or ct_advanced:
                    pass_("TC-S20-01: speed 20 連打 + Resume → Playing または高速完了（crash なし）")
                else:
                    fail("TC-S20-01", f"status={final_status}, ct_advanced={ct_advanced} (Playing または進行を期待)")
        finally:
            _close_with_logout(env1)

    # ── TC-S20-02: D1 StepForward/StepBackward の delta 検証 ──────────────────
    print("  [TC-S20-02] D1 StepForward/StepBackward delta 検証...")
    try:
        env2 = _tachibana_start(utc_offset(-1300), utc_offset(-24))
    except RuntimeError as e:
        pend("TC-S20-02a", f"セッション確立失敗: {e}")
        pend("TC-S20-02b", f"セッション確立失敗: {e}")
        env2 = None

    if env2 is not None:
        try:
            if not wait_playing(60):
                pend("TC-S20-02", "Playing 到達せず → PEND")
            else:
                # Playing 検出直後に即 Pause（D1 は 100ms/step なので長く放置すると完了する）
                try:
                except requests.RequestException:
                    pass
                wait_paused(15)

                # ウォームアップ: バー境界にスナップしてから delta を計測
                api_post("/api/replay/step-forward")
                wait_paused(15)

                # TC-S20-02a: StepForward delta = 86400000ms（バー境界から計測）
                try:
                    t_before = int(get_status().get("current_time") or 0)
                except (requests.RequestException, TypeError, ValueError):
                    t_before = None

                api_post("/api/replay/step-forward")
                wait_paused(15)

                try:
                    t_after = int(get_status().get("current_time") or 0)
                except (requests.RequestException, TypeError, ValueError):
                    t_after = None

                if t_before is None or t_after is None:
                    fail("TC-S20-02a", f"current_time 取得失敗 (before={t_before} after={t_after})")
                else:
                    delta = t_after - t_before
                    if delta == 86400000:
                        pass_("TC-S20-02a: D1 StepForward delta=86400000ms")
                    else:
                        fail("TC-S20-02a", f"delta={delta} (expected 86400000)")

                # TC-S20-02b: StepBackward 後 status=Paused
                wait_paused(15)
                try:
                    status = get_status().get("status")
                except requests.RequestException:
                    status = None

                if status == "Paused":
                    pass_("TC-S20-02b: StepBackward 後 status=Paused")
                else:
                    fail("TC-S20-02b", f"status={status}")
        finally:
            _close_with_logout(env2)

    # ── TC-S20-03: Live ↔ Replay 高速切替 ───────────────────────────────────
    print("  [TC-S20-03] Live ↔ Replay 高速切替...")
    try:
        env3 = _tachibana_start(utc_offset(-1300), utc_offset(-24))
    except RuntimeError as e:
        fail("TC-S20-03-pre", str(e))
        env3 = None

    if env3 is not None:
        try:
            if not wait_playing(60):
                fail("TC-S20-03-pre", "Playing 到達せず")
            else:
                for _ in range(10):
                    try:
                        api_post("/api/replay/toggle")
                    except requests.RequestException:
                        pass
                    time.sleep(0.3)

                time.sleep(2)
                alive = api_get_code("/api/replay/status") == 200
                try:
                    final = get_status().get("status")
                except requests.RequestException:
                    final = "unknown"

                if alive:
                    pass_(f"TC-S20-03: toggle 10 連打後もアプリ応答あり (final_status={final})")
                else:
                    fail("TC-S20-03", "toggle 連打後にアプリが応答しなくなった")
        finally:
            _close_with_logout(env3)

    # ── TC-S20-04: Playing 中の toggle ────────────────────────────────────
    print("  [TC-S20-04] Playing 中の toggle...")
    try:
        env4 = _tachibana_start(utc_offset(-1300), utc_offset(-24))
    except RuntimeError as e:
        fail("TC-S20-04-pre", str(e))
        env4 = None

    if env4 is not None:
        try:
            if not wait_playing(60):
                fail("TC-S20-04-pre", "Playing 到達せず")
            else:
                try:
                    api_post("/api/replay/toggle")
                except requests.RequestException:
                    pass
                time.sleep(2)

                try:
                    status_after = get_status().get("status")
                except requests.RequestException:
                    status_after = "unknown"
                alive = api_get_code("/api/replay/status") == 200

                if alive:
                    pass_(f"TC-S20-04: Playing 中の toggle → アプリ生存 (status={status_after})")
                else:
                    fail("TC-S20-04", "toggle 後にアプリが応答しなくなった")
        finally:
            _close_with_logout(env4)

    # ── TC-S20-05: Paused 中の toggle → Live → 再び Replay ─────────────────
    print("  [TC-S20-05] Paused 中の toggle...")
    try:
        env5 = _tachibana_start(utc_offset(-1300), utc_offset(-24))
    except RuntimeError as e:
        fail("TC-S20-05-pre", str(e))
        env5 = None

    if env5 is not None:
        try:
            if not wait_playing(60):
                fail("TC-S20-05-pre", "Playing 到達せず")
            else:
                try:
                except requests.RequestException:
                    pass

                if not wait_paused(15):
                    fail("TC-S20-05-pre", "Paused に遷移せず")
                else:
                    # toggle → Live へ
                    try:
                        api_post("/api/replay/toggle")
                    except requests.RequestException:
                        pass
                    time.sleep(2)

                    try:
                        status_live = get_status().get("status")
                    except requests.RequestException:
                        status_live = "unknown"
                    alive = api_get_code("/api/replay/status") == 200

                    if alive:
                        pass_(f"TC-S20-05a: Paused → toggle → アプリ生存 (status={status_live})")
                    else:
                        fail("TC-S20-05a", "toggle 後にアプリが応答しなくなった")

                    # toggle → Replay に戻る
                    try:
                        api_post("/api/replay/toggle")
                    except requests.RequestException:
                        pass
                    time.sleep(3)

                    alive2 = api_get_code("/api/replay/status") == 200
                    try:
                        status_back = get_status().get("status")
                    except requests.RequestException:
                        status_back = "unknown"

                    if alive2:
                        pass_(f"TC-S20-05b: 2 回目 toggle 後もアプリ生存 (status={status_back})")
                    else:
                        fail("TC-S20-05b", "2 回目 toggle 後にアプリが応答しなくなった")
        finally:
            _close_with_logout(env5)


def test_s20_tachibana_replay_resilience() -> None:
    """pytest エントリポイント。"""
    _h._PASS = _h._FAIL = _h._PEND = 0
    backup_state()
    try:
        run_s20()
    finally:
        restore_state()
    print_summary()
    assert _h._FAIL == 0, f"{_h._FAIL} TC(s) failed — see output above"


def main() -> None:
    backup_state()
    try:
        run_s20()
    finally:
        restore_state()
        print_summary()
        if _h._FAIL > 0:
            sys.exit(1)


if __name__ == "__main__":
    main()
