"""HTTP client for the flowsurface API (port 9876)."""

from __future__ import annotations

import httpx


class FlowsurfaceNotRunningError(ConnectionError):
    """Raised when the flowsurface app is not reachable."""

    def __init__(self, base_url: str) -> None:
        super().__init__(
            f"flowsurface app is not running at {base_url}. "
            "Start the app before using this helper."
        )


class ApiError(RuntimeError):
    """Raised when the API returns a non-2xx status."""

    def __init__(self, status_code: int, body: str) -> None:
        self.status_code = status_code
        super().__init__(f"API returned {status_code}: {body}")


class Client:
    def __init__(self, base_url: str = "http://127.0.0.1:9876", timeout: float = 5.0) -> None:
        self.base_url = base_url.rstrip("/")
        self.timeout = timeout

    def get(self, path: str, **params: str) -> object:
        url = f"{self.base_url}{path}"
        if params:
            qs = "&".join(f"{k}={v}" for k, v in params.items() if v is not None)
            url = f"{url}?{qs}"
        try:
            r = httpx.get(url, timeout=self.timeout)
        except (httpx.ConnectError, httpx.ConnectTimeout):
            raise FlowsurfaceNotRunningError(self.base_url)
        if not (200 <= r.status_code < 300):
            raise ApiError(r.status_code, r.text)
        return r.json()

    def post(self, path: str, body: dict | None = None) -> object:
        url = f"{self.base_url}{path}"
        payload = {k: v for k, v in (body or {}).items() if v is not None}
        try:
            r = httpx.post(url, json=payload or None, timeout=self.timeout)
        except (httpx.ConnectError, httpx.ConnectTimeout):
            raise FlowsurfaceNotRunningError(self.base_url)
        if not (200 <= r.status_code < 300):
            raise ApiError(r.status_code, r.text)
        return r.json()
