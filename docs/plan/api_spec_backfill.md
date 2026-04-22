# API 仕様書バックフィル計画

## 背景

`src/replay_api.rs` に実装済みだが `docs/spec/*` に記載がない HTTP API が複数存在する。
仕様書と実装の乖離を解消し、エージェント/クライアント実装者が `docs/spec` のみで完結できる状態にする。

## 対象エンドポイント

### 共通系 (→ `docs/spec/replay.md`)
- ✅ `POST /api/app/set-mode` — replay/live モード切替
- ✅ `POST /api/auth/tachibana/logout` — Tachibana セッション破棄
- ✅ `GET  /api/pane/chart-snapshot?pane_id=...[&limit=&since_ts=]` — チャート OHLC スナップショット

### Tachibana 実発注系 (→ `docs/spec/tachibana.md` §3.11)
- ✅ `GET  /api/buying-power` — 買付余力取得
- ✅ `POST /api/tachibana/order` — 新規注文
- ✅ `GET  /api/tachibana/orders[?eig_day=YYYYMMDD]` — 注文一覧
- ✅ `GET  /api/tachibana/order/:order_num[?eig_day=]` — 約定明細
- ✅ `POST /api/tachibana/order/correct` — 訂正
- ✅ `POST /api/tachibana/order/cancel` — 取消
- ✅ `GET  /api/tachibana/holdings?issue_code=XXXX` — 保有現物

### テスト用 (→ `docs/spec/replay.md` §11.2 「E2E テスト専用」)
- ✅ `POST /api/test/tachibana/delete-persisted-session`（他の inject 系含む、debug ビルド限定）

## 結果

- `docs/spec/replay.md` §11.2 に 認証 (logout) / アプリ制御 (set-mode) / ペイン (chart-snapshot) / 立花証券実発注クロスリファレンス / テスト専用エンドポイント の各節を追加
- `docs/spec/tachibana.md` §3.11 に HTTP API ファサード（全 7 エンドポイント）の request/response JSON 例を追加
- `docs/spec/replay.md` §11.6 に set-mode / chart-snapshot / logout の curl 例を追加
- `src/replay_api.rs` の `route()` 全 match アームが `docs/spec/*` のどこかで説明されている状態を達成

## 手順

1. 実装（`src/replay_api.rs` のパーサ + ハンドラ + レスポンス書き出し箇所）を読み取り、エンドポイントごとに request/response JSON schema を確定する
2. `docs/spec/replay.md` の「HTTP API」節に共通系・テスト系を追記
3. `docs/spec/tachibana.md` に新節「HTTP API」を追加
4. 各スペックに curl 例を 1 本添える（既存節との粒度を揃える）

## 完了条件

- `src/replay_api.rs` の `route()` 内の全 match アームが `docs/spec/*` のどこかで説明されている
- 仕様書から request/response の型が一意に再現できる
