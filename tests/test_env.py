"""Unit tests for FlowsurfaceEnv (no live server needed)."""
import subprocess
import pytest
from unittest.mock import MagicMock, patch

import numpy as np

from flowsurface.env import FlowsurfaceEnv


# ── constructor ───────────────────────────────────────────────────────────────

def test_env_gui_mode_allowed():
    env = FlowsurfaceEnv(headless=False)
    assert env.headless is False


def test_env_headless_flag_default_true():
    env = FlowsurfaceEnv()
    assert env.headless is True


def test_env_default_observation_space_shape():
    env = FlowsurfaceEnv(kline_limit=60)
    assert env.observation_space.shape == (240,)  # 60 × 4


def test_env_custom_kline_limit():
    env = FlowsurfaceEnv(kline_limit=10)
    assert env.observation_space.shape == (40,)  # 10 × 4


def test_env_action_space_side_discrete():
    env = FlowsurfaceEnv()
    assert env.action_space["side"].n == 3


def test_env_action_space_qty_box():
    env = FlowsurfaceEnv()
    assert env.action_space["qty"].shape == (1,)
    assert env.action_space["qty"].low[0] == pytest.approx(0.0)
    assert env.action_space["qty"].high[0] == pytest.approx(1.0)


# ── _start_process command construction ──────────────────────────────────────

def _make_popen_mock(env: FlowsurfaceEnv):
    """Patch Popen and requests so _start_process returns without a real binary."""
    import requests as req

    mock_response = MagicMock()
    mock_response.status_code = 200

    with patch("subprocess.Popen") as mock_popen, \
         patch("requests.get", return_value=mock_response):
        mock_proc = MagicMock()
        mock_proc.poll.return_value = None
        mock_popen.return_value = mock_proc
        env._start_process()
        return mock_popen.call_args[0][0]  # the cmd list


def test_start_process_includes_headless_flag():
    env = FlowsurfaceEnv(headless=True)
    cmd = _make_popen_mock(env)
    assert "--headless" in cmd


def test_start_process_excludes_headless_flag_in_gui_mode():
    env = FlowsurfaceEnv(headless=False)
    cmd = _make_popen_mock(env)
    assert "--headless" not in cmd


# ── _observe helper ───────────────────────────────────────────────────────────

def _make_env_no_proc() -> FlowsurfaceEnv:
    env = FlowsurfaceEnv(kline_limit=3)
    return env


def test_observe_returns_zeros_on_empty_klines():
    env = _make_env_no_proc()
    with patch.object(env, "_get", return_value={"klines": []}):
        obs = env._observe()
    assert obs.shape == (12,)
    assert np.all(obs == 0.0)


def test_observe_fills_ohlc_from_api_response():
    env = _make_env_no_proc()
    api_response = {
        "klines": [
            {
                "stream": "HyperliquidLinear:BTC:M1",
                "klines": [
                    {"time": 1000, "open": 100.0, "high": 110.0, "low": 90.0, "close": 105.0},
                    {"time": 2000, "open": 105.0, "high": 115.0, "low": 95.0, "close": 110.0},
                ],
            }
        ]
    }
    with patch.object(env, "_get", return_value=api_response):
        obs = env._observe()
    # kline_limit=3 → obs shape (12,); first 2 bars filled, last 4 zeros
    assert obs.shape == (12,)
    assert obs[0] == pytest.approx(100.0)  # open bar 0
    assert obs[1] == pytest.approx(110.0)  # high bar 0
    assert obs[2] == pytest.approx(90.0)   # low  bar 0
    assert obs[3] == pytest.approx(105.0)  # close bar 0
    assert obs[4] == pytest.approx(105.0)  # open bar 1
    assert obs[8] == pytest.approx(0.0)    # 3rd bar empty


def test_observe_truncates_to_kline_limit():
    env = _make_env_no_proc()  # kline_limit=3
    bars = [
        {"time": i * 1000, "open": float(i), "high": float(i) + 1, "low": float(i) - 1, "close": float(i) + 0.5}
        for i in range(10)
    ]
    api_response = {"klines": [{"stream": "X", "klines": bars}]}
    with patch.object(env, "_get", return_value=api_response):
        obs = env._observe()
    # Should use last 3 bars (i=7,8,9)
    assert obs[0] == pytest.approx(7.0)  # open of bar index 7


def test_observe_returns_zeros_on_connection_error():
    import requests as req
    env = _make_env_no_proc()
    with patch.object(env, "_get", side_effect=req.ConnectionError("connection refused")):
        obs = env._observe()
    assert obs.shape == (12,)
    assert np.all(obs == 0.0)


# ── _at_end ───────────────────────────────────────────────────────────────────

def test_at_end_returns_false_when_below_end_time():
    env = _make_env_no_proc()
    assert env._at_end({"current_time": 1000, "end_time": 2000}) is False


def test_at_end_returns_true_when_at_end_time():
    env = _make_env_no_proc()
    assert env._at_end({"current_time": 2000, "end_time": 2000}) is True


def test_at_end_returns_false_when_times_missing():
    env = _make_env_no_proc()
    assert env._at_end({}) is False


# ── close ─────────────────────────────────────────────────────────────────────

def test_close_is_safe_when_no_process():
    env = _make_env_no_proc()
    env.close()  # should not raise


def test_close_terminates_running_process():
    env = _make_env_no_proc()
    mock_proc = MagicMock()
    mock_proc.poll.return_value = None
    env._proc = mock_proc
    env.close()
    mock_proc.terminate.assert_called_once()
    mock_proc.wait.assert_called_once()
    assert env._proc is None
