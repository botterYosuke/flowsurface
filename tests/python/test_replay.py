"""Replay HTTP API smoke tests for the current contract."""
from __future__ import annotations

import time

import httpx

BASE_URL = "http://127.0.0.1:9876"
TICKER = "BinanceLinear:BTCUSDT"
START = "2024-01-15 09:00"
END = "2024-01-15 15:30"


def _get(path: str, **params) -> dict:
    r = httpx.get(f"{BASE_URL}{path}", params=params, timeout=5.0)
    r.raise_for_status()
    return r.json()


def _post(path: str, body: dict | None = None) -> dict:
    r = httpx.post(f"{BASE_URL}{path}", json=body, timeout=5.0)
    r.raise_for_status()
    return r.json()


def _wait_until_active(timeout: float = 10.0) -> None:
    deadline = time.monotonic() + timeout
    last_status = None
    while time.monotonic() < deadline:
        r = httpx.get(f"{BASE_URL}/api/replay/state", timeout=5.0)
        last_status = r.status_code
        if r.status_code == 200:
            return
        time.sleep(0.2)
    raise AssertionError(f"/api/replay/state did not become active; last_status={last_status}")


def test_status_returns_dict():
    result = _get("/api/replay/status")
    assert isinstance(result, dict)


def test_toggle_with_range_returns_loading_payload():
    result = _post("/api/replay/toggle", {"start": START, "end": END})
    assert result["ok"] is True
    assert result["status"] == "loading"
    assert result["start"] == START
    assert result["end"] == END


def test_status_has_status_field_after_session_start():
    _post("/api/replay/toggle", {"start": START, "end": END})
    result = _get("/api/replay/status")
    assert result["status"] in {"Loading", "Active"}


def test_toggle_without_body_flips_mode():
    _post("/api/app/set-mode", {"mode": "live"})
    result = _post("/api/replay/toggle", {})
    assert result["mode"] == "Replay"


def test_set_mode_round_trip_returns_updated_mode():
    replay = _post("/api/app/set-mode", {"mode": "replay"})
    assert replay["mode"] == "Replay"
    live = _post("/api/app/set-mode", {"mode": "live"})
    assert live["mode"] == "Live"


def test_state_returns_dict_when_session_active():
    _post("/api/replay/toggle", {"start": START, "end": END})
    _wait_until_active()
    result = _get("/api/replay/state")
    assert isinstance(result, dict)


def test_portfolio_returns_dict():
    result = _get("/api/replay/portfolio")
    assert isinstance(result, dict)


def test_orders_returns_dict():
    result = _get("/api/replay/orders")
    assert isinstance(result, dict)


def test_order_buy_returns_dict_after_session_start():
    _post("/api/replay/toggle", {"start": START, "end": END})
    _wait_until_active()
    result = _post(
        "/api/replay/order",
        {"ticker": TICKER, "side": "buy", "qty": 0.01, "order_type": "market"},
    )
    assert isinstance(result, dict)


def test_not_running_error_on_wrong_port():
    try:
        httpx.get("http://127.0.0.1:19999/api/replay/status", timeout=2.0)
        assert False, "expected connect error"
    except (httpx.ConnectError, httpx.ConnectTimeout):
        pass
