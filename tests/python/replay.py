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
    return (mo, httpx, json)


@app.cell
def _(mo):
    mo.md("## 接続設定")
    return


@app.cell
def _(mo):
    api_url = mo.ui.text(value="http://127.0.0.1:9876", label="API URL: ")
    api_url
    return (api_url,)


@app.cell
def _(mo):
    check_button = mo.ui.button(value=0, label="✓ 接続確認", on_click=lambda v: v + 1)
    check_button
    return (check_button,)


@app.cell
def _(mo, httpx, api_url, check_button):
    is_connected = False
    if check_button.value:
        try:
            _resp = httpx.get(f"{api_url.value}/api/replay/status", timeout=2.0)
            is_connected = _resp.status_code == 200
            _label = f"✅ 接続成功（status={_resp.status_code}）"
        except Exception as _e:
            _label = f"❌ 接続失敗: {_e}"
    else:
        _label = "接続確認前"
    mo.md(f"_状態: {_label}_")
    return (is_connected,)


@app.cell
def _(mo, is_connected):
    endpoints_list = [
        ("GET /api/replay/status", "リプレイ状態取得"),
        ("POST /api/replay/toggle", "リプレイ開始/停止"),
        ("POST /api/replay/pause", "一時停止"),
        ("POST /api/replay/resume", "再開"),
        ("POST /api/replay/step-forward", "次へ"),
        ("POST /api/replay/step-backward", "前へ"),
        ("POST /api/replay/speed", "速度変更"),
        ("GET /api/replay/portfolio", "ポートフォリオ"),
        ("GET /api/replay/orders", "注文一覧"),
        ("POST /api/replay/order", "注文登録"),
        ("GET /api/pane/list", "ペイン一覧"),
        ("POST /api/pane/set-ticker", "銘柄変更"),
        ("GET /api/auth/tachibana/status", "セッション確認"),
        ("GET /api/tachibana/orders", "注文一覧（立花）"),
        ("GET /api/tachibana/holdings", "保有株数"),
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
    return (endpoint_dropdown, endpoint_labels)


@app.cell
def _(endpoint_dropdown, endpoint_labels):
    if endpoint_dropdown.value is None:
        path_only = None
        method = None
    else:
        _path, _desc = endpoint_labels[endpoint_dropdown.value]
        method = "GET" if _path.startswith("GET") else "POST"
        path_only = _path.split(" ", 1)[1]
    return (path_only, method)


@app.cell
def _(mo, path_only, method):
    params_form = None
    if path_only is not None and method == "POST":
        if "replay/order" in path_only:
            params_form = mo.ui.dictionary({
                "ticker": mo.ui.text(label="ticker", value="BinanceLinear:BTCUSDT"),
                "side": mo.ui.dropdown(options=["buy", "sell"], label="side", value="buy"),
                "qty": mo.ui.number(label="qty", start=0.001, stop=1000.0, step=0.001, value=0.01),
            })
        elif "pane/set-ticker" in path_only:
            params_form = mo.ui.dictionary({
                "pane_id": mo.ui.text(label="pane_id", value="00000000-0000-0000-0000-000000000001"),
                "ticker": mo.ui.text(label="ticker", value="BinanceLinear:BTCUSDT"),
            })
        elif "replay/speed" in path_only:
            params_form = mo.ui.dictionary({
                "speed": mo.ui.number(label="speed", start=0.1, stop=100.0, step=0.5, value=1.0),
            })

    _content = mo.vstack([mo.md("### パラメータ"), params_form]) if params_form else mo.md("")
    _content
    return (params_form,)


@app.cell
def _(mo):
    request_button = mo.ui.button(value=0, label="📤 リクエスト送信", on_click=lambda v: v + 1)
    request_button
    return (request_button,)


@app.cell
def _(mo, path_only, method, params_form, api_url, httpx, json, request_button):
    if path_only is None or not request_button.value:
        _result = mo.md("")
    else:
        try:
            _url = f"{api_url.value}{path_only}"
            if method == "GET":
                _resp = httpx.get(_url, timeout=5.0)
            else:
                _body = dict(params_form.value) if params_form else {}
                _resp = httpx.post(_url, json=_body, timeout=5.0)

            _code = _resp.status_code
            _icon = "✅" if 200 <= _code < 300 else "⚠️"
            try:
                _text = json.dumps(_resp.json(), indent=2, ensure_ascii=False)
            except Exception:
                _text = _resp.text
            _result = mo.md(f"### レスポンス {_icon}\n\n**Status**: {_code}\n\n```json\n{_text}\n```")
        except Exception as _e:
            _result = mo.md(f"❌ エラー: {_e}")

    _result
    return


if __name__ == "__main__":
    app.run()
