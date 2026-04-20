#!/usr/bin/env python3
"""s50_tachibana_login.py — 立花証券 (Tachibana) への DEV_USER_ID を使ったログインE2Eテスト

検証シナリオ:
  1: DEV_IS_DEMO=true, DEV_USER_ID / DEV_PASSWORD を環境変数として設定してアプリ起動
  2: Tachibana セッション確立（自動ログイン）を待機
  3: セッションが確立されたら成功
"""
from __future__ import annotations

import os
import sys
from pathlib import Path

# `.env` ファイルを読み込んで os.environ にセット
_env_path = Path(__file__).parent.parent.parent / ".env"
if _env_path.exists():
    with open(_env_path, encoding="utf-8") as _f:
        for _line in _f:
            _line = _line.strip()
            if _line and not _line.startswith('#') and '=' in _line:
                _k, _v = _line.split('=', 1)
                os.environ.setdefault(_k.strip(), _v.strip())

# helpers をロード
sys.path.insert(0, str(Path(__file__).parent))
from helpers import (
    FlowsurfaceEnv,
    backup_state,
    restore_state,
    write_live_fixture,
    wait_tachibana_session,
    pass_,
    fail,
    pend,
    print_summary,
)

def _check_env() -> str | None:
    if os.environ.get("DEV_IS_DEMO", "") != "true":
        return "DEV_IS_DEMO=true を設定してください（本番誤発注防止）"
    if not os.environ.get("DEV_USER_ID") or not os.environ.get("DEV_PASSWORD"):
        return "creds_missing"
    return None

def run_s50(env: FlowsurfaceEnv) -> None:
    print("=== Tachibana (立花証券) ログイン E2E テスト ===")
    print()
    print("── Step 1: Tachibana デモセッション待機")
    if wait_tachibana_session(120):
        pass_("Step 1: デモセッション確立(DEV_USER_IDでのログイン成功)")
    else:
        fail("Step 1", "Tachibana セッション未確立 (120秒タイムアウト)")

def test_s50_tachibana_login() -> None:
    guard = _check_env()
    if guard == "DEV_IS_DEMO=true を設定してください（本番誤発注防止）":
        import pytest
        pytest.skip(guard)
    if guard == "creds_missing":
        pend("TC-Login", "DEV_USER_ID / DEV_PASSWORD が未設定")
        return
    backup_state()
    # ライブモードで起動して、Tachibanaのセッション確立を待つ
    write_live_fixture("TachibanaSpot:7203", "D1", "Toyota-Live")
    env = FlowsurfaceEnv(ticker="TachibanaSpot:7203", timeframe="D1", headless=False)
    
    try:
        env._start_process()
        run_s50(env)
    finally:
        env.close()
        restore_state()

def main() -> None:
    guard = _check_env()
    if guard == "DEV_IS_DEMO=true を設定してください（本番誤発注防止）":
        print(f"ERROR: {guard}")
        sys.exit(1)
    if guard == "creds_missing":
        pend("TC-Login", "DEV_USER_ID / DEV_PASSWORD が未設定")
        print_summary()
        sys.exit(0)
        
    test_s50_tachibana_login()
    print_summary()
    
    import helpers as _h
    sys.exit(0 if _h._FAIL == 0 else 1)

if __name__ == "__main__":
    main()
