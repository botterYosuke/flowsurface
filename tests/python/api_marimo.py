# /// script
# requires-python = ">=3.10"
# dependencies = [
#     "marimo",
#     "httpx",
# ]
# ///

import marimo

__generated_with = "0.23.1"
app = marimo.App()


@app.cell
def _():
    import marimo as mo
    import httpx
    import json
    mo.md("# flowsurface Replay API 疎通確認")
    return httpx, json, mo


@app.cell
def _(mo):
    mo.md("""
    ## 接続設定
    """)
    return


@app.cell
def _(mo):
    api_url = mo.ui.text(value="http://127.0.0.1:9876", label="API URL: ")
    api_url
    return (api_url,)


@app.cell
def _(mo):
    get_conn, set_conn = mo.state({"connected": False, "label": "接続確認前"}, allow_self_loops=True)
    return get_conn, set_conn


@app.cell
def _(api_url, get_conn, httpx, mo, set_conn):
    def _on_click(v):
        try:
            _resp = httpx.get(f"{api_url.value}/api/replay/status", timeout=2.0)
            _ok = _resp.status_code == 200
            set_conn({
                "connected": _ok,
                "label": f"✅ 接続成功（status={_resp.status_code}）" if _ok else f"❌ 接続失敗: status={_resp.status_code}",
            })
        except Exception as _e:
            set_conn({"connected": False, "label": f"❌ 接続失敗: {_e}"})
        return v + 1

    check_button = mo.ui.button(value=0, label="✓ 接続確認", on_click=_on_click)
    mo.vstack([check_button, mo.md(f"_状態: {get_conn()['label']}_")])
    return


@app.cell
def _(get_conn, mo):
    is_connected = get_conn()["connected"]
    endpoints_list = [
        # ── Replay 制御 ──────────────────────────────────────────────
        ("GET /api/replay/status", "リプレイ状態取得"),
        ("POST /api/replay/toggle", "リプレイ開始/停止"),
        ("POST /api/replay/play", "期間指定リプレイ開始"),
        ("POST /api/replay/pause", "一時停止"),
        ("POST /api/replay/resume", "再開"),
        ("POST /api/replay/step-forward", "次へ"),
        ("POST /api/replay/step-backward", "前へ"),
        ("POST /api/replay/speed", "速度変更"),
        # ── 仮想約定エンジン ─────────────────────────────────────────
        ("GET /api/replay/state", "仮想取引所の状態取得"),
        ("GET /api/replay/portfolio", "ポートフォリオ"),
        ("GET /api/replay/orders", "注文一覧（仮想）"),
        ("POST /api/replay/order", "注文登録（仮想）"),
        # ── App 制御 ─────────────────────────────────────────────────
        ("POST /api/app/save", "状態保存"),
        ("POST /api/app/screenshot", "スクリーンショット"),
        # ── 認証 ─────────────────────────────────────────────────────
        ("GET /api/auth/tachibana/status", "セッション確認"),
        ("POST /api/auth/tachibana/logout", "ログアウト"),
        # ── ペイン CRUD ───────────────────────────────────────────────
        ("GET /api/pane/list", "ペイン一覧"),
        ("GET /api/pane/chart-snapshot", "チャートスナップショット"),
        ("POST /api/pane/split", "ペイン分割"),
        ("POST /api/pane/close", "ペイン閉じる"),
        ("POST /api/pane/set-ticker", "銘柄変更"),
        ("POST /api/pane/set-timeframe", "時間足変更"),
        # ── その他 ───────────────────────────────────────────────────
        ("GET /api/notification/list", "通知一覧"),
        ("POST /api/sidebar/select-ticker", "サイドバー銘柄選択"),
        ("POST /api/sidebar/open-order-pane", "注文ペイン開く"),
        # ── 立花証券 ─────────────────────────────────────────────────
        ("GET /api/buying-power", "買付余力取得"),
        ("POST /api/tachibana/order", "注文登録（立花）"),
        ("GET /api/tachibana/orders", "注文一覧（立花）"),
        ("GET /api/tachibana/order/{id}", "注文詳細（立花）"),
        ("POST /api/tachibana/order/correct", "注文訂正（立花）"),
        ("POST /api/tachibana/order/cancel", "注文キャンセル（立花）"),
        ("GET /api/tachibana/holdings", "保有株数"),
        # ── テスト専用（debug ビルドのみ）────────────────────────────
        ("POST /api/test/tachibana/delete-persisted-session", "セッション削除（テスト用）"),
    ]
    endpoint_dropdown = mo.ui.dropdown(
        options=[f"{path} — {desc}" for path, desc in endpoints_list],
        label="エンドポイント選択:",
    )
    endpoint_labels = {f"{path} — {desc}": (path, desc) for path, desc in endpoints_list}

    _content = mo.vstack([
        mo.md("## エンドポイント選択"),
        endpoint_dropdown,
    ]) if is_connected else mo.md("")
    _content
    return endpoint_dropdown, endpoint_labels


