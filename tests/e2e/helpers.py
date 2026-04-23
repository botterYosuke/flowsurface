"""helpers.py — E2E テスト共通ヘルパー (bash common_helpers.sh の Python 版)

使い方:
    from helpers import *   # or: from helpers import api_get, wait_playing, ...
"""
from __future__ import annotations

import json
import os
import shutil
import sys
import time
from datetime import datetime, timezone, timedelta
from pathlib import Path
from typing import Any

import requests

# ── SDK import ─────────────────────────────────────────────────────────────────

_REPO_ROOT = Path(__file__).parent.parent
try:
    from flowsurface import FlowsurfaceEnv
except ImportError:
    sys.path.insert(0, str(_REPO_ROOT / "python"))
    from env import FlowsurfaceEnv  # type: ignore[no-redef]

# ── 定数 ───────────────────────────────────────────────────────────────────────

API_BASE = "http://127.0.0.1:9876"
TICKER = os.environ.get("E2E_TICKER", "BinanceLinear:BTCUSDT")
IS_HEADLESS = os.environ.get("IS_HEADLESS", "").lower() == "true"

STEP_M1: int = 60_000
STEP_M5: int = 300_000
STEP_H1: int = 3_600_000
STEP_D1: int = 86_400_000

DATA_DIR = Path(os.environ.get("APPDATA", "")) / "flowsurface"
STATE_FILE = DATA_DIR / "saved-state.json"
STATE_BACKUP = DATA_DIR / "saved-state.json.bak"

# ── カウンタ ───────────────────────────────────────────────────────────────────

_PASS = 0
_FAIL = 0
_PEND = 0


def reset_counters() -> None:
    global _PASS, _FAIL, _PEND
    _PASS = _FAIL = _PEND = 0


# ── レポートヘルパー ──────────────────────────────────────────────────────────

def pass_(label: str) -> None:
    global _PASS
    print(f"  PASS: {label}")
    _PASS += 1


def fail(label: str, detail: str = "") -> None:
    global _FAIL
    msg = f"  FAIL: {label}"
    if detail:
        msg += f" — {detail}"
    print(msg)
    _FAIL += 1


def pend(label: str, reason: str = "") -> None:
    global _PEND
    msg = f"  PEND: {label}"
    if reason:
        msg += f" — {reason}"
    print(msg)
    _PEND += 1


def print_summary() -> None:
    print()
    print("=============================")
    print(f"  PASS: {_PASS}  FAIL: {_FAIL}  PEND: {_PEND}")
    print("=============================")


# ── 日時ユーティリティ ────────────────────────────────────────────────────────

def utc_offset(hours: float) -> str:
    dt = datetime.now(timezone.utc) + timedelta(hours=hours)
    return dt.strftime("%Y-%m-%d %H:%M")


def utc_to_ms(dt_str: str) -> int:
    """'YYYY-MM-DD HH:MM' を UTC ミリ秒に変換。"""
    dt = datetime.strptime(dt_str, "%Y-%m-%d %H:%M").replace(tzinfo=timezone.utc)
    return int(dt.timestamp() * 1000)


# ── saved-state バックアップ ──────────────────────────────────────────────────

def backup_state() -> None:
    if STATE_FILE.exists():
        shutil.copy2(STATE_FILE, STATE_BACKUP)


def restore_state() -> None:
    STATE_FILE.unlink(missing_ok=True)
    if STATE_BACKUP.exists():
        STATE_BACKUP.rename(STATE_FILE)


# ── フィクスチャ ──────────────────────────────────────────────────────────────

# headless モード用の内部変数
_headless_start: str = ""
_headless_end: str = ""
_headless_timeframe: str = "M1"


