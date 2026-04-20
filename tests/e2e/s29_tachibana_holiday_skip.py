#!/usr/bin/env python3
"""s29_tachibana_holiday_skip.py — S29: Tachibana D1 StepBackward 休場日スキップ

検証シナリオ（仕様 §10.1「離散ステップ」休場日スキップ）:
  TC-A: Paused 状態から StepForward で 2025-01-10 (金) 付近まで進める
  TC-B: 金曜から StepForward 1 回 → current_time が前進
  TC-C: 土曜 current_time から StepBackward → 2025-01-10 (金) に戻る（休場日スキップ）
  TC-D: 金曜から StepBackward → 2025-01-09 (木) に戻る（通常ステップ）
  TC-E: StepBackward 連続 5 回 → 毎回取引日に着地（土日に止まらない）

仕様根拠:
  docs/replay_header.md §10.1 — 離散ステップ・休場日スキップ
  Tachibana の EventStore には土日祝の kline が存在しない。

フィクスチャ: TachibanaSpot:7203 D1, 2025-01-07 00:00 〜 2025-01-15 00:00 (UTC)
  取引日: 01-07(火), 01-08(水), 01-09(木), 01-10(金), [01-11 土, 01-12 日], 01-13(月), 01-14(火)

前提条件: DEV_USER_ID / DEV_PASSWORD 環境変数設定済み

使い方:
    DEV_USER_ID=xxx DEV_PASSWORD=yyy python tests/s29_tachibana_holiday_skip.py
    pytest tests/s29_tachibana_holiday_skip.py -v
"""

from __future__ import annotations

import json
import os
import sys
import time
from datetime import datetime, timezone
from pathlib import Path

sys.path.insert(0, str(Path(__file__).parent))
from helpers import (
    FlowsurfaceEnv,
    pass_, fail, pend, print_summary,
    backup_state, restore_state,
    get_status, wait_status, wait_playing,
    wait_tachibana_session, wait_for_pane_streams_ready,
    api_post, api_get_code,
    get_pane_id,
    utc_to_ms,
    STEP_D1,
    DATA_DIR, STATE_FILE,
)

import requests
import helpers as _h

# ── テスト対象日付の定数（UTC ms）────────────────────────────────────────────
RANGE_START = "2025-01-07 00:00"
RANGE_END = "2025-01-15 00:00"

MS_JAN07 = utc_to_ms("2025-01-07 00:00")  # 火
MS_JAN08 = utc_to_ms("2025-01-08 00:00")  # 水
MS_JAN09 = utc_to_ms("2025-01-09 00:00")  # 木
MS_JAN10 = utc_to_ms("2025-01-10 00:00")  # 金
MS_JAN11 = utc_to_ms("2025-01-11 00:00")  # 土（休場）
MS_JAN12 = utc_to_ms("2025-01-12 00:00")  # 日（休場）
MS_JAN13 = utc_to_ms("2025-01-13 00:00")  # 月


def _is_near_ms(ct: int, target: int, tol: int = 172_800_000) -> bool:
    """ct が target の ±tol ms 以内かどうか（デフォルト ±2 日）。"""
    return abs(ct - target) <= tol


def _is_trading_day(ct_ms: int) -> bool:
    """current_time が取引日（月〜金）かどうか確認（土曜・日曜でないこと）。"""
    d = datetime.fromtimestamp(ct_ms / 1000, tz=timezone.utc)
    return d.weekday() < 5  # Mon=0..Fri=4


