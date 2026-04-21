"""test_tachibana_order.py — 立花証券注文系 API の疎通テスト（デモ口座専用）

`@pytest.mark.tachibana_demo` 付きテストは、`conftest.py` のフックが
`GET /api/auth/tachibana/status` をチェックし、`environment == "demo"` 以外
（未ログイン・本番ログイン・通信失敗）なら自動的に skip する。

本番口座での誤発注を防ぐ安全弁。
"""
from __future__ import annotations

import httpx
import pytest

BASE_URL = "http://127.0.0.1:9876"


def _get(path: str, **params) -> dict:
    # 立花証券 API 経由のため、ローカル API 経由よりレイテンシが大きい。
    # 実測 ~5s 前後のため余裕を持たせる。
    r = httpx.get(f"{BASE_URL}{path}", params=params, timeout=15.0)
    r.raise_for_status()
    return r.json()


@pytest.mark.tachibana_demo
def test_order_list_returns_dict():
    result = _get("/api/tachibana/orders")
    assert isinstance(result, dict)
