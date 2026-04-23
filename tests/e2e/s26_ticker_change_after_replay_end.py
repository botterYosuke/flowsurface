#!/usr/bin/env python3
"""s26_ticker_change_after_replay_end.py — S26: リプレイ終了後の銘柄変更で current_time がリセットされること

検証シナリオ:
  TC-A: リプレイ終了（Paused @ end_time）→ 銘柄変更 → current_time が start_time に戻る
  TC-B: 銘柄変更後のステータスは Paused のまま
  TC-C: Resume → Playing に遷移できる（リセット後の再生が正常）

仕様根拠:
  docs/replay_header.md §6.6 — 銘柄変更による初期状態リセット
  修正前の不具合: Task::chain() により ReloadKlineStream が kline_fetch_task 完了待ちになり
  Tachibana セッションなしで無限ブロック → current_time が end_time のまま固定

使い方:
    E2E_TICKER=BinanceLinear:BTCUSDT python tests/s26_ticker_change_after_replay_end.py
    IS_HEADLESS=true python tests/s26_ticker_change_after_replay_end.py
    pytest tests/s26_ticker_change_after_replay_end.py -v
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
    setup_single_pane, headless_play, speed_to_10x,
    get_status, wait_status, wait_playing,
    api_post, api_post_code, api_get,
    utc_offset, utc_to_ms,
    get_pane_id,
    secondary_ticker,
)

import requests
import helpers as _h


def run_s26() -> None:
    print(f"=== S26: リプレイ終了後の銘柄変更で current_time がリセットされること (ticker={TICKER}) ===")

    # BinanceLinear:BTCUSDT M1、過去 15 分のレンジ（10x 加速で ~1秒以内に終端到達）
    start = utc_offset(-0.5)
    end = utc_offset(-0.25)
    start_ms = utc_to_ms(start)
    end_ms = utc_to_ms(end)

    print(f"  range: {start} → {end} ({start_ms} → {end_ms})")
    setup_single_pane(TICKER, "M1", start, end)

    env = FlowsurfaceEnv(ticker=TICKER, timeframe="M1", headless=IS_HEADLESS)
    try:
        env._start_process()
        headless_play()

        # Playing 到達待機（最大 60 秒）
        if not wait_playing(60):
            fail("precond", "auto-play で Playing に到達せず")
            return
        print("  Playing 到達")

        # ── 10x 加速して終端まで再生 ──────────────────────────────────────────
        speed_to_10x()
        print("  10x 加速完了、終端まで待機...")

        # Paused + current_time ≈ end_time になるまでポーリング（最大 120 秒）
        reached_end = False
        ct_at_end = 0
        start_time_ms = 0
        for _ in range(120):
            try:
                st = get_status()
                ct = st.get("current_time")
                status = st.get("status")
                if status == "Paused" and ct is not None:
                    ct_int = int(ct)
                    # end_time の 2 分以内（120000 ms）なら終端到達とみなす
                    if ct_int >= end_ms - 120000:
                        reached_end = True
                        ct_at_end = ct_int
                        start_time_ms = int(st.get("start_time") or 0)
                        break
            except (requests.RequestException, TypeError, ValueError):
                pass
            time.sleep(1)

        if not reached_end:
            try:
                last_st = get_status()
                fail("precond", f"終端到達しなかった: status={last_st.get('status')} current_time={last_st.get('current_time')}")
            except requests.RequestException:
                fail("precond", "終端到達しなかった: API 応答なし")
            return

        print(f"  終端到達: current_time={ct_at_end} start_time={start_time_ms} end_time={end_ms}")

        # 前提確認: current_time が end_time 近くにある（start_time とは異なる）
        if ct_at_end == start_time_ms:
            print("  [SKIP] current_time が既に start_time と一致 — レンジが小さすぎてテスト不成立")
            return

        # ペイン ID 取得
        pane_id = get_pane_id(0)
        if not pane_id:
            fail("precond", "ペイン ID 取得失敗")
            return
        print(f"  PANE_ID={pane_id}")

        # ─────────────────────────────────────────────────────────────────────
        # TC-A: リプレイ終了後に銘柄変更 → current_time が start_time に戻る
        # ─────────────────────────────────────────────────────────────────────
        print()
        print("── TC-A: リプレイ終了後に銘柄変更 → current_time が start_time に戻る")

        sec_ticker = secondary_ticker()
        api_post_code("/api/pane/set-ticker", {"pane_id": pane_id, "ticker": sec_ticker})
        time.sleep(2)

        try:
            ct_after_change = int(get_status().get("current_time") or 0)
        except (requests.RequestException, TypeError, ValueError):
            ct_after_change = 0
        print(f"  銘柄変更後 current_time={ct_after_change} (start_time={start_time_ms})")

        # start_time の ±1 バー（60秒）以内なら OK（bar スナップによるずれを許容）
        diff = abs(ct_after_change - start_time_ms)
        if diff <= 60000:
            pass_(f"TC-A: 銘柄変更後 current_time が start_time 付近にリセットされた (ct={ct_after_change} st={start_time_ms})")
        else:
            fail(
                "TC-A: current_time がリセットされない",
                f"current_time={ct_after_change} start_time={start_time_ms} end_time={end_ms} "
                "(修正前の挙動: end_time 付近のまま)",
            )

        # ─────────────────────────────────────────────────────────────────────
        # TC-B: 銘柄変更後も Paused のまま（自動再生されない）
        # ─────────────────────────────────────────────────────────────────────
        print()
        print("── TC-B: 銘柄変更後 status=Paused のまま")

        try:
            st_after = get_status().get("status")
        except requests.RequestException:
            st_after = "unknown"

        if st_after == "Paused":
            pass_("TC-B: 銘柄変更後 status=Paused")
        else:
            fail("TC-B", f"status={st_after} (expected Paused)")

        # ─────────────────────────────────────────────────────────────────────
        # TC-C: Resume → Playing に遷移できる
        # ─────────────────────────────────────────────────────────────────────
        print()
        print("── TC-C: Paused → Resume → Playing")

        try:
        except requests.RequestException:
            pass

        if wait_status("Playing", 30):
            pass_("TC-C: リセット後 Resume → Playing 到達")
        else:
            try:
                status = get_status().get("status")
            except requests.RequestException:
                status = "unknown"
            fail("TC-C", f"status={status} (expected Playing)")

    finally:
        env.close()


def test_s26_ticker_change_after_replay_end() -> None:
    """pytest エントリポイント。プロセス起動は外部で行うこと。"""
    _h._PASS = _h._FAIL = _h._PEND = 0
    run_s26()
    print_summary()
    assert _h._FAIL == 0, f"{_h._FAIL} TC(s) failed — see output above"


def main() -> None:
    backup_state()
    try:
        run_s26()
    finally:
        restore_state()
        print_summary()
        if _h._FAIL > 0:
            sys.exit(1)


if __name__ == "__main__":
    main()
