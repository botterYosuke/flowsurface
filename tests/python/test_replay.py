"""test_replay.py — Replay クラスの実アプリ疎通テスト

前提: flowsurface が http://127.0.0.1:9876 で起動済みであること。
未起動の場合はすべてスキップされる（conftest.py で制御）。
"""
from __future__ import annotations

import flowsurface as fs
from flowsurface._client import FlowsurfaceNotRunningError

TICKER = "BinanceLinear:BTCUSDT"
START = "2024-01-15 09:00:00"
END = "2024-01-15 15:30:00"


# ── status ────────────────────────────────────────────────────────────────────

def test_status_returns_dict():
    result = fs.replay.status
    assert isinstance(result, dict)


def test_status_has_status_field():
    result = fs.replay.status
    assert "status" in result


# ── play / pause / resume ─────────────────────────────────────────────────────

def test_play_returns_dict():
    result = fs.replay.play(START, END)
    assert isinstance(result, dict)


def test_pause_returns_dict():
    fs.replay.play(START, END)
    result = fs.replay.pause()
    assert isinstance(result, dict)


def test_resume_after_pause():
    fs.replay.play(START, END)
    fs.replay.pause()
    result = fs.replay.resume()
    assert isinstance(result, dict)


# ── toggle ────────────────────────────────────────────────────────────────────

def test_toggle_returns_dict():
    result = fs.replay.toggle()
    assert isinstance(result, dict)
    fs.replay.toggle()  # 元に戻す


# ── step ─────────────────────────────────────────────────────────────────────

def test_step_forward_returns_dict():
    fs.replay.play(START, END)
    fs.replay.pause()
    result = fs.replay.step_forward()
    assert isinstance(result, dict)


def test_step_backward_returns_dict():
    fs.replay.play(START, END)
    fs.replay.pause()
    fs.replay.step_forward()
    result = fs.replay.step_backward()
    assert isinstance(result, dict)


# ── cycle_speed ───────────────────────────────────────────────────────────────

def test_cycle_speed_returns_dict():
    result = fs.replay.cycle_speed()
    assert isinstance(result, dict)


# ── virtual exchange ──────────────────────────────────────────────────────────

def test_state_returns_dict():
    result = fs.replay.state
    assert isinstance(result, dict)


def test_portfolio_returns_dict():
    result = fs.replay.portfolio
    assert isinstance(result, dict)


def test_orders_returns_dict():
    result = fs.replay.orders
    assert isinstance(result, dict)


def test_order_buy_returns_dict():
    fs.replay.play(START, END)
    fs.replay.pause()
    result = fs.replay.order(ticker=TICKER, side="buy", qty=0.01)
    assert isinstance(result, dict)


# ── エラー: アプリ未起動 ──────────────────────────────────────────────────────

def test_not_running_error_on_wrong_port():
    import flowsurface._client as _c
    client = _c.Client(base_url="http://127.0.0.1:19999")
    from flowsurface.replay import Replay
    r = Replay(client)
    try:
        _ = r.status
        assert False, "FlowsurfaceNotRunningError が送出されるべき"
    except FlowsurfaceNotRunningError:
        pass
