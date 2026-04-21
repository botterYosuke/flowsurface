"""Notification endpoints."""

from __future__ import annotations

from ._client import Client


class Notification:
    def __init__(self, client: Client) -> None:
        self._c = client

    @property
    def list(self) -> object:
        """GET /api/notification/list"""
        return self._c.get("/api/notification/list")
