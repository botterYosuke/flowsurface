"""Pane CRUD endpoints."""

from __future__ import annotations

from ._client import Client


class Pane:
    def __init__(self, client: Client) -> None:
        self._c = client

    @property
    def list(self) -> object:
        """GET /api/pane/list"""
        return self._c.get("/api/pane/list")

    def chart_snapshot(self, pane_id: str) -> object:
        """GET /api/pane/chart-snapshot?pane_id=…"""
        return self._c.get("/api/pane/chart-snapshot", pane_id=pane_id)

    def split(self, pane_id: str, axis: str = "Vertical") -> object:
        """POST /api/pane/split

        Args:
            axis: "Vertical" or "Horizontal"
        """
        return self._c.post("/api/pane/split", {"pane_id": pane_id, "axis": axis})

    def close(self, pane_id: str) -> object:
        """POST /api/pane/close"""
        return self._c.post("/api/pane/close", {"pane_id": pane_id})

    def set_ticker(self, pane_id: str, ticker: str) -> object:
        """POST /api/pane/set-ticker"""
        return self._c.post("/api/pane/set-ticker", {"pane_id": pane_id, "ticker": ticker})

    def set_timeframe(self, pane_id: str, timeframe: str) -> object:
        """POST /api/pane/set-timeframe

        Args:
            timeframe: e.g. "1m", "5m", "1h"
        """
        return self._c.post(
            "/api/pane/set-timeframe", {"pane_id": pane_id, "timeframe": timeframe}
        )
