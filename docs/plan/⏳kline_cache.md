# kline ローカルキャッシュ 実装計画

**作成日**: 2026-04-16  
**ブランチ**: sasa/develop  
**担当**: botterYosuke  

---

## 目的

リプレイ再生時に毎回 Binance API を叩いていた kline フェッチを、ローカルキャッシュからの読み込みに置き換える。  
同一 ticker / timeframe / 月の kline は 2 回目以降 HTTP リクエストを省略し、ロード時間を短縮する。

---

## 背景

[debug-waiting-for-data-2026-04-16.md](archive/debug-waiting-for-data-2026-04-16.md) で判明した問題:

- M1 で 1 年分の replay range → 527 ページの API 呼び出し
- Binance レートリミット（`PERP_LIMIT=2400 weight/min`）で 60〜90 秒の待機が発生
- `EventStore` はメモリ上のみ → アプリ再起動で全データが失われる

---

## ファイル形式

j-quants（`S:\j-quants\equities_bars_minute_YYYYMM.csv.gz`）と同形式。

### CSV ヘッダー

```
Date,Time,Open,High,Low,Close,VolumeTotal,VolumeBuy,VolumeSell
```

| カラム | 型 | 備考 |
|---|---|---|
| `Date` | `YYYY-MM-DD` | UTC |
| `Time` | `HH:MM` | UTC、D1 は `00:00` 固定 |
| `Open` | f32 | |
| `High` | f32 | |
| `Low` | f32 | |
| `Close` | f32 | |
| `VolumeTotal` | f32 | 常に存在（= buy + sell または total） |
| `VolumeBuy` | f32 or `""` | `Volume::BuySell` のとき のみ |
| `VolumeSell` | f32 or `""` | `Volume::BuySell` のとき のみ |

**j-quants との対応**:

| j-quants | flowsurface | 差異 |
|---|---|---|
| `Date,Time` | `Date,Time` | UTC vs JST（flowsurface は UTC） |
| `Code` | なし（ファイル名に含む） | |
| `O,H,L,C` | `Open,High,Low,Close` | 同等 |
| `Vo` | `VolumeTotal` | 単位が異なる（枚数 vs コイン） |
| `Va` | なし | 円建て金額不要 |

### サンプル行（BinanceLinear BTCUSDT M1）

```
Date,Time,Open,High,Low,Close,VolumeTotal,VolumeBuy,VolumeSell
2025-04-15,04:49,83000.5,83100.0,82900.0,83050.0,1500.5,800.0,700.5
2025-04-15,04:50,83050.0,83200.0,83000.0,83150.0,1200.0,,
```

---

## ファイル命名規則

### パス構成

```
{APPDATA}/flowsurface/market_data/kline_cache/
  {exchange}_{symbol}_{timeframe}_{period}.csv.gz
```

### period の粒度

実際の `Timeframe::KLINE` バリアント（M1〜D1 の 10 種）に対応する粒度:

| timeframe | 粒度 | 例 |
|---|---|---|
| M1, M3, M5, M15, M30 | 月次（YYYYMM） | `202504` |
| H1, H2, H4, H12 | 月次（YYYYMM） | `202504` |
| D1 | 年次（YYYY） | `2025` |

> `H6`, `H8`, `W1` は `Timeframe` 列挙体に存在しないため対象外。
> j-quants の `equities_bars_minute_YYYYMM.csv.gz` / `equities_bars_daily_YYYYMM.csv.gz` と同じ粒度。

### ファイル名例

| ticker / timeframe | ファイル名 |
|---|---|
| BinanceLinear:BTCUSDT / M1 | `binancelinear_btcusdt_m1_202504.csv.gz` |
| BinanceLinear:BTCUSDT / H1 | `binancelinear_btcusdt_h1_202504.csv.gz` |
| BinanceSpot:ETHUSDT / D1 | `binancespot_ethusdt_d1_2025.csv.gz` |
| OkexLinear:BTCUSDT / M1 | `okxlinear_btcusdt_m1_202504.csv.gz` |

### ticker の正規化（ファイル名用）

`Exchange::to_string()` は `"{Venue} {MarketKind}"` 形式（例: `"Binance Linear"`）。スペースを除去して小文字化する。

```
Exchange::BinanceLinear + "BTCUSDT" → "Binance Linear" → "BinanceLinear" → "binancelinear_btcusdt"
Exchange::BinanceSpot   + "ETHUSDT" → "Binance Spot"   → "BinanceSpot"   → "binancespot_ethusdt"
Exchange::OkexLinear    + "BTCUSDT" → "OKX Linear"     → "OKXLinear"     → "okxlinear_btcusdt"
```

実装: `exchange.to_string().replace(' ', "").to_lowercase() + "_" + symbol.to_lowercase()`

> `Exchange::Tachibana` は `"Tachibana Spot"` → `"tachibanaspot"` となるが、Tachibana はキャッシュ対象外のため `ticker_to_file_prefix` を呼ぶことはない。

