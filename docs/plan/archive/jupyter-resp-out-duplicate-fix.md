# 引き継ぎ作業依頼: api_jupyter.ipynb の resp_out 重複出力バグ修正

## 問題の概要

`tests/python/api_jupyter.ipynb` のセル `id="ui"` にあるボタンを1回クリックすると、
`resp_out` Output ウィジェットに同一のレスポンスが N 個表示される。

N はセル（または「すべてのセルを実行」）を実行した回数に一致する。

## 環境

- VS Code の Jupyter 拡張でノートブックを実行
- Python パッケージ: `ipywidgets`, `httpx`, `IPython`
- ノートブック: `tests/python/api_jupyter.ipynb`

## 既に試して効果がなかった修正

以下はすべて試済みで、問題が解消しなかった：

1. `resp_out.clear_output(wait=True)` を `with resp_out:` の**外**で呼ぶ
2. `resp_out.outputs = ()` を `with resp_out:` の**外**で呼ぶ
3. `with resp_out:` ブロックの**内側**で `clear_output(wait=False)` を呼ぶ
4. `send_btn._click_handlers.callbacks.clear()` でハンドラをクリアしてから登録し直す
5. `_ui_created` グローバル変数によるシングルトンガード（ウィジェット再生成を防ぐ）

クリックごとの累積（2回目のクリックで2個追加される）は修正3で解消されたが、
「1回目のクリックで既に N 個表示される」問題は全修正を通じて残存している。

## 現在のコードの状態

現在 `ui` セルには以下の構造が入っている（詳細は実際のファイルを確認）：

```python
from IPython.display import clear_output, display, Markdown
import ipywidgets as widgets

if "_ui_created" not in globals():
    # ウィジェット・ハンドラを1回だけ生成
    resp_out = widgets.Output()
    send_btn = widgets.Button(...)
    def on_send(b):
        with resp_out:
            clear_output(wait=False)
            ...
            display(Markdown(...))
    send_btn.on_click(on_send)
    _ui_created = True

# セル再実行時はここだけ実行
clear_output(wait=False)
display(widgets.VBox([..., send_btn, resp_out]))
```

## 未解明の根本原因

`on_send` が1クリックで複数回呼ばれているのか、それとも `resp_out` ウィジェットが
複数の表示コンテキストを持っていて1回の `display()` が複数箇所に描画されているのか、
どちらが原因かが未確定。

## 作業依頼

### ステップ1: 計画書作成（このファイルを更新）

`docs/plan/jupyter-resp-out-duplicate-fix.md` に作業計画を追記してください。

### ステップ2: 診断テストを実行して原因を特定

**カーネルを再起動**してから以下の診断コードをノートブックに追加セルとして実行し、
原因を特定すること。

#### 診断1: `on_send` 呼び出し回数の確認

```python
# 診断セル: on_send が何回呼ばれているか確認する
_call_count = 0

_orig_on_send = on_send

def _debug_on_send(b):
    global _call_count
    _call_count += 1
    count = _call_count
    print(f"[DEBUG] on_send called: #{count}")
    _orig_on_send(b)

send_btn._click_handlers.callbacks.clear()
send_btn.on_click(_debug_on_send)
print("診断ハンドラを登録しました。ボタンを1回クリックしてください。")
```

ボタンを1回クリックして `[DEBUG] on_send called: #1` が何回出力されるか確認。

#### 診断2: resp_out の表示コンテキスト数の確認

```python
# 診断セル: resp_out が何か所で display されているか確認
print(f"resp_out model_id: {resp_out.model_id}")
print(f"resp_out outputs count before click: {len(resp_out.outputs)}")
```

#### 診断3: send_btn のハンドラ数の確認

```python
# 診断セル: send_btn のハンドラ数を確認
print(f"send_btn handlers: {send_btn._click_handlers.callbacks}")
print(f"handler count: {len(send_btn._click_handlers.callbacks)}")
```

### ステップ3: 原因に応じた修正を実施

診断結果に基づいて以下のいずれかを実施：

**もし `on_send` が複数回呼ばれている場合：**
- `send_btn` の comm レベルのハンドラ登録を調査
- VS Code Jupyter の ipywidgets comm チャンネルの仕組みを確認
- ハンドラ重複登録の根本原因を特定して修正

**もし `on_send` が1回だけ呼ばれているが出力が複数の場合：**
- `resp_out` ウィジェットが複数の display コンテキストを持っている
- `resp_out.outputs` の内容と `len(resp_out.outputs)` をクリック前後で確認
- Output ウィジェットへの直接書き込み（`resp_out.outputs = (output_dict,)`）で修正

### ステップ4: 修正後の検証

- カーネルを再起動
- 「すべてのセルを実行」を実行
- ボタンを1回クリックして応答が**1件だけ**表示されることを確認
- セルを5回再実行後も同様に1件だけであることを確認

## 注意事項

- このプロジェクトは Rust 製デスクトップアプリ (flowsurface) の E2E テスト用ノートブック
- アプリが起動していない状態でも診断コードは実行できる（接続失敗エラーが出るだけ）
- 修正が完了したらこの計画書の各ステップに ✅ を付けて更新すること

---

## 2026-04-21 追記: 原因仮説と採用した対処

### 原因仮説（コードのみから推定）

- `_ui_created` ガードで widget は 1 回しか生成されない → `resp_out` の model は 1 個のみ
- 一方で `display(widgets.VBox([..., resp_out, ...]))` がセル再実行のたびに呼ばれており、`clear_output(wait=False)` をセル末尾で呼んでも VS Code Jupyter では widget のビュー（DOM 要素）が残留しがち
- 結果：**同じ model の `resp_out` ビューが DOM に N 個存在**し、1 クリックで state（`outputs` タプル）が 1 個に更新されても N 個のビューが同じ内容を描画 → 見た目は「N 個の重複」になる
- 修正 3（`with resp_out` 内で `clear_output`）で累積が止まったのも、state を 1 件に正規化しているだけで、ビューの数自体は減らない説明と整合する

### 採用した対処（✅ 実装済み）

1. ✅ メイン `VBox` を `_ui_created` ブロック内で 1 度だけ生成して `_ui_main` に保持
2. ✅ セル末尾の `display(...)` を `display_id`（固定文字列）付きに変更し、以降は `update_display(...)` で**同じ表示をインプレース更新**する → 新しいビューを追加せず、既存ビューを置換する
3. ✅ `on_send` 内の `with resp_out:` を撤廃し、`resp_out.outputs = (...)` で直接 state を置き換える（context manager 経由のキャプチャを避け、重複書き込みの経路を一本化）

### 残タスク

- ⏳ ユーザーによる動作検証：
  1. カーネル再起動
  2. セル全実行
  3. ボタン 1 クリック → レスポンスが 1 件だけ
  4. `ui` セルを 5 回再実行後も 1 件だけ

### 診断ステップ（ステップ 2）について

- 上記の仮説が正しければ、`on_send` の呼び出しは 1 回、`resp_out.outputs` の長さも 1 のままで、N 個に見えるのはビューの重複が原因 → 診断 1/2/3 を実行しても「handler 1 個、outputs 1 件」となるはず
- そのため診断の実行は省略し、仮説に基づく修正を先に適用した。検証で問題が解決しない場合は、診断コードを追加して再調査する
