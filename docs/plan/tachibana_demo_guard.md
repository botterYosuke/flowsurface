# 計画: 立花証券デモ口座ログイン検知ガードの追加

## 背景

flowsurface は Rust 製デスクトップアプリで、ローカルポート 9876 で HTTP API を公開している。
テスト群のうち、立花証券 API を実際に叩く `POST /api/tachibana/order` 系エンドポイントは、
誤って本番口座でテストを走らせると **実口座で発注が飛ぶ** 重大リスクがある。

現状ガード状況:

1. **アプリ未起動 → skip**: `tests/python/conftest.py` で対応済み
2. **デモ口座以外 → skip**: 未対応 ← 本タスクで追加

## 現状の API レスポンス

`GET /api/auth/tachibana/status` は現在 `{"session": "present" | "none"}` のみで、
デモ/本番判定に必要な情報を返していない。

## 方針（案 A: 恒久対応）

`TachibanaSession` に `is_demo: bool` を持たせ、`/api/auth/tachibana/status` のレスポンスに
`environment: "demo" | "prod"` を追加する。Python テスト側は専用マーカー `@pytest.mark.tachibana_demo`
を導入し、マーカー付きテストのみを「デモログイン中でなければ skip」対象にする。

`#[serde(default)]` を付けることで既存の永続化セッション (keyring) との後方互換性を維持する。
未設定値は `false`（本番扱い）にフォールバックするため、安全側に倒れる。

## TDD ステップ

### タスク 1: Rust 側 API 拡張

1. RED: `TachibanaSession` の `is_demo` フィールドが存在することを検証する単体テストを
   `exchange/src/adapter/tachibana.rs` に追加（インスタンス生成時に `is_demo: true` を渡せること、
   `serde_json` で `"is_demo"` フィールドを含むことを確認）
2. GREEN: `TachibanaSession` に `pub is_demo: bool` を `#[serde(default)]` 付きで追加
3. REFACTOR: 既存テストで `TachibanaSession {...}` を構築している箇所が壊れないか確認し、
   必要なら `is_demo: false` を補う（`#[serde(default)]` で deserialize 側は OK だが
   構造体リテラルは明示が必要）

4. RED: `perform_login(.., is_demo)` 呼び出しの結果セッションに `is_demo` が反映される
   ことを検証する単体テストを `src/connector/auth.rs` に追加
5. GREEN: `perform_login_with_base_url` の戻り値に `is_demo` を上書き設定
   （`exchange::adapter::tachibana::login` の戻り値は `is_demo` を知らないため、呼び出し側で設定）

6. RED: `handle_auth_api(TachibanaSessionStatus)` のレスポンスに `environment` フィールドが
   含まれることを検証する単体テストを `src/app/api/mod.rs` の `#[cfg(test)]` モジュールに追加
   - 未保存 → `{"session": "none"}`
   - デモ保存 → `{"session": "present", "environment": "demo"}`
   - 本番保存 → `{"session": "present", "environment": "prod"}`
7. GREEN: `handle_auth_api` の `TachibanaSessionStatus` 分岐を更新

### タスク 2: Python テスト側ガード

8. `tests/python/conftest.py` に以下を追加:
   - `pytest_configure(config)` で `tachibana_demo` マーカー登録
   - `pytest_collection_modifyitems(config, items)` を拡張し、`tachibana_demo` マーカー付き
     テストには `_demo_session_active()` ヘルパで判定し、デモ以外なら skip マーカー付与

### タスク 3: サンプルテスト

9. `tests/python/test_tachibana_order.py` を新規作成し、`@pytest.mark.tachibana_demo` 付きの
   `test_order_list_returns_dict()` を 1 件追加（`GET /api/tachibana/orders` の dict 確認のみ）

## 完了条件

- [x] `cargo fmt --check` pass（fmt 適用済み）
- [x] `cargo test` pass（lib 391 + data 26 + exchange 147、全 pass）
- [ ] `cargo clippy -- -D warnings` — 既存 pre-existing warnings あり（本タスク非対象）
- [x] `GET /api/auth/tachibana/status` のレスポンスに `environment` を含む
- [x] `uv run pytest tests/python/ --collect-only` 30 件 collected
- [x] アプリ未起動: `test_tachibana_order.py` skip 動作確認
- [ ] アプリ起動 + デモログイン: `test_tachibana_order.py` pass（手動検証必要）
- [ ] アプリ起動 + 本番ログイン: `test_tachibana_order.py` skip（手動検証必要）
- [x] 新規ユニットテスト追加（exchange 3 件、connector::auth 2 件、app::api 3 件）

## コミット分割

1. `feat(tachibana): TachibanaSession に is_demo フィールドを追加`
2. `feat(api): /api/auth/tachibana/status に environment フィールドを追加`
3. `test(python): tachibana_demo マーカーとサンプルテストを追加`

## 進捗

- [x] タスク 1: Rust 側 API 拡張 ✅
- [x] タスク 2: Python テスト側ガード ✅
- [x] タスク 3: サンプルテスト ✅
