---
name: agent-experience-verification
description: flowsurface の HTTP API を「ユーザーの分身としてのエージェント」として実際に叩き、ナラティブ基盤など機能が体験として成立しているかを検証するスキル。E2E テストでは拾えない実環境バグ（バッファ制限・配線漏れ・非決定性）を炙り出す。Phase 4a ナラティブ基盤の検証パターンに準拠。
---

# Agent Experience Verification

E2E テスト（仕様駆動）と単体テスト（実装駆動）の隙間を埋める「**実体験ベース検証**」のスキル。
あなたがエージェントとして HTTP API を叩き、観測 → 判断 → 発注 → 記録 → 振り返り のサイクルを回す。

仕様書通りに動くだけでなく、**実環境で違和感なく使えるか** を確かめる。

## いつ使うか

- 新機能（特に HTTP API + 永続化 + 非同期処理が絡むもの）の実装直後
- E2E と単体テストが全パスしているのに「本当に動くのか」不安が残るとき
- Phase X-a の verification-loop 直前の最終チェック
- 既存機能のリグレッション疑いを手早く確認したいとき

## いつ使わないか

- 純粋な単体ロジック（パース・計算）の検証 → `rust-testing` skill を使う
- GUI ピクセル比較が必要な検証 → 別途 pixel-diff インフラ整備が必要
- TCP/プロセス間通信を持たない機能 → 単体テストで十分

---

## 行動ループ（テンプレート）

```
1. ビルド: cargo build && cargo build --release
   ※ debug / release 両方必須。FlowsurfaceEnv._find_binary() は FLOWSURFACE_BINARY 未指定で
     target/debug を優先するため、release だけビルドすると古い debug で再体験することになる。

2. 起動（ヘッドレス）:
   ./target/release/flowsurface.exe --headless --ticker BinanceLinear:BTCUSDT --timeframe M1 &
   sleep 3
   ※ FLOWSURFACE_DATA_PATH は設定しない（既定 AppData を使う）。env 上書きすると
     data_path() の path_name 引数が捨てられる既知バグ (data/src/lib.rs:133-144) を踏む。

3. 検証対象機能のセットアップ
   例: リプレイなら POST /api/app/set-mode {mode:"replay"} → POST /api/replay/play → pause

4. エージェントとして 1 サイクル実行:
   a. 状態観測（GET /api/replay/state など）
      - 同じ状態を 2-3 回観測して決定的か確認
   b. 観測から判断（reasoning は自然言語で自分で書く）
   c. アクション実行（POST /api/replay/order など）
   d. 判断を記録（POST /api/agent/narrative）
   e. 時間を進める（POST /api/replay/step-forward × 数回）
   f. 結果を確認（GET /api/agent/narrative/:id で outcome 自動充填確認）

5. バリエーションを変えて 3-5 サイクル繰り返す:
   - 成行 / 指値 × buy / sell
   - idempotency_key あり/なし（再送で結果安定するか）
   - public フラグ true / false 往復
   - payload サイズ 極小 / 大きめ（数百 KB） / 上限超え

6. 振り返り:
   - 履歴一覧（GET /api/agent/narratives?...）
   - スナップショット復元（GET /api/agent/narrative/:id/snapshot で記録時と一致するか）
   - 蓄積量（GET /api/agent/narratives/storage）

7. Python SDK でも同サイクルが回るか:
   import flowsurface as fs
   fs.narrative.create(...) / list / get / publish / unpublish / snapshot / storage_stats

8. 破綻を検知 → 根本修正 → debug + release 両方リビルド → 2 へ戻る。
```


---

## 出力フォーマット

```
### 体験して気づいたこと（サイクルごと）
- 期待していた挙動:
- 実際の挙動:
- 差分（違和感の正体）:

### 根本修正したバグ
- [file:line] 症状 → 原因 → 修正

### テスト結果
- cargo test --lib: X/Y
- 関連 E2E（例 S51/S52/S53）: x/N
- 関連 Python テスト: x/N

### 実装ログ追記
docs/plan/<phase>.md §9 末尾に日付付きで追記済み
```

---

## チェックリスト（セッション終了前）

- [ ] debug / release 両ビルド成功
- [ ] `cargo test --lib` 全パス
- [ ] 関連 E2E が release バイナリに対して全パス
- [ ] Python SDK の関連テストが全パス（アプリ起動状態で実行）
- [ ] 起動した flowsurface プロセスを `taskkill //F //IM flowsurface.exe` で確実に終了
- [ ] 検証ログを計画書 §9 末尾に日付付きで追記
- [ ] 新規バグを発見した場合は file:line と根本原因を明記して修正

---

## 参考スキル

- `e2e-testing` — HTTP API 経由のスクリプト型 E2E（仕様駆動の自動検証）
- `rust-testing` — 単体・property-based テスト（実装駆動）
- `verification-loop` — 包括的な PR 前検証（このスキルの上位）
- `tdd-workflow` — RED → GREEN → REFACTOR（バグ修正時に併用）
