# 立花証券セッション復元による自動ログイン

## 概要

立花証券e支店APIのログインで取得する仮想URL群（1日券）をローカルに永続化し、
アプリ再起動時にログインAPI呼び出しを省略してダッシュボードへ直行する。

## 背景

- 現状: 起動ごとにログイン画面でID/パスワードを入力 → API認証 → 仮想URL取得 → ダッシュボード
- 仮想URLは**1日間有効**。同日中の再起動ではログイン不要のはず
- 公式サンプルもこの設計（`e_api_login_tel.py` でURLをファイルに保存、他スクリプトで読み込み利用）

## API仕様（参考）

| 項目 | 値 |
|------|------|
| 仮想URL有効期間 | 1日 |
| 失効時の応答 | `p_errno = "2"` |
| 保存対象 | `url_request`, `url_master`, `url_price`, `url_event`, `url_event_ws` |

---

## 設計

### フロー

```
起動
 ├─ keyring から TachibanaSession を復元
 │    ├─ 復元成功 → セッション検証（url_price へ軽量リクエスト）
 │    │    ├─ 有効（p_errno=0）→ store_session() → ダッシュボード直行
 │    │    └─ 失効（p_errno=2）→ keyring から削除 → ログイン画面
 │    └─ 復元失敗 / 保存なし → ログイン画面（従来どおり）
 └─ ログイン成功時 → keyring にセッションを保存
```

### 保存先

既存の proxy 認証と同じ **keyring**（Windows Credential Manager）パターンを踏襲する。

| 項目 | 値 |
|------|------|
| service | `"flowsurface.tachibana"` |
| key | `"session"` |
| 値 | `TachibanaSession` の JSON 文字列 |

平文ファイルではなく keyring を使うことで、仮想URLの漏洩リスクを低減する。

### 稼働中のセッション失効への対応

ダッシュボード表示中にセッションが失効した場合（日をまたぐなど）：
- API呼び出しで `p_errno = "2"` を受け取った時点で検出
- 本改修のスコープ外（既存の `ApiError` ハンドリングで対応済み）
- 将来的にはダッシュボードから再ログインプロンプトを出すことも検討可能

---

## 改修箇所

### Step 1: TachibanaSession に Serialize/Deserialize を追加

**ファイル:** `exchange/src/adapter/tachibana.rs:45`

```rust
// before
#[derive(Debug, Clone)]
pub struct TachibanaSession { ... }

// after
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct TachibanaSession { ... }
```

exchange クレートの `Cargo.toml` には `serde` が既に依存に入っているため追加不要。

### Step 2: セッションの永続化モジュールを追加

**ファイル:** `data/src/config/tachibana.rs`（新規）

proxy.rs と同じパターンで keyring への保存/読込を実装する。

```rust
use exchange::adapter::tachibana::TachibanaSession;

const KEYCHAIN_SERVICE: &str = "flowsurface.tachibana";
const KEYCHAIN_KEY: &str = "session";

/// keyring からセッションを読み込む。
pub fn load_session() -> Option<TachibanaSession> {
    let entry = keyring::Entry::new(KEYCHAIN_SERVICE, KEYCHAIN_KEY).ok()?;
    let secret = entry.get_password().ok()?;
    serde_json::from_str(&secret).ok()
}

/// keyring にセッションを保存する。
pub fn save_session(session: &TachibanaSession) {
    let Ok(entry) = keyring::Entry::new(KEYCHAIN_SERVICE, KEYCHAIN_KEY) else { return };
    let Ok(json) = serde_json::to_string(session) else { return };
    if let Err(e) = entry.set_password(&json) {
        log::warn!("Failed to save tachibana session to keyring: {e}");
    }
}

/// keyring からセッションを削除する。
pub fn delete_session() {
    let Ok(entry) = keyring::Entry::new(KEYCHAIN_SERVICE, KEYCHAIN_KEY) else { return };
    let _ = entry.delete_credential();
}
```

**ファイル:** `data/src/config.rs` — `pub mod tachibana;` を追加。

### Step 3: セッション検証関数を追加

**ファイル:** `exchange/src/adapter/tachibana.rs`

セッションの有効性を確認する軽量関数を追加する。
url_price に対して空のリクエストを送り、p_errno で判定する。

```rust
/// 保存済みセッションの仮想URLがまだ有効か確認する。
/// 有効なら Ok(()), 失効していれば Err を返す。
pub async fn validate_session(
    client: &reqwest::Client,
    session: &TachibanaSession,
) -> Result<(), TachibanaError> {
    // 時価情報に対して存在しない銘柄コードで問い合わせ。
    // p_errno=0 ならセッション有効、p_errno=2 なら失効。
    let req = MarketPriceRequest::new(&["0000"]);
    let json_body = serialize_request(&req)?;
    let text = post_request(client, &session.url_price, &json_body).await?;
    let api_resp: ApiResponse<serde_json::Value> = serde_json::from_str(&text)?;
    // p_errno チェック — 2 なら TachibanaError::ApiError が返る
    if !api_resp.p_errno.is_empty() && api_resp.p_errno != "0" {
        return Err(TachibanaError::ApiError {
            code: api_resp.p_errno,
            message: api_resp.p_err,
        });
    }
    Ok(())
}
```

### Step 4: auth.rs にセッション保存/復元ロジックを追加

