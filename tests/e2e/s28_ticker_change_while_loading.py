#!/usr/bin/env python3
"""s28_ticker_change_while_loading.py — S28: Loading（Waiting）状態中の銘柄変更

検証シナリオ（仕様 §6.6「銘柄変更による初期状態リセット」Waiting 状態部分）:
  TC-setup: Playing → split + ETHUSDT 設定 → Loading 状態を確認
  TC-A: Loading 中（または直後）に元ペインの ticker を SOLUSDT に変更 → クラッシュなし
  TC-B: 変更後 最大 30s 待機 → status=Paused（自動再生されない）
  TC-C: Paused 状態で current_time≈start_time（リセット発生の確認）
  TC-D: Resume → Playing 到達（回復可能であること）

仕様根拠:
  docs/replay_header.md §6.6 — 銘柄変更による初期状態リセット（Waiting 状態中も適用）
  s23 は Playing/Paused 中をカバー済み。本テストは Waiting（API: "Loading"）中を対象とする。

使い方:
    E2E_TICKER=BinanceLinear:BTCUSDT python tests/s28_ticker_change_while_loading.py
    IS_HEADLESS=true python tests/s28_ticker_change_while_loading.py
    pytest tests/s28_ticker_change_while_loading.py -v
"""

from __future__ import annotations

import sys
import time
from pathlib import Path

sys.path.insert(0, str(Path(__file__).parent))
from helpers import (
    TICKER, IS_HEADLESS, FlowsurfaceEnv,
    pass_, fail, pend, print_summary,
    backup_state, restore_state,
    setup_single_pane, headless_play,
    get_status, wait_status, wait_playing,
    wait_for_pane_streams_ready,
    api_post, api_post_code, api_get,
    utc_offset, utc_to_ms,
    get_pane_id,
    primary_ticker, secondary_ticker, tertiary_ticker,
)

import requests
import helpers as _h


