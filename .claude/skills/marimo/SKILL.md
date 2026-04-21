---
name: marimo
description: marimo のノートブック (.py) 開発における包括的なコーディング規約・ベストプラクティス。リアクティブプログラミング、状態管理、UI連携の基準。
origin: Custom
---

# marimo Coding Standards & Best Practices

AIエージェントが `marimo` アプリケーション（リアクティブな Python ノートブック）を構築・編集する際の、最高品質のガイドラインです。
公式ドキュメント (`https://docs.marimo.io/`) のセオリーおよび、独自性である「リアクティブプログラミングモデル」を正確に反映します。

## When to Activate

- `.py` 形式の marimo スクリプトを新規作成・編集するとき
- データ分析ダッシュボードや双方向な UI ツールを構築するとき
- 既存の Jupyter Notebook から marimo アプリ等への移行処理を行うとき

---

## 1. Core Principles (基本原則)

### 1-1. Reactive Execution (リアクティブな実行)
marimo は **トップダウン（上から下）ではなく、データ依存関係（DAG: 有向非巡回グラフ）** に基づいてセルを実行します。
- **RULE**: セルの配置順序（上下関係）に依存したコードを書かない。
- **RULE**: 変数は「ただ一つのセル」でのみ定義（初期化）可能。同じ変数名を複数のセルで再定義（上書き）してはならない（Multiple Definitions Error になるため）。

### 1-2. No Hidden State（隠し状態の排除）
従来のノートブックのように「手動の実行順序による想定外の変数状態」は発生しません。
- **RULE**: グローバルに共有される変数は明示的に `return` し、他のセルで引数（引数名は変数名と一致）として受け取るか、外部スコープとして参照する。

---

## 2. File Structure (基本構造)

marimo ファイルの構造は厳密に定められています。

```python
import marimo

__generated_with = "0.23.1"
app = marimo.App()

@app.cell
def _():
    import marimo as mo
    # 全てのセルで利用可能な変数は return してエクスポートする
    return mo,

@app.cell
def _(mo):
    # 他のセルで定義・returnされた `mo` に依存して自動的に実行される
    mo.md("# Highest Quality marimo App 🚀")
    return

if __name__ == "__main__":
    app.run()
```

---

## 3. 状態管理（State Management）

UIのボタンクリックなどで、データフローとは別に内部の「状態（State）」を保持・変更する必要がある場合は `mo.state()` を使用します。Pythonのグローバル変数（`global`句）は副作用を伴うため使用してはなりません。

```python
@app.cell
def _(mo):
    # state の初期化。値の取得(getter)と、更新(setter)を明示的にタプルで受け取る
    get_count, set_count = mo.state(0)
    return get_count, set_count

@app.cell
def _(get_count, set_count, mo):
    # getter は呼び出して値を取得し、setter は新しい値を渡す
    increment_btn = mo.ui.button(
        label=f"Count is: {get_count()}",
        on_change=lambda _: set_count(get_count() + 1)
    )
    return increment_btn,
```

---

## 4. ユーザーインターフェース (UI Elements)

marimo の `mo.ui.*` はリアクティブなコンポーネントです。ユーザーがUIを操作すると、そのコンポーネントの `value` (またはコンポーネント自体) に依存しているセルが自動的に再実行されます。

- **RULE**: 「UIコンポーネントを定義するセル」と、「その値を使って計算を行うセル」を明確に分離する。
  分離することで、重い計算を伴う再レンダリングループや、ダッシュボードのフリーズを防ぎます。

```python
# PASS: セルの分離（Good Practice）
@app.cell
def _(mo):
    slider = mo.ui.slider(1, 100)
    # UIエレメント自体を返す
    return slider,

@app.cell
def _(slider, expensive_function):
    # スライダーが操作されるたびに、このセルのみが再評価される
    result = expensive_function(slider.value)
    return result,
```

---

## 5. Anti-Patterns (避けるべきコードの臭い)

1. **セルの肥大化（God Cells）**:
   1つの `@app.cell` 内に、データの読み込み、UI定義、前処理、描画などをすべて同居させることは避けてください。役割（設定、データ抽出、UI定義、ビジネスロジック）ごとに細かくセルを分割します。
   
2. **暗黙のループや副作用 (Side Effects)**:
   `while True` や過度な `sleep` によるポーリングは、データフローの評価をブロックします。
   定期的な更新が必要な処理などは、UIイベント等に紐づけて駆動させてください。

3. **import の乱用とスコープ漏れ**:
   特定のセルでのみ使用し、他のセルで再利用しないモジュールは、そのセル内で閉じる（`return` しない）ようにします。

---

**Remember**: marimo は「Python の書きやすさ」と「React のようなリアクティブな UI 構築」を兼ね備えています。常に**データフロー（DAG: 依存関係グラフ）**を意識し、役割が明確でステートレスなセル設計を心がけてください。