@app.cell
def _(endpoint_dropdown, endpoint_labels):
    if endpoint_dropdown.value is None:
        path_only = None
        method = None
    else:
        _path, _desc = endpoint_labels[endpoint_dropdown.value]
        method = "GET" if _path.startswith("GET") else "POST"
        path_only = _path.split(" ", 1)[1]
    return method, path_only


@app.cell
def _(method, mo, path_only):
    params_form = None
    if path_only is not None:
        if method == "GET" and "chart-snapshot" in path_only:
            params_form = mo.ui.dictionary({
                "pane_id": mo.ui.text(label="pane_id", value="00000000-0000-0000-0000-000000000001"),
            })
        elif method == "GET" and "/api/tachibana/order/{id}" in path_only:
            params_form = mo.ui.dictionary({
                "order_num": mo.ui.text(label="order_num", value=""),
                "eig_day": mo.ui.text(label="eig_day (YYYYMMDD, 省略可)", value=""),
            })
        elif method == "GET" and "tachibana/orders" in path_only:
            params_form = mo.ui.dictionary({
                "eig_day": mo.ui.text(label="eig_day (YYYYMMDD, 省略可)", value=""),
            })
        elif method == "GET" and "tachibana/holdings" in path_only:
            params_form = mo.ui.dictionary({
                "issue_code": mo.ui.text(label="issue_code", value="7203"),
            })
        elif method == "POST":
            if "replay/play" in path_only:
                params_form = mo.ui.dictionary({
                    "start": mo.ui.text(label="start (YYYY-MM-DD HH:MM:SS)", value="2024-01-01 09:00:00"),
                    "end": mo.ui.text(label="end (YYYY-MM-DD HH:MM:SS)", value="2024-01-01 15:30:00"),
                })
            elif "replay/order" in path_only:
                params_form = mo.ui.dictionary({
                    "ticker": mo.ui.text(label="ticker", value="BinanceLinear:BTCUSDT"),
                    "side": mo.ui.dropdown(options=["buy", "sell"], label="side", value="buy"),
                    "qty": mo.ui.number(label="qty", start=0.001, stop=1000.0, step=0.001, value=0.01),
                })
            elif "replay/speed" in path_only:
                params_form = mo.ui.dictionary({
                    "speed": mo.ui.number(label="speed", start=0.1, stop=100.0, step=0.5, value=1.0),
                })
            elif "pane/split" in path_only:
                params_form = mo.ui.dictionary({
                    "pane_id": mo.ui.text(label="pane_id", value="00000000-0000-0000-0000-000000000001"),
                    "axis": mo.ui.dropdown(options=["Vertical", "Horizontal"], label="axis", value="Vertical"),
                })
            elif "pane/close" in path_only:
                params_form = mo.ui.dictionary({
                    "pane_id": mo.ui.text(label="pane_id", value="00000000-0000-0000-0000-000000000001"),
                })
            elif "pane/set-ticker" in path_only:
                params_form = mo.ui.dictionary({
                    "pane_id": mo.ui.text(label="pane_id", value="00000000-0000-0000-0000-000000000001"),
                    "ticker": mo.ui.text(label="ticker", value="BinanceLinear:BTCUSDT"),
                })
            elif "pane/set-timeframe" in path_only:
                params_form = mo.ui.dictionary({
                    "pane_id": mo.ui.text(label="pane_id", value="00000000-0000-0000-0000-000000000001"),
                    "timeframe": mo.ui.text(label="timeframe", value="1m"),
                })
            elif "sidebar/select-ticker" in path_only:
                params_form = mo.ui.dictionary({
                    "pane_id": mo.ui.text(label="pane_id", value="00000000-0000-0000-0000-000000000001"),
                    "ticker": mo.ui.text(label="ticker", value="BinanceLinear:BTCUSDT"),
                    "kind": mo.ui.text(label="kind (省略可)", value=""),
                })
            elif "sidebar/open-order-pane" in path_only:
                params_form = mo.ui.dictionary({
                    "kind": mo.ui.dropdown(options=["OrderEntry", "OrderList", "BuyingPower"], label="kind", value="OrderEntry"),
                })
            elif path_only == "/api/tachibana/order" or ("tachibana/order" in path_only and "correct" not in path_only and "cancel" not in path_only):
                params_form = mo.ui.dictionary({
                    "issue_code": mo.ui.text(label="issue_code", value="7203"),
                    "qty": mo.ui.text(label="qty", value="100"),
                    "side": mo.ui.dropdown(options=["buy", "sell"], label="side", value="buy"),
                    "price": mo.ui.text(label="price (0=成行)", value="0"),
                    "account_type": mo.ui.text(label="account_type (省略可, 既定=1)", value=""),
                    "market_code": mo.ui.text(label="market_code (省略可, 既定=00)", value=""),
                    "condition": mo.ui.text(label="condition (省略可, 既定=0)", value=""),
                    "cash_margin": mo.ui.text(label="cash_margin (省略可, 既定=0)", value=""),
                    "expire_day": mo.ui.text(label="expire_day (省略可, 既定=0)", value=""),
                    "second_password": mo.ui.text(label="second_password", value=""),
                })
            elif "tachibana/order/correct" in path_only:
                params_form = mo.ui.dictionary({
                    "order_number": mo.ui.text(label="order_number", value=""),
                    "eig_day": mo.ui.text(label="eig_day (YYYYMMDD)", value=""),
                    "condition": mo.ui.text(label="condition (省略可, 既定=*)", value=""),
                    "price": mo.ui.text(label="price (省略可, 既定=*)", value=""),
                    "qty": mo.ui.text(label="qty (省略可, 既定=*)", value=""),
                    "expire_day": mo.ui.text(label="expire_day (省略可, 既定=*)", value=""),
                    "second_password": mo.ui.text(label="second_password", value=""),
                })
            elif "tachibana/order/cancel" in path_only:
                params_form = mo.ui.dictionary({
                    "order_number": mo.ui.text(label="order_number", value=""),
                    "eig_day": mo.ui.text(label="eig_day (YYYYMMDD)", value=""),
                    "second_password": mo.ui.text(label="second_password", value=""),
                })

    _content = mo.vstack([mo.md("### パラメータ"), params_form]) if params_form else mo.md("")
    _content
    return (params_form,)


