#!/usr/bin/env python3
"""s8_error_boundary.py — Suite S8: エラー・境界値ケース

検証シナリオ:
  TC-S8-01: 存在しないパス → HTTP 404
  TC-S8-02: 不正 JSON → HTTP 400
  TC-S8-03: 必須フィールド (end) 欠損 → HTTP 400
  TC-S8-04: GET on POST エンドポイント → HTTP 404
  TC-S8-05a〜c: start > end → HTTP 200（GUI）/ HTTP 400（headless）・Playing 遷移なし・エラートースト発火
  TC-S8-06a〜c: 未来日時 → HTTP 200・Playing/Paused 到達（Loading ハングなし）
  TC-S8-07: 不正フォーマット（複数パターン）→ HTTP 400
  TC-S8-08: pane/split に不正 UUID → HTTP 400
  TC-S8-09: pane/split に不正 axis → HTTP 400
  TC-S8-10: pane/set-ticker pane_id edge case
  TC-S8-11: set-timeframe validation

仕様根拠:
  docs/replay_header.md §10 — エラーハンドリング・入力バリデーション

フィクスチャ: BinanceLinear:BTCUSDT M1, Live モード起動（HTTP API エラー系テスト）
"""

from __future__ import annotations

import json
import sys
import time
from pathlib import Path

import requests

sys.path.insert(0, str(Path(__file__).parent))
from helpers import (
    API_BASE,
    FlowsurfaceEnv,
    IS_HEADLESS,
    TICKER,
    api_get,
    api_get_code,
    api_post,
    api_post_code,
    backup_state,
    fail,
    get_pane_id,
    has_notification,
    pass_,
    pend,
    print_summary,
    restore_state,
    wait_for_pane_streams_ready,
    wait_status,
    write_live_fixture,
)


def _post_raw(path: str, body: str | bytes) -> tuple[int, dict]:
    """生データで POST し、(status_code, parsed_body) を返す。JSON パース失敗時は {}。"""
    try:
        r = requests.post(
            f"{API_BASE}{path}",
            data=body if isinstance(body, (str, bytes)) else body,
            headers={"Content-Type": "application/json"},
            timeout=5,
        )
        try:
            parsed = r.json()
        except Exception:
            parsed = {}
        return r.status_code, parsed
    except requests.RequestException:
        return 0, {}


