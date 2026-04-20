"""conftest.py — pytest 共通設定

E2E テスト（s*/x* スイート）はポート 9876 で flowsurface が起動している必要があります。
サーバーが応答しない場合は自動的にスキップされます。
"""
from __future__ import annotations

import pytest
import requests


def _server_available() -> bool:
    try:
        r = requests.get("http://127.0.0.1:9876/api/replay/status", timeout=2)
        return r.status_code == 200
    except requests.exceptions.RequestException:
        return False


def pytest_collection_modifyitems(config, items):
    skip_e2e = pytest.mark.skip(reason="flowsurface server not running on port 9876")
    if not _server_available():
        for item in items:
            if item.fspath.basename.startswith(("s", "x")) and item.fspath.basename.endswith(".py"):
                item.add_marker(skip_e2e)
