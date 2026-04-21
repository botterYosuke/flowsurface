"""test_narrative.py — Phase 4a ナラティブ API の実アプリ疎通テスト。

アプリ未起動時は conftest.py で全 skip。
"""
from __future__ import annotations

import json
import time
from typing import Any

import httpx
import pytest

BASE_URL = "http://127.0.0.1:9876"


def _post(path: str, body: dict[str, Any] | None = None, *, expect: int = 201) -> dict:
    r = httpx.post(f"{BASE_URL}{path}", json=body, timeout=5.0)
    assert r.status_code == expect, f"unexpected {r.status_code}: {r.text}"
    return r.json()


def _get(path: str, **params: Any) -> dict:
    r = httpx.get(f"{BASE_URL}{path}", params=params, timeout=5.0)
    r.raise_for_status()
    return r.json()


def _patch(path: str, body: dict[str, Any]) -> dict:
    r = httpx.patch(f"{BASE_URL}{path}", json=body, timeout=5.0)
    r.raise_for_status()
    return r.json()


def _sample_payload(*, agent_id: str = "test_agent", idempotency_key: str | None = None) -> dict:
    payload: dict[str, Any] = {
        "agent_id": agent_id,
        "ticker": "BTCUSDT",
        "timeframe": "1h",
        "observation_snapshot": {"rsi": 28.3, "ohlcv": [[1, 2, 3, 4, 5]]},
        "reasoning": "RSI divergence",
        "action": {"side": "buy", "qty": 0.1, "price": 92500.0},
        "confidence": 0.76,
    }
    if idempotency_key is not None:
        payload["idempotency_key"] = idempotency_key
    return payload


# ── 基本 CRUD ──────────────────────────────────────────────────────────────────

def test_create_returns_201_and_id():
    resp = _post("/api/agent/narrative", _sample_payload())
    assert "id" in resp
    assert resp["idempotent_replay"] is False
    assert resp["snapshot_bytes"] > 0


def test_get_narrative_roundtrip():
    key = f"test_get_{int(time.time() * 1000)}"
    created = _post("/api/agent/narrative", _sample_payload(idempotency_key=key))
    narrative = _get(f"/api/agent/narrative/{created['id']}")
    assert narrative["agent_id"] == "test_agent"
    assert narrative["action"]["side"] == "buy"
    assert narrative["public"] is False


def test_list_narratives_includes_recent_create():
    key = f"test_list_{int(time.time() * 1000)}"
    _post("/api/agent/narrative", _sample_payload(agent_id="list_agent", idempotency_key=key))
    resp = _get("/api/agent/narratives", agent_id="list_agent")
    assert "narratives" in resp
    assert len(resp["narratives"]) >= 1
    assert all(n["agent_id"] == "list_agent" for n in resp["narratives"])


def test_patch_toggles_public_flag():
    key = f"test_patch_{int(time.time() * 1000)}"
    created = _post("/api/agent/narrative", _sample_payload(idempotency_key=key))
    updated = _patch(f"/api/agent/narrative/{created['id']}", {"public": True})
    assert updated["public"] is True
    untoggled = _patch(f"/api/agent/narrative/{created['id']}", {"public": False})
    assert untoggled["public"] is False


def test_idempotency_key_prevents_duplicates():
    key = f"test_idem_{int(time.time() * 1000)}"
    first = _post("/api/agent/narrative", _sample_payload(idempotency_key=key))
    second = _post("/api/agent/narrative", _sample_payload(idempotency_key=key))
    assert first["id"] == second["id"]
    assert second["idempotent_replay"] is True


def test_get_snapshot_returns_observation_body():
    key = f"test_snap_{int(time.time() * 1000)}"
    created = _post("/api/agent/narrative", _sample_payload(idempotency_key=key))
    snap = _get(f"/api/agent/narrative/{created['id']}/snapshot")
    assert "rsi" in snap
    assert snap["rsi"] == 28.3


def test_storage_stats_returns_counts():
    stats = _get("/api/agent/narratives/storage")
    assert "total_count" in stats
    assert "total_bytes" in stats
    assert "warn_count" in stats
    assert stats["total_count"] >= 0


# ── バリデーション ─────────────────────────────────────────────────────────────

def test_create_rejects_empty_agent_id():
    payload = _sample_payload()
    payload["agent_id"] = ""
    r = httpx.post(f"{BASE_URL}/api/agent/narrative", json=payload, timeout=5.0)
    assert r.status_code == 400


def test_create_rejects_out_of_range_confidence():
    payload = _sample_payload()
    payload["confidence"] = 2.0
    r = httpx.post(f"{BASE_URL}/api/agent/narrative", json=payload, timeout=5.0)
    assert r.status_code == 400


def test_get_unknown_id_returns_404():
    r = httpx.get(
        f"{BASE_URL}/api/agent/narrative/00000000-0000-0000-0000-000000000000",
        timeout=5.0,
    )
    assert r.status_code == 404


# ── SDK helper ────────────────────────────────────────────────────────────────

def test_sdk_dataclass_roundtrip():
    """`Narrative.from_dict` accepts the server JSON and reconstructs fields."""
    from flowsurface import Narrative, NarrativeAction

    key = f"test_sdk_{int(time.time() * 1000)}"
    created = _post("/api/agent/narrative", _sample_payload(idempotency_key=key))
    meta = _get(f"/api/agent/narrative/{created['id']}")
    n = Narrative.from_dict(meta)
    assert isinstance(n.action, NarrativeAction)
    assert n.agent_id == "test_agent"
    assert n.confidence == pytest.approx(0.76)
