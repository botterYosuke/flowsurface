"""conftest.py — tests/python/ 共通設定

既存ガード:
- アプリ未起動 (port 9876 不通): 全テスト skip

追加ガード (本ファイルで導入):
- `@pytest.mark.tachibana_demo` 付きテスト: 立花証券セッションが
  「デモ口座でログイン中」でなければ skip。
  本番口座での誤発注を防ぐための安全弁。
"""
from __future__ import annotations

import httpx
import pytest


def _server_available() -> bool:
    try:
        r = httpx.get("http://127.0.0.1:9876/api/replay/status", timeout=2)
        return r.status_code == 200
    except (httpx.ConnectError, httpx.ConnectTimeout):
        return False


def _tachibana_demo_session_active() -> bool:
    """立花証券セッションがデモ口座でログイン中かを判定する。

    `GET /api/auth/tachibana/status` のレスポンスが
    `{"session": "present", "environment": "demo"}` の場合のみ True。
    未ログイン・本番ログイン・通信失敗はすべて False（安全側）。
    """
    try:
        r = httpx.get(
            "http://127.0.0.1:9876/api/auth/tachibana/status",
            timeout=2,
        )
    except (httpx.ConnectError, httpx.ConnectTimeout):
        return False
    if r.status_code != 200:
        return False
    try:
        data = r.json()
    except ValueError:
        return False
    return data.get("session") == "present" and data.get("environment") == "demo"


def pytest_configure(config):
    config.addinivalue_line(
        "markers",
        "tachibana_demo: 立花証券デモ口座でログイン中の場合のみ実行する"
        "（本番口座での誤発注防止）",
    )


def pytest_collection_modifyitems(items):
    server_up = _server_available()
    skip_no_server = pytest.mark.skip(reason="flowsurface app not running on port 9876")
    # デモ判定はサーバ起動時のみ意味があるので、サーバが落ちている場合は False のままで OK
    demo_active = server_up and _tachibana_demo_session_active()
    skip_no_demo = pytest.mark.skip(
        reason="tachibana session not in demo mode (本番口座への誤発注防止)"
    )

    for item in items:
        if item.fspath.dirpath().basename != "python":
            continue
        if not server_up:
            item.add_marker(skip_no_server)
            continue
        if "tachibana_demo" in item.keywords and not demo_active:
            item.add_marker(skip_no_demo)
