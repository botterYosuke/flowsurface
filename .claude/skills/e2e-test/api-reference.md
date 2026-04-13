# API リファレンス

## リプレイ API

| メソッド | パス | 用途 |
|---------|------|------|
| `GET` | `/api/replay/status` | 現在状態の JSON 取得 |
| `POST` | `/api/replay/toggle` | Live↔Replay 切替 |
| `POST` | `/api/replay/play` | 再生開始（body: `{"start":"...","end":"..."}` 必須） |
| `POST` | `/api/replay/pause` | 一時停止 |
| `POST` | `/api/replay/resume` | 再開 |
| `POST` | `/api/replay/step-forward` | EventStore の次 kline 時刻へジャンプ |
| `POST` | `/api/replay/step-backward` | EventStore の前 kline 時刻へジャンプ |
| `POST` | `/api/replay/speed` | 速度サイクル（1x→2x→5x→10x→1x） |

## アプリ API

| メソッド | パス | 用途 |
|---------|------|------|
| `POST` | `/api/app/save` | 状態をディスクに保存（saved-state.json） |
| `POST` | `/api/app/screenshot` | デスクトップ全体を `C:/tmp/screenshot.png` に保存 |

## ReplayStatus レスポンス形式

```json
// Live モード（playback なし）
{"mode":"Live","range_start":"","range_end":""}

// Replay モード（playback なし、復元直後）
{"mode":"Replay","range_start":"2026-04-10 09:00","range_end":"2026-04-10 15:00"}

// Replay モード（再生中）
{
  "mode":"Replay",
  "status":"Playing",
  "current_time":1775869740288,
  "speed":"1x",
  "start_time":1775869740000,
  "end_time":1775912940000,
  "range_start":"2026-04-11 01:09",
  "range_end":"2026-04-11 13:09"
}
```

**フィールド説明**:
- `mode`: `"Live"` or `"Replay"`
- `status`: `"Loading"` / `"Playing"` / `"Paused"` / null（playback なし時は省略）
- `current_time`: 現在の仮想時刻 (Unix ms)。playback なし時は省略
- `speed`: `"1x"` / `"2x"` / `"5x"` / `"10x"`。playback なし時は省略
- `start_time` / `end_time`: パース済み範囲 (Unix ms)。playback なし時は省略
- `range_start` / `range_end`: UI の範囲入力テキスト（常に存在）

## 追加予定 API（機能別）

### アプリ状態 `/api/app/*`

| メソッド | パス | Body | 用途 |
|---------|------|------|------|
| `GET` | `/api/app/status` | — | アプリ全体の状態（画面、テーマ、timezone 等） |
| `POST` | `/api/app/theme` | `{"theme":"..."}` | テーマ切替 |
| `POST` | `/api/app/timezone` | `{"timezone":"UTC"}` | タイムゾーン変更 |
| `POST` | `/api/app/scale` | `{"factor":1.0}` | UI スケール変更 |
| `POST` | `/api/app/trade-fetch` | `{"enabled":true}` | Trades 自動フェッチの ON/OFF |
| `POST` | `/api/app/volume-unit` | `{"unit":"Base"}` | 出来高の表示単位（Base/Quote） |

### レイアウト `/api/layout/*`

| メソッド | パス | Body | 用途 |
|---------|------|------|------|
| `GET` | `/api/layout/list` | — | レイアウト一覧 |
| `GET` | `/api/layout/active` | — | アクティブレイアウトの詳細（ペイン構成含む） |
| `POST` | `/api/layout/select` | `{"name":"..."}` | アクティブレイアウト切替 |
| `POST` | `/api/layout/create` | `{"name":"..."}` | 新規レイアウト作成 |
| `POST` | `/api/layout/delete` | `{"name":"..."}` | レイアウト削除 |
| `POST` | `/api/layout/rename` | `{"from":"...","to":"..."}` | リネーム |

### ペイン操作 `/api/pane/*`

| メソッド | パス | Body | 用途 |
|---------|------|------|------|
| `GET` | `/api/pane/list` | — | 現在のペイン一覧（種類・ティッカー・設定） |
| `POST` | `/api/pane/add` | `{"type":"KlineChart","ticker":"BinanceLinear:BTCUSDT","timeframe":"M1"}` | ペイン追加 |
| `POST` | `/api/pane/remove` | `{"id":"..."}` | ペイン削除 |
| `POST` | `/api/pane/replace` | `{"id":"...","type":"...","ticker":"..."}` | ペイン差替え |
| `POST` | `/api/pane/link-group` | `{"id":"...","group":"A"}` | リンクグループ変更 |

### 接続・データ `/api/connection/*`

| メソッド | パス | Body | 用途 |
|---------|------|------|------|
| `GET` | `/api/connection/status` | — | WebSocket 接続状態（exchange ごと） |
| `GET` | `/api/connection/streams` | — | アクティブなストリーム一覧 |

## API 追加の実装パターン

新しい API エンドポイントは `src/replay_api.rs` を拡張する形で追加する。

### 手順

1. **コマンド enum にバリアント追加**（`src/replay_api.rs`）
2. **`route()` にパスマッチ追加**
3. **`main.rs` の `Message::ReplayApi` ハンドラにケース追加**
4. **`route()` のユニットテストを `#[cfg(test)]` に追加**

### 追加する場合 vs しない場合

**追加する**: テストで以下を検証したいが API がない場合
- リプレイ: 指定時刻へのジャンプ、再生の完全停止
- アプリ設定: テーマ/TZ/スケール変更
- レイアウト: CRUD・切替
- ペイン: 一覧・追加・削除・差替え
- 接続: WebSocket 状態確認

**追加しない**:
- GUI 描画の検証（スクリーンショット回帰テストの領域）
- WebSocket ストリームの直接操作（内部実装の詳細）
- Exchange アダプターの直接テスト（統合テストで別途実施）
- ログイン/認証（立花証券のセッション管理は手動テスト or モック）
