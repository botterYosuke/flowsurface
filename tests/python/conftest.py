"""conftest.py — tests/python/ 共通設定"""
from __future__ import annotations

import httpx
import pytest


def _server_available() -> bool:
    try:
        r = httpx.get("http://127.0.0.1:9876/api/replay/status", timeout=2)
        return r.status_code == 200
    except (httpx.ConnectError, httpx.ConnectTimeout):
        return False


def pytest_collection_modifyitems(items):
    skip = pytest.mark.skip(reason="flowsurface app not running on port 9876")
    if not _server_available():
        for item in items:
            if item.fspath.dirpath().basename == "python":
                item.add_marker(skip)