def run_s8() -> None:
    mode_label = "headless" if IS_HEADLESS else "GUI"
    print(f"=== S8: エラー・境界値ケース ({mode_label}) ===")

    # Live モード用フィクスチャを書き込む
    write_live_fixture(ticker=TICKER, timeframe="M1", name="S8")

    env = FlowsurfaceEnv(ticker=TICKER, timeframe="M1", headless=IS_HEADLESS)
    try:
        env._start_process()

        # Live ストリームが Ready になるまで待つ（Binance メタデータ取得に数秒かかる）
        # headless はストリーム接続なし → スキップ
        if not IS_HEADLESS:
            pane0 = get_pane_id(0)
            if pane0:
                wait_for_pane_streams_ready(pane0, 30)
            else:
                time.sleep(5)

        # TC-S8-01: 存在しないパス → 404
        code = api_get_code("/nonexistent")
        if code == 404:
            pass_("TC-S8-01: 存在しないパス → 404")
        else:
            fail("TC-S8-01", f"code={code}")

        # TC-S8-02: 不正 JSON → 400 + error フィールド
        code2, body2 = _post_raw("/api/replay/play", b"not json")
        if code2 == 400:
            pass_("TC-S8-02a: 不正 JSON → 400")
        else:
            fail("TC-S8-02a", f"code={code2}")
        if body2.get("error"):
            pass_("TC-S8-02b: 不正 JSON → error フィールドあり")
        else:
            fail("TC-S8-02b", f"body={body2}")

        # TC-S8-03: 必須フィールド (end) 欠損 → 400 + error フィールド
        code3, body3 = _post_raw(
            "/api/replay/play",
            b'{"start":"2026-04-10 09:00"}',
        )
        if code3 == 400:
            pass_("TC-S8-03a: end 欠損 → 400")
        else:
            fail("TC-S8-03a", f"code={code3}")
        if body3.get("error"):
            pass_("TC-S8-03b: end 欠損 → error フィールドあり")
        else:
            fail("TC-S8-03b", f"body={body3}")

        # TC-S8-04: GET on POST endpoint → 404
        code4 = api_get_code("/api/replay/toggle")
        if code4 == 404:
            pass_("TC-S8-04: GET on POST endpoint → 404")
        else:
            fail("TC-S8-04", f"code={code4}")

        # Replay モードへ
        try:
            api_post("/api/replay/toggle")
        except Exception:
            pass

        # TC-S8-05: start > end
        code5 = api_post_code(
            "/api/replay/play",
            {"start": "2026-04-13 10:00", "end": "2026-04-13 09:00"},
        )
        if IS_HEADLESS:
            if code5 == 400:
                pass_("TC-S8-05a: start>end → HTTP 400 (headless rejection)")
            else:
                fail("TC-S8-05a", f"code={code5} (expected 400 in headless)")
        else:
            if code5 == 200:
                pass_("TC-S8-05a: start>end → HTTP 200")
            else:
                fail("TC-S8-05a", f"code={code5}")

        st_after = api_get("/api/replay/status").get("status")
        if st_after in (None, "Paused"):
            pass_("TC-S8-05b: Playing に遷移しない")
        else:
            fail("TC-S8-05b", f"status={st_after}")

        if IS_HEADLESS:
            pend("TC-S8-05c", "headless は Toast 通知なし（400 エラーで拒否）")
        else:
            if has_notification("Start time"):
                pass_("TC-S8-05c: エラートーストが発火")
            else:
                fail("TC-S8-05c", "start>end の toast が発火していない")

        # TC-S8-06: 未来日時 → 受理 → Playing/Paused（Loading ハングなし）
        future_start = "2030-01-01 00:00"
        future_end = "2030-01-01 06:00"
        import datetime as _dt  # noqa: PLC0415
        future_start_ms = int(
            _dt.datetime(2030, 1, 1, 0, 0, tzinfo=_dt.timezone.utc).timestamp() * 1000
        )
        future_end_ms = int(
            _dt.datetime(2030, 1, 1, 6, 0, tzinfo=_dt.timezone.utc).timestamp() * 1000
        )

        code6 = api_post_code(
            "/api/replay/play",
            {"start": future_start, "end": future_end},
        )
        if code6 == 200:
            pass_("TC-S8-06a: 未来日時 → HTTP 200")
        else:
            fail("TC-S8-06a", f"code={code6}")

        time.sleep(30)
        st6 = api_get("/api/replay/status").get("status")
        if st6 in ("Playing", "Paused"):
            pass_(f"TC-S8-06b: 未来日時でも Playing/Paused に遷移 (Loading ハングなし, status={st6})")
        else:
            fail("TC-S8-06b", f"status={st6} (expected Playing or Paused — Loading ハングの疑い)")

        ct6 = api_get("/api/replay/status").get("current_time")
        if ct6 is not None:
            ct6_int = int(ct6)
            if future_start_ms <= ct6_int <= future_end_ms:
                pass_(f"TC-S8-06c: current_time={ct6_int} は future range 内（clock 正常起動）")
            else:
                fail(
                    "TC-S8-06c",
                    f"current_time={ct6_int} は range [{future_start_ms}, {future_end_ms}] 外",
                )
        else:
            fail("TC-S8-06c", "current_time が null（clock 未起動）")

        # TC-S8-07: 不正フォーマット → 400 + error フィールド
        bad_dates = ["2026/04/10 09:00", "2026-04-10", "not-a-date", ""]
        for bad_date in bad_dates:
            payload = json.dumps({"start": bad_date, "end": "2026-04-10 15:00"}).encode()
            c7, b7 = _post_raw("/api/replay/play", payload)
            if c7 == 400:
                pass_(f"TC-S8-07a: 不正フォーマット '{bad_date}' → 400")
            else:
                fail("TC-S8-07a", f"'{bad_date}' → {c7} (expected 400)")
            if b7.get("error"):
                pass_(f"TC-S8-07b: '{bad_date}' → error フィールドあり")
            else:
                fail("TC-S8-07b", f"'{bad_date}' body={b7}")

        # TC-S8-07c: 不正日付（越境・時刻超過・月超過）→ 400
        invalid_dates = ["2026-02-30 10:00", "2026-04-10 25:00", "2026-13-01 09:00"]
        for inv_date in invalid_dates:
            payload = json.dumps({"start": inv_date, "end": "2026-04-10 15:00"}).encode()
            c7c, b7c = _post_raw("/api/replay/play", payload)
            if c7c == 400:
                pass_(f"TC-S8-07c: 不正日付 '{inv_date}' → 400")
            else:
                fail("TC-S8-07c", f"'{inv_date}' → {c7c} (expected 400)")
            if b7c.get("error"):
                pass_(f"TC-S8-07d: '{inv_date}' → error フィールドあり")
            else:
                fail("TC-S8-07d", f"'{inv_date}' body={b7c}")

        # うるう年 2/29 は有効 → 200
        code_leap = api_post_code(
            "/api/replay/play",
            {"start": "2024-02-29 10:00", "end": "2024-02-29 12:00"},
        )
        if code_leap == 200:
            pass_("TC-S8-07e: うるう年 2024-02-29 → 200（有効日付）")
        else:
            fail("TC-S8-07e", f"code={code_leap} (expected 200)")

        # TC-S8-08: pane/split に不正 UUID → 400
        if IS_HEADLESS:
            pend("TC-S8-08", "headless は pane/split API 非対応（501）")
        else:
            c8, b8 = _post_raw(
                "/api/pane/split",
                b'{"pane_id":"not-a-uuid","axis":"Vertical"}',
            )
            if c8 == 400:
                pass_("TC-S8-08a: 不正 UUID → 400")
            else:
                fail("TC-S8-08a", f"code={c8}")
            if b8.get("error"):
                pass_("TC-S8-08b: 不正 UUID → error フィールドあり")
            else:
                fail("TC-S8-08b", f"body={b8}")

        # TC-S8-09: pane/split に不正 axis → 400
        if IS_HEADLESS:
            pend("TC-S8-09", "headless は pane/split API 非対応（501）")
        else:
            pane_id = get_pane_id(0)
            if pane_id:
                payload9 = json.dumps({"pane_id": pane_id, "axis": "Diagonal"}).encode()
                c9, b9 = _post_raw("/api/pane/split", payload9)
                if c9 == 400:
                    pass_("TC-S8-09a: 不正 axis → 400")
                else:
                    fail("TC-S8-09a", f"code={c9}")
                if b9.get("error"):
                    pass_("TC-S8-09b: 不正 axis → error フィールドあり")
                else:
                    fail("TC-S8-09b", f"body={b9}")

        # TC-S8-10: pane/set-ticker pane_id edge case
        if IS_HEADLESS:
            pend("TC-S8-10", "headless は pane/set-ticker API 非対応（501）")
        else:
            c10a, _ = _post_raw(
                "/api/pane/set-ticker",
                b'{"pane_id":"","ticker":"BinanceLinear:BTCUSDT"}',
            )
            if c10a == 400:
                pass_("TC-S8-10a: pane_id 空文字 → 400")
            else:
                fail("TC-S8-10a", f"code={c10a}")

            c10b, _ = _post_raw(
                "/api/pane/set-ticker",
                b'{"pane_id":"not-a-uuid","ticker":"BinanceLinear:BTCUSDT"}',
            )
            if c10b in (400, 404):
                pass_("TC-S8-10b: 不正 UUID → 404 or 400")
            else:
                fail("TC-S8-10b", f"code={c10b} (expected 404 or 400)")

        # TC-S8-11: set-timeframe validation
        if IS_HEADLESS:
            pend("TC-S8-11", "headless は pane/set-timeframe API 非対応（501）")
        else:
            pane_id_11 = get_pane_id(0)
            if pane_id_11:
                for bad_tf in ("M999", ""):
                    payload11 = json.dumps(
                        {"pane_id": pane_id_11, "timeframe": bad_tf}
                    ).encode()
                    c11, b11 = _post_raw("/api/pane/set-timeframe", payload11)
                    if c11 == 400 and b11.get("error"):
                        pass_(
                            f"TC-S8-11a: timeframe='{bad_tf}' → HTTP 400 + error フィールド"
                        )
                    else:
                        fail(
                            "TC-S8-11a",
                            f"timeframe='{bad_tf}' code={c11} has_err={bool(b11.get('error'))}"
                            " (expected 400+error)",
                        )
                    if b11.get("error"):
                        pass_(f"TC-S8-11b: timeframe='{bad_tf}' → error フィールドあり")
                    else:
                        fail("TC-S8-11b", f"timeframe='{bad_tf}' body={b11}")

    finally:
        env.close()


def test_s8_error_boundary() -> None:
    """pytest エントリポイント。"""
    import helpers as _h
    _h._PASS = _h._FAIL = _h._PEND = 0
    backup_state()
    try:
        run_s8()
    finally:
        restore_state()
    print_summary()
    assert _h._FAIL == 0, f"{_h._FAIL} TC(s) failed — see output above"


def main() -> None:
    backup_state()
    try:
        run_s8()
    finally:
        restore_state()
        print_summary()
        import helpers as _h
        if _h._FAIL > 0:
            sys.exit(1)


if __name__ == "__main__":
    main()
