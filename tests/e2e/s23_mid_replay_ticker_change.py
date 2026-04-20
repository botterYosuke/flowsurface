#!/usr/bin/env python3
"""s23_mid_replay_ticker_change.py — S23: mid-replay 銘柄・timeframe 変更時の自動再生防止

検証シナリオ:
  TC-A: Play → 銘柄変更 → status = Paused
  TC-B: Play → timeframe 変更 → status = Paused
  TC-C: Play → 銘柄変更 → データロード待機 → status = Paused（自動再生されない）
  TC-D: Play → 銘柄変更 → Resume → status = Playing
  TC-E: Pause → 銘柄変更 → status = Paused のまま
  TC-F: Play のみ（通常フロー）→ Loading → Playing（回帰）
  TC-G: Play → 銘柄変更 → 別銘柄に再変更 → Resume → status = Playing

仕様根拠:
  docs/replay_header.md §6.6 — 銘柄変更による初期状態リセット（Playing/Paused 時）

フィクスチャ: BinanceLinear:BTCUSDT M1, auto-play (UTC[-3h, -1h])
"""

from __future__ import annotations

import json
import sys
import time
from pathlib import Path

sys.path.insert(0, str(Path(__file__).parent))
from helpers import (
    API_BASE,
    DATA_DIR,
    IS_HEADLESS,
    STATE_FILE,
    TICKER,
    api_get,
    api_post,
    backup_state,
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
    utc_offset,
    wait_for_pane_streams_ready,
    wait_playing,
    wait_status,
)

_REPO_ROOT = Path(__file__).parent.parent
try:
    from flowsurface import FlowsurfaceEnv
except ImportError:
    sys.path.insert(0, str(_REPO_ROOT / "python"))
    from env import FlowsurfaceEnv  # type: ignore[no-redef]


