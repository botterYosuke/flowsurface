# 計画書: 未カバーのユーザー操作 E2E テスト追加

**作成日**: 2026-04-15
**ブランチ**: sasa/develop

---

## 背景

`docs/replay_header.md` に記載されたユーザー操作のうち、`tests/e2e_scripts/` でカバーされていない操作が以下の通り判明した。

| # | 未カバー操作 | 仕様箇所 | 備考 |
|---|---|---|---|
| 1 | `POST /api/sidebar/select-ticker` | §11.2 | `set-ticker` とは別フロー（Sidebar::TickerSelected 経路） |
| 2 | `POST /api/app/screenshot` | §11.2 | ファイル保存を含む動作未確認 |
| 3 | `GET /api/auth/tachibana/status` — session=none ケース | §11.2 | present ケースは s5 のみ |
| 4 | F5 キーバインドによるトグル | §3.3 | HTTP API で再現不可 → 対象外 |
| 5 | StartTimeChanged / EndTimeChanged（clock が Some のとき）| §6.6 | HTTP API で再現不可 → 対象外 |

操作 4, 5 は HTTP API に対応エンドポイントが存在しないため、bash スクリプトによる E2E テストは不可能。

---

## 追加するスクリプト

### s24_sidebar_select_ticker.sh

`POST /api/sidebar/select-ticker` の 2 経路を検証する。

| TC | 操作 | 期待 |
|---|---|---|
| TC-A | Replay Paused 中に kind=null で ticker 変更 | HTTP 200、pane/list の ticker が変わる |
| TC-B | Replay Playing 中に kind=null で ticker 変更 | status が Paused になる |
| TC-C | Playing 中変更後 Resume → Playing 復帰 | status=Playing |
| TC-D | kind="KlineChart" を指定 | HTTP 200、エラーなし |
| TC-E | 不正な pane_id | HTTP 400 |
| TC-F | ticker フィールド欠落 | HTTP 400 |

### s25_screenshot_and_auth.sh

`POST /api/app/screenshot` と `GET /api/auth/tachibana/status`（session=none ケース）を検証する。

| TC | 操作 | 期待 |
|---|---|---|
| TC-A | POST /api/app/screenshot | HTTP 200、`{"ok":true}` |
| TC-B | C:/tmp/screenshot.png が存在する | ファイル確認 |
| TC-C | Replay 再生中でも screenshot が動作する | `{"ok":true}` |
| TC-D | GET /api/app/screenshot（誤メソッド）| HTTP 404 |
| TC-E | GET /api/auth/tachibana/status（Binance-only）| HTTP 200、session="none" |
| TC-F | session フィールドが存在する | スキーマ確認 |

---

## 進捗

- ✅ 計画書作成
- ✅ `tests/e2e_scripts/s24_sidebar_select_ticker.sh` 作成
- ✅ `tests/e2e_scripts/s25_screenshot_and_auth.sh` 作成
- ✅ 計画書に完了マークを付ける
