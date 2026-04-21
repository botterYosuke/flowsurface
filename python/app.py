"""App control endpoints."""

from __future__ import annotations

from ._client import Client


class App:
    def __init__(self, client: Client) -> None:
        self._c = client

    def save(self) -> object:
        """POST /api/app/save — persist current state to disk"""
        return self._c.post("/api/app/save")

    def set_mode(self, mode: str) -> object:
        """POST /api/app/set-mode

        Args:
            mode: "live" or "replay"
        """
        return self._c.post("/api/app/set-mode", {"mode": mode})