def run_s29() -> None:
    print("=== S29: Tachibana D1 StepBackward 休場日スキップ ===")

    # 環境変数チェック
    if not os.environ.get("DEV_USER_ID") or not os.environ.get("DEV_PASSWORD"):
        pend("TC-A", "DEV_USER_ID / DEV_PASSWORD が未設定 — Tachibana セッション不要環境ではスキップ")
        pend("TC-B", "DEV_USER_ID / DEV_PASSWORD が未設定")
        pend("TC-C1", "DEV_USER_ID / DEV_PASSWORD が未設定")
        pend("TC-C2", "DEV_USER_ID / DEV_PASSWORD が未設定")
        pend("TC-C3", "DEV_USER_ID / DEV_PASSWORD が未設定")
        pend("TC-D1", "DEV_USER_ID / DEV_PASSWORD が未設定")
        pend("TC-D2", "DEV_USER_ID / DEV_PASSWORD が未設定")
        pend("TC-E", "DEV_USER_ID / DEV_PASSWORD が未設定")
        return

    print(f"  レンジ: {RANGE_START} → {RANGE_END}")
    print(f"  取引日 ms: 01-07={MS_JAN07}, 01-09={MS_JAN09}, 01-10={MS_JAN10}, 01-11={MS_JAN11}, 01-13={MS_JAN13}")

    # ── フィクスチャ: Tachibana D1 (Live 起動 → セッション確立後に toggle + play) ─
    DATA_DIR.mkdir(parents=True, exist_ok=True)
    fixture = {
        "layout_manager": {"layouts": [{"name": "S29-TachibanaHoliday", "dashboard": {"pane": {
            "KlineChart": {
                "layout": {"splits": [0.78], "autoscale": "FitToVisible"}, "kind": "Candles",
                "stream_type": [{"Kline": {"ticker": "TachibanaSpot:7203", "timeframe": "D1"}}],
                "settings": {"tick_multiply": None, "visual_config": None, "selected_basis": {"Time": "D1"}},
                "indicators": ["Volume"], "link_group": "A"
            }
        }, "popout": []}}], "active_layout": "S29-TachibanaHoliday"},
        "timezone": "UTC", "trade_fetch_enabled": False, "size_in_quote_ccy": "Base"
    }
    STATE_FILE.write_text(json.dumps(fixture, indent=2))

    env = FlowsurfaceEnv(ticker="TachibanaSpot:7203", timeframe="D1", headless=False)
    env._start_process()
    try:
        # Tachibana セッション確立まで待機（最大 120 秒）
        print("  Tachibana セッション待機...")
        if not wait_tachibana_session(120):
            fail("precond", "Tachibana セッション確立失敗（120s タイムアウト）")
            return
        print("  Tachibana セッション確立")

        pane_id = get_pane_id(0)
        if not pane_id:
            fail("precond", "ペイン ID 取得失敗")
            return
        print(f"  PANE_ID={pane_id}")

        # D1 klines が Ready になるまで待機
        print("  D1 klines 待機 (streams_ready)...")
        if not wait_for_pane_streams_ready(pane_id, 120):
            print("  WARN: streams_ready timeout (continuing)")

        # Replay モードへ toggle
        api_post("/api/replay/toggle")
        print("  Replay モードに切替")

        # Play 発火（固定レンジ）
        api_post("/api/replay/play", {"start": RANGE_START, "end": RANGE_END})
        print("  Play 送信")

        # Playing に到達するまで待機（最大 180 秒）
        print("  Playing 待機（最大 180s）...")
        if not wait_status("Playing", 180):
            fail("precond", "Playing に到達せず")
            return
        print("  Playing 到達")

        # Pause して step 操作を開始
        api_post("/api/replay/pause")
        time.sleep(0.5)

        try:
            ct_init_raw = get_status().get("current_time")
            ct_init = int(ct_init_raw) if ct_init_raw not in (None, "null", "") else 0
        except (requests.RequestException, TypeError, ValueError):
            ct_init = 0
        print(f"  Pause 後 current_time={ct_init} (expected ≈ {MS_JAN07})")

        # CT_INIT が range_start より 3 日以上先にあれば StepBackward で巻き戻す
        if ct_init > MS_JAN07 + 3 * STEP_D1:
            print("  CT_INIT が range_start から 3 日超 — StepBackward で巻き戻す...")
            for _ in range(10):
                api_post("/api/replay/step-backward")
                time.sleep(0.5)
                try:
                    ct_back_raw = get_status().get("current_time")
                    ct_back = int(ct_back_raw) if ct_back_raw not in (None, "null", "") else 0
                except (requests.RequestException, TypeError, ValueError):
                    ct_back = 0
                print(f"  StepBackward: current_time={ct_back}")
                if abs(ct_back - MS_JAN07) <= 12 * 3600 * 1000:
                    break
            try:
                ct_init_raw = get_status().get("current_time")
                ct_init = int(ct_init_raw) if ct_init_raw not in (None, "null", "") else 0
            except (requests.RequestException, TypeError, ValueError):
                pass
            print(f"  調整後 CT_INIT={ct_init}")

        # ── TC-A: StepForward × 3 で 2025-01-10 (金) 付近まで前進 ──────────────
        print("\n── TC-A: StepForward × 3 で 2025-01-10 (金) 付近まで前進")
        for _ in range(3):
            api_post("/api/replay/step-forward")
            time.sleep(0.3)

        try:
            ct_a_raw = get_status().get("current_time")
            ct_a = int(ct_a_raw) if ct_a_raw not in (None, "null", "") else 0
        except (requests.RequestException, TypeError, ValueError):
            ct_a = 0
        print(f"  3 回 StepForward 後 current_time={ct_a}")

        if _is_near_ms(ct_a, MS_JAN10):
            pass_(f"TC-A: StepForward × 3 後 current_time ≈ 2025-01-10 ({ct_a})")
        else:
            fail("TC-A", f"current_time={ct_a} は 2025-01-10 ({MS_JAN10}) から 2 日以上離れている")

        # ── TC-B: 金曜から StepForward 1 回 → current_time が前進 ───────────────
        print("\n── TC-B: StepForward 1 回 → current_time 前進")
        try:
            ct_before_b_raw = get_status().get("current_time")
            ct_before_b = int(ct_before_b_raw) if ct_before_b_raw not in (None, "null", "") else 0
        except (requests.RequestException, TypeError, ValueError):
            ct_before_b = ct_a

        api_post("/api/replay/step-forward")
        time.sleep(0.3)

        try:
            ct_b_raw = get_status().get("current_time")
            ct_b = int(ct_b_raw) if ct_b_raw not in (None, "null", "") else 0
        except (requests.RequestException, TypeError, ValueError):
            ct_b = 0
        print(f"  StepForward 前={ct_before_b} → 後={ct_b}")

        if ct_b > ct_before_b:
            pass_(f"TC-B: StepForward で current_time が前進 ({ct_before_b} → {ct_b})")
        else:
            fail("TC-B", f"StepForward で current_time が変化しない")

        # ── TC-C: 土曜 current_time から StepBackward → 2025-01-10 (金) に戻る ──
        print("\n── TC-C: 土曜 current_time から StepBackward → 2025-01-10 (金) にスキップ")
        ct_before_c = ct_b  # 土曜（TC-B 後）

        api_post("/api/replay/step-backward")
        time.sleep(0.5)

        try:
            ct_c_raw = get_status().get("current_time")
            ct_c = int(ct_c_raw) if ct_c_raw not in (None, "null", "") else 0
        except (requests.RequestException, TypeError, ValueError):
            ct_c = 0
        print(f"  StepBackward 前={ct_before_c} → 後={ct_c}")

        if ct_c < ct_before_c:
            pass_(f"TC-C1: StepBackward で current_time が後退 ({ct_before_c} → {ct_c})")
        else:
            fail("TC-C1", "StepBackward で current_time が変化しない")

        if _is_near_ms(ct_c, MS_JAN10):
            pass_(f"TC-C2: StepBackward が土曜をスキップし 2025-01-10 (金) 付近に着地 ({ct_c})")
        else:
            fail("TC-C2", f"current_time={ct_c} は 2025-01-10 ({MS_JAN10}) から 2 日以上離れている（休場日スキップ不成立の疑い）")

        if ct_c > 0 and _is_trading_day(ct_c):
            pass_("TC-C3: StepBackward 後 current_time は取引日（土日でない）")
        else:
            fail("TC-C3", f"current_time={ct_c} が土曜または日曜 — 休場日スキップ失敗")

        # ── TC-D: 金曜 current_time から StepBackward → 2025-01-09 (木) に戻る ──
        print("\n── TC-D: 金曜 current_time から StepBackward → 2025-01-09 (木)")
        ct_before_d = ct_c  # 金曜

        api_post("/api/replay/step-backward")
        time.sleep(0.5)

        try:
            ct_d_raw = get_status().get("current_time")
            ct_d = int(ct_d_raw) if ct_d_raw not in (None, "null", "") else 0
        except (requests.RequestException, TypeError, ValueError):
            ct_d = 0
        print(f"  StepBackward 前={ct_before_d} → 後={ct_d}")

        if ct_d < ct_before_d:
            pass_("TC-D1: 通常 StepBackward で current_time が後退")
        else:
            fail("TC-D1", "StepBackward で current_time が変化しない")

        if _is_near_ms(ct_d, MS_JAN09):
            pass_(f"TC-D2: 金曜 → StepBackward → 2025-01-09 (木) 付近 ({ct_d})")
        else:
            fail("TC-D2", f"current_time={ct_d} は 2025-01-09 ({MS_JAN09}) から 2 日以上離れている")

        # ── TC-E: StepBackward 連続 5 回 → 毎回取引日に着地すること ───────────
        print("\n── TC-E: StepBackward 連続 5 回 → 毎回取引日に着地")
        # TC-A の位置（2025-01-10 付近）に戻してから実施
        api_post("/api/replay/step-forward")
        time.sleep(0.3)

        all_trading = True
        try:
            prev_ct_raw = get_status().get("current_time")
            prev_ct = int(prev_ct_raw) if prev_ct_raw not in (None, "null", "") else 0
        except (requests.RequestException, TypeError, ValueError):
            prev_ct = 0

        for i in range(5):
            api_post("/api/replay/step-backward")
            time.sleep(0.4)

            try:
                ct_step_raw = get_status().get("current_time")
                ct_step = int(ct_step_raw) if ct_step_raw not in (None, "null", "") else 0
            except (requests.RequestException, TypeError, ValueError):
                ct_step = 0

            # 後退していることを確認
            if ct_step >= prev_ct:
                print(f"  TC-E[{i + 1}]: 後退なし ({prev_ct} → {ct_step}) — start_time に到達した可能性あり")
                break

            # 取引日に着地しているか確認
            is_td = ct_step > 0 and _is_trading_day(ct_step)
            day_dt = datetime.fromtimestamp(ct_step / 1000, tz=timezone.utc) if ct_step > 0 else None
            day_name = ["月", "火", "水", "木", "金", "土", "日"][day_dt.weekday()] if day_dt else "?"
            print(f"  TC-E[{i + 1}]: current_time={ct_step} ({day_name}) trading_day={is_td}")

            if not is_td:
                all_trading = False
            prev_ct = ct_step

        if all_trading:
            pass_("TC-E: StepBackward 5 回全て取引日に着地（土日スキップ確認）")
        else:
            fail("TC-E", "StepBackward が土曜または日曜に止まった（休場日スキップ失敗）")

    finally:
        env.close()


def test_s29_tachibana_holiday_skip() -> None:
    """pytest エントリポイント。"""
    _h._PASS = _h._FAIL = _h._PEND = 0
    backup_state()
    try:
        run_s29()
    finally:
        restore_state()
    print_summary()
    assert _h._FAIL == 0, f"{_h._FAIL} TC(s) failed — see output above"


def main() -> None:
    backup_state()
    try:
        run_s29()
    finally:
        restore_state()
        print_summary()
        if _h._FAIL > 0:
            sys.exit(1)


if __name__ == "__main__":
    main()
