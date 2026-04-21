"""Narrative API helpers (Phase 4a).

Usage::

    import flowsurface as fs
    narrative_id = fs.narrative.create(
        agent_id="my_agent",
        ticker="BTCUSDT",
        timeframe="1h",
        observation_snapshot={"rsi": 28.3},
        reasoning="RSI divergence",
        action={"side": "buy", "qty": 0.1, "price": 92500.0},
        confidence=0.76,
        linked_order_id="ord_123",
    )
    fs.narrative.list(agent_id="my_agent")
    fs.narrative.publish(narrative_id)
"""
from __future__ import annotations

from dataclasses import dataclass, field
from typing import Any, Literal

import httpx

from ._client import ApiError, Client, FlowsurfaceNotRunningError

NarrativeSide = Literal["buy", "sell"]


@dataclass
class NarrativeAction:
    side: NarrativeSide
    qty: float
    price: float

    def to_dict(self) -> dict[str, Any]:
        return {"side": self.side, "qty": self.qty, "price": self.price}


@dataclass
class NarrativeOutcome:
    fill_price: float
    fill_time_ms: int
    closed_at_ms: int | None = None
    realized_pnl: float | None = None

    @classmethod
    def from_dict(cls, d: dict[str, Any]) -> "NarrativeOutcome":
        return cls(
            fill_price=float(d["fill_price"]),
            fill_time_ms=int(d["fill_time_ms"]),
            closed_at_ms=int(d["closed_at_ms"]) if d.get("closed_at_ms") is not None else None,
            realized_pnl=float(d["realized_pnl"]) if d.get("realized_pnl") is not None else None,
        )


@dataclass
class Narrative:
    """Narrative record returned by ``GET /api/agent/narrative/:id``.

    Attributes:
        id:                  UUID primary key (server-generated).
        agent_id:            Free-form agent identifier.
        ticker / timeframe:  Market context.
        timestamp_ms:        Virtual time of observation (StepClock.now_ms()).
        reasoning:           Natural-language rationale.
        action:              NarrativeAction (side / qty / price).
        confidence:          0.0..=1.0.
        outcome:             Populated automatically when the linked order fills.
        public:              Publication flag (toggled via PATCH).
    """

    id: str
    agent_id: str
    timestamp_ms: int
    ticker: str
    timeframe: str
    reasoning: str
    action: NarrativeAction
    confidence: float
    created_at_ms: int
    snapshot_ref: dict[str, Any] = field(default_factory=dict)
    uagent_address: str | None = None
    outcome: NarrativeOutcome | None = None
    linked_order_id: str | None = None
    public: bool = False
    idempotency_key: str | None = None

    @classmethod
    def from_dict(cls, d: dict[str, Any]) -> "Narrative":
        action = NarrativeAction(
            side=d["action"]["side"],
            qty=float(d["action"]["qty"]),
            price=float(d["action"]["price"]),
        )
        outcome = (
            NarrativeOutcome.from_dict(d["outcome"])
            if d.get("outcome") is not None
            else None
        )
        return cls(
            id=str(d["id"]),
            agent_id=str(d["agent_id"]),
            timestamp_ms=int(d["timestamp_ms"]),
            ticker=str(d["ticker"]),
            timeframe=str(d["timeframe"]),
            reasoning=str(d["reasoning"]),
            action=action,
            confidence=float(d["confidence"]),
            created_at_ms=int(d["created_at_ms"]),
            snapshot_ref=dict(d.get("snapshot_ref") or {}),
            uagent_address=d.get("uagent_address"),
            outcome=outcome,
            linked_order_id=d.get("linked_order_id"),
            public=bool(d.get("public", False)),
            idempotency_key=d.get("idempotency_key"),
        )


class NarrativeApi:
    """Thin wrapper around the ``/api/agent/narrative`` endpoints.

    Errors:
        - ``FlowsurfaceNotRunningError`` if the app is not reachable.
        - ``ApiError`` for non-2xx responses (400 / 404 / 410 / 413 / 500).
    """

    def __init__(self, client: Client) -> None:
        self._client = client

    def create(
        self,
        *,
        agent_id: str,
        ticker: str,
        timeframe: str,
        observation_snapshot: dict[str, Any],
        reasoning: str,
        action: dict[str, Any] | NarrativeAction,
        confidence: float,
        linked_order_id: str | None = None,
        timestamp_ms: int | None = None,
        idempotency_key: str | None = None,
        uagent_address: str | None = None,
    ) -> dict[str, Any]:
        """POST /api/agent/narrative — returns ``{"id": ..., "snapshot_bytes": ..., "idempotent_replay": bool}``."""
        if isinstance(action, NarrativeAction):
            action = action.to_dict()
        payload: dict[str, Any] = {
            "agent_id": agent_id,
            "ticker": ticker,
            "timeframe": timeframe,
            "observation_snapshot": observation_snapshot,
            "reasoning": reasoning,
            "action": action,
            "confidence": confidence,
        }
        if linked_order_id is not None:
            payload["linked_order_id"] = linked_order_id
        if timestamp_ms is not None:
            payload["timestamp_ms"] = timestamp_ms
        if idempotency_key is not None:
            payload["idempotency_key"] = idempotency_key
        if uagent_address is not None:
            payload["uagent_address"] = uagent_address
        return self._client.post("/api/agent/narrative", payload)  # type: ignore[return-value]

    def list(
        self,
        *,
        agent_id: str | None = None,
        ticker: str | None = None,
        since_ms: int | None = None,
        limit: int | None = None,
    ) -> list[Narrative]:
        params: dict[str, str] = {}
        if agent_id:
            params["agent_id"] = agent_id
        if ticker:
            params["ticker"] = ticker
        if since_ms is not None:
            params["since_ms"] = str(since_ms)
        if limit is not None:
            params["limit"] = str(limit)
        resp = self._client.get("/api/agent/narratives", **params)
        narratives = resp.get("narratives", []) if isinstance(resp, dict) else []
        return [Narrative.from_dict(n) for n in narratives]

    def get(self, narrative_id: str) -> Narrative:
        resp = self._client.get(f"/api/agent/narrative/{narrative_id}")
        if not isinstance(resp, dict):
            raise ApiError(500, "unexpected response")
        return Narrative.from_dict(resp)

    def snapshot(self, narrative_id: str) -> dict[str, Any]:
        """Fetch the gzip-decoded observation_snapshot body."""
        resp = self._client.get(f"/api/agent/narrative/{narrative_id}/snapshot")
        return resp  # type: ignore[return-value]

    def publish(self, narrative_id: str) -> Narrative:
        """Convenience: PATCH :id with ``public=True``."""
        return self._patch(narrative_id, public=True)

    def unpublish(self, narrative_id: str) -> Narrative:
        return self._patch(narrative_id, public=False)

    def _patch(self, narrative_id: str, *, public: bool) -> Narrative:
        url = f"{self._client.base_url}/api/agent/narrative/{narrative_id}"
        try:
            r = httpx.patch(url, json={"public": public}, timeout=self._client.timeout)
        except (httpx.ConnectError, httpx.ConnectTimeout):
            raise FlowsurfaceNotRunningError(self._client.base_url)
        if not (200 <= r.status_code < 300):
            raise ApiError(r.status_code, r.text)
        return Narrative.from_dict(r.json())

    def storage_stats(self) -> dict[str, Any]:
        return self._client.get("/api/agent/narratives/storage")  # type: ignore[return-value]

    def orphans(self) -> list[str]:
        resp = self._client.get("/api/agent/narratives/orphans")
        if isinstance(resp, dict):
            return list(resp.get("orphan_files") or [])
        return []
