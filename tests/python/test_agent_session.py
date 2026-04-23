"""test_agent_session.py 窶・Phase 4b-1 agent API 縺ｮ螳溘い繝励Μ逍朱壹ユ繧ｹ繝医・

繧｢繝励Μ譛ｪ襍ｷ蜍墓凾縺ｯ conftest.py 縺ｧ蜈ｨ skip縲・

蜑肴署:
- 繝ｪ繝励Ξ繧､繧ｻ繝・す繝ｧ繝ｳ繧・UI 繝ｪ繝｢繧ｳ繝ｳ API `/api/replay/play` 縺ｧ襍ｷ蜍輔＠縺溷ｾ後↓
  agent API 縺悟虚菴懊☆繧具ｼ・lan ﾂｧ5.1 縺ｮ縲悟酔荳 VirtualExchange 蜈ｱ譛峨肴婿驥晢ｼ峨・
- `/advance` 縺ｯ Headless 繝ｩ繝ｳ繧ｿ繧､繝縺ｧ縺ｮ縺ｿ蜿礼炊縺輔ｌ繧具ｼ・DR-0001・峨・UI 襍ｷ蜍輔・
  繧｢繝励Μ縺ｧ縺ｯ 400 縺瑚ｿ斐ｋ縺溘ａ譛ｬ繧ｹ繧､繝ｼ繝医〒縺ｯ `test_advance_*` 縺ｯ skip 縺輔ｌ繧九・
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


def _current_clock_ms() -> int:
    r = httpx.get(f"{BASE_URL}/api/replay/status", timeout=5.0)
    r.raise_for_status()
    return int(r.json()["current_time"])


def _is_headless() -> bool:
    """Advance 縺碁壹ｋ繝ｩ繝ｳ繧ｿ繧､繝縺九ｒ蜑阪ｂ縺｣縺ｦ蛻､螳壹☆繧九・

    `/advance` 縺ｫ遨ｺ body 繧呈兜縺偵・00 縺ｮ body 縺ｫ "headless" 縺悟性縺ｾ繧後◆繧・GUI 縺ｨ蛻､譁ｭ縲・
    縺昴・莉厄ｼ・xx / 4xx with 蛻･繝｡繝・そ繝ｼ繧ｸ / 5xx・峨↑繧・Headless 謇ｱ縺・・
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
    """蜷・ユ繧ｹ繝亥燕縺ｫ replay 繧ｻ繝・す繝ｧ繝ｳ繧定ｵｷ蜍輔☆繧九・""
    _post("/api/replay/toggle", {"start": START, "end": END})
    # loading 縺・1 遘剃ｻ･蜀・↓ active 縺ｫ驕ｷ遘ｻ縺吶ｋ縺薙→繧呈悄蠕・りｩｳ邏ｰ縺ｪ barrier 縺ｯ譌｢蟄・
    # E2E 繝・せ繝医→蜷後§ pattern・育洒縺・sleep・峨・
    time.sleep(1.0)


# 笏笏 step 笏笏笏笏笏笏笏笏笏笏笏笏笏笏笏笏笏笏笏笏笏笏笏笏笏笏笏笏笏笏笏笏笏笏笏笏笏笏笏笏笏笏笏笏笏笏笏笏笏笏笏笏笏笏笏笏笏笏笏笏笏笏笏笏笏笏笏笏

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


# 笏笏 order + idempotency 笏笏笏笏笏笏笏笏笏笏笏笏笏笏笏笏笏笏笏笏笏笏笏笏笏笏笏笏笏笏笏笏笏笏笏笏笏笏笏笏笏笏笏笏笏笏笏笏笏笏笏笏笏笏

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
    assert resp.order_id  # 髱樒ｩｺ


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
            qty=0.2,  # 逡ｰ縺ｪ繧・
            order_type={"market": {}},
        )
    assert exc.value.status_code == 409


def test_place_order_400_on_string_ticker_rejects_at_http_layer():
    # SDK 縺ｯ dict 繧貞女縺代ｋ縺後∵焔蜍輔〒譁・ｭ怜・繧帝√ｋ縺ｨ server 縺・400 繧定ｿ斐☆縺薙→繧堤｢ｺ隱阪・
    r = httpx.post(
        f"{BASE_URL}/api/agent/session/default/order",
        json={
            "client_order_id": _unique_cli(),
            "ticker": "BinanceLinear:BTCUSDT",  # 譁・ｭ怜・ = 400
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


# 笏笏 session_id 竕 default 竊・501 笏笏笏笏笏笏笏笏笏笏笏笏笏笏笏笏笏笏笏笏笏笏笏笏笏笏笏笏笏笏笏笏笏笏笏笏笏笏笏笏笏笏笏笏笏笏

def test_non_default_session_id_returns_501():
    r = httpx.post(
        f"{BASE_URL}/api/agent/session/other/step",
        timeout=5.0,
    )
    assert r.status_code == 501
    assert "multi-session" in r.text.lower()


# 笏笏 advance (headless only) 笏笏笏笏笏笏笏笏笏笏笏笏笏笏笏笏笏笏笏笏笏笏笏笏笏笏笏笏笏笏笏笏笏笏笏笏笏笏笏笏笏笏笏笏笏笏笏笏笏

_HEADLESS = _is_headless()
_skip_if_gui = pytest.mark.skipif(
    False,
    reason="advance now runs on both GUI and headless runtimes",
)


def test_advance_available_in_gui_and_headless():
    first_clock = _current_clock_ms()
    r = httpx.post(
        f"{BASE_URL}/api/agent/session/default/advance",
        json={"until_ms": first_clock + 60_000},
        timeout=5.0,
    )
    assert r.status_code == 200, r.text
    body = r.json()
    assert body["stopped_reason"] in {"until_reached", "end"}
    assert body["clock_ms"] >= first_clock


@_skip_if_gui
def test_advance_returns_until_reached_when_target_in_range():
    current_clock = _current_clock_ms()
    target = current_clock + 60_000  # M1 = 1 蛻・
    resp = fs.agent_session.advance(until_ms=target)
    assert isinstance(resp, fs.AgentAdvanceResponse)
    assert resp.stopped_reason == "until_reached"
    assert resp.clock_ms == target


@_skip_if_gui
def test_advance_include_fills_false_omits_fills_array():
    current_clock = _current_clock_ms()
    resp = fs.agent_session.advance(
        until_ms=current_clock + 60_000 * 2,
        include_fills=False,
    )
    assert resp.fills is None


def test_rewind_to_start_initializes_session_when_idle():
    _post("/api/app/set-mode", {"mode": "live"})
    r = httpx.post(
        f"{BASE_URL}/api/agent/session/default/rewind-to-start",
        json={"start": START, "end": END},
        timeout=5.0,
    )
    assert r.status_code == 200, r.text
    body = r.json()
    assert body["ok"] is True
    assert body["status"] == "loading"


# 笏笏 繧ｪ繝輔Λ繧､繝ｳ dataclass 繝・せ繝茨ｼ・pp 譛ｪ襍ｷ蜍輔〒繧りｵｰ繧具ｼ俄楳笏笏笏笏笏笏笏笏笏笏笏笏笏笏笏笏笏笏笏笏笏

class _Offline:
    """`pytest_collection_modifyitems` 縺ｮ skip 縺九ｉ縺ｯ縺壹☆縺溘ａ class 縺ｧ縺ｾ縺ｨ繧√ｋ縲・

    conftest.py 縺ｯ `tests/python/` 逶ｴ荳九・ item 繧貞・ skip 蟇ｾ雎｡縺ｫ縺吶ｋ縺後・
    class 驟堺ｸ九・繝｡繧ｽ繝・ラ縺ｯ `tests/python/test_agent_session.py::_Offline::...` 縺ｮ
    fspath 繧・`tests/python` 縺ｮ縺ｾ縺ｾ縺ｪ縺ｮ縺ｧ蜷梧ｧ倥↓ skip 縺輔ｌ繧九・
    => 莉｣繧上ｊ縺ｫ `pytestmark` 縺ｧ譏守､ｺ逧・↓ `offline` mark 繧剃ｻ倥￠縲∥pp 譛臥┌縺ｧ
    skip 縺吶ｋ item 繝ｫ繝ｼ繝励°繧牙､悶☆譁ｹ驥昴・螟ｧ謗帙°繧翫↓縺ｪ繧九◆繧√・*dataclass 螟画鋤縺ｮ
    讀懆ｨｼ縺ｯ pure 髢｢謨ｰ蜻ｼ縺ｳ蜃ｺ縺励→縺励※ `test_*` 蜻ｽ蜷阪○縺・`_assert_*` 繝倥Ν繝代・邨檎罰縺ｧ
    collection 蟇ｾ雎｡螟悶→縺吶ｋ** 窶ｦ 縺ｮ縺ｯ蛻・°繧翫↓縺上＞縲・

    縺薙％縺ｧ縺ｯ邏逶ｴ縺ｫ `test_` 繝励Ξ繝輔ぅ繝・け繧ｹ縺ｧ荳ｦ縺ｹ縲√い繝励Μ譛ｪ襍ｷ蜍墓凾縺ｯ skip 繧・
    蜿励￠蜈･繧後ｋ縲４DK 縺ｮ繝ｦ繝九ャ繝医き繝舌Ξ繝・ず縺ｯ Rust 蛛ｴ + integration 縺ｧ蜊∝・縲・
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
