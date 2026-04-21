"""Sidebar and notification endpoints."""

from __future__ import annotations

from ._client import Client


class Sidebar:
    def __init__(self, client: Client) -> None:
        self._c = client

    def select_ticker(
        self, pane_id: str, ticker: str, kind: str | None = None
    ) -> object:
        """POST /api/sidebar/select-ticker"""
        return self._c.post(
            "/api/sidebar/select-ticker",
            {"pane_id": pane_id, "ticker": ticker, "kind": kind},
        )

    def open_order_pane(self, kind: str) -> object:
        """POST /api/sidebar/open-order-pane

        Args:
            kind: "OrderEntry", "OrderList", or "BuyingPower"
        """
        return self._c.post("/api/sidebar/open-order-pane", {"kind": kind})


