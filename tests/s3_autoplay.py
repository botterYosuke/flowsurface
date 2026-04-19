#!/usr/bin/env python3
"""s3_autoplay.py — スイート S3: 起動時 Auto-play

検証シナリオ:
  TC-S3-01: saved-state に range 設定済み → 手動操作なしで Playing 到達（30s 以内）
  TC-S3-02: current_time が range 内
  TC-S3-03: mode=Replay
  TC-S3-04: Pause → StepForward +60000ms
  TC-S3-05a〜c: range 未設定 → auto-play しない・status=null・error toast なし

仕様根拠:
  docs/replay_header.md §5.1 — auto-play（pending_auto_play フラグ）

tests/archive/s3_autoplay.sh の Python 版。

使い方:
    uv run tests/s3_autoplay.py
    IS_HEADLESS=true uv run tests/s3_autoplay.py
    pytest tests/s3_autoplay.py -v
"""

from __future__ import annotations

import json
import sys
import time
from pathlib import Path

sys.path.insert(0, str(Path(__file__).parent))
from helpers import *


def run_s3() -> None:
    start = utc_offset(-3)
    end = utc_offset(-1)
    start_ms = utc_to_ms(start)
    end_ms = utc_to_ms(end)
    mode_label = "headless" if IS_HEADLESS else "GUI"

    print(f"=== S3: Auto-play (Fixture 直接起動) (ticker={TICKER} {mode_label}) ===")

    setup_single_pane(TICKER, "M1", start, end)
    headless_play()

    # --- TC-S3-01: 手動 toggle / play なしで Playing になる（最大 30s） ---
    if wait_playing(30):
        pass_("TC-S3-01: auto-play → Playing（sleep 15 不要）")
    else:
        fail("TC-S3-01", "30s 以内に Playing にならなかった（streams 解決失敗？）")

    status = get_status()

    # --- TC-S3-02: current_time が range 内 ---
    ct_val = status.get("current_time")
    ct = int(ct_val) if ct_val is not None else 0
    if start_ms <= ct <= end_ms:
        pass_("TC-S3-02: current_time in range")
    else:
        fail("TC-S3-02", f"CT={ct} range=[{start_ms},{end_ms}]")

    # --- TC-S3-03: mode=Replay ---
    mode = status.get("mode")
    if mode == "Replay":
        pass_("TC-S3-03: mode=Replay")
    else:
        fail("TC-S3-03", f"mode={mode}")

    # --- TC-S3-04: Pause → StepForward → diff=60000ms ---
    api_post("/api/replay/pause")
    time.sleep(1)
    pre_val = get_status().get("current_time")
    pre = int(pre_val) if pre_val is not None else 0
    api_post("/api/replay/step-forward")
    time.sleep(1)
    post_val = get_status().get("current_time")
    post_sf = int(post_val) if post_val is not None else 0
    diff = post_sf - pre
    if diff == 60000:
        pass_("TC-S3-04: StepForward +60000ms")
    else:
        fail("TC-S3-04", f"diff={diff} (expected 60000)")


def run_s3_no_autoplay(env: object) -> None:
    """TC-S3-05: range 未設定 → auto-play しない（GUI モードのみ）"""
    print()
    print("── TC-S3-05: range_start が空文字のとき auto-play しない")

    # range_start/end が空の fixture を書き込んでアプリを再起動
    DATA_DIR.mkdir(parents=True, exist_ok=True)
    fixture = {
        "layout_manager": {
            "layouts": [
                {
                    "name": "S3-NoAutoPlay",
                    "dashboard": {
                        "pane": {
                            "KlineChart": {
                                "layout": {"splits": [0.78], "autoscale": "FitToVisible"},
                                "kind": "Candles",
                                "stream_type": [{"Kline": {"ticker": "BinanceLinear:BTCUSDT", "timeframe": "M1"}}],
                                "settings": {
                                    "tick_multiply": None,
                                    "visual_config": None,
                                    "selected_basis": {"Time": "M1"},
                                },
                                "indicators": ["Volume"],
                                "link_group": "A",
                            }
                        },
                        "popout": [],
                    },
                }
            ],
            "active_layout": "S3-NoAutoPlay",
        },
        "timezone": "UTC",
        "trade_fetch_enabled": False,
        "size_in_quote_ccy": "Base",
        "replay": {"mode": "replay", "range_start": "", "range_end": ""},
    }
    STATE_FILE.write_text(json.dumps(fixture, indent=2))

    env._start_process()  # type: ignore[attr-defined]

    # API が応答するまで待つ（最大 10s）
    alive = False
    st_check = None
    for _ in range(20):
        try:
            st_check = get_status().get("status")
            alive = True
            if st_check is None:
                break
        except Exception:
            pass
        time.sleep(0.5)

    if not alive:
        fail("TC-S3-05a", "API not ready after start_app")
        fail("TC-S3-05b", "API not ready after start_app")
        return

    st_check = get_status().get("status")
    mode_check = get_status().get("mode")

    if st_check is None:
        pass_("TC-S3-05a: range 未設定 → status=null")
    else:
        fail("TC-S3-05a", f"status={st_check} (expected null)")

    if mode_check == "Replay":
        pass_("TC-S3-05b: range 未設定でも mode は fixture 通り")
    else:
        fail("TC-S3-05b", f"mode={mode_check}")

    # --- TC-S3-05c: トーストに auto-play 起動エラーが無いこと ---
    err_count = count_error_notifications()
    if err_count == 0:
        pass_("TC-S3-05c: error/warning toast なし")
    else:
        fail("TC-S3-05c", f"error/warning toast が {err_count} 件発火")


def test_s3_autoplay() -> None:
    """pytest エントリポイント。TC-S3-05（range未設定→auto-play しない）は run_s3_no_autoplay() に属するため pytest では未テスト。"""
    import helpers as _h
    _h._PASS = _h._FAIL = _h._PEND = 0
    run_s3()
    print_summary()
    assert _h._FAIL == 0, f"{_h._FAIL} TC(s) failed — see output above"


def main() -> None:
    backup_state()

    start = utc_offset(-3)
    end = utc_offset(-1)
    setup_single_pane(TICKER, "M1", start, end)

    env = FlowsurfaceEnv(ticker=TICKER, timeframe="M1", headless=IS_HEADLESS)
    try:
        env._start_process()
        run_s3()

        if not IS_HEADLESS:
            env.close()  # close first instance before restarting for no-autoplay run
            run_s3_no_autoplay(env)

    finally:
        env.close()
        restore_state()
        print_summary()
        import helpers as _h
        if _h._FAIL > 0:
            sys.exit(1)


if __name__ == "__main__":
    main()
