"""Agent session API helpers.

Usage::

    import flowsurface as fs

    fs.replay.toggle(start="2024-01-15 09:00", end="2024-01-15 15:30")

    step = fs.agent_session.step()
    order = fs.agent_session.place_order(
        client_order_id="cli_42",
        ticker={"exchange": "BinanceLinear", "symbol": "BTCUSDT"},
        side="buy",
        qty=0.1,
        order_type={"market": {}},
    )
    advance = fs.agent_session.advance(
        until_ms=1_706_659_200_000,
        stop_on=["fill"],
        include_fills=True,
    )
    fs.agent_session.rewind_to_start()
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
    """Response from ``POST /api/agent/session/:id/step``."""

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
    """Response from ``POST /api/agent/session/:id/advance``."""

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
class AgentRewindResponse:
    """Response from ``POST /api/agent/session/:id/rewind-to-start``."""

    ok: bool
    status: str | None = None
    clock_ms: int | None = None
    start: str | None = None
    end: str | None = None
    final_portfolio: dict[str, Any] | None = None

    @classmethod
    def from_dict(cls, d: dict[str, Any]) -> "AgentRewindResponse":
        clock_raw = d.get("clock_ms")
        return cls(
            ok=bool(d["ok"]),
            status=str(d["status"]) if d.get("status") is not None else None,
            clock_ms=int(clock_raw) if clock_raw is not None else None,
            start=str(d["start"]) if d.get("start") is not None else None,
            end=str(d["end"]) if d.get("end") is not None else None,
            final_portfolio=(
                dict(d["final_portfolio"])
                if isinstance(d.get("final_portfolio"), dict)
                else None
            ),
        )


@dataclass
class AgentOrderResponse:
    """Response from ``POST /api/agent/session/:id/order``."""

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
    """Thin wrapper around ``/api/agent/session/:id/*``."""

    def __init__(self, client: Client, session_id: str = DEFAULT_SESSION) -> None:
        self._client = client
        self._session_id = session_id

    @property
    def session_id(self) -> str:
        return self._session_id

    def step(self) -> AgentStepResponse:
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
        path = f"/api/agent/session/{self._session_id}/advance"
        body: dict[str, Any] = {"until_ms": int(until_ms)}
        if stop_on is not None:
            body["stop_on"] = list(stop_on)
        if include_fills:
            body["include_fills"] = True
        resp = self._post_raw(path, body=body)
        return AgentAdvanceResponse.from_dict(resp)

    def rewind_to_start(
        self,
        *,
        start: str | None = None,
        end: str | None = None,
    ) -> AgentRewindResponse:
        """Rewind the active session to its range start, or initialize one.

        Pass both ``start`` and ``end`` when no replay session is active. When a
        session is already active, omit them to reset clock, virtual fills, and
        order idempotency state to the beginning of the current range.
        """
        path = f"/api/agent/session/{self._session_id}/rewind-to-start"
        body: dict[str, Any] | None = None
        if start is not None or end is not None:
            if start is None or end is None:
                raise ValueError("start and end must be provided together")
            body = {"start": start, "end": end}
        resp = self._post_raw(path, body=body)
        return AgentRewindResponse.from_dict(resp)

    def place_order(
        self,
        *,
        client_order_id: str,
        ticker: dict[str, str],
        side: AgentSide,
        qty: float,
        order_type: dict[str, Any],
    ) -> AgentOrderResponse:
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

    def _post_raw(self, path: str, *, body: dict[str, Any] | None) -> dict[str, Any]:
        url = f"{self._client.base_url}{path}"
        try:
            response = httpx.post(url, json=body, timeout=self._client.timeout)
        except (httpx.ConnectError, httpx.ConnectTimeout):
            raise FlowsurfaceNotRunningError(self._client.base_url)
        if not (200 <= response.status_code < 300):
            raise ApiError(response.status_code, response.text)
        return response.json()