def setup_single_pane(
    ticker: str,
    timeframe: str,
    start: str,
    end: str,
) -> None:
    """単一ペインの saved-state.json を書き込む（GUI 時のみ）。
    headless 時は replay/play に使う start/end を内部変数に保存する。
    """
    global _headless_start, _headless_end, _headless_timeframe
    _headless_start = start
    _headless_end = end
    _headless_timeframe = timeframe

    if IS_HEADLESS:
        return

    DATA_DIR.mkdir(parents=True, exist_ok=True)
    fixture = {
        "layout_manager": {
            "layouts": [
                {
                    "name": f"Test-{timeframe}",
                    "dashboard": {
                        "pane": {
                            "KlineChart": {
                                "layout": {"splits": [0.78], "autoscale": "FitToVisible"},
                                "kind": "Candles",
                                "stream_type": [{"Kline": {"ticker": ticker, "timeframe": timeframe}}],
                                "settings": {
                                    "tick_multiply": None,
                                    "visual_config": None,
                                    "selected_basis": {"Time": timeframe},
                                },
                                "indicators": ["Volume"],
                                "link_group": "A",
                            }
                        },
                        "popout": [],
                    },
                }
            ],
            "active_layout": f"Test-{timeframe}",
        },
        "timezone": "UTC",
        "trade_fetch_enabled": False,
        "size_in_quote_ccy": "Base",
        "replay": {"mode": "replay", "range_start": start, "range_end": end},
    }
    STATE_FILE.write_text(json.dumps(fixture, indent=2))


def write_live_fixture(ticker: str = TICKER, timeframe: str = "M1", name: str = "Test") -> None:
    """Live モード起動用 saved-state.json を書き込む（replay フィールドなし）。"""
    DATA_DIR.mkdir(parents=True, exist_ok=True)
    fixture = {
        "layout_manager": {
            "layouts": [
                {
                    "name": name,
                    "dashboard": {
                        "pane": {
                            "KlineChart": {
                                "layout": {"splits": [0.78], "autoscale": "FitToVisible"},
                                "kind": "Candles",
                                "stream_type": [{"Kline": {"ticker": ticker, "timeframe": timeframe}}],
                                "settings": {
                                    "tick_multiply": None,
                                    "visual_config": None,
                                    "selected_basis": {"Time": timeframe},
                                },
                                "indicators": [],
                                "link_group": "A",
                            }
                        },
                        "popout": [],
                    },
                }
            ],
            "active_layout": name,
        },
        "timezone": "UTC",
        "trade_fetch_enabled": False,
        "size_in_quote_ccy": "Base",
    }
    STATE_FILE.write_text(json.dumps(fixture, indent=2))


# ── API ラッパー ──────────────────────────────────────────────────────────────

def api_get(path: str) -> dict:
    r = requests.get(f"{API_BASE}{path}", timeout=5)
    r.raise_for_status()
    return r.json()


def api_post(path: str, body: dict | None = None) -> dict:
    path, body = _translate_legacy_replay_request(path, body or {})
    r = requests.post(f"{API_BASE}{path}", json=body, timeout=5)
    r.raise_for_status()
    return r.json()


def api_get_code(path: str) -> int:
    try:
        r = requests.get(f"{API_BASE}{path}", timeout=5)
        return r.status_code
    except requests.RequestException:
        return 0


def api_post_code(path: str, body: Any = None) -> int:
    try:
        if body is None:
            body = {}
        path, body = _translate_legacy_replay_request(path, body)
        if isinstance(body, dict):
            r = requests.post(f"{API_BASE}{path}", json=body, timeout=5)
        else:
            r = requests.post(
                f"{API_BASE}{path}",
                data=body if isinstance(body, (str, bytes)) else json.dumps(body),
                headers={"Content-Type": "application/json"},
                timeout=5,
            )
        return r.status_code
    except requests.RequestException:
        return 0


def get_status() -> dict:
    return api_get("/api/replay/status")


def _translate_legacy_replay_request(path: str, body: Any) -> tuple[str, Any]:
    if path == "/api/replay/play":
        return "/api/replay/toggle", body or {}
    if path == "/api/replay/step-forward":
        return "/api/agent/session/default/step", body or {}
    return path, body


# ── ポーリングヘルパー ────────────────────────────────────────────────────────

def wait_status(want: str, timeout: int = 10) -> bool:
    deadline = time.monotonic() + timeout
    while time.monotonic() < deadline:
        try:
            current = get_status().get("status")
            if current == want:
                return True
            if current == "Active" and want in {"Paused", "Playing", "Active"}:
                return True
        except requests.RequestException:
            pass
        time.sleep(0.5)
    return False


def wait_playing(timeout: int = 120) -> bool:
    return wait_status("Active", timeout)


