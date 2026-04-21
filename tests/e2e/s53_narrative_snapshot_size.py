#!/usr/bin/env python3
"""s53_narrative_snapshot_size.py — Phase 4a サブフェーズ F: スナップショットサイズ上限と破損検知

検証シナリオ:
- 11MB ペイロードを POST → 413 Payload Too Large
- 正常なナラティブ POST → スナップショット取得で本体 JSON が返る
- ディスク上のファイルを破壊 → /snapshot で 410 Gone

使い方:
    IS_HEADLESS=true python tests/e2e/s53_narrative_snapshot_size.py
    pytest tests/e2e/s53_narrative_snapshot_size.py -v
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

_DATA_ROOT = Path(os.environ.get("FLOWSURFACE_DATA_PATH") or (
    Path(os.environ.get("APPDATA") or "") / "flowsurface"
))

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


def run_s53() -> None:
    mode_label = "headless" if IS_HEADLESS else "GUI"
    print(f"=== S53: スナップショットサイズ / 破損検知 ({mode_label}) ===")

    # TC-S53-01: 11MB ペイロード → 413
    big = "x" * (11 * 1024 * 1024)
    over_payload = {
        "agent_id": "s53_agent",
        "ticker": "BTCUSDT",
        "timeframe": "1h",
        "observation_snapshot": {"blob": big},
        "reasoning": "oversized",
        "action": {"side": "buy", "qty": 0.1, "price": 1.0},
        "confidence": 0.5,
    }
    r = requests.post(
        f"{API_BASE}/api/agent/narrative", json=over_payload, timeout=30
    )
    if r.status_code == 413:
        pass_("TC-S53-01: 11MB payload → 413 Payload Too Large")
    else:
        fail("TC-S53-01", f"status={r.status_code} (expected 413)")

    # TC-S53-02: 正常な POST → スナップショット取得 → 本体 JSON が返る
    key = f"s53_snap_{int(time.time() * 1000)}"
    normal_payload = {
        "agent_id": "s53_agent",
        "ticker": "BTCUSDT",
        "timeframe": "1h",
        "observation_snapshot": {"marker": "valid", "value": 42},
        "reasoning": "valid",
        "action": {"side": "buy", "qty": 0.1, "price": 100.0},
        "confidence": 0.5,
        "idempotency_key": key,
    }
    r = requests.post(
        f"{API_BASE}/api/agent/narrative", json=normal_payload, timeout=10
    )
    if r.status_code != 201:
        fail("TC-S53-02", f"normal POST failed: {r.status_code} {r.text}")
        return
    narrative_id = r.json()["id"]

    r = requests.get(
        f"{API_BASE}/api/agent/narrative/{narrative_id}/snapshot", timeout=5
    )
    if r.status_code == 200 and r.json().get("marker") == "valid":
        pass_("TC-S53-02: スナップショット本体が取得できる (gzip 解凍 + sha256 検証通過)")
    else:
        fail("TC-S53-02", f"status={r.status_code} body={r.text[:200]}")

    # TC-S53-03: ファイルを破壊 → 410 Gone
    meta = requests.get(
        f"{API_BASE}/api/agent/narrative/{narrative_id}", timeout=5
    ).json()
    snap_path = meta.get("snapshot_ref", {}).get("path")
    if not snap_path:
        fail("TC-S53-03", f"no snapshot_ref.path in meta: {meta}")
        return

    abs_path = _DATA_ROOT / snap_path
    if not abs_path.exists():
        fail("TC-S53-03", f"snapshot file not found at {abs_path}")
        return

    # バイトを 1 つ XOR して sha256 不一致を起こす
    data = abs_path.read_bytes()
    corrupted = bytearray(data)
    corrupted[0] ^= 0xFF
    abs_path.write_bytes(bytes(corrupted))

    r = requests.get(
        f"{API_BASE}/api/agent/narrative/{narrative_id}/snapshot", timeout=5
    )
    if r.status_code == 410:
        pass_("TC-S53-03: sha256 不一致 → 410 Gone")
    else:
        fail("TC-S53-03", f"status={r.status_code} (expected 410)")


def test_s53_narrative_snapshot_size() -> None:
    global _PASS, _FAIL
    _PASS = _FAIL = 0
    run_s53()
    print_summary()
    assert _FAIL == 0, f"{_FAIL} TC(s) failed"


def main() -> None:
    env = FlowsurfaceEnv(ticker=TICKER, timeframe="M1", headless=IS_HEADLESS)
    try:
        env._start_process()
        run_s53()
    finally:
        env.close()
        print_summary()
        if _FAIL > 0:
            sys.exit(1)


if __name__ == "__main__":
    main()
