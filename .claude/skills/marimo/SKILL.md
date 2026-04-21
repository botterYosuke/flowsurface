---
name: marimo
description: marimo ノートブック (.py) を作成・編集するためのガイドライン。リアクティブなデータフロー・UI 要素・マークダウン・レイアウト・SQL・ファイルフォーマット。公式ドキュメント (https://docs.marimo.io/) と `.claude/skills/marimo/tutorial/` のサンプルに準拠。
origin: User Request
---

# marimo Skill Guidelines

marimo は「純粋な Python ファイルとして保存される、リアクティブなノートブック」です。
Jupyter と異なり、セルの実行順序はユーザーの操作順ではなく **データフローグラフ (DAG)** によって決まります。この「リアクティブ性」こそが marimo の中核であり、本ガイドラインが守るべき最重要原則です。

参考資料:
- 公式: https://docs.marimo.io/
- サンプル集: `.claude/skills/marimo/tutorial/` (intro / dataflow / ui / markdown / layout / sql / plots / fileformat / for_jupyter_users)

## When to Activate

- marimo ノートブック (`.py` ファイル、先頭に `import marimo` + `app = marimo.App()` を含むもの) を新規作成または編集するとき
- mo.ui 系 UI 要素、`mo.md`、`mo.sql`、レイアウト、キャッシュ、`@mo.cache` を扱うとき
- Jupyter `.ipynb` から marimo への移行、または逆方向の検討をするとき

## 1. ファイル構造 — 骨格テンプレート

marimo は `marimo edit` で編集するのが本流ですが、AI が直接 `.py` を書き換える場合は以下の構造を厳格に守ってください。

```python
# Copyright/header などは任意
# /// script                                # (任意) PEP 723 スクリプトヘッダ
# requires-python = ">=3.12"
# dependencies = [
#     "marimo",
#     "polars==1.27.1",
# ]
# ///

import marimo

__generated_with = "0.17.4"                  # 生成時の marimo バージョン
app = marimo.App(width="medium")             # App() だけでも可。width は "compact"/"medium"/"full"


with app.setup:                              # (任意) セットアップセル — トップレベル関数/クラスから参照可能
    import dataclasses


@app.cell
def _():
    import marimo as mo
    return (mo,)                             # この変数を他セルに公開する


@app.cell(hide_code=True)                    # コードを隠して出力だけ見せる
def _(mo):
    mo.md("# Title")
    return


@app.function                                # 依存が setup セル or 他の @app.function/@app.class_definition のみ
def add(a, b):
    return a + b


@app.class_definition
@dataclasses.dataclass
class Point:
    x: float
    y: float


if __name__ == "__main__":                   # 必須
    app.run()
```

**絶対に守るルール**:

1. ファイル末尾に `if __name__ == "__main__": app.run()` を置く。
2. 各セルは `@app.cell` (または `@app.cell(hide_code=True)`) でデコレートした **関数** として定義する。
3. 関数名は原則 `_` とする (特別な命名をしたい場合のみ `def echo(...)` のように名前を付ける)。
4. 関数の **引数** がそのセルの **refs** (外部から参照する変数)、**戻り値のタプル** がそのセルの **defs** (他セルに公開する変数)。
   - 公開しない場合は `return` のみ、または `return` 省略。
   - 1 変数公開でも **タプル**: `return (mo,)` のように末尾カンマを付ける (marimo エディタはこの形式で再生成する)。
5. `__generated_with` / `app` の代入は削除・改変しない (marimo エディタが自動管理する領域)。

## 2. リアクティブモデル — marimo の魂

### 2.1 基本ルール

> **あるセルを実行すると、そのセルが定義するグローバル変数を参照している他の全セルが自動的に再実行される。**

これは Jupyter のような「上から順に手動実行」とは根本的に異なります。セルは DAG のノードであり、変数の依存関係がエッジを作ります。

### 2.2 絶対に破ってはいけない制約

| 制約 | 違反例 | 正しい書き方 |
| :--- | :--- | :--- |
| **グローバル変数名は 1 セルにつき 1 回だけ定義** | 2 つのセルで `planet = ...` | 1 つのセルにまとめる、または片方の名前を変える |
| **循環依存の禁止** (セル A→B→A) | セル A で `one = two - 1`、セル B で `two = one + 1` | 1 つのセルにまとめる |
| **`+=` / `-=` での再代入は禁止** (別セルから) | セル A: `count = 0`、セル B: `count += 1` | 同じセルに統合、または `count + 1` を新しい名前で返す |
| **属性への代入は追跡されない** | セル A: `state.x = 1`、セル B: `state.x` を参照 → B は再実行されない | `mo.state()` を使うか、新しいオブジェクトを返す |
| **ミューテーションは追跡されない** | セル A: `lst = [1,2]`、セル B: `lst.append(3)` | 新しいリストを作る (`more = lst + [3]`)、または同じセル内で変更 |

### 2.3 ローカル変数 (アンダースコア接頭辞)

`_` で始まる変数は **そのセルに閉じたプライベート変数** です。他セルには公開されず、複数セルで同じ名前を使える。

```python
@app.cell
def _():
    _tmp = expensive_calc()    # 他セルから見えない
    _tmp * 2
    return
```

補助関数・一時変数・ループ変数はすべて `_` プレフィックスを付けるのが標準のスタイル。

### 2.4 セルを消すと変数も消える

マリモはセルを削除すると、そのセルが定義したグローバル変数も内部状態から消し、依存セルを再実行します。「エディタ上からは消えたのに残ってる謎変数」問題が発生しません。

## 3. マークダウン — `mo.md`

```python
@app.cell
def _(mo, slider):
    mo.md(
        f"""
        ## 現在のスライダー値は **{slider.value}** です

        - UI 要素をそのまま埋め込める: {slider}
        - LaTeX も書ける: $f : \\mathbf{{R}} \\to \\mathbf{{R}}$
        """
    )
    return
```

ポイント:
- **f-string** で Python 値を埋め込める。LaTeX のバックスラッシュを生で書きたい場合は `mo.md(r"...")`。
- UI 要素は `{slider}` のように直接埋め込めば表示される (`mo.as_html` は不要)。
- 任意の Python オブジェクト (pandas/polars DataFrame、matplotlib/altair/plotly Figure、list/dict など) は `mo.as_html(obj)` で HTML 化。
- セル内で `mo.md(...)` が **最後の式** ならそれがセル出力。文字列で返すのではなく式の位置に置くこと。
- 出力を揃えたいときは `mo.md("hello").center()` / `.right()` / `.left()`、および `.callout(kind="info"|"warn"|"success"|"danger"|"neutral")`。

## 4. UI 要素 — `mo.ui.*`

リアクティビティの真価は UI 要素で発揮されます。**UI 要素をグローバル変数に代入すると、ユーザー操作が自動的に全依存セルを再実行します**。

```python
@app.cell
def _(mo):
    slider = mo.ui.slider(start=1, stop=10, step=1, label="count: ")
    slider                                   # セルの最後の式として表示
    return (slider,)


@app.cell
def _(slider):
    slider.value                             # スライダーを動かすと自動で再実行
    return
```

**重要**: UI 要素は **必ずグローバル変数に代入してから表示** する。
`mo.ui.slider(1, 10)` を直接表示するだけではリアクティブにならない。

主な UI 要素 (詳細: `tutorial/ui.py`):

| カテゴリ | 要素 |
| :--- | :--- |
| 入力 | `slider` / `range_slider` / `number` / `text` / `text_area` / `date` |
| 選択 | `dropdown` / `multiselect` / `radio` / `checkbox` / `switch` |
| アクション | `button` / `run_button` / `file` (アップロード) |
| 表示 | `table` / `tabs` / `code_editor` |
| 合成 | `array` / `dictionary` / `batch` / `form` |

合成要素の例:

```python
array = mo.ui.array([mo.ui.text(), mo.ui.slider(1, 10), mo.ui.date()])
# array.value は [text値, slider値, date値] のリスト
```

フォーム確定ボタンで値を確定するには `.form()`:

```python
form = mo.ui.text_area(placeholder="...").form()   # 送信ボタンが付く
```

## 5. レイアウト

```python
mo.hstack([a, b, c], justify="space-between", align="center", gap=0.5, wrap=False)
mo.vstack([a, b], align="stretch")
mo.ui.tabs({"Tab1": content1, "Tab2": content2})
mo.accordion({"Section": body, "More": more_body})   # multiple=True で複数開閉
mo.tree({"nested": {"data": [1, 2, 3]}})
mo.md("hello").callout(kind="info")
```

- 組み合わせでグリッドが作れる: `mo.vstack([mo.hstack([...]), mo.hstack([...])]).center()`
- 複数の統計表示: `mo.hstack([mo.stat(label=..., value=...), ...])`

## 6. SQL — `mo.sql`

DuckDB ベースの SQL セル。依存: `pip install 'marimo[sql]'` (duckdb, polars or pandas)。

```python
@app.cell
def _(mo, token_prefix):
    result = mo.sql(
        f"""
        SELECT * FROM df
        WHERE starts_with(token, '{token_prefix.value}')
        """
    )
    return (result,)                         # result は Polars/Pandas DataFrame
```

重要な挙動:
- SQL 文は **f-string**。UI 要素の `.value` を埋め込めばクエリがリアクティブに。
- グローバル DataFrame (`df`) をそのままテーブル名として参照できる。
- 出力変数名がアンダースコア始まり (`_df`) だとセル内プライベート、名前を普通に付ければ (`result`) 他セルから利用可能。
- CSV/Parquet/S3/HTTP を直接クエリ可能:
  ```sql
  SELECT * FROM 's3://bucket/file.parquet';
  SELECT * FROM read_csv('path.csv');
  SELECT * FROM 'https://example.com/data.csv';
  ```

## 7. ベストプラクティス

ガイドライン (`tutorial/dataflow.py` より):

1. **グローバル変数を最小化する**: 名前衝突を避けるため、1 セルが公開する変数を小さく保つ。
2. **説明的な命名**: 特にグローバル変数は意味のある名前にする。
3. **関数で括り出す**: 補助ロジックは関数にしてセル内スコープに閉じる (アンダースコア変数と組み合わせる)。
4. **ミューテーションは最小化**: オブジェクトの変更は生成セルの中だけで行う。別セルから変更するのはアンチパターン。
5. **冪等なセル**: 同じ refs に対して同じ出力を返すセルは、バグを減らしキャッシュしやすい。
6. **重い計算には `@mo.cache`**:
   ```python
   @mo.cache
   def expensive(params):
       ...
   ```
   `functools.cache` と似ているが、セル再生成後もキャッシュが持続する。

## 8. トップレベル関数・クラスの公開 (モジュールとしての利用)

marimo ノートブックは **他の Python ファイルから `from notebook import func` の形で import 可能** です。この挙動を狙うには:

1. **setup セル** でのみ import (または他のトップレベル関数/クラスを参照):
   ```python
   with app.setup:
       import random
       import dataclasses
   ```
2. 関数・クラスをそれぞれ単独セルに置き、`@app.function` / `@app.class_definition` デコレータを使う:
   ```python
   @app.function
   def roll_die():
       return random.randint(1, 7)

   @app.class_definition
   @dataclasses.dataclass
   class Config:
       n: int
   ```
3. セル内で他の `@app.cell` が定義したグローバル変数に依存していると、トップレベル化されない (通常の `@app.cell` に降格する)。

## 9. よくあるミスと対処

| 症状 | 原因 | 対処 |
| :--- | :--- | :--- |
| `This cell redefines 'X'` エラー | 2 セルで同じ名前をトップレベル定義 | 片方を `_x` にする、またはセルを統合 |
| UI 要素を動かしても再計算されない | UI オブジェクトをグローバル変数に代入していない | `slider = mo.ui.slider(...)` の形で一旦束縛する |
| `state.x = 1` したのに下流セルが動かない | 属性代入は追跡されない | `mo.state()` を使う、または新しいオブジェクトを返す |
| 循環依存エラー | セル A→B→A の変数依存 | 1 つのセルにまとめる |
| import した名前が他セルから見えない | その import を `return` していない | `return (np, plt)` のように公開する、または全セルで個別 import |

## 10. 作業フロー (AI 向け)

1. **既存ノートに対する編集**: まず対象 `.py` を読み、どのセルに `@app.cell` が付いて何を公開しているか把握する。編集後は変更したセルの引数 (refs) と戻り値 (defs) を必ず整合させる。
2. **セルを新設するとき**: 新しい `@app.cell` ブロックを既存のセル間に挿入。変数名の衝突を必ず確認 (既存コードを grep)。
3. **リファクタリング**: 「1 セル = 1 責務」を意識しつつ、巨大セルを分割。ただし **循環依存を作らない** よう DAG を先に頭の中で描く。
4. **テンプレートから新規作成**: 1. のファイル構造テンプレートをコピーし、`mo` を公開するセル → マークダウンタイトル → UI 要素 → 出力、の順で育てる。
5. **迷ったら**: `https://docs.marimo.io/` を参照 (API Reference / User Guide / Interactive elements / Working with data)。

## 11. 参照マップ (tutorial/)

| ファイル | 学べること |
| :--- | :--- |
| `intro.py` | `marimo` の概要、editor の操作方法 |
| `dataflow.py` | リアクティブ実行の原理、refs/defs、循環禁止、アンダースコア変数 |
| `ui.py` | 全 UI 要素のギャラリー、`mo.ui.array` / `mo.ui.dictionary` |
| `markdown.py` | `mo.md` / f-string / LaTeX / `mo.as_html` / `@mo.cache` |
| `layout.py` | `hstack` / `vstack` / `tabs` / `accordion` / `callout` / `tree` |
| `plots.py` | matplotlib / altair / plotly のレンダリング |
| `sql.py` | `mo.sql` / DuckDB / DataFrame 連携 / CSV・Parquet 読み込み |
| `fileformat.py` | `@app.function` / `@app.class_definition` / setup セル / モジュール利用 |
| `for_jupyter_users.py` | Jupyter 経験者向けの落とし穴と読み替え |
