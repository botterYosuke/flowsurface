#!/usr/bin/env python3
"""s52_narrative_outcome_link.py — Phase 4a サブフェーズ F: FillEvent → outcome 自動更新

検証シナリオ:
- REPLAY モードを開始
- 仮想注文を POST /api/replay/order で発注 → order_id を取得
- その order_id を linked_order_id としてナラティブを POST
- StepForward で約定を進める
- GET /api/agent/narrative/:id の outcome が自動で埋まっていることを確認

使い方:
    IS_HEADLESS=true python tests/e2e/s52_narrative_outcome_link.py
    pytest tests/e2e/s52_narrative_outcome_link.py -v
"""
from __future__ import annotations

import os
import sys
import time
from datetime import datetime, timezone, timedelta
from pathlib import Path

import requests

_REPO_ROOT = Path(__file__).parent.parent.parent
try:
    from flowsurface import FlowsurfaceEnv
except ImportError:
    sys.path.insert(0, str(_REPO_ROOT / "python"))
    from env import FlowsurfaceEnv  # type: ignore[no-redef]

API_BASE = "http://127.0.0.1:9876"
TICKER = os.environ.get("E2E_TICKER", "BinanceLinear:BTCUSDT")
IS_HEADLESS = os.environ.get("IS_HEADLESS", "").lower() == "true"

_PASS = 0
_FAIL = 0


def pass_(label: str) -> None:
    global _PASS
    print(f"  PASS: {label}")
    _PASS += 1


def fail(label: str, detail: str) -> None:
    global _FAIL
    print(f"  FAIL: {label} - {detail}")
    _FAIL += 1


def print_summary() -> None:
    print()
    print("=============================")
    print(f"  PASS: {_PASS}  FAIL: {_FAIL}")
    print("=============================")


def utc_offset(hours: float) -> str:
    dt = datetime.now(timezone.utc) + timedelta(hours=hours)
    return dt.strftime("%Y-%m-%d %H:%M")


def api_post(path: str, body: dict | None = None) -> dict:
    r = requests.post(f"{API_BASE}{path}", json=body or {}, timeout=10)
    r.raise_for_status()
    return r.json()


def wait_status(want: str, timeout: int = 60) -> bool:
    deadline = time.monotonic() + timeout
    while time.monotonic() < deadline:
        try:
            r = requests.get(f"{API_BASE}/api/replay/status", timeout=5)
            if r.status_code == 200 and r.json().get("status") == want:
                return True
        except requests.RequestException:
            pass
        time.sleep(0.5)
    return False


def run_s52() -> None:
    mode_label = "headless" if IS_HEADLESS else "GUI"
    print(f"=== S52: FillEvent → outcome 連携 ({mode_label}) ===")

    # 起動直後は Live の場合があるので明示的に Replay へ
    try:
        requests.post(f"{API_BASE}/api/app/set-mode", json={"mode": "replay"}, timeout=5)
    except requests.RequestException:
        pass

    # 過去 2h 分をリプレイ
    start = utc_offset(-3)
    end = utc_offset(-1)
    try:
        api_post("/api/replay/toggle", {"start": start, "end": end})
    except requests.HTTPError as e:
        fail("TC-S52-00", f"play failed: {e}")
        return

    if not wait_status("Playing", 120):
        # Playing に到達しなくても Paused なら step-forward で進められる
        if not wait_status("Paused", 10):
            fail("TC-S52-00", "replay never became active")
            return

    # 一時停止
    wait_status("Paused", 10)

    # TC-S52-01: 仮想注文を発注
    try:
        order_resp = api_post(
            "/api/replay/order",
            {
                "ticker": "BTCUSDT",
                "side": "buy",
                "qty": 0.001,
                "order_type": "market",
            },
        )
    except requests.HTTPError as e:
        fail("TC-S52-01", f"order failed: {e}")
        return

    order_id = order_resp.get("order_id")
    if order_id:
        pass_(f"TC-S52-01: 仮想注文成功 (order_id={order_id[:8]}...)")
    else:
        fail("TC-S52-01", f"no order_id in {order_resp}")
        return

    # TC-S52-02: ナラティブを order_id と紐付けて POST
    payload = {
        "agent_id": "s52_agent",
        "ticker": "BTCUSDT",
        "timeframe": "1h",
        "observation_snapshot": {"test": True},
        "reasoning": "test outcome auto-update",
        "action": {"side": "buy", "qty": 0.001, "price": 0.0},
        "confidence": 0.5,
        "linked_order_id": order_id,
    }
    r = requests.post(f"{API_BASE}/api/agent/narrative", json=payload, timeout=10)
    if r.status_code != 201:
        fail("TC-S52-02", f"POST failed: {r.status_code} {r.text}")
        return
    narrative_id = r.json()["id"]
    pass_(f"TC-S52-02: ナラティブ作成 linked_order_id={order_id[:8]}... → id={narrative_id[:8]}...")

    # TC-S52-03: 約定を進めるため step-forward を複数回
    for _ in range(5):
        try:
            api_post("/api/replay/step-forward")
        except requests.HTTPError:
            break
        time.sleep(0.3)

    # TC-S52-04: outcome が更新されているかポーリング
    deadline = time.monotonic() + 30
    outcome = None
    while time.monotonic() < deadline:
        r = requests.get(f"{API_BASE}/api/agent/narrative/{narrative_id}", timeout=5)
        if r.status_code == 200:
            outcome = r.json().get("outcome")
            if outcome is not None:
                break
        time.sleep(0.5)

    if outcome is not None:
        fill_price = outcome.get("fill_price")
        fill_time_ms = outcome.get("fill_time_ms")
        if fill_price and fill_time_ms:
            pass_(
                f"TC-S52-04: outcome 自動更新 (fill_price={fill_price}, fill_time_ms={fill_time_ms})"
            )
        else:
            fail("TC-S52-04", f"outcome incomplete: {outcome}")
    else:
        fail("TC-S52-04", "outcome was not populated within 30s")


def test_s52_narrative_outcome_link() -> None:
    global _PASS, _FAIL
    _PASS = _FAIL = 0
    run_s52()
    print_summary()
    assert _FAIL == 0, f"{_FAIL} TC(s) failed"


def main() -> None:
    env = FlowsurfaceEnv(ticker=TICKER, timeframe="M1", headless=IS_HEADLESS)
    try:
        env._start_process()
        run_s52()
    finally:
        env.close()
        print_summary()
        if _FAIL > 0:
            sys.exit(1)


if __name__ == "__main__":
    main()
