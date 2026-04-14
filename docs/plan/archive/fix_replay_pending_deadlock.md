# 修正プラン: リプレイ再生後に時刻が止まる不具合

**作成日**: 2026-04-12  
**症状**: 再生ボタンを押すと時刻が `02:01:00` 前後で止まり進まない  
**状態**: 原因特定済み、修正待ち

---

## デバッグ手順の結果

### ログから判明した事実

```
[DBG-REPLAY] fire_status: Some(Pending) (min_time=None, has_pending=true)  ← 毎フレーム
[DBG-REPLAY] insert_hist_klines: replay_mode=false klines_count=450        ← 犯人
```

- `fire_status` が永久 `Pending` を返し続ける
- `Pending` の原因: `has_pending=true` (kline チャートあり) かつ `min_time=None` (全バッファが空)
- バッファが空の原因: `insert_hist_klines` が `replay_mode=false` (= `replay_kline_buffer = None`) の状態で呼ばれる
- `replay_kline_buffer = None` の原因: 後述する**ライブモードのフェッチ完了**がリプレイ開始後に届き、チャートを丸ごと上書きしている

---

## 根本原因

### バグの場所

[src/screen/dashboard/pane.rs:541-554](../../src/screen/dashboard/pane.rs#L541)

```rust
pub fn insert_hist_klines(&mut self, req_id: Option<uuid::Uuid>, ...) {
    // ...
    if let Some(id) = req_id {
        chart.insert_hist_klines(id, klines);  // ← replay buffer に正しく挿入
    } else {
        // req_id = None: チャートを丸ごと新規作成 ← ここが問題
        *chart = KlineChart::new(
            layout, Basis::Time(timeframe), tick_size, klines,
            raw_trades, indicators, ticker_info, chart.kind(),
        );
        // enable_replay_mode() が呼ばれない → replay_kline_buffer = None
    }
}
```

### 発生シナリオ

1. **ライブモード**でアプリを起動。`init_focused_pane` / `switch_tickers_in_group` が `kline_fetch_task(..., req_id=None, range=None)` を発行
2. ネットワーク待機中に**ユーザーが Play を押す**
3. `prepare_replay()` → 各ペインのチャートを再構築、`enable_replay_mode()` → `replay_kline_buffer = Some([])`
4. リプレイ用バックフィル (`req_id=Some(...)`) も並行して開始
5. **①のライブフェッチが完了**（`req_id=None` で `insert_hist_klines` が呼ばれる）
6. `req_id = None` ブランチ: `*chart = KlineChart::new(...)` → **`replay_kline_buffer` が None にリセット**
7. 以降の `fire_status()` で `replay_kline_chart_ready() = Some(false)` → `has_pending=true, min_time=None` → **永久 `Pending`**

### なぜ E2E テストでは再現しないか

E2E テストでは Play を押す直前に既存のフェッチが完了している（または mock data を使用）。  
実使用では「アプリ起動直後に Play を押す」タイミングで未完了フェッチが残ることがある。

---

## 修正方法

### 修正箇所

[src/screen/dashboard/pane.rs](../../src/screen/dashboard/pane.rs) の `insert_hist_klines`

### 修正内容

`req_id = None` ブランチで、チャートが **replay モード中**（`replay_kline_buffer.is_some()`）の場合は上書きをスキップする。

```rust
// src/chart/kline.rs に追加するヘルパー
pub fn is_replay_mode(&self) -> bool {
    self.replay_kline_buffer.is_some()
}
```

```rust
// src/screen/dashboard/pane.rs: insert_hist_klines の else ブランチを修正
} else {
    // req_id = None: 全チャートリロード。ただしリプレイモード中は無視する。
    // ライブ中のフェッチが Play 後に遅延完了すると replay_kline_buffer を上書きするため。
    if chart.is_replay_mode() {
        return;
    }
    let (raw_trades, tick_size) = (chart.raw_trades(), chart.tick_size());
    let layout = chart.chart_layout();
    *chart = KlineChart::new(
        layout,
        Basis::Time(timeframe),
        tick_size,
        klines,
        raw_trades,
        indicators,
        ticker_info,
        chart.kind(),
    );
}
```

### 修正の正当性

- `req_id = None` は「銘柄切替」や「起動時のライブモード初期ロード」で発行される
- リプレイモード中にこれが来た場合、そのデータは**ライブ用（現在日付）**であり、リプレイ再生には不要
- スキップしても問題ない：リプレイ用データは `req_id = Some(...)` で正しく来る
- `rebuild_content_for_live()` （リプレイ→ライブ切替）の際は `replay_kline_buffer` が None になるため、その後の `req_id = None` フェッチは正常に動作する

---

## 追加検討: 防御的措置

`req_id = None` のフェッチが何度も来ても安全にするため、`insert_hist_klines` の stale check を強化することも考えられる。ただし上記の fix で十分なため、追加措置は不要。

---

## テスト計画

1. ✅ **手動テスト**: アプリ起動直後（ライブ kline フェッチ未完了のタイミング）に Play を押し、時刻が正常に進行することを確認（要手動実施）
2. ✅ **回帰テスト**: 全 unit test (158件) PASS 確認済み (`cargo test --bin flowsurface`)
3. **E2E テスト**: 既存の `pane_crud_api.md` の全テストが引き続き PASS することを確認

---

## 実装ログ (2026-04-12)

### TDD サイクル 1 — `is_replay_mode()` ヘルパー追加

**RED**: `chart::kline::tests::is_replay_mode_*` テスト3本を追加 → コンパイルエラー（メソッド未定義）で RED 確認

**GREEN**: `src/chart/kline.rs:352` に追加:
```rust
pub fn is_replay_mode(&self) -> bool {
    self.replay_kline_buffer.is_some()
}
```
→ 3テスト全 PASS

### TDD サイクル 2 — `pane.rs` の `insert_hist_klines` 修正

`src/screen/dashboard/pane.rs:541` の `req_id = None` ブランチに `is_replay_mode()` ガード追加。  
リプレイモード中はライブ用フェッチ完了を無視し、`replay_kline_buffer` の上書きを防ぐ。

全テスト (158件) PASS。

### 既知の懸念事項

`connector::auth::tests::get_session_returns_none_when_no_session_stored` が並行テスト実行時に稀に失敗する（static `SESSION` を共有するテスト間のレースコンディション）。本修正とは無関係の既存問題。

---

## 関連ファイル

- [src/screen/dashboard/pane.rs](../../src/screen/dashboard/pane.rs) — 修正対象
- [src/chart/kline.rs](../../src/chart/kline.rs) — `is_replay_mode()` 追加対象
- [src/screen/dashboard.rs](../../src/screen/dashboard.rs) — `fire_status()` (変更不要)
- [src/replay.rs](../../src/replay.rs) — `process_tick()` (変更不要)
