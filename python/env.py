"""FlowsurfaceEnv — Gymnasium-compatible RL environment backed by flowsurface headless mode."""
from __future__ import annotations

import subprocess
import time
import shutil
import os
from pathlib import Path
from typing import Any

import gymnasium as gym
import numpy as np
import requests

_DEFAULT_API_PORT = 9876
_STARTUP_TIMEOUT_S = 30
_LOAD_POLL_INTERVAL_S = 0.2
_LOAD_TIMEOUT_S = 120


class FlowsurfaceEnv(gym.Env):
    """Gymnasium environment that controls flowsurface in ``--headless`` mode.

    Parameters
    ----------
    ticker:
        Ticker string in ``ExchangeName:Symbol`` format,
        e.g. ``"HyperliquidLinear:BTC"``.
    timeframe:
        Timeframe string, e.g. ``"M1"``, ``"H1"``.  Default ``"M1"``.
    binary_path:
        Path to the ``flowsurface`` binary.  ``None`` searches ``PATH``.
    api_port:
        HTTP API port (default 9876).
    initial_cash:
        Starting cash for the virtual exchange engine (informational; the
        Rust side initialises to 1_000_000).
    kline_limit:
        Number of most-recent OHLCV bars returned in each observation.
    headless:
        Must be ``True``; kept for clarity / forward-compatibility.
    """

    metadata = {"render_modes": []}

    # Observation: flat array of (open, high, low, close) × kline_limit
    # Action: Dict with "side" ∈ {0=hold, 1=buy, 2=sell} and "qty" ∈ [0, 1]

    def __init__(
        self,
        *,
        ticker: str = "HyperliquidLinear:BTC",
        timeframe: str = "M1",
        binary_path: str | None = None,
        api_port: int = _DEFAULT_API_PORT,
        initial_cash: float = 1_000_000.0,
        kline_limit: int = 60,
        headless: bool = True,
    ) -> None:
        super().__init__()
        self.headless = headless

        self.ticker = ticker
        self.timeframe = timeframe
        self.api_port = api_port
        self.initial_cash = initial_cash
        self.kline_limit = kline_limit

        self._binary = binary_path or self._find_binary()
        self._proc: subprocess.Popen | None = None
        self._base_url = f"http://127.0.0.1:{api_port}"

        # Observation: 4 floats (OHLC) × kline_limit
        obs_shape = (kline_limit * 4,)
        self.observation_space = gym.spaces.Box(
            low=0.0, high=np.inf, shape=obs_shape, dtype=np.float32
        )
        # Action: {"side": Discrete(3), "qty": Box(0,1)}
        self.action_space = gym.spaces.Dict(
            {
                "side": gym.spaces.Discrete(3),  # 0=hold, 1=buy, 2=sell
                "qty": gym.spaces.Box(low=0.0, high=1.0, shape=(1,), dtype=np.float32),
            }
        )

        self._prev_total_equity: float = initial_cash
        self._done: bool = False

    # ── Gymnasium API ─────────────────────────────────────────────────────────

    def reset(
        self,
        *,
        start: str,
        end: str,
        seed: int | None = None,
        options: dict | None = None,
    ):
        """Start (or restart) a replay session.

        Parameters
        ----------
        start / end:
            Date-time strings in ``"YYYY-MM-DD HH:MM"`` format (UTC).
        """
        super().reset(seed=seed)

        if self._proc is None or self._proc.poll() is not None:
            self._start_process()

        # POST /api/replay/play
        resp = self._post("/api/replay/play", {"start": start, "end": end})
        resp.raise_for_status()

        # Poll until status transitions from "loading" to "Paused"
        self._wait_until_active()

        self._prev_total_equity = self.initial_cash
        self._done = False

        obs = self._observe()
        info = self._get_status()
        return obs, info

    def step(self, action: dict):
        """Advance one time-step.

        *action* must be a dict with:
        - ``"side"``: int  0=hold, 1=buy, 2=sell
        - ``"qty"``: float in [0, 1]  (fraction of available cash / position)
        """
        if self._done:
            raise RuntimeError("Episode is done; call reset() first.")

        side = int(action["side"])
        qty_frac = float(np.asarray(action["qty"]).flat[0])

        if side != 0:
            side_str = "buy" if side == 1 else "sell"
            qty = max(qty_frac, 1e-8)
            resp_order = self._post(
                "/api/replay/order",
                {
                    "ticker": self.ticker,
                    "side": side_str,
                    "qty": qty,
                    "order_type": "market",
                },
            )
            resp_order.raise_for_status()

        # Advance clock one step
        resp = self._post("/api/replay/step-forward", {})
        resp.raise_for_status()

        # Check if replay ended
        status = self._get_status()
        done = status.get("status") == "Paused" and self._at_end(status)
        truncated = False

        # Reward = change in total equity
        portfolio = self._get_portfolio()
        total_equity = portfolio.get("total_equity", self._prev_total_equity)
        reward = total_equity - self._prev_total_equity
        self._prev_total_equity = total_equity

        self._done = done
        obs = self._observe()
        info = {"status": status, "portfolio": portfolio}
        return obs, reward, done, truncated, info

    def close(self):
        if self._proc is not None and self._proc.poll() is None:
            self._proc.terminate()
            try:
                self._proc.wait(timeout=5)
            except subprocess.TimeoutExpired:
                self._proc.kill()
        self._proc = None

    def render(self):
        pass

    # ── Internal helpers ──────────────────────────────────────────────────────

    @staticmethod
    def _find_binary() -> str:
        """Locate the flowsurface binary: env var → PATH → repo target dirs."""
        env_bin = os.environ.get("FLOWSURFACE_BINARY")
        if env_bin:
            return env_bin
        in_path = shutil.which("flowsurface")
        if in_path:
            return in_path
        repo_root = Path(__file__).parent.parent
        for candidate in (
            repo_root / "target" / "debug" / "flowsurface.exe",
            repo_root / "target" / "release" / "flowsurface.exe",
            repo_root / "target" / "debug" / "flowsurface",
            repo_root / "target" / "release" / "flowsurface",
        ):
            if candidate.exists():
                return str(candidate)
        return "flowsurface"

    def _start_process(self) -> None:
        env = os.environ.copy()
        if "DEV_IS_DEMO" not in env:
            env["DEV_IS_DEMO"] = "true"
        env["FLOWSURFACE_API_PORT"] = str(self.api_port)

        cmd = [self._binary]
        if self.headless:
            cmd.append("--headless")
        cmd += ["--ticker", self.ticker, "--timeframe", self.timeframe]
        self._proc = subprocess.Popen(
            cmd,
            env=env,
            stdout=subprocess.DEVNULL,
            stderr=subprocess.DEVNULL,
        )

        # Wait until API responds
        deadline = time.monotonic() + _STARTUP_TIMEOUT_S
        while time.monotonic() < deadline:
            try:
                r = requests.get(f"{self._base_url}/api/replay/status", timeout=1)
                if r.status_code == 200:
                    return
            except requests.ConnectionError:
                pass
            time.sleep(0.2)

        self._proc.terminate()
        try:
            self._proc.wait(timeout=3)
        except subprocess.TimeoutExpired:
            self._proc.kill()
        raise TimeoutError(
            f"flowsurface headless did not start within {_STARTUP_TIMEOUT_S}s"
        )

    def _wait_until_active(self) -> None:
        deadline = time.monotonic() + _LOAD_TIMEOUT_S
        while time.monotonic() < deadline:
            status = self._get_status()
            s = status.get("status")
            if s in ("Paused", "Playing"):
                return
            time.sleep(_LOAD_POLL_INTERVAL_S)
        raise TimeoutError(
            f"flowsurface replay did not become active within {_LOAD_TIMEOUT_S}s"
        )

    def _at_end(self, status: dict) -> bool:
        ct = status.get("current_time")
        et = status.get("end_time")
        if ct is None or et is None:
            return False
        return ct >= et

    def _get(self, path: str) -> dict:
        r = requests.get(f"{self._base_url}{path}", timeout=5)
        r.raise_for_status()
        return r.json()

    def _post(self, path: str, body: dict) -> requests.Response:
        return requests.post(f"{self._base_url}{path}", json=body, timeout=5)

    def _get_status(self) -> dict:
        return self._get("/api/replay/status")

    def _get_portfolio(self) -> dict:
        return self._get("/api/replay/portfolio")

    def _observe(self) -> np.ndarray:
        """Return a flat float32 array of (open, high, low, close) × kline_limit."""
        try:
            data = self._get("/api/replay/state")
        except (requests.ConnectionError, requests.HTTPError):
            return np.zeros(self.kline_limit * 4, dtype=np.float32)

        klines_groups = data.get("klines", [])
        if not klines_groups:
            return np.zeros(self.kline_limit * 4, dtype=np.float32)

        bars = klines_groups[0].get("klines", [])
        # Take the last kline_limit bars
        bars = bars[-self.kline_limit :]

        obs = np.zeros(self.kline_limit * 4, dtype=np.float32)
        for i, bar in enumerate(bars):
            base = i * 4
            obs[base + 0] = float(bar.get("open", 0))
            obs[base + 1] = float(bar.get("high", 0))
            obs[base + 2] = float(bar.get("low", 0))
            obs[base + 3] = float(bar.get("close", 0))
        return obs