---

## ディレクトリ構造

```
%APPDATA%/flowsurface/
└── market_data/
    ├── binance/          ← 既存（空）
    └── kline_cache/      ← 新規作成
        ├── binancelinear_btcusdt_m1_202504.csv.gz
        ├── binancelinear_btcusdt_m1_202505.csv.gz
        ├── binancelinear_btcusdt_h1_202504.csv.gz
        └── okxlinear_btcusdt_m1_202504.csv.gz
```

---

## キャッシュ戦略

### 読み取りロジック（`load_klines` 呼び出し時）

```
1. fetch range [start_ms, end_ms] を「期間ファイル単位」に分割
   例: 2025-04-15 〜 2025-06-30 → [202504, 202505, 202506] の 3 ファイル

2. 全ファイルが kline_cache に存在する？
   → YES: CSV.gz を読み込み、Kline に変換 → Vec<Kline> を返す（API 不要）
   → NO:  API fetch（range 全体）→ 取得した klines を月ごとに分割して CSV.gz に保存 → Vec<Kline> を返す
```

> **部分キャッシュ（一部のファイルが欠落）は range 全体を再フェッチする。**  
> 欠落ファイルのみを個別フェッチする最適化は Phase 3 のスコープ外とする。  
> `try_load_from_cache` が `Ok(None)` を返した時点で range 全量を API フェッチし、その結果で全期間ファイルを上書き保存する。

### キャッシュの有効性

- **過去のファイル**（現在月より前）: 不変 → 常に有効、再フェッチしない
- **現在月のファイル**（今月分）: 追記が発生するため無効化が必要  
  → 今月のファイルは **常に API フェッチ**（キャッシュしない）

### キャッシュ対象外

- **Tachibana**: セッション依存のため現状はキャッシュ対象外（将来対応）
- **現在月**: 上記参照

---

## 実装箇所

### 主な変更ファイル

| ファイル | 変更内容 |
|---|---|
| `src/replay/loader.rs` | `load_klines` にキャッシュチェックを追加 |
| `src/replay/cache.rs` | 新規: キャッシュ読み書きモジュール |
| `Cargo.toml` | `flate2` を依存に追加 |

### `src/replay/cache.rs` の公開インターフェース

```rust
/// キャッシュから klines を読み込む。
/// 指定 range のすべての期間ファイルが存在する場合のみ Ok(Some(klines)) を返す。
/// いずれかが欠落している場合は Ok(None) を返す（API フェッチにフォールバック）。
///
/// `ticker_info.min_ticksize` は Kline::new() の価格丸め処理に使用する。
pub fn try_load_from_cache(
    ticker_info: &TickerInfo,
    timeframe: Timeframe,
    range: Range<u64>,
) -> Result<Option<Vec<Kline>>, CacheError>;

/// API から取得した klines をキャッシュに保存する。
/// klines を月（または年）ごとに分割して各ファイルに書き込む。
/// 現在月は保存しない。
pub fn save_to_cache(
    ticker: &Ticker,
    timeframe: Timeframe,
    klines: &[Kline],
) -> Result<(), CacheError>;

/// キャッシュディレクトリのパス。
/// `data::data_path(Some("market_data/kline_cache/"))` を呼び出す。
/// `FLOWSURFACE_DATA_PATH` 環境変数による上書きにも自動対応。
pub fn cache_dir() -> PathBuf;
```

### `loader.rs` の変更イメージ

```rust
pub async fn load_klines(stream: StreamKind, range: Range<u64>) -> Result<KlineLoadResult, String> {
    // Tachibana は既存パスへ（キャッシュ対象外）
    if is_tachibana(&ticker_info) {
        return fetch_and_return(stream, range).await;
    }

    // キャッシュを試みる
    match cache::try_load_from_cache(&ticker_info, timeframe, range.clone()) {
        Ok(Some(klines)) => {
            log::debug!("[cache] hit: {} {} range={:?}", ticker, timeframe, range);
            return Ok(KlineLoadResult { stream, range, klines });
        }
        Ok(None) => {
            log::debug!("[cache] miss: {} {} — fetching from API", ticker, timeframe);
        }
        Err(e) => {
            log::warn!("[cache] read error: {} — falling back to API", e);
        }
    }

    // API フェッチ
    let klines = fetch_all_klines(ticker_info, timeframe, range.clone()).await?;

    // spawn_blocking でブロッキング I/O をオフロード（エラーは warn ログのみ）
    let ticker_for_cache = ticker_info.ticker;
    let klines_for_cache = klines.clone();
    tokio::task::spawn_blocking(move || {
        if let Err(e) = cache::save_to_cache(&ticker_for_cache, timeframe, &klines_for_cache) {
            log::warn!("[cache] write error: {}", e);
        }
    });

    Ok(KlineLoadResult { stream, range, klines })
}
```