**ファイル:** `src/connector/auth.rs`

```rust
/// ログイン成功後にセッションを keyring に永続化する。
pub fn persist_session(session: &TachibanaSession) {
    data::config::tachibana::save_session(session);
}

/// keyring から保存済みセッションを復元し、有効性を検証する。
/// 有効なセッションがあれば返す。失効/未保存なら None。
pub async fn try_restore_session() -> Option<TachibanaSession> {
    let session = data::config::tachibana::load_session()?;
    let client = reqwest::Client::new();
    match exchange::adapter::tachibana::validate_session(&client, &session).await {
        Ok(()) => {
            log::info!("Restored valid tachibana session from keyring");
            // マスタキャッシュも再初期化
            let client_for_master = reqwest::Client::new();
            if let Err(e) = tachibana::init_issue_master(&client_for_master, &session).await {
                log::error!("Tachibana master download failed on restore: {e}");
            }
            Some(session)
        }
        Err(e) => {
            log::info!("Saved tachibana session expired or invalid: {e}");
            data::config::tachibana::delete_session();
            None
        }
    }
}
```

### Step 5: main.rs の起動フローを変更

**ファイル:** `src/main.rs`

#### 5-1. Message に新バリアントを追加

```rust
enum Message {
    // ... 既存 ...
    SessionRestoreResult(Option<TachibanaSession>),
}
```

#### 5-2. new() でセッション復元を試行

```rust
fn new() -> (Self, Task<Message>) {
    let saved_state = layout::load_saved_state();

    // ログインウィンドウを開く（従来通り）
    let (login_window_id, open_login_window) = window::open(/* ... */);

    // ... 既存の初期化 ...

    // セッション復元を非同期で試行
    let restore_task = Task::perform(
        connector::auth::try_restore_session(),
        Message::SessionRestoreResult,
    );

    (
        state,
        open_login_window
            .discard()
            .chain(launch_sidebar.map(Message::Sidebar))
            .chain(restore_task),
    )
}
```

#### 5-3. SessionRestoreResult ハンドラを追加

```rust
Message::SessionRestoreResult(result) => {
    match result {
        Some(session) => {
            // セッション有効 → LoginCompleted(Ok(session)) と同じ処理
            connector::auth::store_session(session);
            // ログインウィンドウを閉じてダッシュボードを開く
            // （既存の LoginCompleted::Ok と同じロジックを呼ぶ）
            return self.transition_to_dashboard();
        }
        None => {
            // 復元失敗 → ログイン画面のまま（何もしない）
        }
    }
    Task::none()
}
```

#### 5-4. LoginCompleted に保存処理を追加

既存の `Message::LoginCompleted(Ok(session))` ハンドラ内で、`store_session()` の直後にセッションの永続化を追加：

```rust
Ok(session) => {
    connector::auth::store_session(session.clone());
    connector::auth::persist_session(&session);  // ← 追加
    // ... 既存のダッシュボード遷移 ...
}
```

#### 5-5. ダッシュボード遷移を共通メソッドに抽出

`LoginCompleted(Ok)` と `SessionRestoreResult(Some)` で同じ遷移処理を使うため、
ログインウィンドウを閉じてメインウィンドウを開くロジックを `fn transition_to_dashboard()` に切り出す。

### Step 6: ログアウト時のクリーンアップ

将来ログアウト機能を追加する場合に備え、`clear_session()` 内で keyring も削除する。

**ファイル:** `src/connector/auth.rs`

```rust
pub fn clear_session() {
    if let Ok(mut guard) = SESSION.write() {
        *guard = None;
    }
    data::config::tachibana::delete_session();
}
```

---

## UX

| シナリオ | 挙動 |
|----------|------|
| 初回起動 / セッション未保存 | 通常のログイン画面 |
| 同日中の再起動 | ログイン画面が一瞬表示 → 自動でダッシュボードへ遷移 |
| 翌日の起動（セッション失効） | ログイン画面が表示される（失敗ログが出る） |
| ネットワークエラー | 検証リクエスト失敗 → ログイン画面にフォールバック |

復元チェック中はログイン画面に「セッション復元中...」などの表示があると良い（任意）。

---

## 変更ファイル一覧

| ファイル | 変更内容 |
|----------|----------|
| `exchange/src/adapter/tachibana.rs` | `TachibanaSession` に Serialize/Deserialize 追加、`validate_session()` 追加 |
| `data/src/config/tachibana.rs` | **新規** — keyring への保存/読込/削除 |
| `data/src/config.rs` | `pub mod tachibana;` 追加 |
| `src/connector/auth.rs` | `persist_session()`, `try_restore_session()` 追加、`clear_session()` に keyring 削除追加 |
| `src/main.rs` | `SessionRestoreResult` メッセージ追加、起動時の復元タスク、`transition_to_dashboard()` 抽出 |

## テスト方針

| テスト | 内容 |
|--------|------|
| `TachibanaSession` の JSON ラウンドトリップ | serialize → deserialize で全フィールドが一致 |
| `validate_session` の成功/失効パス | mockito で p_errno=0 / p_errno=2 を返す |
| `try_restore_session` の統合テスト | keyring mock は困難なので手動確認を中心とする |
| 既存テストの不退行 | `cargo test` で全テスト PASS を確認 |