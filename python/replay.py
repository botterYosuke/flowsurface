from __future__ import annotations

from typing import Any

from ._client import Client


class ReplayApi:
    """Thin wrapper around the replay and app mode endpoints."""

    def __init__(self, client: Client) -> None:
        self._client = client

    def toggle(self, start: str | None = None, end: str | None = None) -> dict[str, Any]:
        """Toggle replay mode or initialize a replay range."""
        body: dict[str, Any] = {}
        if start is not None:
            body["start"] = start
        if end is not None:
            body["end"] = end
        return self._client.post("/api/replay/toggle", body)  # type: ignore[return-value]

    def get_status(self) -> dict[str, Any]:
        return self._client.get("/api/replay/status")  # type: ignore[return-value]

    def save_state(self) -> dict[str, Any]:
        return self._client.post("/api/app/save")  # type: ignore[return-value]

    def set_mode(self, mode: str) -> dict[str, Any]:
        return self._client.post("/api/app/set-mode", {"mode": mode})  # type: ignore[return-value]