---

## 依存クレート

### 追加: `flate2` （gzip 圧縮/展開）

`Cargo.toml` に追加:

```toml
flate2 = "1.0"
```

### 追加: `csv`（CSV 読み書き）

`csv` は `exchange` クレートに直接依存として存在するが workspace dependency ではない。  
`flowsurface` 本体の `Cargo.toml` にも直接追加する:

```toml
csv = "1.3.1"
```

### 既存クレート（追加不要）

- `chrono` — workspace dependency に既存（UTC 変換に使用）

---

## エラー型

```rust
#[derive(Debug, thiserror::Error)]
pub enum CacheError {
    #[error("IO error: {0}")]
    Io(#[from] std::io::Error),
    #[error("CSV error: {0}")]
    Csv(#[from] csv::Error),
    #[error("invalid timestamp in cache file: {ts}")]
    InvalidTimestamp { ts: String },
}
```

---

## 実装マイルストーン

### Phase 1: 読み書き基盤
- [ ] `src/replay/cache.rs` 作成（`cache_dir`, `ticker_to_file_prefix`, `range_to_periods`）
- [ ] `save_to_cache`: klines → 月別 CSV.gz に書き込み
- [ ] `try_load_from_cache`: CSV.gz → Vec<Kline>
- [ ] `loader.rs` に統合（miss → API fetch → save）

### Phase 2: テスト
- [ ] `cache.rs` 単体テスト（save → load roundtrip, 月またぎ range）
- [ ] `cache.rs` 単体テスト（現在月スキップ）: `save_to_cache` に `current_month` パラメータを注入する形にしてテスト時に固定値を渡す
- [ ] E2E テスト: 2 回目の `start_app` → `wait_playing` が 5 秒以内（キャッシュヒット確認）

### Phase 3: 将来対応（スコープ外）
- [ ] Tachibana D1 キャッシュ対応
- [ ] 現在月キャッシュ（TTL ベースの無効化）
- [ ] ローディング進捗表示（Loading... N/M pages）

---

## 設計上の注意点

1. **現在月は保存しない**: `kline.time` の最大月が現在月であるファイルは `save_to_cache` でスキップする。これにより「不完全なキャッシュが残る」問題を防ぐ。

2. **キャッシュミスはサイレント**: `try_load_from_cache` のエラーは `warn` ログのみ。API フェッチに透過的にフォールバックし、ユーザーへのエラー通知は行わない。

3. **過去月は immutable**: 取引所の kline データは後から修正されないと仮定し、`file_exists = valid` とみなす。有効期限チェックは不要。

4. **save は `spawn_blocking` でオフロード**: `save_to_cache` は同期 I/O。`load_klines` は tokio タスクで実行されるため、`tokio::task::spawn_blocking` でブロッキング処理をスレッドプールにオフロードする。`save` の完了を `await` しないことでレスポンスタイムへの影響を最小化する。

5. **`cache_dir()` は `data::data_path()` を使用**: 独自に `dirs_next` を呼ばず `data::data_path(Some("market_data/kline_cache/"))` を使う。`FLOWSURFACE_DATA_PATH` 環境変数による上書きや、テスト時のパス差し替えが自動的に機能する。

6. **`try_load_from_cache` は `TickerInfo` を受け取る**: `Kline::new()` の価格丸め処理に `MinTicksize` が必要なため、`ticker_info.min_ticksize` を使用する。`Ticker` だけでなく `&TickerInfo` を渡す。

7. **`csv` クレートを `flowsurface` 本体に追加**: 現在は `exchange` クレートのみに依存（非 workspace）。`cache.rs` は `src/replay/` に置くため `Cargo.toml` の `[dependencies]` への直接追加が必要。

---

## 先行実装（参考）

**Tachibana マスターデータキャッシュ**（2026-04-17 実装済み）が本計画と同じパターンを採用した:

| 項目 | Tachibana master cache | kline cache（本計画） |
|---|---|---|
| パス | `data_path(Some("market_data/tachibana_master_cache.json"))` | `data_path(Some("market_data/kline_cache/..."))` |
| フォーマット | JSON（`serde_json`） | CSV.gz（`csv` + `flate2`） |
| ロードタイミング | セッション復元時（起動直後） | `load_klines` 呼び出し時 |
| 無効化戦略 | なし（マスターは変化が少ない） | 現在月は保存しない |
| 実装ファイル | `exchange/src/adapter/tachibana.rs`, `src/main.rs` | `src/replay/cache.rs`, `src/replay/loader.rs` |

効果: 初回 79〜108 秒 → 2 回目以降 約 2 秒で解決。本 kline キャッシュでも同様の効果を見込む。

---

## 進捗

- ✅ 計画書作成
- ✅ キャッシュパターン確立（Tachibana master cache で先行実装・検証済み）
- [ ] Phase 1: 読み書き基盤実装
- [ ] Phase 2: テスト
