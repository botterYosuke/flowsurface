"""Agent 蟆ら畑 Replay API・・hase 4b-1・峨・ Python SDK 繝ｩ繝・ヱ繝ｼ縲・

ADR-0001 / `docs/plan/phase4b_agent_replay_api.md` ﾂｧ4 縺ｫ蟇ｾ蠢懊・
UI 繝ｪ繝｢繧ｳ繝ｳ API `/api/replay/*` 縺ｨ縺ｯ蛻･邨瑚ｷｯ縺ｧ縲∝梛螂醍ｴ・→豎ｺ螳夊ｫ匁ｧ繧呈球菫昴☆繧九・

Usage::

    import flowsurface as fs

    # 繝ｪ繝励Ξ繧､繧ｻ繝・す繝ｧ繝ｳ繧定ｵｷ蜍包ｼ・I 繝ｪ繝｢繧ｳ繝ｳ API 邨檎罰縲￣hase 4b-1 縺ｧ縺ｯ蠢・茨ｼ峨・
    fs._client.post("/api/replay/toggle", {"start": "2024-01-15 09:00", "end": "2024-01-15 15:30"})

    # agent API 繧貞娼縺上・
    resp = fs.agent_session.step()
    for fill in resp.fills:
        print(fill.client_order_id, fill.fill_price)

    fs.agent_session.place_order(
        client_order_id="cli_42",
        ticker={"exchange": "BinanceLinear", "symbol": "BTCUSDT"},
        side="buy",
        qty=0.1,
        order_type={"market": {}},
    )

    # Headless 繝ｩ繝ｳ繧ｿ繧､繝髯仙ｮ・ 莉ｻ諢丞玄髢薙ｒ instant 螳溯｡後・
    adv = fs.agent_session.advance(until_ms=1_706_659_200_000, stop_on=["fill"])

蛯呵・
- `session_id` 縺ｯ Phase 4b-1 縺ｧ縺ｯ `"default"` 蝗ｺ螳夲ｼ磯撼 default 縺ｯ 501・峨・
- `advance` 縺ｯ GUI / headless 縺ｮ荳｡譁ｹ縺ｧ蛻・ｊ菴ｿ縺医ｋ縲・
"""
from __future__ import annotations

from dataclasses import dataclass, field
from typing import Any, Literal

import httpx

from ._client import ApiError, Client, FlowsurfaceNotRunningError

AgentSide = Literal["buy", "sell"]
StopCondition = Literal["fill", "narrative"]
StoppedReason = Literal["until_reached", "fill", "narrative", "end"]
DEFAULT_SESSION = "default"


@dataclass
class AgentFill:
    order_id: str
    fill_price: float
    qty: float
    side: AgentSide
    fill_time_ms: int
    client_order_id: str | None = None

    @classmethod
    def from_dict(cls, d: dict[str, Any]) -> "AgentFill":
        return cls(
            order_id=str(d["order_id"]),
            fill_price=float(d["fill_price"]),
            qty=float(d["qty"]),
            side=d["side"],  # type: ignore[arg-type]
            fill_time_ms=int(d["fill_time_ms"]),
            client_order_id=d.get("client_order_id"),
        )


@dataclass
class AgentStepResponse:
    """`POST /api/agent/session/:id/step` 縺ｮ繝ｬ繧ｹ繝昴Φ繧ｹ縲・""

    clock_ms: int
    reached_end: bool
    observation: dict[str, Any]
    fills: list[AgentFill] = field(default_factory=list)
    updated_narrative_ids: list[str] = field(default_factory=list)

    @classmethod
    def from_dict(cls, d: dict[str, Any]) -> "AgentStepResponse":
        return cls(
            clock_ms=int(d["clock_ms"]),
            reached_end=bool(d["reached_end"]),
            observation=dict(d.get("observation") or {}),
            fills=[AgentFill.from_dict(f) for f in d.get("fills") or []],
            updated_narrative_ids=list(d.get("updated_narrative_ids") or []),
        )


@dataclass
class AgentAdvanceResponse:
    """`POST /api/agent/session/:id/advance` 縺ｮ繝ｬ繧ｹ繝昴Φ繧ｹ縲・""

    clock_ms: int
    stopped_reason: StoppedReason
    ticks_advanced: int
    aggregate_fills: int
    aggregate_updated_narratives: int
    final_portfolio: dict[str, Any]
    fills: list[AgentFill] | None = None

    @classmethod
    def from_dict(cls, d: dict[str, Any]) -> "AgentAdvanceResponse":
        fills_raw = d.get("fills")
        fills = (
            [AgentFill.from_dict(f) for f in fills_raw]
            if isinstance(fills_raw, list)
            else None
        )
        return cls(
            clock_ms=int(d["clock_ms"]),
            stopped_reason=d["stopped_reason"],  # type: ignore[arg-type]
            ticks_advanced=int(d["ticks_advanced"]),
            aggregate_fills=int(d["aggregate_fills"]),
            aggregate_updated_narratives=int(d["aggregate_updated_narratives"]),
            final_portfolio=dict(d.get("final_portfolio") or {}),
            fills=fills,
        )


