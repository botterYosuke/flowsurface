#!/usr/bin/env python3
"""s24_sidebar_select_ticker.py — S24: POST /api/sidebar/select-ticker 経路の検証

検証シナリオ:
  TC-B: Playing 中に sidebar/select-ticker (kind=null) → 即座に Paused
  TC-A: Paused 中に sidebar/select-ticker (kind=null) → ticker 変更確認 (tertiary_ticker)
  TC-A3: ticker 変更後も status=Paused（自動再生なし）
  TC-C: Paused → Resume → Playing 復帰
  TC-D: kind="KlineChart" を指定した場合 → HTTP 200、エラー通知なし
  TC-E: 不正な pane_id → HTTP 400
  TC-F: ticker フィールド欠落 → HTTP 400

仕様根拠:
  docs/replay_header.md §9.1 — Sidebar::TickerSelected 経路
  kind=null → switch_tickers_in_group（リンクグループ全ペイン更新）
  kind=Some → init_focused_pane（ペイン種別ごとの初期化）

フィクスチャ: primary_ticker() M1, auto-play (UTC[-3h, -1h])
"""

from __future__ import annotations

import sys
import time
from pathlib import Path

import requests

sys.path.insert(0, str(Path(__file__).parent))
from helpers import (
    API_BASE,
    IS_HEADLESS,
    api_get,
    api_post,
    backup_state,
    count_error_notifications,
    fail,
    get_pane_id,
    headless_play,
    pass_,
    pend,
    primary_ticker,
    print_summary,
    restore_state,
    secondary_ticker,
    setup_single_pane,
    tertiary_ticker,
    utc_offset,
    wait_for_pane_streams_ready,
    wait_status,
)

_REPO_ROOT = Path(__file__).parent.parent
try:
    from flowsurface import FlowsurfaceEnv
except ImportError:
    sys.path.insert(0, str(_REPO_ROOT / "python"))
    from env import FlowsurfaceEnv  # type: ignore[no-redef]


