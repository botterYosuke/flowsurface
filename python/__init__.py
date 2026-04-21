"""flowsurface Python helper.

Gymnasium RL environment::

    from flowsurface import FlowsurfaceEnv

HTTP API client (app must be running)::

    import flowsurface as fs

    fs.narrative.create(agent_id="my_agent", ticker="BTCUSDT", ...)
    fs.narrative.list(agent_id="my_agent")

Raises ``FlowsurfaceNotRunningError`` when the app is not reachable.
Reconfigure URL/timeout::

    fs.configure(base_url="http://127.0.0.1:9876", timeout=5.0)
"""

from ._client import ApiError, Client, FlowsurfaceNotRunningError
from .env import FlowsurfaceEnv
from .narrative import Narrative, NarrativeAction, NarrativeApi, NarrativeOutcome

__all__ = [
    "FlowsurfaceEnv",
    "configure",
    "narrative",
    "Narrative",
    "NarrativeAction",
    "NarrativeOutcome",
    "NarrativeApi",
    "FlowsurfaceNotRunningError",
    "ApiError",
]

_client = Client()

narrative: NarrativeApi = NarrativeApi(_client)


def configure(
    base_url: str = "http://127.0.0.1:9876",
    timeout: float = 5.0,
) -> None:
    """Reconfigure the shared HTTP client.

    Args:
        base_url: Base URL of the flowsurface HTTP API.
        timeout:  Request timeout in seconds.
    """
    global _client, narrative
    _client = Client(base_url=base_url, timeout=timeout)
    narrative = NarrativeApi(_client)
