# Flowsurface

[![Crates.io](https://img.shields.io/crates/v/flowsurface)](https://crates.io/crates/flowsurface)
[![Lint](https://github.com/flowsurface-rs/flowsurface/actions/workflows/lint.yml/badge.svg)](https://github.com/flowsurface-rs/flowsurface/actions/workflows/lint.yml)
[![Format](https://github.com/flowsurface-rs/flowsurface/actions/workflows/format.yml/badge.svg)](https://github.com/flowsurface-rs/flowsurface/actions/workflows/format.yml)
[![Discord](https://img.shields.io/badge/Discord-%235865F2.svg?&logo=discord&logoColor=white)](https://discord.gg/RN2XAF7ZuR)
[![License: GPL v3](https://img.shields.io/badge/License-GPLv3-blue.svg)](https://github.com/flowsurface-rs/flowsurface/blob/main/LICENSE)
[![Made with iced](https://iced.rs/badge.svg)](https://github.com/iced-rs/iced)

暗号資産市場向けのオープンソース・ネイティブデスクトップチャートアプリケーション。Binance / Bybit / Hyperliquid / OKX / MEXC に対応。

<div align="center">
  <img
    src="https://github.com/user-attachments/assets/baddc444-e079-48e5-82b2-4f97094eba07"
    alt="Flowsurface screenshot"
    style="max-width: 100%; height: auto;"
  />
</div>

---

## 思想：情報の透明性と集合知のインフラへ

Flowsurface は単なる分析ツールではありません。

市場の価格は、ニュースや世論調査よりも遥かに正確な「リアルタイムの確率」を示します。実際にお金が動いている市場には、バイアスが入り込む余地がありません。私たちはその価格を、**世界で最も信頼できる事実のオラクル**と考えています。

このアプリが API をはじめとする外部連携を重視するのは、その信念に基づいています。データを囲い込むのではなく、誰もが引用・利用・検証できる**公共のデータインフラ**として機能させる。それが Flowsurface の目指す姿です。

---

### 1. 情報の「真実」を測るオラクル

市場価格は、世界中のトレーダーが自らのお金を賭けて形成するコンセンサスです。ニュースメディアや専門家の意見よりも、ずっと正直な指標といえます。

Flowsurface はそのデータをリアルタイムで可視化し、ニュースメディア・金融機関・研究者が**「基準値」として参照できる窓口**になることを目指します。

### 2. コンポーザビリティ（構成可能性）

自社で全ての機能を作るのではなく、他のアプリやボットが Flowsurface の機能を自由に組み込める設計を目指します。

- **取引ボット:** API を通じた自動取引が市場の流動性を高め、価格精度を向上させます。
- **他プラットフォームへの埋め込み:** ニュースサイトやダッシュボードに市場データを表示させ、エコシステムを広げます。

レゴブロックのように機能を組み合わせられる。それが外部連携充実の理由です。

### 3. 検証可能な透明性（Trustless）

「私を信じるな、データを検証せよ」——これが私たちの原則です。

オープンな API を提供することで、第三者がいつでも取引履歴や価格の妥当性を独自に検証できます。プラットフォームによる操作の疑いを排除し、信頼をコードとデータで担保します。

### 4. 裁定取引による市場の健全化

市場に歪みがあれば、外部のアルゴリズムが API 経由でそれを即座に修正します。手動では追いきれない複数市場間の価格差を埋めることで、最終的に市場全体の予測精度が高まります。

---

## 主な機能

- **ヒートマップ（Historical DOM）:** ライブトレードと L2 板情報から時系列ヒートマップを生成。価格グルーピング・時間集計・ボリュームプロファイルに対応。
- **キャンドルスティック:** 時間軸・ティック数ベース両対応のトラディショナルな K ラインチャート。
- **フットプリント:** キャンドルスティック上にトレードを価格帯集計で表示。クラスタリング手法・イミバランス・ネイキッド POC スタディに対応。
- **タイム & セールス:** ライブトレードのスクロールリスト。
- **DOM（板）/ ラダー:** グループ化された価格帯での L2 板と直近の出来高を表示。
- **コンパリゾン:** 複数データソースをパーセンテージスケールで正規化して比較するラインチャート。
- リアルタイム売買音声エフェクト
- マルチウィンドウ / マルチモニター対応
- ペインリンクによるティッカー一括切替
- 永続レイアウトとカスタマイズ可能なテーマ・カラーパレット

市場データは各取引所の公開 REST API と WebSocket から直接受信しています。

---

## インストール

### 方法 1: ビルド済みバイナリ

Windows / macOS / Linux 向けのスタンドアロン実行ファイルを [Releases ページ](https://github.com/flowsurface-rs/flowsurface/releases) からダウンロードできます。

<details>
<summary><strong>実行できない場合（権限・セキュリティ警告）</strong></summary>

バイナリは現在署名されていないため、警告が出る場合があります。

- **Windows:** 「Windows によって PC が保護されました」が表示されたら、**詳細情報** → **実行** を選択。
- **macOS:** 「開発元を確認できません」が表示されたら、右クリックで **開く** を選択するか、_システム設定 > プライバシーとセキュリティ_ から許可。
</details>

### 方法 2: ソースからビルド

#### 必要なもの

- [Rust ツールチェーン](https://www.rust-lang.org/tools/install)
- [Git](https://git-scm.com/)
- システム依存ライブラリ:
  - **Linux (Debian/Ubuntu):** `sudo apt install build-essential pkg-config libasound2-dev`
  - **Linux (Arch):** `sudo pacman -S base-devel alsa-lib`
  - **Linux (Fedora):** `sudo dnf install gcc make alsa-lib-devel`
  - **macOS:** `xcode-select --install`
  - **Windows:** 追加の依存関係なし

#### Option A: `cargo install`

```bash
cargo install --git https://github.com/flowsurface-rs/flowsurface flowsurface
flowsurface
```

#### Option B: リポジトリをクローン

```bash
git clone https://github.com/flowsurface-rs/flowsurface
cd flowsurface
cargo build --release
cargo run --release
```

---

## クレジット

- [Kraken Desktop](https://www.kraken.com/desktop)（旧 Cryptowatch）— このプロジェクトの着想源
- [Halloy](https://github.com/squidowl/halloy) — コード設計とアーキテクチャの優れた参考実装
- [iced](https://github.com/iced-rs/iced) — このアプリを支える GUI ライブラリ

## コミュニティ

フィードバック・質問・雑談は Discord へ:  
https://discord.gg/RN2XAF7ZuR

## ライセンス

Flowsurface は [GPLv3](./LICENSE) ライセンスのもとで公開されています。コントリビューションも同ライセンスが適用されます。
