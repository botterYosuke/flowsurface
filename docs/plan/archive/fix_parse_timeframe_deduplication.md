# 計画: `parse_timeframe` 重複排除による小文字エイリアス対応

## 背景

`tests/python/test_pane.py::test_set_timeframe_returns_dict` が HTTP 400
`{"error":"invalid timeframe: 1m"}` で失敗している。

原因は、Timeframe パース関数が 2 箇所に重複して定義されており、両者の受け付け
形式が一致していないこと。

| ファイル | 関数 | `"1m"` を受け付けるか |
| :--- | :--- | :--- |
| `src/headless.rs` | `parse_timeframe_str` | ✅ (`"M1" \| "1m"`) |
| `src/app/api/helpers.rs` | `Flowsurface::parse_timeframe` | ❌ (`"M1"` のみ) |

`/api/pane/set-timeframe` は後者を使っているため小文字形式を拒否している。
Python SDK (`python/pane.py`) と E2E スクリプトは `"1m"` を渡す仕様のため、
SDK 側からは常に 400 が返る。

## 方針

Option 2: **重複排除**。`helpers.rs::parse_timeframe` を
`headless::parse_timeframe_str` に委譲し、単一の真実源 (single source of truth)
にする。戻り値は既存シグネチャ (`Option<exchange::Timeframe>`) を維持するため、
`.ok()` で変換する。

理由:
- パース仕様が 2 箇所で分岐する事態を今後防げる（将来タイムフレームを追加した
  ときの更新漏れリスクを消す）
- `helpers.rs` の呼び出し側は Option しか使っていないので、エラーメッセージを
  捨てても実害なし（API 側は呼び出し元で `"invalid timeframe: {s}"` を自前で
  整形している）

## TDD ステップ

1. ✅ RED: `helpers.rs::parse_timeframe` に対し `"1m"`・`"5m"`・`"1h"`・`"1d"` を
   検証する単体テスト `parse_timeframe_accepts_lowercase_alias` を追加し、
   `cargo test --bin flowsurface app::api::helpers::tests` で失敗を確認
2. ✅ GREEN: `helpers.rs::parse_timeframe` の本体を
   `crate::headless::parse_timeframe_str(s).ok()` に置き換え、3 テストすべてパス
3. ✅ VERIFY(unit): `cargo test --bin flowsurface app::api::helpers::tests` ── PASS
   (3/3)。helpers.rs 自体への clippy/fmt 違反は新規発生なし（既存の別ファイル
   の警告は本タスクスコープ外）
4. ✅ E2E: headless サーバを起動し
   `pytest tests/python/test_pane.py::test_set_timeframe_returns_dict` ── PASS

## 影響範囲

- `src/app/api/helpers.rs` のみを変更する
- `parse_timeframe` を呼ぶのは `src/app/api/pane_ticker.rs::pane_api_set_timeframe`
  のみ。挙動拡張（大文字→大小文字両対応）なので既存呼び出しに回帰は発生しない