@dataclass
class AgentOrderResponse:
    """`POST /api/agent/session/:id/order` 縺ｮ繝ｬ繧ｹ繝昴Φ繧ｹ縲・

    `idempotent_replay` 縺・True 縺ｪ繧牙酔荳繝ｪ繧ｯ繧ｨ繧ｹ繝医・蜀埼√→縺励※謇ｱ繧上ｌ縲∵里蟄・order_id
    縺瑚ｿ斐＆繧後ｋ・・lan ﾂｧ3.3・峨・
    """

    order_id: str
    client_order_id: str
    idempotent_replay: bool

    @classmethod
    def from_dict(cls, d: dict[str, Any]) -> "AgentOrderResponse":
        return cls(
            order_id=str(d["order_id"]),
            client_order_id=str(d["client_order_id"]),
            idempotent_replay=bool(d["idempotent_replay"]),
        )


class AgentSessionApi:
    """Agent 蟆ら畑 Replay API `/api/agent/session/:id/*` 縺ｮ繝ｩ繝・ヱ繝ｼ縲・

    Errors:
        - ``FlowsurfaceNotRunningError``: 繧｢繝励Μ縺瑚ｵｷ蜍輔＠縺ｦ縺・↑縺・・
        - ``ApiError``: 髱・2xx 蠢懃ｭ費ｼ・00 / 404 / 409 / 501 / 503・峨・
    """

    def __init__(self, client: Client, session_id: str = DEFAULT_SESSION) -> None:
        self._client = client
        self._session_id = session_id

    @property
    def session_id(self) -> str:
        return self._session_id

    def step(self) -> AgentStepResponse:
        """`POST /api/agent/session/:id/step` 窶・1 繝舌・騾ｲ陦・+ 蜑ｯ菴懃畑蜷梧｢ｱ縲・""
        path = f"/api/agent/session/{self._session_id}/step"
        resp = self._post_raw(path, body=None)
        return AgentStepResponse.from_dict(resp)

    def advance(
        self,
        *,
        until_ms: int,
        stop_on: list[StopCondition] | None = None,
        include_fills: bool = False,
    ) -> AgentAdvanceResponse:
        """`POST /api/agent/session/:id/advance` 窶・莉ｻ諢丞玄髢薙ｒ instant 螳溯｡後・

        `stop_on` / `include_fills` 縺ｯ headless 縺ｧ繧医ｊ螳悟・縺ｪ蜿ｯ閭ｽ諤ｧ縺後≠繧九・
        """
        path = f"/api/agent/session/{self._session_id}/advance"
        body: dict[str, Any] = {"until_ms": int(until_ms)}
        if stop_on is not None:
            body["stop_on"] = list(stop_on)
        if include_fills:
            body["include_fills"] = True
        resp = self._post_raw(path, body=body)
        return AgentAdvanceResponse.from_dict(resp)

    def place_order(
        self,
        *,
        client_order_id: str,
        ticker: dict[str, str],
        side: AgentSide,
        qty: float,
        order_type: dict[str, Any],
    ) -> AgentOrderResponse:
        """`POST /api/agent/session/:id/order` 窶・莉ｮ諠ｳ豕ｨ譁・ｼ亥・遲画ｧ縺ゅｊ・峨・

        Args:
            client_order_id: `[A-Za-z0-9_-]{1,64}`縲ょ酔縺倥く繝ｼ縺ｧ蜷後§ body 繧貞・騾√☆繧九→
                ``idempotent_replay=True`` 縺ｧ譌｢蟄・order_id 繧定ｿ斐☆縲Ｃody 縺檎焚縺ｪ繧九→ 409縲・
            ticker: ``{"exchange": "...", "symbol": "..."}``縲よｧ矩菴灘ｿ・茨ｼ域枚蟄怜・邨仙粋縺ｯ 400・峨・
            side: ``"buy"`` / ``"sell"``縲・
            qty: 豁｣縺ｮ譛蛾剞蛟､縲・
            order_type: ``{"market": {}}`` 縺ｾ縺溘・ ``{"limit": {"price": X}}``縲ら怐逡･縺ｯ 400縲・
        """
        path = f"/api/agent/session/{self._session_id}/order"
        body: dict[str, Any] = {
            "client_order_id": client_order_id,
            "ticker": dict(ticker),
            "side": side,
            "qty": float(qty),
            "order_type": dict(order_type),
        }
        resp = self._post_raw(path, body=body)
        return AgentOrderResponse.from_dict(resp)

    # 笏笏 菴弱Ξ繧､繝､: _client.post 縺ｯ遨ｺ霎樊嶌繧・None 蛹悶☆繧九◆繧√∥dvance 縺ｪ縺ｩ譏守､ｺ逧・↓
    #    body 譛臥┌繧貞宛蠕｡縺励◆縺・ｮ・園縺ｯ逶ｴ謗･ httpx 繧貞娼縺上ゅお繝ｩ繝ｼ繝上Φ繝峨Μ繝ｳ繧ｰ縺縺大粋繧上○繧九や楳笏

    def _post_raw(self, path: str, *, body: dict[str, Any] | None) -> dict[str, Any]:
        url = f"{self._client.base_url}{path}"
        try:
            r = httpx.post(url, json=body, timeout=self._client.timeout)
        except (httpx.ConnectError, httpx.ConnectTimeout):
            raise FlowsurfaceNotRunningError(self._client.base_url)
        if not (200 <= r.status_code < 300):
            raise ApiError(r.status_code, r.text)
        return r.json()
