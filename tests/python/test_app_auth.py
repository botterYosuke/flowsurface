"""test_app_auth.py — App / Auth / Notification / Sidebar HTTP API の実アプリ疎通テスト"""
from __future__ import annotations

import httpx

BASE_URL = "http://127.0.0.1:9876"


def _get(path: str, **params) -> dict:
    r = httpx.get(f"{BASE_URL}{path}", params=params, timeout=5.0)
    r.raise_for_status()
    return r.json()


def _post(path: str, body: dict | None = None) -> dict:
    r = httpx.post(f"{BASE_URL}{path}", json=body, timeout=5.0)
    r.raise_for_status()
    return r.json()


# ── App ───────────────────────────────────────────────────────────────────────

def test_app_save_returns_dict():
    result = _post("/api/app/save", {})
    assert isinstance(result, dict)


def test_app_set_mode_replay():
    result = _post("/api/app/set-mode", {"mode": "replay"})
    assert isinstance(result, dict)


def test_app_set_mode_live():
    result = _post("/api/app/set-mode", {"mode": "live"})
    assert isinstance(result, dict)
    _post("/api/app/set-mode", {"mode": "replay"})  # テスト後に replay へ戻す


# ── Auth ──────────────────────────────────────────────────────────────────────

def test_auth_tachibana_status_returns_dict():
    result = _get("/api/auth/tachibana/status")
    assert isinstance(result, dict)


def test_auth_tachibana_status_has_session_field():
    result = _get("/api/auth/tachibana/status")
    assert "session" in result


# ── Notification ──────────────────────────────────────────────────────────────

def test_notification_list_returns_dict():
    result = _get("/api/notification/list")
    assert isinstance(result, dict)


def test_notification_list_has_notifications_key():
    result = _get("/api/notification/list")
    assert "notifications" in result


# ── Sidebar ───────────────────────────────────────────────────────────────────

def test_sidebar_open_order_pane_returns_dict():
    result = _post("/api/sidebar/open-order-pane", {"kind": "OrderEntry"})
    assert isinstance(result, dict)


def test_sidebar_select_ticker_returns_dict():
    body = _get("/api/pane/list")
    panes = body.get("panes", [])
    if not panes:
        return
    pane_id = panes[0]["id"]
    result = _post(
        "/api/sidebar/select-ticker",
        {"pane_id": pane_id, "ticker": "BinanceLinear:BTCUSDT", "kind": None},
    )
    assert isinstance(result, dict)
