"""test_replay.py — Replay HTTP API の実アプリ疎通テスト

前提: アプリが http://127.0.0.1:9876 で起動済みであること。
未起動の場合はすべてスキップされる（conftest.py で制御）。
"""
from __future__ import annotations

import time

import httpx

BASE_URL = "http://127.0.0.1:9876"

TICKER = "BinanceLinear:BTCUSDT"
# API の日時フォーマットは "%Y-%m-%d %H:%M"（秒なし）。
# 参照: src/replay_api.rs validate_datetime_str / 既存 E2E スクリプト全件。
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


# ── status ────────────────────────────────────────────────────────────────────

def test_status_returns_dict():
    result = _get("/api/replay/status")
    assert isinstance(result, dict)


def test_status_has_status_field():
    # ReplayStatus.status は Option<String> + skip_serializing_if="Option::is_none" で、
    # Idle セッション／Live モードでは JSON から omit される（src/replay/mod.rs:93）。
    # Active／Loading セッション中のみ present であることを検証する。
    _post("/api/replay/play", {"start": START, "end": END})
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
    # GET /api/replay/state は ReplaySession::Active のときのみ 200 を返す。
    # Idle は 400、Loading は 503（src/app/api/replay.rs:224-272）。
    # play を呼び、Active 遷移を最大 10 秒ポーリングする。
    _post("/api/replay/play", {"start": START, "end": END})
    deadline = time.monotonic() + 10.0
    last = None
    while time.monotonic() < deadline:
        r = httpx.get(f"{BASE_URL}/api/replay/state", timeout=5.0)
        last = r
        if r.status_code == 200:
            result = r.json()
            assert isinstance(result, dict)
            return
        # Loading 中は 503 が返るため短い間隔でリトライ
        time.sleep(0.2)
    raise AssertionError(
        f"/api/replay/state が 10 秒以内に Active にならなかった: "
        f"status={last.status_code if last else 'N/A'} body={last.text if last else 'N/A'}"
    )


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
    # Windows では未バインドポートへの TCP が即時 RST ではなくタイムアウトまで
    # 待つため、httpx は ConnectTimeout を送出する。httpx 1.x で ConnectTimeout は
    # ConnectError の兄弟（サブクラスではない）ため、両方を許容する。
    try:
        httpx.get("http://127.0.0.1:19999/api/replay/status", timeout=2.0)
        assert False, "httpx.ConnectError または httpx.ConnectTimeout が送出されるべき"
    except (httpx.ConnectError, httpx.ConnectTimeout):
        pass
