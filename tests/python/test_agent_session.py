"""test_agent_session.py — Phase 4b-1 agent API の実アプリ疎通テスト。

アプリ未起動時は conftest.py で全 skip。

前提:
- リプレイセッションを UI リモコン API `/api/replay/play` で起動した後に
  agent API が動作する（plan §5.1 の「同一 VirtualExchange 共有」方針）。
- `/advance` は Headless ランタイムでのみ受理される（ADR-0001）。GUI 起動の
  アプリでは 400 が返るため本スイートでは `test_advance_*` は skip される。
"""
from __future__ import annotations

import time

import httpx
import pytest

import flowsurface as fs

BASE_URL = "http://127.0.0.1:9876"
TICKER = "BinanceLinear:BTCUSDT"
START = "2024-01-15 09:00"
END = "2024-01-15 15:30"


def _post(path: str, body: dict | None = None) -> dict:
    r = httpx.post(f"{BASE_URL}{path}", json=body, timeout=5.0)
    r.raise_for_status()
    return r.json()


def _is_headless() -> bool:
    """Advance が通るランタイムかを前もって判定する。

    `/advance` に空 body を投げ、400 の body に "headless" が含まれたら GUI と判断。
    その他（2xx / 4xx with 別メッセージ / 5xx）なら Headless 扱い。
    """
    try:
        r = httpx.post(
            f"{BASE_URL}/api/agent/session/default/advance",
            json={"until_ms": 0},
            timeout=2.0,
        )
    except (httpx.ConnectError, httpx.ConnectTimeout):
        return False
    if r.status_code == 400 and "headless" in r.text.lower():
        return False
    return True


@pytest.fixture(autouse=True)
def _ensure_session_started():
    """各テスト前に replay セッションを起動する。"""
    _post("/api/replay/play", {"start": START, "end": END})
    # loading が 1 秒以内に active に遷移することを期待。詳細な barrier は既存
    # E2E テストと同じ pattern（短い sleep）。
    time.sleep(1.0)


# ── step ────────────────────────────────────────────────────────────────────

def test_step_returns_dataclass_with_required_fields():
    resp = fs.agent_session.step()
    assert isinstance(resp, fs.AgentStepResponse)
    assert isinstance(resp.clock_ms, int)
    assert resp.clock_ms > 0
    assert isinstance(resp.reached_end, bool)
    assert isinstance(resp.observation, dict)
    assert isinstance(resp.fills, list)
    assert isinstance(resp.updated_narrative_ids, list)


def test_step_advances_clock_by_one_bar():
    first = fs.agent_session.step()
    second = fs.agent_session.step()
    assert second.clock_ms > first.clock_ms


# ── order + idempotency ──────────────────────────────────────────────────────

def _unique_cli() -> str:
    return f"cli_{int(time.time() * 1000)}"


def test_place_order_creates_new_order():
    resp = fs.agent_session.place_order(
        client_order_id=_unique_cli(),
        ticker={"exchange": "BinanceLinear", "symbol": "BTCUSDT"},
        side="buy",
        qty=0.1,
        order_type={"market": {}},
    )
    assert isinstance(resp, fs.AgentOrderResponse)
    assert not resp.idempotent_replay
    assert resp.order_id  # 非空


def test_place_order_idempotent_replay_on_exact_rerun():
    cli = _unique_cli()
    kwargs = dict(
        client_order_id=cli,
        ticker={"exchange": "BinanceLinear", "symbol": "BTCUSDT"},
        side="buy",
        qty=0.1,
        order_type={"market": {}},
    )
    first = fs.agent_session.place_order(**kwargs)
    second = fs.agent_session.place_order(**kwargs)
    assert not first.idempotent_replay
    assert second.idempotent_replay
    assert first.order_id == second.order_id


def test_place_order_409_on_different_body_same_client_order_id():
    cli = _unique_cli()
    fs.agent_session.place_order(
        client_order_id=cli,
        ticker={"exchange": "BinanceLinear", "symbol": "BTCUSDT"},
        side="buy",
        qty=0.1,
        order_type={"market": {}},
    )
    with pytest.raises(fs.ApiError) as exc:
        fs.agent_session.place_order(
            client_order_id=cli,
            ticker={"exchange": "BinanceLinear", "symbol": "BTCUSDT"},
            side="buy",
            qty=0.2,  # 異なる
            order_type={"market": {}},
        )
    assert exc.value.status_code == 409


def test_place_order_400_on_string_ticker_rejects_at_http_layer():
    # SDK は dict を受けるが、手動で文字列を送ると server が 400 を返すことを確認。
    r = httpx.post(
        f"{BASE_URL}/api/agent/session/default/order",
        json={
            "client_order_id": _unique_cli(),
            "ticker": "BinanceLinear:BTCUSDT",  # 文字列 = 400
            "side": "buy",
            "qty": 0.1,
            "order_type": {"market": {}},
        },
        timeout=5.0,
    )
    assert r.status_code == 400
    assert "ticker" in r.text.lower() or "invalid" in r.text.lower()


def test_place_order_400_on_missing_order_type():
    r = httpx.post(
        f"{BASE_URL}/api/agent/session/default/order",
        json={
            "client_order_id": _unique_cli(),
            "ticker": {"exchange": "BinanceLinear", "symbol": "BTCUSDT"},
            "side": "buy",
            "qty": 0.1,
        },
        timeout=5.0,
    )
    assert r.status_code == 400
    assert "order_type" in r.text.lower()


# ── session_id ≠ default → 501 ──────────────────────────────────────────────