def _write_s23_fixture(primary: str, start: str, end: str) -> None:
    """S23 専用フィクスチャを saved-state.json に書き込む。"""
    DATA_DIR.mkdir(parents=True, exist_ok=True)
    fixture = {
        "layout_manager": {
            "layouts": [
                {
                    "name": "S23",
                    "dashboard": {
                        "pane": {
                            "KlineChart": {
                                "layout": {"splits": [0.78], "autoscale": "FitToVisible"},
                                "kind": "Candles",
                                "stream_type": [{"Kline": {"ticker": primary, "timeframe": "M1"}}],
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
            "active_layout": "S23",
        },
        "timezone": "UTC",
        "trade_fetch_enabled": False,
        "size_in_quote_ccy": "Base",
        "replay": {"mode": "replay", "range_start": start, "range_end": end},
    }
    STATE_FILE.write_text(json.dumps(fixture, indent=2))


def run_s23() -> None:
    mode_label = "headless" if IS_HEADLESS else "GUI"
    print(f"=== S23: mid-replay 銘柄・timeframe 変更時の自動再生防止 ({mode_label}) ===")

    start = utc_offset(-3)
    end = utc_offset(-1)
    primary = primary_ticker()
    secondary = secondary_ticker()

    if not IS_HEADLESS:
        _write_s23_fixture(primary, start, end)
    else:
        # headless 用に内部変数を設定
        setup_single_pane(primary, "M1", start, end)

    env = FlowsurfaceEnv(ticker=TICKER, timeframe="M1", headless=IS_HEADLESS)
    try:
        env._start_process()
        headless_play(start, end)

        # ──────────────────────────────────────────────────────────────────────
        # TC-F: 通常 Play フロー（回帰）— autoplay 起動後に Loading → Playing に遷移する
        # ──────────────────────────────────────────────────────────────────────
        print()
        print("── TC-F: 通常 Play フロー（回帰）")

        init_resp = api_get("/api/replay/status")
        init_mode = init_resp.get("mode")
        print(f"  F: 起動直後 mode={init_mode}")

        current_status = api_get("/api/replay/status").get("status")
        if init_mode == "Replay" or current_status in ("Loading", "Playing"):
            pass_(
                "TC-F1: autoplay 起動後 mode=Replay (status フィールドは Play 発火後に現れる)"
            )
        else:
            fail("TC-F1", f"mode={init_mode} (expected Replay)")

        if wait_status("Playing", 90):
            pass_("TC-F2: 通常 Play フロー → Playing 到達")
        else:
            fail("TC-F2", f"Playing 未到達（90s timeout）: status={api_get('/api/replay/status').get('status')}")
            return

        pane_id = get_pane_id(0)
        print(f"  PANE_ID={pane_id}")
        if not pane_id:
            fail("precond", "ペイン ID 取得失敗")
            return

        # ──────────────────────────────────────────────────────────────────────
        # TC-A: Playing 中に銘柄変更 → 即座に Paused
        # ──────────────────────────────────────────────────────────────────────
        print()
        print("── TC-A: Playing 中に銘柄変更 → 即座に Paused")
        api_post("/api/pane/set-ticker", {"pane_id": pane_id, "ticker": secondary})
        time.sleep(0.5)

        st_a = api_get("/api/replay/status").get("status")
        if st_a == "Paused":
            pass_("TC-A: 銘柄変更後 status=Paused")
        else:
            fail("TC-A", f"status={st_a} (expected Paused)")

        # ──────────────────────────────────────────────────────────────────────
        # TC-C: データロード完了後も Paused のまま（自動再生されない）
        # ──────────────────────────────────────────────────────────────────────
        print()
        print("── TC-C: データロード後も Paused のまま")
        if wait_for_pane_streams_ready(pane_id, 30):
            print("  C: streams_ready=true 確認")
            st_c = api_get("/api/replay/status").get("status")
            if st_c == "Paused":
                pass_("TC-C: streams_ready 後も status=Paused（自動再生なし）")
            else:
                fail("TC-C", f"status={st_c} (expected Paused — 自動再生が発生した)")
        else:
            print("  C: streams_ready 未到達、3 秒待機して status を確認")
            time.sleep(3)
            st_c = api_get("/api/replay/status").get("status")
            if st_c == "Paused":
                pass_("TC-C: 3 秒後も status=Paused（自動再生なし）")
            else:
                fail("TC-C", f"status={st_c} (expected Paused — 自動再生が発生した)")

        # ──────────────────────────────────────────────────────────────────────
        # TC-D: Paused 状態から Resume → Playing
        # ──────────────────────────────────────────────────────────────────────
        print()
        print("── TC-D: Paused → Resume → Playing")
        api_post("/api/replay/resume")
        if wait_status("Playing", 30):
            pass_("TC-D: Resume → Playing 到達")
        else:
            fail("TC-D", f"status={api_get('/api/replay/status').get('status')} (expected Playing)")

        # ──────────────────────────────────────────────────────────────────────
        # TC-B: Playing 中に timeframe 変更 → Paused
        # ──────────────────────────────────────────────────────────────────────
        print()
        print("── TC-B: Playing 中に timeframe 変更 → Paused")
        api_post("/api/pane/set-timeframe", {"pane_id": pane_id, "timeframe": "M5"})
        time.sleep(0.5)

        st_b = api_get("/api/replay/status").get("status")
        if st_b == "Paused":
            pass_("TC-B: timeframe 変更後 status=Paused")
        else:
            fail("TC-B", f"status={st_b} (expected Paused)")

        # ──────────────────────────────────────────────────────────────────────
        # TC-E: Paused 中に銘柄変更 → Paused のまま
        # ──────────────────────────────────────────────────────────────────────
        print()
        print("── TC-E: Paused 中に銘柄変更 → Paused のまま")
        # 現在 Paused（TC-B の結果）
        api_post("/api/pane/set-ticker", {"pane_id": pane_id, "ticker": primary})
        time.sleep(1)

        st_e = api_get("/api/replay/status").get("status")
        if st_e == "Paused":
            pass_("TC-E: Paused 中に銘柄変更後も status=Paused")
        else:
            fail("TC-E", f"status={st_e} (expected Paused)")

        # ──────────────────────────────────────────────────────────────────────
        # TC-G: 連続銘柄変更（Playing → 2 回変更 → Resume → Playing）
        # ──────────────────────────────────────────────────────────────────────
        print()
        print("── TC-G: 連続銘柄変更後 Resume → Playing")

        # まず Playing に戻す
        api_post("/api/replay/resume")
        if not wait_status("Playing", 30):
            fail(
                "TC-G-pre",
                f"Playing 到達失敗（前提条件） status={api_get('/api/replay/status').get('status')}",
            )
            return

        # Playing 中に 2 回連続で銘柄変更
        api_post("/api/pane/set-ticker", {"pane_id": pane_id, "ticker": secondary})
        time.sleep(0.3)
        api_post("/api/pane/set-ticker", {"pane_id": pane_id, "ticker": primary})
        time.sleep(0.5)

        st_g_after = api_get("/api/replay/status").get("status")
        print(f"  G: 連続変更後 status={st_g_after}")

        # Resume → Playing
        api_post("/api/replay/resume")
        if wait_status("Playing", 30):
            pass_("TC-G: 連続銘柄変更後 Resume → Playing 到達")
        else:
            fail("TC-G", f"status={api_get('/api/replay/status').get('status')} (expected Playing)")

    finally:
        env.close()


def test_s23_mid_replay_ticker_change() -> None:
    """pytest エントリポイント。プロセス起動は外部で行うこと。"""
    import helpers as _h
    _h._PASS = _h._FAIL = _h._PEND = 0
    run_s23()
    print_summary()
    assert _h._FAIL == 0, f"{_h._FAIL} TC(s) failed — see output above"


def main() -> None:
    backup_state()
    try:
        run_s23()
    finally:
        restore_state()
        print_summary()
        import helpers as _h
        if _h._FAIL > 0:
            sys.exit(1)


if __name__ == "__main__":
    main()
