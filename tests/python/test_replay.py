"""test_replay.py — Replay HTTP API の実アプリ疎通テスト

前提: アプリが http://127.0.0.1:9876 で起動済みであること。
未起動の場合はすべてスキップされる（conftest.py で制御）。
"""
from __future__ import annotations

import httpx

BASE_URL = "http://127.0.0.1:9876"

TICKER = "BinanceLinear:BTCUSDT"
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


# ── status ────────────────────────────────────────────────────────────────────

def test_status_returns_dict():
    result = _get("/api/replay/status")
    assert isinstance(result, dict)


def test_status_has_status_field():
    result = _get("/api/replay/status")
    assert "status" in result


# ── play / pause / resume ─────────────────────────────────────────────────────

def test_play_returns_dict():
    result = _post("/api/replay/play", {"start": START, "end": END})
    assert isinstance(result, dict)


def test_pause_returns_dict():
    _post("/api/replay/play", {"start": START, "end": END})
    result = _post("/api/replay/pause", {})
    assert isinstance(result, dict)


def test_resume_after_pause():
    _post("/api/replay/play", {"start": START, "end": END})
    _post("/api/replay/pause", {})
    result = _post("/api/replay/resume", {})
    assert isinstance(result, dict)


# ── toggle ────────────────────────────────────────────────────────────────────

def test_toggle_returns_dict():
    result = _post("/api/replay/toggle", {})
    assert isinstance(result, dict)
    _post("/api/replay/toggle", {})  # 元に戻す


# ── step ─────────────────────────────────────────────────────────────────────

def test_step_forward_returns_dict():
    _post("/api/replay/play", {"start": START, "end": END})
    _post("/api/replay/pause", {})
    result = _post("/api/replay/step-forward", {})
    assert isinstance(result, dict)


def test_step_backward_returns_dict():
    _post("/api/replay/play", {"start": START, "end": END})
    _post("/api/replay/pause", {})
    _post("/api/replay/step-forward", {})
    result = _post("/api/replay/step-backward", {})
    assert isinstance(result, dict)


# ── cycle_speed ───────────────────────────────────────────────────────────────

def test_cycle_speed_returns_dict():
    result = _post("/api/replay/speed", {})
    assert isinstance(result, dict)


# ── virtual exchange ──────────────────────────────────────────────────────────

def test_state_returns_dict():
    result = _get("/api/replay/state")
    assert isinstance(result, dict)


def test_portfolio_returns_dict():
    result = _get("/api/replay/portfolio")
    assert isinstance(result, dict)


def test_orders_returns_dict():
    result = _get("/api/replay/orders")
    assert isinstance(result, dict)


def test_order_buy_returns_dict():
    _post("/api/replay/play", {"start": START, "end": END})
    _post("/api/replay/pause", {})
    result = _post(
        "/api/replay/order",
        {"ticker": TICKER, "side": "buy", "qty": 0.01, "order_type": "market"},
    )
    assert isinstance(result, dict)


# ── エラー: アプリ未起動 ──────────────────────────────────────────────────────

def test_not_running_error_on_wrong_port():
    try:
        httpx.get("http://127.0.0.1:19999/api/replay/status", timeout=2.0)
        assert False, "httpx.ConnectError が送出されるべき"
    except httpx.ConnectError:
        pass
