"""test_pane.py — Pane HTTP API の実アプリ疎通テスト"""
from __future__ import annotations

import httpx

BASE_URL = "http://127.0.0.1:9876"

START = "2024-01-15 09:00:00"
END = "2024-01-15 15:30:00"


def _get(path: str, **params) -> dict:
    r = httpx.get(f"{BASE_URL}{path}", params=params, timeout=5.0)
    r.raise_for_status()
    return r.json()


def _post(path: str, body: dict | None = None) -> dict:
    r = httpx.post(f"{BASE_URL}{path}", json=body, timeout=5.0)
    r.raise_for_status()
    return r.json()


def _first_pane_id() -> str:
    body = _get("/api/pane/list")
    panes = body.get("panes", [])
    assert panes, "ペインが存在しない"
    return panes[0]["id"]


# ── list ──────────────────────────────────────────────────────────────────────

def test_pane_list_returns_dict():
    result = _get("/api/pane/list")
    assert isinstance(result, dict)


def test_pane_list_has_panes_key():
    result = _get("/api/pane/list")
    assert "panes" in result


# ── chart_snapshot ────────────────────────────────────────────────────────────

def test_chart_snapshot_returns_dict():
    pane_id = _first_pane_id()
    result = _get("/api/pane/chart-snapshot", pane_id=pane_id)
    assert isinstance(result, dict)


# ── set_ticker ────────────────────────────────────────────────────────────────

def test_set_ticker_returns_dict():
    pane_id = _first_pane_id()
    result = _post(
        "/api/pane/set-ticker",
        {"pane_id": pane_id, "ticker": "BinanceLinear:BTCUSDT"},
    )
    assert isinstance(result, dict)


# ── set_timeframe ─────────────────────────────────────────────────────────────

def test_set_timeframe_returns_dict():
    pane_id = _first_pane_id()
    result = _post(
        "/api/pane/set-timeframe",
        {"pane_id": pane_id, "timeframe": "1m"},
    )
    assert isinstance(result, dict)


# ── split / close ─────────────────────────────────────────────────────────────

def test_split_and_close():
    pane_id = _first_pane_id()
    split_result = _post(
        "/api/pane/split",
        {"pane_id": pane_id, "axis": "Vertical"},
    )
    assert isinstance(split_result, dict)

    after = _get("/api/pane/list")
    panes = after.get("panes", [])
    new_ids = [p["id"] for p in panes if p["id"] != pane_id]
    assert new_ids, "分割後に新しいペインが存在しない"

    close_result = _post("/api/pane/close", {"pane_id": new_ids[0]})
    assert isinstance(close_result, dict)
