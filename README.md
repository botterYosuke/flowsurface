# Flowsurface

[![Crates.io](https://img.shields.io/crates/v/flowsurface)](https://crates.io/crates/flowsurface)
[![Lint](https://github.com/flowsurface-rs/flowsurface/actions/workflows/lint.yml/badge.svg)](https://github.com/flowsurface-rs/flowsurface/actions/workflows/lint.yml)
[![Format](https://github.com/flowsurface-rs/flowsurface/actions/workflows/format.yml/badge.svg)](https://github.com/flowsurface-rs/flowsurface/actions/workflows/format.yml)
[![Discord](https://img.shields.io/badge/Discord-%235865F2.svg?&logo=discord&logoColor=white)](https://discord.gg/RN2XAF7ZuR)
[![License: GPL v3](https://img.shields.io/badge/License-GPLv3-blue.svg)](https://github.com/flowsurface-rs/flowsurface/blob/main/LICENSE)
[![Made with iced](https://iced.rs/badge.svg)](https://github.com/iced-rs/iced)

暗号資産マーケットおよび国内株式向けのオープンソース・ネイティブデスクトップチャートアプリケーション。  
Binance、Bybit、Hyperliquid、OKX、MEXC、および **立花証券 e支店** に対応。

<div align="center">
  <img
    src="https://github.com/user-attachments/assets/baddc444-e079-48e5-82b2-4f97094eba07"
    alt="Flowsurface screenshot"
    style="max-width: 100%; height: auto;"
  />
</div>

### 主な機能

-   複数のチャート・パネル種別:
    -   **ヒートマップ（Historical DOM）:** ライブ約定と L2 板情報を使って時系列ヒートマップを生成。価格グルーピング・時間集計・固定 or 表示範囲ボリュームプロファイルをカスタマイズ可能。
    -   **ローソク足:** 時間ベースおよびカスタムティックベース間隔に対応した従来型 Kline チャート。
    -   **フットプリント:** ローソク足チャート上に価格グループ・間隔集計された約定ビューを重ね表示。クラスタリング手法・インバランス・ネイキッド POC スタディを設定可能。
    -   **タイム & セールス:** ライブ約定のスクロールリスト。
    -   **DOM（板）/ ラダー:** グループ化された価格レベルに直近の約定数量を加えた L2 板情報を表示。
    -   **比較:** 複数データソースの終値を基準に正規化したパーセンテージスケールの折れ線グラフ。
-   約定ストリーム連動のリアルタイムサウンドエフェクト
-   マルチウィンドウ / マルチモニター対応
-   複数ペイン間で銘柄を一括切替できるペインリンク
-   編集可能なカラーパレットを含む永続レイアウトとカスタマイズ可能なテーマ

##### マーケットデータは各取引所の公開 REST API および WebSocket から直接受信します

---

### リプレイ機能

過去の Kline / Trades データを時系列順に再生し、ライブチャートと同等のビュー更新を行います。

| 機能 | 内容 |
|---|---|
| モード切替 | LIVE / REPLAY をヘッダーバー or F5 or HTTP API でトグル |
| 範囲指定 | `YYYY-MM-DD HH:MM` 形式で開始・終了（UTC 解釈）|
| 再生制御 | Step / Advance / RewindToStart |
| 再生速度 | 自動再生と速度切替は廃止。agent session API で明示的に進行 |
| mid-replay ペイン操作 | リプレイ中のペイン追加・削除・timeframe / ticker 変更 |
| HTTP 制御 API | `127.0.0.1:9876` でリプレイ・ペイン操作を外部から駆動 |
| 起動時リプレイ復元 | `saved-state.json` に replay 構成が含まれる場合、停止状態で復元 |

**取引所別リプレイ対応状況:**

| 取引所 | Kline | Trades | リプレイ可否 |
|---|:-:|:-:|:-:|
| Binance (Spot / Linear / Inverse) | ✅ 全 tf | ✅ | ✅ 完全 |
| Bybit | ✅ 全 tf | ❌ | ⚠️ kline のみ |
| Hyperliquid | ✅ 全 tf | ❌ | ⚠️ kline のみ |
| OKX | ✅ 全 tf | ❌ | ⚠️ kline のみ |
| MEXC | ✅ 全 tf | ❌ | ⚠️ kline のみ |
| **立花証券** | ✅ D1 のみ | ❌ | ⚠️ D1 kline のみ |

詳細仕様は [docs/replay_header.md](docs/replay_header.md) を参照してください。

---

### 立花証券 e支店 対応

国内株式の板情報・歩み値・日足チャートをリアルタイムで表示します。

| 機能 | 内容 |
|---|---|
| 認証 | ログイン画面からユーザー ID・パスワードを入力（セッションは keyring で永続化）|
| 銘柄検索 | 銘柄マスタ（約4200件）をダウンロードし、銘柄コード・名称で検索 |
| 日足チャート | 最大約20年分の OHLCV データ（株式分割調整値対応）|
| 板情報 | HTTP Long-polling による 10 本板のリアルタイム表示 |
| 歩み値 | リアルタイム約定の表示 |
| リプレイ | D1 kline のみ対応 |

**制約事項:**
- 日足（D1）のみ対応。分足・時間足は API 非提供
- 電話認証が事前に必要（ユーザー手動）
- 東証立会時間のみ板データ更新（9:00-11:30, 12:30-15:30 JST）

詳細仕様は [docs/tachibana_spec.md](docs/tachibana_spec.md) を参照してください。

---

### 注文機能（立花証券 e支店）

立花証券 e支店 API を使った国内株式の発注機能を追加中です。

| パネル | 機能 | 状態 |
|---|---|---|
| **注文入力パネル** | 買い・売り注文の入力と発注（成行 / 指値、現物 / 信用） | 実装中 |
| **注文約定照会パネル** | 発注済み注文の一覧表示と約定状況の確認 | 実装中 |
| **余力情報パネル** | 買付可能額・委託保証金率・追証フラグの確認 | 実装中 |
| **注文訂正・取消** | 発注済み注文の値段・株数変更およびキャンセル | 実装中 |

**主な設計方針:**
- 注文確認モーダルによる 2 段階発注（バイパス不可）
- 第二パスワードはメモリ上にのみ保持し、ログ・設定ファイルへの書き込みを禁止
- 約定検出時にトースト通知で通知
- チャートペインの銘柄変更と注文入力パネルを自動連動

詳細仕様は [docs/plan/order_windows.md](docs/plan/order_windows.md) を参照してください。

---

#### フットプリントチャートの過去約定データ:

-   デフォルトでは WebSocket 経由でライブ約定をリアルタイムに取得・描画します。
-   Binance 銘柄については、設定で約定フェッチを有効にすることで表示範囲をバックフィルできます:
    -   [data.binance.vision](https://data.binance.vision/): 高速な日次一括ダウンロード（当日分なし）。
    -   REST API（例: `/fapi/v1/aggTrades`）: 低速なページネーション方式の当日分フェッチ（レート制限あり）。
    -   Binance コネクターは必要に応じてどちらか一方または両方を使用してデータを取得します。
-   Bybit / Hyperliquid の約定フェッチは、適切な REST API がないため未対応。OKX は対応予定。

## インストール

### 方法 1: ビルド済みバイナリ

Windows・macOS・Linux 向けのスタンドアロン実行ファイルが [リリースページ](https://github.com/flowsurface-rs/flowsurface/releases) からダウンロードできます。

<details>
<summary><strong>実行できない場合（権限・セキュリティ警告）</strong></summary>
 
バイナリは現在未署名のため、フラグが立つ場合があります。

-   **Windows**: 「Windows によって PC が保護されました」と表示された場合は、**詳細情報** → **実行** をクリックしてください。
-   **macOS**: 「開発元を検証できません」と表示された場合は、アプリを Control クリック（右クリック）して **開く** を選択するか、_システム設定 > プライバシーとセキュリティ_ から許可してください。
</details>

### 方法 2: ソースからビルド

#### 要件

-   [Rust ツールチェーン](https://www.rust-lang.org/tools/install)
-   [Git バージョン管理システム](https://git-scm.com/)
-   システム依存パッケージ:
    -   **Linux**:
        -   Debian/Ubuntu: `sudo apt install build-essential pkg-config libasound2-dev`
        -   Arch: `sudo pacman -S base-devel alsa-lib`
        -   Fedora: `sudo dnf install gcc make alsa-lib-devel`
    -   **macOS**: Xcode コマンドラインツールをインストール: `xcode-select --install`
    -   **Windows**: 追加の依存パッケージは不要

#### オプション A: `cargo install`

```bash
# 最新版をグローバルインストール
cargo install --git https://github.com/flowsurface-rs/flowsurface flowsurface

# 実行
flowsurface
```

#### オプション B: リポジトリをクローン

```bash
# リポジトリをクローン
git clone https://github.com/flowsurface-rs/flowsurface

cd flowsurface

# ビルドして実行
cargo build --release
cargo run --release
```

## クレジット・謝辞

-   [Kraken Desktop](https://www.kraken.com/desktop)（旧 [Cryptowatch](https://blog.kraken.com/product/cryptowatch-to-sunset-kraken-pro-to-integrate-cryptowatch-features)）— このプロジェクトのインスピレーション源
-   [Halloy](https://github.com/squidowl/halloy) — 基盤となるコード設計とプロジェクトアーキテクチャの優れたオープンソースリファレンス
-   [iced](https://github.com/iced-rs/iced) — このプロジェクトを可能にしている GUI ライブラリ

## コミュニティ

フィードバック・質問・プロジェクトに関する雑談は Discord コミュニティへどうぞ:  
https://discord.gg/3YUUqzWWxr

## ライセンス

Flowsurface は [GPLv3](./LICENSE) ライセンスの下でリリースされています。プロジェクトへの貢献は同ライセンスで共有されます。
