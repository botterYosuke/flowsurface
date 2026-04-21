"""Authentication endpoints."""

from __future__ import annotations

from ._client import Client


class Auth:
    def __init__(self, client: Client) -> None:
        self._c = client

    @property
    def tachibana_status(self) -> object:
        """GET /api/auth/tachibana/status"""
        return self._c.get("/api/auth/tachibana/status")

    def tachibana_logout(self) -> object:
        """POST /api/auth/tachibana/logout"""
        return self._c.post("/api/auth/tachibana/logout")
