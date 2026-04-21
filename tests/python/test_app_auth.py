"""test_app_auth.py — App / Auth / Notification / Sidebar クラスの実アプリ疎通テスト"""
from __future__ import annotations

import flowsurface as fs


# ── App ───────────────────────────────────────────────────────────────────────

def test_app_save_returns_dict():
    result = fs.app.save()
    assert isinstance(result, dict)


def test_app_set_mode_replay():
    result = fs.app.set_mode("replay")
    assert isinstance(result, dict)


def test_app_set_mode_live():
    result = fs.app.set_mode("live")
    assert isinstance(result, dict)
    fs.app.set_mode("replay")  # テスト後に replay へ戻す


# ── Auth ──────────────────────────────────────────────────────────────────────

def test_auth_tachibana_status_returns_dict():
    result = fs.auth.tachibana_status
    assert isinstance(result, dict)


def test_auth_tachibana_status_has_session_field():
    result = fs.auth.tachibana_status
    assert "session" in result


# ── Notification ──────────────────────────────────────────────────────────────

def test_notification_list_returns_dict():
    result = fs.notification.list
    assert isinstance(result, dict)


def test_notification_list_has_notifications_key():
    result = fs.notification.list
    assert "notifications" in result


# ── Sidebar ───────────────────────────────────────────────────────────────────

def test_sidebar_open_order_pane_returns_dict():
    result = fs.sidebar.open_order_pane("OrderEntry")
    assert isinstance(result, dict)


def test_sidebar_select_ticker_returns_dict():
    body = fs.pane.list
    panes = body.get("panes", [])  # type: ignore[union-attr]
    if not panes:
        return
    pane_id = panes[0]["id"]
    result = fs.sidebar.select_ticker(pane_id, "BinanceLinear:BTCUSDT")
    assert isinstance(result, dict)


# ── configure ─────────────────────────────────────────────────────────────────

def test_configure_changes_base_url_and_restores():
    """configure() でクライアントが差し替わることを確認する。"""
    from flowsurface._client import FlowsurfaceNotRunningError

    fs.configure(base_url="http://127.0.0.1:19999")
    try:
        _ = fs.replay.status
        assert False, "FlowsurfaceNotRunningError が送出されるべき"
    except FlowsurfaceNotRunningError:
        pass
    finally:
        fs.configure()  # デフォルト URL に戻す

    result = fs.replay.status
    assert isinstance(result, dict)