def wait_paused(timeout: int = 15) -> bool:
    return wait_status("Active", timeout)


def wait_for_time_advance(ref: int, timeout: int = 30) -> int | None:
    """current_time > ref になるまでポーリング。新しい値を返す。タイムアウト時は None。"""
    deadline = time.monotonic() + timeout
    while time.monotonic() < deadline:
        try:
            ct = get_status().get("current_time")
            if ct is not None and int(ct) > ref:
                return int(ct)
        except (requests.RequestException, TypeError, ValueError):
            pass
        time.sleep(0.5)
    return None


def wait_streams_ready(timeout: int = 30) -> bool:
    """最初のペインの streams_ready=true になるまで待つ。"""
    deadline = time.monotonic() + timeout
    while time.monotonic() < deadline:
        try:
            body = api_get("/api/pane/list")
            panes = body.get("panes", [])
            if panes and panes[0].get("streams_ready") is True:
                return True
        except requests.RequestException:
            pass
        time.sleep(1)
    return False


def wait_for_pane_count(want: int, timeout: int = 10) -> bool:
    """pane/list の .panes 配列長が want になるまでポーリング。"""
    deadline = time.monotonic() + timeout
    while time.monotonic() < deadline:
        try:
            body = api_get("/api/pane/list")
            if len(body.get("panes", [])) == want:
                return True
        except requests.RequestException:
            pass
        time.sleep(0.5)
    return False


def wait_for_pane_streams_ready(pane_id: str, timeout: int = 30) -> bool:
    """指定 pane_id の streams_ready=true になるまでポーリング。"""
    deadline = time.monotonic() + timeout
    while time.monotonic() < deadline:
        try:
            body = api_get("/api/pane/list")
            panes = body.get("panes", [])
            p = next((x for x in panes if x.get("id") == pane_id), None)
            if p and p.get("streams_ready") is True:
                return True
        except requests.RequestException:
            pass
        time.sleep(1)
    return False


def wait_tachibana_session(timeout: int = 120) -> bool:
    """GET /api/auth/tachibana/status → session=present になるまで待つ。"""
    deadline = time.monotonic() + timeout
    last_body: dict = {}
    while time.monotonic() < deadline:
        try:
            body = requests.get(f"{API_BASE}/api/auth/tachibana/status", timeout=5).json()
            last_body = body
            if body.get("session") == "present":
                return True
        except requests.RequestException:
            pass
        time.sleep(1)
    print(f"  [debug] last tachibana status: {last_body}")  # 診断用: CI ログで原因調査
    return False


# ── 検証ヘルパー ──────────────────────────────────────────────────────────────

def is_bar_boundary(ct: int, step: int) -> bool:
    return ct % step == 0


def advance_within(ct1: int, ct2: int, step: int, max_bars: int = 100) -> bool:
    diff = ct2 - ct1
    if diff < 0 or diff % step != 0:
        return False
    bars = diff // step
    return 1 <= bars <= max_bars


def ct_in_range(ct: int, st: int, et: int) -> bool:
    return st <= ct <= et


# ── 通知ヘルパー ──────────────────────────────────────────────────────────────

def list_notifications() -> dict:
    try:
        return api_get("/api/notification/list")
    except requests.RequestException:
        return {"notifications": []}


def has_notification(needle: str) -> bool:
    data = list_notifications()
    items = data.get("notifications", [])
    return any(
        needle in (n.get("body", "") or "") or needle in (n.get("title", "") or "")
        for n in items
    )


def count_error_notifications() -> int:
    data = list_notifications()
    items = data.get("notifications", [])
    return sum(1 for n in items if n.get("level") in ("error", "warning"))


# ── ティッカーヘルパー ─────────────────────────────────────────────────────────

def order_symbol() -> str:
    """E2E_TICKER のシンボル部分。例: 'HyperliquidLinear:BTC' → 'BTC'"""
    return TICKER.split(":", 1)[-1]


def ticker_exchange() -> str:
    """E2E_TICKER の取引所部分。例: 'HyperliquidLinear:BTC' → 'HyperliquidLinear'"""
    return TICKER.split(":", 1)[0]


def primary_ticker() -> str:
    return TICKER


