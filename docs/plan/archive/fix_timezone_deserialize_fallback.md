# `UserTimezone` 不正値フォールバック修正計画

## 1. 背景と現象

E2E 検証中、`$APPDATA/flowsurface/saved-state.json` の `timezone` フィールドに
`"Asia/Tokyo"` 等の未対応文字列を入れると、**サイレントに全設定がロスト**する
現象が判明した（layout / replay / pane 構成すべて）。

具体的な流れ:

1. `data::read_from_file()`
   ([data/src/lib.rs:66](../../data/src/lib.rs#L66)) が
   `serde_json::from_str::<State>()` を呼ぶ。
2. `UserTimezone::deserialize`
   ([data/src/config/timezone.rs:104-116](../../data/src/config/timezone.rs#L104-L116))
   が未知文字列に対し `Err` を返す。
3. `State` 全体の deserialize が失敗。
4. `read_from_file()` が `saved-state.json` を `saved-state_old.json` に rename
   ([data/src/lib.rs:81](../../data/src/lib.rs#L81))。
5. アプリは `State::default()` で起動 →
   ペイン無し・Live モードの空の画面。
6. ユーザーは原因不明の「設定消失」として体感する。

## 2. 根本原因

`State` struct には `#[serde(default)]`
([data/src/config/state.rs:36-38](../../data/src/config/state.rs#L36-L38))
が付いているが、serde の `#[serde(default)]` は
**「フィールドが欠損したとき」** に default を使う機能であって、
**「値は存在するが deserialize がエラーを返したとき」** のフォールバックには
ならない。

従って、`UserTimezone::deserialize` が `Err` を返した瞬間、
**エラーは `State` ルートまで伝播して** deserialize 全体が失敗する。
結果、timezone 1 フィールドの不正値を契機に、他の全フィールドが
`old` ファイルに退避されてしまう。

## 3. 修正方針

### 採用案: `UserTimezone::Deserialize` をトレラントに書き換える（Option A）

`UserTimezone::deserialize` を、未知値に対してエラーを返さず
**`UserTimezone::default()` (= `Utc`) にフォールバックし `log::warn!` を出す**
実装に変更する。

#### 採用理由

- **最小侵襲**: 修正点は timezone.rs の 1 箇所のみ。`State` struct や
  `serde` アノテーションには手を入れない。
- **効果範囲が明確**: `UserTimezone` を deserialize するあらゆる経路
  （現状は `State.timezone` のみだが将来増える可能性もある）で一貫した
  挙動になる。
- **警告ログで検知可能**: warn ログが出るので、ユーザーは
  アプリログから原因に辿り着ける。
- **`State` 全体のロスを防げる**: 受け入れ条件 #1
  （layout / replay が温存されること）を直接満たす。

### 不採用案: `State.timezone` に `#[serde(deserialize_with = ...)]` を付ける（Option B）

- メリット: `UserTimezone::deserialize` 自体は strict なまま保てる。
- デメリット: `State` 側で serde glue 関数を持つ必要があり、
  `Option<UserTimezone>` を介した回りくどい実装になりがち。
  また、将来 `UserTimezone` を別の場所で deserialize するときに
  同じガードを再実装する羽目になる。
- 今回は **saved-state.json のデータ保護が目的** で、
  `UserTimezone` そのものに厳密な契約を持たせる必要性は低いため、
  Option A を優先する。

## 4. 実装手順

### 4.1 `data/src/config/timezone.rs` の修正

現在:

```rust
impl<'de> Deserialize<'de> for UserTimezone {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where D: serde::Deserializer<'de> {
        let timezone_str = String::deserialize(deserializer)?;
        match timezone_str.to_lowercase().as_str() {
            "utc" => Ok(UserTimezone::Utc),
            "local" => Ok(UserTimezone::Local),
            _ => Err(serde::de::Error::custom("Invalid UserTimezone")),
        }
    }
}
```

修正後:

```rust
impl<'de> Deserialize<'de> for UserTimezone {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where D: serde::Deserializer<'de> {
        let timezone_str = String::deserialize(deserializer)?;
        match timezone_str.to_lowercase().as_str() {
            "utc" => Ok(UserTimezone::Utc),
            "local" => Ok(UserTimezone::Local),
            _ => {
                ::log::warn!(
                    "Unknown UserTimezone value {:?}; falling back to default ({:?}). \
                     Supported values: \"UTC\", \"Local\".",
                    timezone_str,
                    UserTimezone::default(),
                );
                Ok(UserTimezone::default())
            }
        }
    }
}
```

要点:

- `String::deserialize` が失敗した場合（型が文字列でない場合）は
  引き続きエラーを返す（受け入れ条件 #4「既存挙動を変えない」の
  趣旨: 型レベルの壊れには従来通りのエラーを維持）。
  - ただし、型不一致も同様にサイレントロスを起こすため、
    追加対応するかは **§6 フォローアップ** で議論する。
- `::log` クレートは既に `data/src/lib.rs:23` で使用されており、
  `data/Cargo.toml` に依存があるはず（要確認）。
- フォーマット文字列はユーザーがログから原因に辿り着ける
  具体的なメッセージにする。

### 4.2 依存の確認

`data/Cargo.toml` に `log` クレート依存があることを確認する。
無ければ追加する（`lib.rs` で `::log::warn` が既に使われているので
おそらく存在する）。

### 4.3 ユニットテスト追加

`data/src/config/state.rs` の `#[cfg(test)] mod tests` に以下 2 件を追加:

```rust
#[test]
fn state_with_invalid_timezone_falls_back_to_default() {
    // "Asia/Tokyo" のような IANA タイムゾーン名が入っても
    // State 全体の deserialize は成功し、timezone は default になる。
    let json = r#"{"timezone":"Asia/Tokyo"}"#;
    let state: State = serde_json::from_str(json).unwrap();
    assert_eq!(state.timezone, UserTimezone::default());
}

#[test]
fn state_with_invalid_timezone_preserves_other_fields() {
    // timezone が壊れていても、layout / replay など他フィールドが
    // 巻き添えでロストしないことを確認。
    let json = r#"{
        "timezone":"Asia/Tokyo",
        "replay":{
            "mode":"replay",
            "range_start":"2026-04-10 09:00",
            "range_end":"2026-04-10 15:00"
        }
    }"#;
    let state: State = serde_json::from_str(json).unwrap();
    assert_eq!(state.timezone, UserTimezone::default());
    assert_eq!(state.replay.mode, "replay");
    assert_eq!(state.replay.range_start, "2026-04-10 09:00");
    assert_eq!(state.replay.range_end, "2026-04-10 15:00");
}
```

注: `UserTimezone` に `PartialEq` が必要。既に
[data/src/config/timezone.rs:5](../../data/src/config/timezone.rs#L5)
で `#[derive(Debug, Clone, Copy, PartialEq, Default)]` が
付いているので追加対応は不要。

### 4.4 既存テストの回帰確認

次のコマンドで既存の `cargo test -p data` が全 PASS することを確認する:

```sh
cargo test -p data
```

特に注意すべき既存テスト:

- `replay_config_*` 系
  ([data/src/config/state.rs:90-257](../../data/src/config/state.rs#L90))
- `state_*` 系（`state_empty_json_deserializes_with_all_defaults` 等）
- `UserTimezone` を使う他モジュール（grep 結果次第）

### 4.5 スキルドキュメントの更新

修正完了後、以下のドキュメントを「不明値は warn ログ + Utc にフォールバック」
の内容に更新する:

- [.claude/skills/e2e-test/fixtures.md](../../.claude/skills/e2e-test/fixtures.md)
- [.claude/skills/e2e-test/SKILL.md](../../.claude/skills/e2e-test/SKILL.md)

具体的には、現在の「timezone は UTC/Local のみ」という警告を以下に差し替え:

> `timezone` フィールドは `"UTC"` または `"Local"` のみ受け付ける。
> それ以外の値（例: `"Asia/Tokyo"` 等の IANA タイムゾーン名）を指定した場合、
> warn ログを出して `UTC` にフォールバックし、他フィールドは温存される。
> ただし E2E の fixture では原則として `"UTC"` を明示すること。

## 5. 受け入れ条件チェックリスト

| # | 条件 | 満たし方 |
|---|------|----------|
| 1 | `"Asia/Tokyo"` を含む state.json でも layout / replay が温存される | §4.1 修正 + §4.3 新規テスト 2 |
| 2 | timezone は default 値（Utc）にフォールバック | §4.1 修正 + §4.3 新規テスト 1 |
| 3 | warn ログが出る | §4.1 の `::log::warn!` |
| 4 | 既存の "UTC" / "Local" の挙動は変えない | §4.1 の match 分岐は変更なし |
| 5 | 既存 `cargo test -p data` が全 PASS | §4.4 で確認 |
| 6 | 新規ユニットテスト 2 件追加 | §4.3 |

## 6. フォローアップ（スコープ外・後続検討）

以下は本修正のスコープ外だが、同根の問題として将来検討する価値あり:

- **型レベルの壊れへの対応**:
  `"timezone": 42`（数値）のような型違いでも現状はサイレントロスする。
  `serde::Deserializer::deserialize_any` + `Visitor` パターンで
  任意の型入力に対してもフォールバックする実装は可能だが、実装コスト高く、
  今回の E2E で判明した「IANA 名を入れたい誘惑」というユースケースには
  オーバースペック。今回は見送り。

- **フィールド単位のフォールバック機構**:
  `State` の他フィールド（`selected_theme`, `proxy_cfg` 等）にも
  同様の脆弱性がある可能性。本修正で `timezone` だけを直すのは応急処置で、
  恒久的には `serde(deserialize_with = ...)` + `Option` フォールバックの
  汎用ヘルパーを用意するのが望ましい。別 issue として切り出す。

- **`read_from_file` 側での段階的リカバリ**:
  現状は「全部失敗 → rename」か「全部成功 → Ok」の二択。
  `serde_json::Value` 経由で partial parse するフォールバックがあれば、
  どの fields も取り漏らさない。これは大改修になるので別計画。

## 7. リスクとトレードオフ

- **リスク**: タイポで `"Utx"` 等を入れてしまったケースも警告だけで
  通過するようになるため、「設定が効いていない」と気付きにくくなる可能性。
  → warn ログで検知可能。アプリケーションログを見る運用で緩和。

- **トレードオフ**: `UserTimezone` 型の契約が
  「strict enum」から「fallible enum with fallback」に変わる。
  ただし現状 deserialize 元は saved-state.json のみなので、
  型契約が緩む影響範囲は限定的。
