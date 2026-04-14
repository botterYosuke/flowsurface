---
name: Replay fixture 直接起動対応
description: saved-state.json に replay 構成を含めた状態での直接起動（自動 Play）を可能にする設計
type: project
---

# Replay fixture 直接起動対応 設計プラン

**作成日**: 2026-04-13
**対象**: [src/main.rs](../../src/main.rs), [src/replay/mod.rs](../../src/replay/mod.rs), [.claude/skills/e2e-test/SKILL.md](../../.claude/skills/e2e-test/SKILL.md)
**状態**: 実装完了
**関連**: [replay_bar_step_loop.md](./replay_bar_step_loop.md)

## 目的

E2E テスト（および通常利用）で **`saved-state.json` に `replay` フィールドを含めた fixture から直接起動** できるようにする。
現状は以下のワークアラウンドを強制されている（[SKILL.md:107-113](../../.claude/skills/e2e-test/SKILL.md#L107-L113)）：

> Live モード fixture で起動 → 15s 待機 → `POST /api/replay/toggle` → `POST /api/replay/play`

このプランでは上記 4 ステップを **「Replay fixture を置いて起動するだけ」** に短縮する。

## 現状の挙動トレース

`replay_config.mode = "replay"` を含む saved-state.json で起動した場合：

1. [src/main.rs:211-226](../../src/main.rs#L211-L226) で `ReplayState { mode: Replay, range_input: {start,end}, clock: None, .. }` が構築される
2. 各ペインの `streams` は [data/src/config/state.rs](../../data/src/config/state.rs) 経由で `ResolvedStream::Waiting { streams: Vec<PersistStreamKind>, .. }` として復元される（[src/connector/stream.rs:10-18](../../src/connector/stream.rs#L10-L18)）
3. `Message::Tick` が 16ms ごとに届き、各ペインが `due_streams_to_resolve(now)` を返す
4. `Dashboard::Event::ResolveStreams` が [src/main.rs:495-544](../../src/main.rs#L495-L544) で処理され、`sidebar.tickers_info()` が埋まった時点で `ResolvedStream::Ready(Vec<StreamKind>)` に昇格する
5. **ここでアプリは停止**。Replay モード UI は表示されているが `clock = None`、EventStore も空
6. ユーザー（または E2E クライアント）が明示的に `POST /api/replay/play` を叩いて初めて `ReplayMessage::Play` → `prepare_replay()` → `ready_iter()` で kline_targets を収集 → EventStore 構築という正常フローに入る

**問題はステップ 5 の停止**。起動直後に Play を叩いても、ステップ 4 より前（streams が Waiting のまま）だと `prepare_replay()` の [dashboard.rs:1303](../../src/screen/dashboard.rs#L1303) で `ready_iter() = None` となり kline_targets が空になる。
これが SKILL.md の「15s 待機必須」の正体である。

## 検討した方針

### 方針 A（採用）: 起動時の auto-play

**「Replay 構成付きで起動した場合、全ペインの streams が Ready になった瞬間に `ReplayMessage::Play` を自動発火する」** という boot intent を 1 つ追加する。

- `ReplayState` に transient フィールド `pending_auto_play: bool` を追加（永続化しない）
- 起動時、`replay_config.mode == "replay"` かつ `range_start` / `range_end` が有効なら `pending_auto_play = true`
- `Dashboard::Event::ResolveStreams` 処理完了後にゲート判定（全ペイン Ready？ replay.pending_auto_play？）→ true なら `Message::Replay(ReplayMessage::Play)` を dispatch して flag を下ろす

**利点**
- 既存の `prepare_replay()` / `start()` / load パスを**一切変更しない**。正常フローに乗せるだけ
- 「UI で Play ボタンを押す」のと完全等価になるので挙動・テストカバレッジの差分がない
- Ticker metadata の取得という非同期ワークの完了を正しく待つので、race condition が起きない
- E2E フロー: ワークアラウンドの `POST /replay/toggle` と `POST /replay/play` の 2 リクエスト + 15s 待機が丸ごと消える

**欠点**
- boot 時に自動で副作用が発生する（kline fetch が始まる）。ただしこれは E2E fixture の意図と一致
- 通常ユーザが `replay` 構成を保存した状態で起動した場合も自動再生が始まる
  → **これはむしろ望ましい**（SKILL.md の `Toggle→Live 時に range_input がリセットされる` 注記 [SKILL.md:158-161](../../.claude/skills/e2e-test/SKILL.md#L158-L161) を踏まえると、Replay 構成のまま保存した＝再生したかったと解釈するのが自然）

### 方針 B（却下）: prepare_replay を Waiting 対応に拡張

`prepare_replay()` 内で `ResolvedStream::Waiting { streams, .. }` からも `PersistStreamKind::into_stream_kinds(resolver)` を呼んで kline_targets を抽出する案。

**却下理由**
- `into_stream_kinds(resolver)` は `sidebar.tickers_info()` を必要とするが、`Dashboard` は sidebar への参照を持たない（層が離れている）
- 起動直後（tickers_info が空）の Play を成功させるには結局「解決完了を待つ」ロジックが必要 → 方針 A のゲートと実質同じになる
- `prepare_replay()` の責務が膨らみ、State mutation と resolver 注入が絡んで単体テストが難しくなる

方針 A は「既存の解決パスを待って、解決完了をトリガーに発火する」という形で、副作用の置き場所が main.rs の 1 箇所に収まる。

## 新設計

### 型変更

```rust
// src/replay/mod.rs
pub struct ReplayState {
    pub mode: ReplayMode,
    pub range_input: ReplayRangeInput,
    pub clock: Option<StepClock>,
    pub event_store: EventStore,
    pub active_streams: HashSet<StreamKind>,
    /// 起動時 fixture 復元の結果として次の「全ペイン Ready」で Play を発火する。
    /// 一度発火したら false に戻す。永続化しない。
    pub pending_auto_play: bool,
    /// auto-play の発火期限。超過したらタイムアウトとして flag を下ろす。
    pub pending_auto_play_deadline: Option<Instant>,
}
```

`Default::default()` は `pending_auto_play: false`、`pending_auto_play_deadline: None`。
`toggle_mode()` の Replay→Live 経路でも両フィールドをリセット（取り残し防止）。

### 起動時の flag セット

[src/main.rs:211-226](../../src/main.rs#L211-L226) の ReplayState 初期化で：

```rust
replay: {
    let replay_mode = match saved_state.replay_config.mode.as_str() {
        "replay" => replay::ReplayMode::Replay,
        _ => replay::ReplayMode::Live,
    };
    // 文字列が空かどうかだけでなく、パース可能かも検証する
    let has_valid_range = replay::parse_replay_range(
        &saved_state.replay_config.range_start,
        &saved_state.replay_config.range_end,
    ).is_ok();
    let pending_auto_play = replay_mode == replay::ReplayMode::Replay && has_valid_range;
    ReplayState {
        mode: replay_mode,
        range_input: replay::ReplayRangeInput {
            start: saved_state.replay_config.range_start,
            end: saved_state.replay_config.range_end,
        },
        clock: None,
        event_store: replay::store::EventStore::new(),
        active_streams: HashSet::new(),
        pending_auto_play,
        pending_auto_play_deadline: pending_auto_play
            .then(|| Instant::now() + Duration::from_secs(30)),
    }
}
```

`parse_replay_range` で事前バリデーションすることで、フォーマット不正な文字列が `pending_auto_play = true` になって Play が失敗する状態を防ぐ。

### 発火ゲート

`ResolveStreams` ハンドラ [src/main.rs:525-544](../../src/main.rs#L525-L544) の `Ok(resolved)` 分岐で、resolved streams を dashboard に注入した後に判定する。

**注意**: `dashboard.resolve_streams()` は `&mut self`（dashboard）の借用を必要とするため、その後に `self.active_dashboard()` を再借用しようとするとコンパイルエラーになる。`resolve_streams` の戻り値 `Task<Message>` は dashboard への参照を含まないため、Rust の NLL により `resolve_task` 生成後に借用は解放される。ただし `dashboard` が let 束縛として同スコープに残るため、re-borrow を確実にするには別スコープに切り出す：

```rust
Ok(resolved) => {
    if resolved.is_empty() {
        return Task::none();
    }

    // (1) pane を Ready に昇格させる（resolve_streams は同期的に state を更新する）
    let resolve_task = {
        let dashboard = self.active_dashboard_mut();
        dashboard
            .resolve_streams(main_window.id, pane_id, resolved)
            .map(move |msg| Message::Dashboard { layout_id: None, event: msg })
    }; // ← ここで dashboard の借用が解放される

    // (2) 全ペイン Ready かつ pending_auto_play が立っているか判定
    if self.replay.pending_auto_play
        && self.replay.is_replay()
        && self.active_dashboard().all_panes_have_ready_streams(main_window.id)
    {
        self.replay.pending_auto_play = false;
        self.replay.pending_auto_play_deadline = None;
        let play_task = Task::done(Message::Replay(ReplayMessage::Play));
        Task::batch([resolve_task, play_task])
    } else {
        resolve_task
    }
}
```

`resolve_streams` は [dashboard.rs:1199-1202](../../src/screen/dashboard.rs#L1199-L1202) で `state.streams = ResolvedStream::Ready(...)` を同期的に更新してから `Task` を返すため、スコープを抜けた後の `all_panes_have_ready_streams()` チェックは最新の状態を参照できる。

**注意**: `resolved.is_empty()` の早期リターンにより、empty バッチは gate チェックをスキップする。これで十分な理由：pane の stream が空解決になった場合、その pane は Ready に昇格しないので `all_panes_have_ready_streams()` が true になることはなく、30s タイムアウトで適切に処理される。

### `all_panes_have_ready_streams` の実装

[src/screen/dashboard.rs](../../src/screen/dashboard.rs) に追加（`iter_all_panes` の immutable 版は [dashboard.rs:580-590](../../src/screen/dashboard.rs#L580-L590) に既存）：

```rust
pub fn all_panes_have_ready_streams(&self, main_window: window::Id) -> bool {
    self.iter_all_panes(main_window)
        .all(|(_, _, state)| match &state.streams {
            // stream が設定されていないペイン（空 Waiting）は Ready 扱いで無視
            ResolvedStream::Waiting { streams, .. } => streams.is_empty(),
            // stream 設定あり → Ready になっているか
            ResolvedStream::Ready(_) => true,
        })
}
```

**空ペインの扱い**: `Waiting { streams: vec![] }` は ResolveStreams イベントが発火しないため永久に Waiting のままとなる。これらを `true` 扱いすることで gate がブロックされない。

### 「全ペインが Ready」の定義

| ペインの種別 | `Waiting.streams` | 判定 |
|---|---|---|
| stream 未設定（空ペイン） | `[]` | OK（無視） |
| Trades-only ペイン | non-empty | Ready になるまで待つ |
| kline ペイン | non-empty | Ready になるまで待つ |
| 解決失敗ペイン（不正 ticker 等） | non-empty → 永久 Waiting | 30s タイムアウトで諦める |
| Waiting でリトライ中 | non-empty | Ready になるまで待つ |

### タイムアウト処理

[src/main.rs:347](../../src/main.rs#L347) の `Message::Tick(now)` ハンドラ先頭で：

```rust
if self.replay.pending_auto_play {
    if let Some(deadline) = self.replay.pending_auto_play_deadline {
        if now >= deadline {
            self.replay.pending_auto_play = false;
            self.replay.pending_auto_play_deadline = None;
            self.notifications.push(Toast::error(
                "Replay auto-play timed out: streams did not resolve within 30s"
            ));
        }
    }
}
```

## 実装ステップ

### ✅ Phase 1: ReplayState に pending_auto_play を追加

1. [src/replay/mod.rs](../../src/replay/mod.rs) の `ReplayState` 構造体に `pending_auto_play: bool` と `pending_auto_play_deadline: Option<Instant>` を追加
2. `Default` impl で両フィールドを `false` / `None` に
3. `toggle_mode()` Replay→Live 分岐で両フィールドをリセット
4. ユニットテスト追加:
   - `default_state_has_no_pending_auto_play` ✅
   - `toggle_replay_to_live_clears_pending_auto_play` ✅

### ✅ Phase 2: 起動時 flag セット

5. [src/main.rs](../../src/main.rs) の ReplayState 初期化で、`parse_replay_range` が成功かつ `replay_mode == Replay` のとき `pending_auto_play = true`、`pending_auto_play_deadline = Some(Instant::now() + Duration::from_secs(30))` とする

### ✅ Phase 3: dashboard.rs にヘルパー追加

6. [src/screen/dashboard.rs](../../src/screen/dashboard.rs) に `all_panes_have_ready_streams(window::Id) -> bool` を追加（`iter_all_panes` を使用）
   - ユニットテスト追加: `all_panes_have_ready_streams_true_for_default_dashboard` / `..._false_when_pane_has_non_empty_waiting_streams` ✅

### ✅ Phase 4: ResolveStreams ハンドラでの発火

7. [src/main.rs](../../src/main.rs) の `Ok(resolved)` 処理を「スコープ分割 → resolve_task 生成 → gate 判定 → batch or 単発」のパターンに書き直す
8. 発火時は `pending_auto_play = false`、`pending_auto_play_deadline = None` をリセット

### ✅ Phase 5: タイムアウト処理

9. [src/main.rs](../../src/main.rs) の `Message::Tick(now)` ハンドラ先頭で deadline チェックを追加

### ✅ Phase 6: E2E 検証と SKILL.md 更新

10. `e2e-test` スキルの scenarios.md に「カテゴリ 4: Replay fixture 直接起動（auto-play）」シナリオを追加 ✅
11. [SKILL.md](../../.claude/skills/e2e-test/SKILL.md) の「重要な注意点」を更新済み（auto-play 対応説明に差し替え）✅
12. `fixtures.md` に「Replay 直接起動 fixture」テンプレート（#2, #5）が既存 ✅

## トレードオフ / リスク

### ✓ 得るもの

- E2E テストの待機時間 15s 削減、リクエスト数 2 削減（toggle + play が不要）
- Fixture 数の整理：Live 起動用と Replay 起動用を独立した fixture として書ける（トグル前後の state 整合を気にしなくて良い）
- UI 的にも「Replay 保存して閉じた → 次回起動で続きから」が自然に動くようになる

### ✗ 失うもの

- 起動時に勝手に kline API fetch が走る（ユーザーが UI 上で範囲を見て Play したかった場合、fetch 待ちが発生）
  → `parse_replay_range` が成功している = Play したかったと解釈するので妥当
- `pending_auto_play` は永続化しないため、Play → 保存 → 再起動 のサイクルを回すと毎回 fetch される
  → SKILL.md の `POST /api/app/save` は Play 後の state を保存する運用なので意図的に再生されるのが正しい

### ⚠ 注意点

- **ログイン画面で止まっているケース** [SKILL.md:146-155](../../.claude/skills/e2e-test/SKILL.md#L146-L155): 立花証券などログイン必須のアダプタを含む fixture で auto-play が発火しうる。ログイン後 `transition_to_dashboard()` [src/main.rs:272](../../src/main.rs#L272) で Dashboard に入る経路でも ResolveStreams が走るので、ゲート自体は動くはず。ただし初期化順序を Phase 6 の E2E で検証必須
- **Ticker metadata が一部欠落しているケース**: `resolved_streams` の `try_fold` が `Err` を返すと `ResolveStreams` ハンドラが `Task::none()` を返す。この場合 streams は Ready に昇格しないため、auto-play ゲートは false のまま → 30s タイムアウトで toast 通知。ユーザが気付けるので OK
- **Tick が最初に届くタイミング**: アプリ起動直後、sidebar の tickers_info がまだ空で ResolveStreams が `Deferring persisted stream resolution` で return する [main.rs:501-506](../../src/main.rs#L501-L506)。`tickers_info` が埋まるのはどの時点か要確認（master download 完了後？）。pending_auto_play の deadline は余裕を持たせる
- **複数ペインの Ready タイミングがずれる**: ペインごとに個別に ResolveStreams が呼ばれるので、最後のペインが Ready になった瞬間にゲートが成立する。中間状態（一部だけ Ready）では発火しないので問題なし

## 未決事項

- [ ] タイムアウトを 30s で十分か、調整が必要か（master download が遅い環境を考慮）
- [ ] E2E fixture として「Replay 直接起動」と「Live 起動 → Toggle」を両方テストとして残すか、前者に一本化するか
- [ ] UI ユーザ向けに「起動時自動再生」を設定で無効化できるようにすべきか（今回スコープ外で良い）

## 参考

- 既存設計: [replay_bar_step_loop.md](./replay_bar_step_loop.md) — StepClock / EventStore / dispatch_tick の前提
- 既存ヘルパー: `connector::ResolvedStream` ([src/connector/stream.rs](../../src/connector/stream.rs)), `PersistStreamKind::into_stream_kinds` (data crate)
- E2E 運用: [.claude/skills/e2e-test/SKILL.md](../../.claude/skills/e2e-test/SKILL.md)