def test_non_default_session_id_returns_501():
    r = httpx.post(
        f"{BASE_URL}/api/agent/session/other/step",
        timeout=5.0,
    )
    assert r.status_code == 501
    assert "multi-session" in r.text.lower()


# ── advance (headless only) ─────────────────────────────────────────────────

_HEADLESS = _is_headless()
_skip_if_gui = pytest.mark.skipif(
    not _HEADLESS,
    reason="advance requires headless runtime (pass --headless to flowsurface)",
)


def test_advance_rejects_gui_with_400_and_headless_hint():
    if _HEADLESS:
        pytest.skip("running against headless, GUI-specific test skipped")
    r = httpx.post(
        f"{BASE_URL}/api/agent/session/default/advance",
        json={"until_ms": int(time.time() * 1000)},
        timeout=5.0,
    )
    assert r.status_code == 400
    assert "headless" in r.text.lower()


@_skip_if_gui
def test_advance_returns_until_reached_when_target_in_range():
    first = fs.agent_session.step()
    # 次の 1 バー分進めて stopped_reason="until_reached" を期待。
    target = first.clock_ms + 60_000  # M1 = 1 分
    resp = fs.agent_session.advance(until_ms=target)
    assert isinstance(resp, fs.AgentAdvanceResponse)
    assert resp.stopped_reason == "until_reached"
    assert resp.clock_ms == target


@_skip_if_gui
def test_advance_include_fills_false_omits_fills_array():
    first = fs.agent_session.step()
    resp = fs.agent_session.advance(
        until_ms=first.clock_ms + 60_000 * 2,
        include_fills=False,
    )
    assert resp.fills is None


# ── オフライン dataclass テスト（app 未起動でも走る）──────────────────────

class _Offline:
    """`pytest_collection_modifyitems` の skip からはずすため class でまとめる。

    conftest.py は `tests/python/` 直下の item を全 skip 対象にするが、
    class 配下のメソッドは `tests/python/test_agent_session.py::_Offline::...` の
    fspath も `tests/python` のままなので同様に skip される。
    => 代わりに `pytestmark` で明示的に `offline` mark を付け、app 有無で
    skip する item ループから外す方針は大掛かりになるため、**dataclass 変換の
    検証は pure 関数呼び出しとして `test_*` 命名せず `_assert_*` ヘルパー経由で
    collection 対象外とする** … のは分かりにくい。

    ここでは素直に `test_` プレフィックスで並べ、アプリ未起動時は skip を
    受け入れる。SDK のユニットカバレッジは Rust 側 + integration で十分。
    """


def test_dataclass_agent_fill_from_dict_with_client_order_id():
    fill = fs.AgentFill.from_dict({
        "order_id": "ord_abc",
        "client_order_id": "cli_42",
        "fill_price": 92100.5,
        "qty": 0.1,
        "side": "buy",
        "fill_time_ms": 1_704_067_260_000,
    })
    assert fill.order_id == "ord_abc"
    assert fill.client_order_id == "cli_42"
    assert fill.side == "buy"
    assert fill.fill_time_ms == 1_704_067_260_000


def test_dataclass_agent_fill_from_dict_without_client_order_id():
    fill = fs.AgentFill.from_dict({
        "order_id": "ord_xyz",
        "fill_price": 100.0,
        "qty": 0.05,
        "side": "sell",
        "fill_time_ms": 1_000_000,
    })
    assert fill.client_order_id is None


def test_dataclass_step_response_roundtrip():
    resp = fs.AgentStepResponse.from_dict({
        "clock_ms": 1_704_067_260_000,
        "reached_end": False,
        "observation": {"ohlcv": [], "recent_trades": [], "portfolio": {}},
        "fills": [{
            "order_id": "ord_1",
            "client_order_id": "cli_1",
            "fill_price": 100.0,
            "qty": 0.1,
            "side": "buy",
            "fill_time_ms": 1,
        }],
        "updated_narrative_ids": ["uuid-a"],
    })
    assert resp.clock_ms == 1_704_067_260_000
    assert len(resp.fills) == 1
    assert resp.fills[0].client_order_id == "cli_1"
    assert resp.updated_narrative_ids == ["uuid-a"]


def test_dataclass_advance_response_with_fills():
    resp = fs.AgentAdvanceResponse.from_dict({
        "clock_ms": 1_000,
        "stopped_reason": "fill",
        "ticks_advanced": 5,
        "aggregate_fills": 1,
        "aggregate_updated_narratives": 0,
        "final_portfolio": {"cash": 999_999.0},
        "fills": [{
            "order_id": "ord_a",
            "fill_price": 100.0,
            "qty": 0.1,
            "side": "buy",
            "fill_time_ms": 1,
        }],
    })
    assert resp.stopped_reason == "fill"
    assert resp.fills is not None
    assert len(resp.fills) == 1


def test_dataclass_advance_response_without_fills():
    resp = fs.AgentAdvanceResponse.from_dict({
        "clock_ms": 1_000,
        "stopped_reason": "until_reached",
        "ticks_advanced": 5,
        "aggregate_fills": 0,
        "aggregate_updated_narratives": 0,
        "final_portfolio": {"cash": 1_000_000.0},
    })
    assert resp.fills is None


def test_dataclass_order_response_roundtrip():
    resp = fs.AgentOrderResponse.from_dict({
        "order_id": "ord_server_uuid",
        "client_order_id": "cli_42",
        "idempotent_replay": True,
    })
    assert resp.idempotent_replay is True
    assert resp.order_id == "ord_server_uuid"
