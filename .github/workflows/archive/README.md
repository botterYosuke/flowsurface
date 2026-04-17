# アーカイブ済みワークフロー

## e2e.yml — 無効化理由

**無効化日: 2026-04-17**

### 問題

GitHub Actions の E2E テスト (S1〜S22) が、リプレイ時間が前進しないことで複数のテストケースが FAIL していた。

```
FAIL: TC-S1-05 — 30 秒待機しても current_time が前進しなかった
FAIL: TC-S1-08 — Resume 後に前進しない
FAIL: TC-S1-14 — diff=0 (expected 60000)
```

### 根本原因

**GUI レンダリング自体は正常動作している。** `iced` は Windows runner の
「Microsoft Basic Render Driver」(ソフトウェアレンダラー + DX12) で描画でき、
HTTP API (ポート 9876) も正常に応答する。

真の原因は **GitHub Actions の US IP から取引所 API がジオブロックされること**：

| 取引所 | エラー | 内容 |
|---|---|---|
| Binance | `HTTP 451 Unavailable For Legal Reasons` | US IP からのアクセスを規約で禁止 |
| Bybit | `HTTP 403 Forbidden` | 同様に US IP をブロック |

リプレイ機能は kline (OHLCV) データをオンデマンドで取引所 API から取得する設計
(`src/replay/loader.rs` → `exchange/src/adapter.rs::fetch_klines()`)。
データ取得に失敗すると `EventStore` が空のままとなり `current_time` が前進しない。

### 再有効化するために必要なこと

以下のいずれかの対応が必要：

1. **フィクスチャデータ方式** (推奨)
   - ローカル (日本 IP) で kline データを事前収録し JSON ファイルとしてリポジトリに同梱
   - 環境変数 `E2E_FIXTURE_DIR` が設定されている場合は API コールをバイパスしてローカルファイルを読み込む
   - `exchange/src/adapter.rs` に条件分岐を追加

2. **セルフホストランナー** (日本拠点)
   - GitHub-hosted runner の代わりに日本 IP のセルフホストランナーを使用
   - 取引所 API のジオブロックを回避できる

このファイルは `.github/workflows/archive/` に置かれているため、
GitHub Actions のスキャン対象外となり自動実行されない。
再有効化する場合は `.github/workflows/` 直下に移動すること。
