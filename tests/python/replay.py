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
    from datetime import datetime

    mo.md("# flowsurface Replay API 疎通確認")
    return (mo, httpx, json, datetime)


@app.cell
def _(mo):
    mo.md("## 接続設定")
    return


@app.cell
def _(mo):
    api_url = mo.ui.text(value="http://127.0.0.1:9876", label="API URL: ")
    return (api_url,)


@app.cell
def _(mo, httpx, api_url):
    check_button = mo.ui.button(label="✓ 接続確認")

    _status = None
    if check_button.clicked:
        try:
            resp = httpx.get(f"{api_url.value}/api/replay/status", timeout=2.0)
            _status = f"✅ 接続成功（status={resp.status_code}）"
        except Exception as e:
            _status = f"❌ 接続失敗: {e}"

    status_text = mo.md(f"_状態: {_status}_" if _status else "接続確認前")

    mo.vstack([check_button, status_text])
    return (check_button, status_text)


@app.cell
def _(mo, check_button, api_url, httpx):
    if not check_button.clicked:
        mo.md("接続確認ボタンを押してください")
        return (None,)

    try:
        resp = httpx.get(f"{api_url.value}/api/replay/status", timeout=2.0)
        is_connected = resp.status_code == 200
    except:
        is_connected = False

    if not is_connected:
        mo.md("❌ API に接続できません")
        return (None,)

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
        label="エンドポイント選択:"
    )

    endpoint_labels = {f"{path} — {desc}": (path, desc) for path, desc in endpoints_list}

    mo.vstack([
        mo.md("## エンドポイント選択"),
        endpoint_dropdown,
    ])
    return (endpoint_dropdown, endpoint_labels)


@app.cell
def _(mo, endpoint_dropdown, endpoint_labels):
    if endpoint_dropdown.value is None:
        return (None, None, None)

    path, desc = endpoint_labels[endpoint_dropdown.value]
    method = "GET" if path.startswith("GET") else "POST"
    path_only = path.split(" ", 1)[1]

    mo.md(f"### 選択: {path}")
    return (path_only, method, desc)


@app.cell
def _(mo, path_only, method):
    if path_only is None:
        return (None,)

    # POST エンドポイントの場合のみパラメータフォーム
    if method == "POST":
        if "replay/play" in path_only:
            form_start = mo.ui.text(label="start: ", value="2026-04-21 09:00")
            form_end = mo.ui.text(label="end: ", value="2026-04-21 15:00")
            params_form = mo.ui.form(mo.vstack([form_start, form_end]))

        elif "replay/order" in path_only:
            form_ticker = mo.ui.text(label="ticker: ", value="BinanceLinear:BTCUSDT")
            form_side = mo.ui.dropdown(options=["buy", "sell"], label="side: ")
            form_qty = mo.ui.number(label="qty: ", value=0.01)
            params_form = mo.ui.form(mo.vstack([form_ticker, form_side, form_qty]))

        elif "pane/set-ticker" in path_only:
            form_pane_id = mo.ui.text(label="pane_id: ", value="00000000-0000-0000-0000-000000000001")
            form_ticker = mo.ui.text(label="ticker: ", value="BinanceLinear:BTCUSDT")
            params_form = mo.ui.form(mo.vstack([form_pane_id, form_ticker]))

        else:
            params_form = None

        if params_form:
            mo.md("### パラメータ")
            return (params_form,)
    else:
        return (None,)


@app.cell
def _(mo, path_only, method, params_form, api_url, httpx, json):
    if path_only is None:
        return

    request_button = mo.ui.button(label="📤 リクエスト送信")

    _response = None
    if request_button.clicked:
        try:
            url = f"{api_url.value}{path_only}"

            if method == "GET":
                resp = httpx.get(url, timeout=5.0)
            else:  # POST
                body = {}
                if params_form:
                    form_val = params_form.value
                    if isinstance(form_val, tuple) and len(form_val) == 2:
                        body = {
                            "start": form_val[0].value,
                            "end": form_val[1].value,
                        }
                    elif isinstance(form_val, tuple) and len(form_val) == 3:
                        body = {
                            "ticker": form_val[0].value,
                            "side": form_val[1].value,
                            "qty": form_val[2].value,
                        }
                    elif isinstance(form_val, tuple) and len(form_val) == 2:
                        body = {
                            "pane_id": form_val[0].value,
                            "ticker": form_val[1].value,
                        }

                resp = httpx.post(url, json=body, timeout=5.0)

            _response = {
                "status": resp.status_code,
                "body": resp.text,
            }
            try:
                _response["json"] = resp.json()
            except:
                pass
        except Exception as e:
            _response = {"error": str(e)}

    if _response:
        if "error" in _response:
            result = mo.md(f"❌ エラー: {_response['error']}")
        else:
            status = _response["status"]
            status_icon = "✅" if 200 <= status < 300 else "⚠️"
            resp_text = json.dumps(
                _response.get("json") or _response["body"],
                indent=2,
                ensure_ascii=False
            )
            result = mo.md(
                f"""
### レスポンス {status_icon}

**Status**: {status}

```json
{resp_text}
```
                """
            )
    else:
        result = None

    mo.vstack([request_button, result] if result else [request_button])
    return


if __name__ == "__main__":
    app.run()
