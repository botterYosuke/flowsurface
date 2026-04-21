#!/usr/bin/env python3
"""s51_narrative_crud.py — Phase 4a サブフェーズ F: ナラティブ CRUD ライフサイクル

検証シナリオ:
- POST /api/agent/narrative → 201 Created + id
- GET /api/agent/narrative/:id でメタ取得
- GET /api/agent/narratives で一覧フィルタ
- PATCH /api/agent/narrative/:id で public true/false の両方向
- idempotency_key 再送で同じ id が返る

使い方:
    IS_HEADLESS=true python tests/e2e/s51_narrative_crud.py
    pytest tests/e2e/s51_narrative_crud.py -v
"""
from __future__ import annotations

import os
import sys
import time
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


def _sample_payload(*, agent_id: str = "s51_agent", idempotency_key: str | None = None) -> dict:
    payload: dict = {
        "agent_id": agent_id,
        "ticker": "BTCUSDT",
        "timeframe": "1h",
        "observation_snapshot": {"rsi": 28.3, "volume_ratio": 1.42},
        "reasoning": "RSI divergence on 4h, volume confirmed",
        "action": {"side": "buy", "qty": 0.1, "price": 92500.0},
        "confidence": 0.76,
    }
    if idempotency_key is not None:
        payload["idempotency_key"] = idempotency_key
    return payload


def run_s51() -> None:
    mode_label = "headless" if IS_HEADLESS else "GUI"
    print(f"=== S51: ナラティブ CRUD ({mode_label}) ===")

    # TC-S51-01: POST 成功 → 201 + id
    r = requests.post(
        f"{API_BASE}/api/agent/narrative", json=_sample_payload(), timeout=10
    )
    if r.status_code == 201:
        pass_("TC-S51-01: POST /api/agent/narrative → 201")
        body = r.json()
        narrative_id = body.get("id")
        if narrative_id:
            pass_(f"TC-S51-01b: id returned ({narrative_id[:8]}...)")
        else:
            fail("TC-S51-01b", f"no id in {body}")
            return
    else:
        fail("TC-S51-01", f"status={r.status_code} body={r.text}")
        return

    # TC-S51-02: GET by id
    r = requests.get(f"{API_BASE}/api/agent/narrative/{narrative_id}", timeout=5)
    if r.status_code == 200:
        meta = r.json()
        if meta["agent_id"] == "s51_agent" and meta["ticker"] == "BTCUSDT":
            pass_("TC-S51-02: GET /api/agent/narrative/:id")
        else:
            fail("TC-S51-02", f"unexpected body: {meta}")
    else:
        fail("TC-S51-02", f"status={r.status_code}")

    # TC-S51-03: GET 404 for unknown id
    r = requests.get(
        f"{API_BASE}/api/agent/narrative/00000000-0000-0000-0000-000000000000",
        timeout=5,
    )
    if r.status_code == 404:
        pass_("TC-S51-03: unknown id → 404")
    else:
        fail("TC-S51-03", f"status={r.status_code}")

    # TC-S51-04: List filtered by agent_id
    r = requests.get(
        f"{API_BASE}/api/agent/narratives",
        params={"agent_id": "s51_agent"},
        timeout=5,
    )
    if r.status_code == 200:
        narratives = r.json().get("narratives", [])
        if narratives and all(n["agent_id"] == "s51_agent" for n in narratives):
            pass_(f"TC-S51-04: list filtered by agent_id ({len(narratives)} items)")
        else:
            fail("TC-S51-04", f"filter broken: {[n.get('agent_id') for n in narratives]}")
    else:
        fail("TC-S51-04", f"status={r.status_code}")

    # TC-S51-05: PATCH public=true → public=false
    r = requests.patch(
        f"{API_BASE}/api/agent/narrative/{narrative_id}",
        json={"public": True},
        timeout=5,
    )
    if r.status_code == 200 and r.json().get("public") is True:
        pass_("TC-S51-05: PATCH public=true")
    else:
        fail("TC-S51-05", f"status={r.status_code} body={r.text}")

    r = requests.patch(
        f"{API_BASE}/api/agent/narrative/{narrative_id}",
        json={"public": False},
        timeout=5,
    )
    if r.status_code == 200 and r.json().get("public") is False:
        pass_("TC-S51-05b: PATCH public=false (取消)")
    else:
        fail("TC-S51-05b", f"status={r.status_code} body={r.text}")

    # TC-S51-06: idempotency_key で再送すると同じ id が返る
    key = f"s51_idem_{int(time.time() * 1000)}"
    r1 = requests.post(
        f"{API_BASE}/api/agent/narrative",
        json=_sample_payload(idempotency_key=key),
        timeout=10,
    )
    r2 = requests.post(
        f"{API_BASE}/api/agent/narrative",
        json=_sample_payload(idempotency_key=key),
        timeout=10,
    )
    if r1.status_code == 201 and r2.status_code == 201:
        id1 = r1.json().get("id")
        id2 = r2.json().get("id")
        replay = r2.json().get("idempotent_replay")
        if id1 == id2 and replay is True:
            pass_(f"TC-S51-06: idempotency_key で重複 POST が同じ id ({id1[:8]}...)")
        else:
            fail("TC-S51-06", f"id1={id1} id2={id2} replay={replay}")
    else:
        fail("TC-S51-06", f"statuses=({r1.status_code}, {r2.status_code})")

    # TC-S51-07: storage stats
    r = requests.get(f"{API_BASE}/api/agent/narratives/storage", timeout=5)
    if r.status_code == 200:
        stats = r.json()
        if "total_count" in stats and "total_bytes" in stats:
            pass_(f"TC-S51-07: storage stats (count={stats['total_count']})")
        else:
            fail("TC-S51-07", f"missing fields: {stats}")
    else:
        fail("TC-S51-07", f"status={r.status_code}")


def test_s51_narrative_crud() -> None:
    global _PASS, _FAIL
    _PASS = _FAIL = 0
    run_s51()
    print_summary()
    assert _FAIL == 0, f"{_FAIL} TC(s) failed"


def main() -> None:
    env = FlowsurfaceEnv(ticker=TICKER, timeframe="M1", headless=IS_HEADLESS)
    try:
        env._start_process()
        run_s51()
    finally:
        env.close()
        print_summary()
        if _FAIL > 0:
            sys.exit(1)


if __name__ == "__main__":
    main()