@app.cell
def _(mo):
    get_resp, set_resp = mo.state({"label": ""}, allow_self_loops=True)
    return get_resp, set_resp


@app.cell
def _(
    api_url,
    get_resp,
    httpx,
    json,
    method,
    mo,
    params_form,
    path_only,
    set_resp,
):
    def _on_click(v):
        if path_only is None:
            set_resp({"label": ""})
            return v + 1
        try:
            _params = dict(params_form.value) if params_form else {}
            if method == "GET" and "chart-snapshot" in path_only:
                _url = f"{api_url.value}/api/pane/chart-snapshot?pane_id={_params.get('pane_id', '')}"
                _resp = httpx.get(_url, timeout=5.0)
            elif method == "GET" and "/api/tachibana/order/{id}" in path_only:
                _oid = _params.get("order_num", "")
                _day = _params.get("eig_day", "")
                _url = f"{api_url.value}/api/tachibana/order/{_oid}"
                if _day:
                    _url += f"?eig_day={_day}"
                _resp = httpx.get(_url, timeout=5.0)
            elif method == "GET" and "tachibana/orders" in path_only:
                _day = _params.get("eig_day", "")
                _url = f"{api_url.value}/api/tachibana/orders"
                if _day:
                    _url += f"?eig_day={_day}"
                _resp = httpx.get(_url, timeout=5.0)
            elif method == "GET" and "tachibana/holdings" in path_only:
                _code = _params.get("issue_code", "")
                _url = f"{api_url.value}/api/tachibana/holdings?issue_code={_code}"
                _resp = httpx.get(_url, timeout=5.0)
            elif method == "GET":
                _url = f"{api_url.value}{path_only}"
                _resp = httpx.get(_url, timeout=5.0)
            else:
                _body = {k: v for k, v in _params.items() if v != ""}
                _url = f"{api_url.value}{path_only}"
                _resp = httpx.post(_url, json=_body, timeout=5.0)

            _code = _resp.status_code
            _icon = "✅" if 200 <= _code < 300 else "⚠️"
            try:
                _text = json.dumps(_resp.json(), indent=2, ensure_ascii=False)
            except Exception:
                _text = _resp.text
            set_resp({"label": f"### レスポンス {_icon}\n\n**Status**: {_code}\n\n```json\n{_text}\n```"})
        except Exception as _e:
            set_resp({"label": f"❌ エラー: {_e}"})
        return v + 1

    request_button = mo.ui.button(value=0, label="📤 リクエスト送信", on_click=_on_click)
    mo.vstack([request_button, mo.md(get_resp()["label"])])
    return


if __name__ == "__main__":
    app.run()
