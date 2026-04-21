"""Replay control and virtual exchange endpoints."""

from __future__ import annotations

from ._client import Client


class Replay:
    def __init__(self, client: Client) -> None:
        self._c = client

    # ── GET (property-style) ──────────────────────────────────────────────

    @property
    def status(self) -> object:
        """GET /api/replay/status"""
        return self._c.get("/api/replay/status")

    @property
    def state(self) -> object:
        """GET /api/replay/state — virtual exchange state"""
        return self._c.get("/api/replay/state")

    @property
    def portfolio(self) -> object:
        """GET /api/replay/portfolio"""
        return self._c.get("/api/replay/portfolio")

    @property
    def orders(self) -> object:
        """GET /api/replay/orders — virtual order list"""
        return self._c.get("/api/replay/orders")

    # ── POST actions ─────────────────────────────────────────────────────

    def toggle(self) -> object:
        """POST /api/replay/toggle — start/stop replay"""
        return self._c.post("/api/replay/toggle")

    def play(self, start: str, end: str) -> object:
        """POST /api/replay/play — start replay for the given time range.

        Args:
            start: e.g. "2024-01-01 09:00:00"
            end:   e.g. "2024-01-01 15:30:00"
        """
        return self._c.post("/api/replay/play", {"start": start, "end": end})

    def pause(self) -> object:
        """POST /api/replay/pause"""
        return self._c.post("/api/replay/pause")

    def resume(self) -> object:
        """POST /api/replay/resume"""
        return self._c.post("/api/replay/resume")

    def step_forward(self) -> object:
        """POST /api/replay/step-forward"""
        return self._c.post("/api/replay/step-forward")

    def step_backward(self) -> object:
        """POST /api/replay/step-backward"""
        return self._c.post("/api/replay/step-backward")

    def cycle_speed(self) -> object:
        """POST /api/replay/speed — cycle through speed presets"""
        return self._c.post("/api/replay/speed")

    def order(
        self,
        ticker: str,
        side: str,
        qty: float,
        order_type: str | dict = "market",
    ) -> object:
        """POST /api/replay/order — place a virtual order.

        Args:
            ticker:     e.g. "BinanceLinear:BTCUSDT"
            side:       "buy" or "sell"
            qty:        quantity
            order_type: "market" (default) or {"limit": 50000.0}
        """
        return self._c.post(
            "/api/replay/order",
            {"ticker": ticker, "side": side, "qty": qty, "order_type": order_type},
        )