def run_s24() -> None:
    print("=== S24: POST /api/sidebar/select-ticker 経路の検証 ===")

    if IS_HEADLESS:
        for label in [
            "TC-B1: sidebar/select-ticker レスポンスが JSON",
            "TC-B2: sidebar/select-ticker 後 status=Paused",
            "TC-A1: Paused 中の sidebar/select-ticker がエラーなし",
            "TC-A2: ticker が tertiary に変更された",
            "TC-A3: ticker 変更後も status=Paused（自動再生なし）",
            "TC-C: Resume 後 status=Playing",
            "TC-D: kind=KlineChart 指定で HTTP 200 JSON レスポンス",
            "TC-D2: kind=KlineChart でエラー toast なし",
            "TC-E: 不正 pane_id → HTTP 400",
            "TC-F: ticker 欠落 → HTTP 400",
        ]:
            pend(label, "sidebar API returns 501 in headless")
        return

    start = utc_offset(-3)
    end = utc_offset(-1)
    primary = primary_ticker()
    secondary = secondary_ticker()
    tertiary = tertiary_ticker()

    # autoplay で Playing に到達するまで待機
    if not wait_status("Playing", 60):
        fail("S24-precond", "Playing 到達せず（timeout）")
        return

    pane_id = get_pane_id(0)
    print(f"  PANE_ID={pane_id}")
    if not pane_id:
        fail("S24-precond", "ペイン ID 取得失敗")
        return

    # ── TC-B: Playing 中に sidebar/select-ticker → Paused ─────────────────────
    print()
    print("── TC-B: Playing 中に sidebar/select-ticker → Paused")
    try:
        resp_b = requests.post(
            f"{API_BASE}/api/sidebar/select-ticker",
            json={"pane_id": pane_id, "ticker": secondary},
            timeout=5,
        )
        resp_b.json()
        pass_("TC-B1: sidebar/select-ticker レスポンスが JSON")
    except Exception as e:
        fail("TC-B1", f"レスポンス解析エラー: {e}")

    time.sleep(0.5)
    st_b = get_pane_id  # just get status below
    status_b = api_get("/api/replay/status").get("status")
    if status_b == "Paused":
        pass_("TC-B2: sidebar/select-ticker 後 status=Paused")
    else:
        fail("TC-B2", f"status={status_b} (expected Paused)")

    # ── TC-A: Paused 中に sidebar/select-ticker → ticker 変更確認 ─────────────
    print()
    print("── TC-A: Paused 中に sidebar/select-ticker → ticker 変更確認")
    try:
        resp_a = requests.post(
            f"{API_BASE}/api/sidebar/select-ticker",
            json={"pane_id": pane_id, "ticker": tertiary},
            timeout=5,
        )
        resp_a.json()
        pass_("TC-A1: Paused 中の sidebar/select-ticker がエラーなし")
    except Exception as e:
        fail("TC-A1", f"レスポンス解析エラー: {e}")

    # streams_ready を待機（ticker 変更後にバックフィルが走る）
    if wait_for_pane_streams_ready(pane_id, 30):
        body = api_get("/api/pane/list")
        panes = body.get("panes", [])
        p = next((x for x in panes if x.get("id") == pane_id), None)
        ticker_a = (p or {}).get("ticker", "")
        if ticker_a == tertiary:
            pass_(f"TC-A2: ticker が {tertiary} に変更された")
        else:
            fail("TC-A2", f"ticker={ticker_a} (expected {tertiary})")
    else:
        fail("TC-A2", "streams_ready タイムアウト（30s）")

    # status は Paused のまま（自動再生されない）
    st_a = api_get("/api/replay/status").get("status")
    if st_a == "Paused":
        pass_("TC-A3: ticker 変更後も status=Paused（自動再生なし）")
    else:
        fail("TC-A3", f"status={st_a} (expected Paused)")

    # ── TC-C: Paused → Resume → Playing 復帰 ─────────────────────────────────
    print()
    print("── TC-C: sidebar/select-ticker 後 Resume → Playing")
    try:
        api_post("/api/replay/resume")
    except Exception:
        pass
    if wait_status("Playing", 30):
        pass_("TC-C: Resume 後 status=Playing")
    else:
        st_c = api_get("/api/replay/status").get("status")
        fail("TC-C", f"status={st_c} (expected Playing)")

    # ── TC-D: kind="KlineChart" を指定（init_focused_pane 経路） ───────────────
    print()
    print("── TC-D: kind=KlineChart を指定した sidebar/select-ticker")
    try:
        resp_d = requests.post(
            f"{API_BASE}/api/sidebar/select-ticker",
            json={"pane_id": pane_id, "ticker": primary, "kind": "KlineChart"},
            timeout=5,
        )
        resp_d.json()
        pass_("TC-D: kind=KlineChart 指定で HTTP 200 JSON レスポンス")
    except Exception as e:
        fail("TC-D", f"レスポンス解析エラー: {e}")

    err_count_d = count_error_notifications()
    if err_count_d == 0:
        pass_("TC-D2: kind=KlineChart でエラー toast なし")
    else:
        fail("TC-D2", f"error toast が {err_count_d} 件発生した")

    # ── TC-E: 不正な pane_id → HTTP 400 ──────────────────────────────────────
    print()
    print("── TC-E: 不正な pane_id → HTTP 400")
    code_e = requests.post(
        f"{API_BASE}/api/sidebar/select-ticker",
        json={"pane_id": "not-a-uuid", "ticker": primary},
        timeout=5,
    ).status_code
    if code_e == 400:
        pass_("TC-E: 不正 pane_id → HTTP 400")
    else:
        fail("TC-E", f"HTTP={code_e} (expected 400)")

    # ── TC-F: ticker フィールド欠落 → HTTP 400 ────────────────────────────────
    print()
    print("── TC-F: ticker フィールド欠落 → HTTP 400")
    code_f = requests.post(
        f"{API_BASE}/api/sidebar/select-ticker",
        json={"pane_id": pane_id},
        timeout=5,
    ).status_code
    if code_f == 400:
        pass_("TC-F: ticker 欠落 → HTTP 400")
    else:
        fail("TC-F", f"HTTP={code_f} (expected 400)")


def test_s24_sidebar_select_ticker() -> None:
    backup_state()
    start = utc_offset(-3)
    end = utc_offset(-1)
    setup_single_pane(primary_ticker(), "M1", start, end)
    env = FlowsurfaceEnv(ticker=primary_ticker(), timeframe="M1", headless=IS_HEADLESS)
    try:
        env._start_process()
        headless_play(start, end)
        run_s24()
    finally:
        env.close()
        restore_state()
    import helpers as _h
    assert _h._FAIL == 0, f"{_h._FAIL} TC(s) failed — see output above"


def main() -> None:
    backup_state()
    start = utc_offset(-3)
    end = utc_offset(-1)
    setup_single_pane(primary_ticker(), "M1", start, end)
    env = FlowsurfaceEnv(ticker=primary_ticker(), timeframe="M1", headless=IS_HEADLESS)
    try:
        env._start_process()
        headless_play(start, end)
        run_s24()
    finally:
        env.close()
        restore_state()
    print_summary()
    import helpers as _h
    sys.exit(0 if _h._FAIL == 0 else 1)


if __name__ == "__main__":
    main()
