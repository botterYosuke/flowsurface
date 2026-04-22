"""Agent 専用 Replay API（Phase 4b-1）の Python SDK ラッパー。

ADR-0001 / `docs/plan/phase4b_agent_replay_api.md` §4 に対応。
UI リモコン API `/api/replay/*` とは別経路で、型契約と決定論性を担保する。

Usage::

    import flowsurface as fs

    # リプレイセッションを起動（UI リモコン API 経由、Phase 4b-1 では必須）。
    fs._client.post("/api/replay/play", {"start": "2024-01-15 09:00", "end": "2024-01-15 15:30"})

    # agent API を叩く。
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

    # Headless ランタイム限定: 任意区間を instant 実行。
    adv = fs.agent_session.advance(until_ms=1_706_659_200_000, stop_on=["fill"])

備考:
- `session_id` は Phase 4b-1 では `"default"` 固定（非 default は 501）。
- `advance` は GUI ランタイムでは 400（ADR-0001 不変条件）。
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
    """`POST /api/agent/session/:id/step` のレスポンス。"""

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
    """`POST /api/agent/session/:id/advance` のレスポンス。"""

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
    """`POST /api/agent/session/:id/order` のレスポンス。

    `idempotent_replay` が True なら同一リクエストの再送として扱われ、既存 order_id
    が返される（plan §3.3）。
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
    """Agent 専用 Replay API `/api/agent/session/:id/*` のラッパー。

    Errors:
        - ``FlowsurfaceNotRunningError``: アプリが起動していない。
        - ``ApiError``: 非 2xx 応答（400 / 404 / 409 / 501 / 503）。
    """

    def __init__(self, client: Client, session_id: str = DEFAULT_SESSION) -> None:
        self._client = client
        self._session_id = session_id

    @property
    def session_id(self) -> str:
        return self._session_id

    def step(self) -> AgentStepResponse:
        """`POST /api/agent/session/:id/step` — 1 バー進行 + 副作用同梱。"""
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
        """`POST /api/agent/session/:id/advance` — 任意区間を instant 実行。

        GUI ランタイム（`--headless` なし）では 400 で拒否される（ADR-0001）。
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
        """`POST /api/agent/session/:id/order` — 仮想注文（冪等性あり）。

        Args:
            client_order_id: `[A-Za-z0-9_-]{1,64}`。同じキーで同じ body を再送すると
                ``idempotent_replay=True`` で既存 order_id を返す。body が異なると 409。
            ticker: ``{"exchange": "...", "symbol": "..."}``。構造体必須（文字列結合は 400）。
            side: ``"buy"`` / ``"sell"``。
            qty: 正の有限値。
            order_type: ``{"market": {}}`` または ``{"limit": {"price": X}}``。省略は 400。
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

    # ── 低レイヤ: _client.post は空辞書を None 化するため、advance など明示的に
    #    body 有無を制御したい箇所は直接 httpx を叩く。エラーハンドリングだけ合わせる。──

    def _post_raw(self, path: str, *, body: dict[str, Any] | None) -> dict[str, Any]:
        url = f"{self._client.base_url}{path}"
        try:
            r = httpx.post(url, json=body, timeout=self._client.timeout)
        except (httpx.ConnectError, httpx.ConnectTimeout):
            raise FlowsurfaceNotRunningError(self._client.base_url)
        if not (200 <= r.status_code < 300):
            raise ApiError(r.status_code, r.text)
        return r.json()