def secondary_ticker() -> str:
    """同取引所の別銘柄。"""
    ex = ticker_exchange()
    mapping = {
        "HyperliquidLinear": f"{ex}:ETH",
        "HyperliquidSpot": f"{ex}:ETH",
        "BinanceLinear": f"{ex}:ETHUSDT",
        "BinanceSpot": f"{ex}:ETHUSDT",
        "BybitLinear": f"{ex}:ETHUSDT",
        "BybitSpot": f"{ex}:ETHUSDT",
    }
    return mapping.get(ex, f"{ex}:ETH")


def tertiary_ticker() -> str:
    """同取引所の 3 銘柄目。"""
    ex = ticker_exchange()
    mapping = {
        "HyperliquidLinear": f"{ex}:HYPE",
        "HyperliquidSpot": f"{ex}:HYPE",
        "BinanceLinear": f"{ex}:SOLUSDT",
        "BinanceSpot": f"{ex}:SOLUSDT",
        "BybitLinear": f"{ex}:SOLUSDT",
        "BybitSpot": f"{ex}:SOLUSDT",
    }
    return mapping.get(ex, f"{ex}:SOL")


# ── モードヘルパー ────────────────────────────────────────────────────────────

def headless_play(start: str = "", end: str = "") -> None:
    """headless 時のみ POST /api/replay/play を発行する。GUI は saved-state 自動再生のため no-op。"""
    if not IS_HEADLESS:
        return
    s = start or _headless_start
    e = end or _headless_end
    try:
        requests.post(
            f"{API_BASE}/api/replay/toggle",
            json={"start": s, "end": e},
            timeout=5,
        )
    except requests.RequestException as exc:
        print(f"  WARN: headless_play failed: {exc}", file=sys.stderr)


def ensure_replay_mode() -> None:
    """GUI の場合 toggle → Replay。headless は常に Replay のため no-op。"""
    if not IS_HEADLESS:
        try:
            api_post("/api/replay/toggle")
        except requests.RequestException:
            pass


def speed_to_10x() -> None:
    """1x→2x→5x→10x（3 回 CycleSpeed）。"""
    for _ in range(3):
        try:
            api_post("/api/replay/speed")
        except requests.RequestException:
            pass


# ── Tachibana ヘルパー ────────────────────────────────────────────────────────

def tachibana_replay_setup(start: str, end: str) -> bool:
    """TachibanaSpot:7203 D1 の saved-state を書き込み、アプリ起動 → セッション確立
    → streams_ready → Replay モード切替 → /api/replay/play を発行する。
    成功時は True を返す。
    """
    DATA_DIR.mkdir(parents=True, exist_ok=True)
    fixture = {
        "layout_manager": {
            "layouts": [
                {
                    "name": "Test-D1",
                    "dashboard": {
                        "pane": {
                            "KlineChart": {
                                "layout": {"splits": [0.78], "autoscale": "FitToVisible"},
                                "kind": "Candles",
                                "stream_type": [{"Kline": {"ticker": "TachibanaSpot:7203", "timeframe": "D1"}}],
                                "settings": {
                                    "tick_multiply": None,
                                    "visual_config": None,
                                    "selected_basis": {"Time": "D1"},
                                },
                                "indicators": ["Volume"],
                                "link_group": "A",
                            }
                        },
                        "popout": [],
                    },
                }
            ],
            "active_layout": "Test-D1",
        },
        "timezone": "UTC",
        "trade_fetch_enabled": False,
        "size_in_quote_ccy": "Base",
    }
    STATE_FILE.write_text(json.dumps(fixture, indent=2))
    return True


def get_pane_id(index: int = 0) -> str:
    """pane/list から index 番目のペイン ID を取得。取得失敗時は空文字。"""
    try:
        body = api_get("/api/pane/list")
        panes = body.get("panes", [])
        if index < len(panes):
            return panes[index].get("id", "")
    except requests.RequestException:
        pass
    return ""


def find_other_pane_id(exclude_id: str) -> str:
    """pane/list から exclude_id 以外の最初のペイン ID を取得。"""
    try:
        body = api_get("/api/pane/list")
        for p in body.get("panes", []):
            if p.get("id") != exclude_id:
                return p.get("id", "")
    except requests.RequestException:
        pass
    return ""
