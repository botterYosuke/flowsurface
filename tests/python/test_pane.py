"""test_pane.py — Pane クラスの実アプリ疎通テスト"""
from __future__ import annotations

import flowsurface as fs

START = "2024-01-15 09:00:00"
END = "2024-01-15 15:30:00"


def _first_pane_id() -> str:
    body = fs.pane.list
    panes = body.get("panes", [])  # type: ignore[union-attr]
    assert panes, "ペインが存在しない"
    return panes[0]["id"]


# ── list ──────────────────────────────────────────────────────────────────────

def test_pane_list_returns_dict():
    result = fs.pane.list
    assert isinstance(result, dict)


def test_pane_list_has_panes_key():
    result = fs.pane.list
    assert "panes" in result


# ── chart_snapshot ────────────────────────────────────────────────────────────

def test_chart_snapshot_returns_dict():
    pane_id = _first_pane_id()
    result = fs.pane.chart_snapshot(pane_id)
    assert isinstance(result, dict)


# ── set_ticker ────────────────────────────────────────────────────────────────

def test_set_ticker_returns_dict():
    pane_id = _first_pane_id()
    result = fs.pane.set_ticker(pane_id, "BinanceLinear:BTCUSDT")
    assert isinstance(result, dict)


# ── set_timeframe ─────────────────────────────────────────────────────────────

def test_set_timeframe_returns_dict():
    pane_id = _first_pane_id()
    result = fs.pane.set_timeframe(pane_id, "1m")
    assert isinstance(result, dict)


# ── split / close ─────────────────────────────────────────────────────────────

def test_split_and_close():
    pane_id = _first_pane_id()
    split_result = fs.pane.split(pane_id, axis="Vertical")
    assert isinstance(split_result, dict)

    after = fs.pane.list
    panes = after.get("panes", [])  # type: ignore[union-attr]
    new_ids = [p["id"] for p in panes if p["id"] != pane_id]
    assert new_ids, "分割後に新しいペインが存在しない"

    close_result = fs.pane.close(new_ids[0])
    assert isinstance(close_result, dict)
