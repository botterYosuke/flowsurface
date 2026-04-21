"""flowsurface Python helper.

Gymnasium RL environment::

    from flowsurface import FlowsurfaceEnv

HTTP API client (app must be running)::

    import flowsurface as fs

    fs.replay.status
    fs.replay.play("2024-01-01 09:00:00", "2024-01-01 15:30:00")
    fs.pane.list
    fs.tachibana.buying_power

Raises ``FlowsurfaceNotRunningError`` when the app is not reachable.
Reconfigure URL/timeout::

    fs.configure(base_url="http://127.0.0.1:9876", timeout=5.0)
"""

from .env import FlowsurfaceEnv
from ._client import ApiError, Client, FlowsurfaceNotRunningError
from .app import App
from .auth import Auth
from .notification import Notification
from .pane import Pane
from .replay import Replay
from .sidebar import Sidebar
from .tachibana import Tachibana

__all__ = [
    "FlowsurfaceEnv",
    "configure",
    "replay",
    "pane",
    "app",
    "auth",
    "tachibana",
    "sidebar",
    "notification",
    "FlowsurfaceNotRunningError",
    "ApiError",
]

_client = Client()

replay: Replay = Replay(_client)
pane: Pane = Pane(_client)
app: App = App(_client)
auth: Auth = Auth(_client)
tachibana: Tachibana = Tachibana(_client)
sidebar: Sidebar = Sidebar(_client)
notification: Notification = Notification(_client)


def configure(
    base_url: str = "http://127.0.0.1:9876",
    timeout: float = 5.0,
) -> None:
    """Reconfigure the shared HTTP client.

    Args:
        base_url: Base URL of the flowsurface HTTP API.
        timeout:  Request timeout in seconds.
    """
    global _client, replay, pane, app, auth, tachibana, sidebar, notification
    _client = Client(base_url=base_url, timeout=timeout)
    replay = Replay(_client)
    pane = Pane(_client)
    app = App(_client)
    auth = Auth(_client)
    tachibana = Tachibana(_client)
    sidebar = Sidebar(_client)
    notification = Notification(_client)