def run_s28() -> None:
    print(f"=== S28: Loading（Waiting）状態中の銘柄変更 (ticker={TICKER}) ===")

    # 5h レンジ（300 bar M1）を使い、ロードに 2〜5 秒かかることで Loading を捕捉しやすくする
    start = utc_offset(-6)
    end = utc_offset(-1)
    start_ms = utc_to_ms(start)

    print(f"  range: {start} → {end} (start_ms={start_ms})")

    primary = primary_ticker()
    secondary = secondary_ticker()
    tertiary = tertiary_ticker()

    setup_single_pane(primary, "M1", start, end)

    env = FlowsurfaceEnv(ticker=TICKER, timeframe="M1", headless=IS_HEADLESS)
    try:
        env._start_process()
        headless_play()

        # Playing に到達するまで待機（最大 90 秒: 5h レンジのフェッチ時間を考慮）
        if not wait_playing(90):
            fail("precond", "auto-play で Playing に到達せず")
            return
        print("  Playing 到達")

        # ── ペイン ID 取得 ──────────────────────────────────────────────────────
        pane0 = get_pane_id(0)
        if not pane0:
            fail("precond", "初期ペイン ID 取得失敗")
            return
        print(f"  PANE0={pane0}")

        # ────────────────────────────────────────────────────────────────────────
        # TC-setup: Playing 中に split → 新ペインに ETHUSDT → Loading 状態を確認
        # ────────────────────────────────────────────────────────────────────────
        print()
        print("── TC-setup: split + ETHUSDT 設定 → Loading 遷移を確認")

        api_post_code("/api/pane/split", {"pane_id": pane0, "axis": "Vertical"})
        time.sleep(0.3)

        # 新ペイン ID を取得
        try:
            panes_body = api_get("/api/pane/list")
            panes = panes_body.get("panes", [])
            new_pane = next((p.get("id", "") for p in panes if p.get("id") != pane0), "")
        except requests.RequestException:
            new_pane = ""

        if not new_pane:
            fail("TC-setup", "split 後の新ペイン ID 取得失敗")
            return
        print(f"  NEW_PANE={new_pane}")

        # 新ペインに secondary ticker を設定 → 新ストリームのロードが始まる → Loading 遷移
        api_post_code("/api/pane/set-ticker", {"pane_id": new_pane, "ticker": secondary})

        # Loading 状態を 100ms ポーリングで最大 5 秒間確認
        loading_caught = False
        for _ in range(50):
            try:
                st = get_status().get("status")
                if st == "Loading":
                    loading_caught = True
                    break
            except requests.RequestException:
                pass
            time.sleep(0.1)

        # Loading 捕捉は保証できない（ロードが瞬時に完了した場合）ため、INFO 扱い
        if loading_caught:
            print("  INFO: Loading 状態を確認（TC-A は Waiting 中の ticker 変更をテスト）")
        else:
            print("  INFO: Loading を捕捉できず（ロードが高速で完了した可能性あり）")
            print("        TC-A は Playing または Paused 中の ticker 変更をテストする（§6.6 の別ケース）")

        # ────────────────────────────────────────────────────────────────────────
        # TC-A: Loading 中（または直後）に元ペインの ticker を SOLUSDT に変更
        # ────────────────────────────────────────────────────────────────────────
        print()
        print(f"── TC-A: 元ペイン ticker を {tertiary} に変更 → クラッシュなし")

        # ticker 変更を実行（Loading / Playing / Paused のいずれの状態でも §6.6 リセットが適用される）
        api_post_code("/api/pane/set-ticker", {"pane_id": pane0, "ticker": tertiary})
        print(f"  ticker 変更送信完了 (PANE0 → {tertiary})")

        # ────────────────────────────────────────────────────────────────────────
        # TC-B: 変更後 最大 30s 待機 → status=Paused（自動再生されない）
        # ────────────────────────────────────────────────────────────────────────
        print()
        print("── TC-B: 変更後 status=Paused になること")

        if wait_status("Paused", 30):
            pass_("TC-B: ticker 変更後 status=Paused（自動再生なし）")
        else:
            try:
                last_st = get_status().get("status")
            except requests.RequestException:
                last_st = "unknown"
            fail("TC-B", f"30s 待機後 status={last_st} (expected Paused)")

        # ────────────────────────────────────────────────────────────────────────
        # TC-C: Paused 状態で current_time≈start_time
        # ────────────────────────────────────────────────────────────────────────
        print()
        print("── TC-C: current_time≈start_time（リセット発生の確認）")

        try:
            ct_after_str = get_status().get("current_time")
            ct_after = int(ct_after_str) if ct_after_str is not None else None
        except (requests.RequestException, TypeError, ValueError):
            ct_after = None
        print(f"  current_time={ct_after} (start_ms={start_ms})")

        if ct_after is not None:
            diff = abs(ct_after - start_ms)
            tol = 60000  # 1 bar = 60s
            if diff <= tol:
                pass_(f"TC-C: ticker 変更後 current_time≈start_time (ct={ct_after} st={start_ms})")
            else:
                fail(
                    "TC-C",
                    f"current_time={ct_after} は start_time={start_ms} から 1 bar 以上離れている（リセット未発生の疑い）",
                )
        else:
            fail("TC-C", "current_time が null")

        # ────────────────────────────────────────────────────────────────────────
        # TC-D: tertiary ticker のデータロード待機 → Resume → Playing 到達
        # ────────────────────────────────────────────────────────────────────────
        print()
        print(f"── TC-D: {tertiary} streams_ready 待機 → Resume → Playing 到達")

        # tertiary ticker のロードが完了するまで待機
        if wait_for_pane_streams_ready(pane0, 30):
            print(f"  PANE0 ({tertiary}) streams_ready=true")
        else:
            print("  WARN: PANE0 streams_ready timeout (continuing)")

        try:
        except requests.RequestException:
            pass

        if wait_status("Playing", 30):
            pass_("TC-D: Resume 後 status=Playing（回復可能）")
        else:
            try:
                status = get_status().get("status")
            except requests.RequestException:
                status = "unknown"
            fail("TC-D", f"status={status} (expected Playing)")

    finally:
        env.close()


def test_s28_ticker_change_while_loading() -> None:
    """pytest エントリポイント。プロセス起動は外部で行うこと。"""
    _h._PASS = _h._FAIL = _h._PEND = 0
    run_s28()
    print_summary()
    assert _h._FAIL == 0, f"{_h._FAIL} TC(s) failed — see output above"


def main() -> None:
    backup_state()
    try:
        run_s28()
    finally:
        restore_state()
        print_summary()
        if _h._FAIL > 0:
            sys.exit(1)


if __name__ == "__main__":
    main()
