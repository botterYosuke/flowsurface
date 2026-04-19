"""動作確認スクリプト: headless / GUI 両モードの起動テスト。"""
import os
import subprocess
import sys
import time

import requests

BINARY = os.path.join(
    os.path.dirname(__file__), "target", "release", "flowsurface.exe"
)
API_BASE = "http://127.0.0.1:9876"
STARTUP_TIMEOUT = 30
TICKER = "HyperliquidLinear:BTC"
TIMEFRAME = "M1"


def wait_for_api(timeout: int = STARTUP_TIMEOUT) -> bool:
    deadline = time.monotonic() + timeout
    while time.monotonic() < deadline:
        try:
            r = requests.get(f"{API_BASE}/api/replay/status", timeout=2)
            if r.status_code == 200:
                return True
        except requests.exceptions.RequestException:
            pass
        time.sleep(0.3)
    return False


def test_mode(label: str, headless: bool) -> bool:
    print(f"\n{'='*50}")
    print(f"[{label}] 起動中...")

    cmd = [BINARY]
    if headless:
        cmd.append("--headless")
    cmd += ["--ticker", TICKER, "--timeframe", TIMEFRAME]

    env = os.environ.copy()
    env["DEV_IS_DEMO"] = "true"

    proc = subprocess.Popen(
        cmd,
        env=env,
        stdout=subprocess.PIPE,
        stderr=subprocess.PIPE,
    )

    print(f"[{label}] PID={proc.pid}, API 待機中...")
    ok = wait_for_api()

    if not ok:
        print(f"[{label}] [NG] API が {STARTUP_TIMEOUT}s 以内に応答しなかった")
        proc.terminate()
        proc.wait(timeout=5)
        return False

    print(f"[{label}] [OK] API 応答あり")

    # ステータス取得
    try:
        status = requests.get(f"{API_BASE}/api/replay/status", timeout=3).json()
        print(f"[{label}] status={status}")
    except Exception as e:
        print(f"[{label}] status 取得失敗: {e}")

    # ポートフォリオ取得
    try:
        portfolio = requests.get(f"{API_BASE}/api/replay/portfolio", timeout=3).json()
        print(f"[{label}] portfolio={portfolio}")
    except Exception as e:
        print(f"[{label}] portfolio 取得失敗: {e}")

    print(f"[{label}] プロセス終了...")
    proc.terminate()
    try:
        proc.wait(timeout=5)
    except subprocess.TimeoutExpired:
        proc.kill()
        proc.wait()

    print(f"[{label}] [OK] 完了")
    return True


def main():
    if not os.path.exists(BINARY):
        print(f"バイナリが見つかりません: {BINARY}")
        sys.exit(1)

    results = {}
    results["headless"] = test_mode("HEADLESS", headless=True)
    # GUI モードはウィンドウが開くので少し待つ
    time.sleep(1)
    results["gui"] = test_mode("GUI", headless=False)

    print(f"\n{'='*50}")
    print("結果:")
    for mode, ok in results.items():
        mark = "[OK]" if ok else "[NG]"
        print(f"  {mark} {mode}")

    if not all(results.values()):
        sys.exit(1)


if __name__ == "__main__":
    main()
