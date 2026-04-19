#!/usr/bin/env python3
"""s33_sidebar_split_pane.py — S33: sidebar/select-ticker + kind 指定によるペイン分割テスト

検証シナリオ:
  TC-A: kind=KlineChart で secondary_ticker() を選択 → wait_for_pane_count(2, 15)
  TC-B: 新ペインの ticker に secondary symbol が含まれる
  TC-C: 元ペインの ticker は PRIMARY symbol のまま
  TC-D: エラー通知 0 件
  TC-E: 2 回目 split (tertiary, kind=KlineChart) → ペイン数 3

仕様根拠:
  docs/replay_header.md §9.1 — Sidebar::TickerSelected + kind 指定によるペイン分割フロー
  kind=KlineChart → init_focused_pane 経路（フォーカスペインを上書きせず Horizontal Split）

フィクスチャ: primary_ticker() M1, auto-play (UTC[-3h, -1h])
"""

from __future__ import annotations

import sys
from pathlib import Path

sys.path.insert(0, str(Path(__file__).parent))
from helpers import (
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
    wait_for_pane_count,
    wait_status,
)

_REPO_ROOT = Path(__file__).parent.parent
try:
    from flowsurface import FlowsurfaceEnv
except ImportError:
    sys.path.insert(0, str(_REPO_ROOT / "python"))
    from env import FlowsurfaceEnv  # type: ignore[no-redef]


def run_s33() -> None:
    print("=== S33: sidebar/select-ticker + kind 指定によるペイン分割テスト ===")

    if IS_HEADLESS:
        for label in [
            "TC-A: kind=KlineChart → ペイン数 2",
            "TC-B: 新ペインの ticker に secondary symbol が含まれる",
            "TC-C: 元ペインの ticker は PRIMARY symbol のまま",
            "TC-D: エラー通知 0 件",
            "TC-E: 2 回目 kind=KlineChart → ペイン数 3",
        ]:
            pend(label, "sidebar API returns 501 in headless")
        return

    primary = primary_ticker()
    secondary = secondary_ticker()
    tertiary = tertiary_ticker()
    sec_symbol = secondary.split(":", 1)[-1]
    pri_symbol = primary.split(":", 1)[-1]

    # autoplay で Playing に到達するまで待機
    if not wait_status("Playing", 60):
        fail("S33-precond", "Playing 到達せず（timeout）")
        return

    pane0_id = get_pane_id(0)
    print(f"  PANE0={pane0_id}")
    if not pane0_id:
        fail("S33-precond", "初期ペイン ID 取得失敗")
        return

    # ── TC-A: kind=KlineChart で secondary を選択 → ペイン数 2 ─────────────────
    print()
    print(f"── TC-A: kind=KlineChart で {secondary} を選択 → ペイン数 2")
    try:
        api_post(
            "/api/sidebar/select-ticker",
            {"pane_id": pane0_id, "ticker": secondary, "kind": "KlineChart"},
        )
    except Exception:
        pass

    if wait_for_pane_count(2, 15):
        pass_("TC-A: kind=KlineChart → ペイン数 2")
    else:
        body = api_get("/api/pane/list")
        actual = len(body.get("panes", []))
        fail("TC-A", f"15 秒以内に pane count が 2 にならなかった (actual={actual})")
        return

    # ── TC-B / TC-C: 新・旧ペインの ticker 確認 ──────────────────────────────
    print()
    print("── TC-B/TC-C: ペイン ticker 確認")
    body = api_get("/api/pane/list")
    panes = body.get("panes", [])

    new_pane = next((p for p in panes if p["id"] != pane0_id), None)
    if new_pane is None:
        fail("TC-B", "新ペイン ID 取得失敗")
        return
    print(f"  NEW_PANE={new_pane['id']}")

    new_ticker = new_pane.get("ticker") or ""
    print(f"  new pane ticker={new_ticker}")
    if sec_symbol.lower() in new_ticker.lower():
        pass_(f"TC-B: 新ペインの ticker に {sec_symbol} が含まれる (={new_ticker})")
    else:
        fail("TC-B", f"新ペイン ticker={new_ticker} (expected to contain {sec_symbol})")

    orig_pane = next((p for p in panes if p["id"] == pane0_id), None)
    orig_ticker = (orig_pane or {}).get("ticker") or ""
    print(f"  orig pane ticker={orig_ticker}")
    if pri_symbol.lower() in orig_ticker.lower():
        pass_(f"TC-C: 元ペインの ticker は {pri_symbol} のまま (={orig_ticker})")
    else:
        fail("TC-C", f"元ペイン ticker={orig_ticker} (expected to contain {pri_symbol})")

    # ── TC-D: エラー通知が出ていない ─────────────────────────────────────────
    print()
    print("── TC-D: エラー通知なし確認")
    err_count = count_error_notifications()
    print(f"  error notification count={err_count}")
    if err_count == 0:
        pass_("TC-D: エラー通知 0 件")
    else:
        fail("TC-D", f"エラー通知が {err_count} 件発生")

    # ── TC-E: 2 回目の split（tertiary, kind=KlineChart）→ ペイン数 3 ──────────
    print()
    print(f"── TC-E: 2 回目 split {tertiary} → ペイン数 3")
    try:
        api_post(
            "/api/sidebar/select-ticker",
            {"pane_id": pane0_id, "ticker": tertiary, "kind": "KlineChart"},
        )
    except Exception:
        pass

    if wait_for_pane_count(3, 15):
        pass_(f"TC-E: 2 回目 kind=KlineChart ({tertiary}) → ペイン数 3")
    else:
        body2 = api_get("/api/pane/list")
        actual2 = len(body2.get("panes", []))
        fail("TC-E", f"15 秒以内に pane count が 3 にならなかった (actual={actual2})")


def test_s33_sidebar_split_pane() -> None:
    backup_state()
    start = utc_offset(-3)
    end = utc_offset(-1)
    setup_single_pane(primary_ticker(), "M1", start, end)
    env = FlowsurfaceEnv(ticker=primary_ticker(), timeframe="M1", headless=IS_HEADLESS)
    try:
        env._start_process()
        headless_play(start, end)
        run_s33()
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
        run_s33()
    finally:
        env.close()
        restore_state()
    print_summary()
    import helpers as _h
    sys.exit(0 if _h._FAIL == 0 else 1)


if __name__ == "__main__":
    main()
