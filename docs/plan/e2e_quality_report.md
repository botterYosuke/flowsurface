# E2E テスト品質レポート

> 生成日: 2026-04-17  
> 対象: `tests/e2e_scripts/s1〜s40` + `common_helpers.sh` + `src/replay_api.rs`

---

## サマリー

flowsurface E2E テストは 40 スクリプト・180+ TC で主要フローを広くカバーしており、
Binance / Tachibana 両ソースの検証を持つ点は強みである。
一方で以下の課題が顕在化している。

1. **assertion の弱さ**: HTTP ステータスコードのみ確認し、レスポンス body を検証しない TC が多い。仕様 breaking change を検出できない。
2. **PEND TC の放置**: `src/replay_api.rs` に実装済みのエンドポイントに対して、PEND フラグが残ったままのスクリプトが存在する（s15, s19）。
3. **sleep 依存**: `sleep N` で固定待機する箇所が散在し、ネットワーク・CPU 状態によって flaky になる。
4. **エラーパステストの偏り**: 4xx/5xx のテストが s8/s17/s21 に集中しており、個別エンドポイントの入力 validation テストが薄い。
5. **Tachibana 基本フローの欠落**: Binance では s1 で網羅している基本ライフサイクルが、Tachibana では相当スクリプトが存在しない。
6. **スクリプト間の重複**: Tachibana テスト群（s14/s19/s20）がセッション初期化ロジックを個別に実装しており、保守コストが高い。

改善により、テスト総実行時間を約 30〜40% 短縮でき、CI での信頼性も向上する見込み。

---

## 1. 統合・整理提案

### 1-1. 統合できるスクリプトグループ

| 対象スクリプト | 理由 | 提案 |
|---|---|---|
| s1_basic_lifecycle + s2_persistence + s3_autoplay | 前提条件が同一（BinanceLinear:BTCUSDT M1, Live 起動）。3 スクリプト合計 24 TC が連続性の高いフロー | `s1_lifecycle_full.sh` に統合。TC ラベルを S1/S2/S3 のまま維持し保守性を保つ。なお、統合により s2 の `saved-state.json` fixture 依存（TC-S2-01 の副作用を TC-S2-02 が前提とする問題）は自動解消する |
| s4_multi_pane_binance + s6_mixed_timeframes + s11_bar_step_discrete | マルチペイン・マルチタイムフレーム・離散化の三層が相互に前提を共有 | s11 を s6 内の TC として吸収。s4 は独立維持（3 ペイン構成が特有） |
| s14_autoplay_event_driven + s19_tachibana_chart_snapshot + s20_tachibana_replay_resilience | DEV 環境・TachibanaSpot:7203 D1・DEV_USER_ID/DEV_PASSWORD 必須。セッション初期化ロジックが 3 つで重複 | セッション共通ヘルパー `tachibana_dev_session()` を common_helpers.sh に移植。3 スクリプトの `start_app` 回数を 1 回に削減 |
| s8_error_boundary + s17_error_boundary | 同一 Binance 設定・同一エラー境界観点。TC 番号体系だけ違う（S8/S17）| **assertion 強化（Section 4-3 参照）を先に s8・s17 に対して各々実施した後**、s17 の TC を s8 に追記して 1 スクリプトに統合する。順序を逆にすると assertion 修正を 2 回行う羽目になる |

### 1-2. 分割すべき肥大化スクリプト

| 対象スクリプト | 問題 | 提案 |
|---|---|---|
| s18_endurance | TC-S18-01（2h 完走: 最大 20 分）・TC-S18-02（Step 1000 回: 5〜10 分）・TC-S18-03（CRUD 20 サイクル: 3〜5 分）が 1 スクリプトに混在。1 TC 失敗で残り全失敗 | `s18a_long_replay_completion.sh`・`s18b_high_frequency_step.sh`・`s18c_pane_crud_endurance.sh` に分割 |
| s7_mid_replay_pane | TC-S7-01〜06（Playing 中 CRUD）と TC-S7-07（range end 後の split）は前提となる `start_app` フローが異なる | `s7a_mid_replay_crud.sh` と `s7b_range_end_safety.sh` に分割 |

---

## 2. カバレッジ不足

### 2-1. エンドポイント別カバレッジ

`src/replay_api.rs` のエンドポイント 24 件に対する現状評価。

| エンドポイント | 正常系 | 4xx/5xx | 評価 | 主なギャップ |
|---|---|---|---|---|
| GET `/api/replay/status` | 十分 | 少 | 良好 | - |
| POST `/api/replay/toggle` | 十分 | 少 | 良好 | range 値の保持確認が不足 |
| POST `/api/replay/play` | 十分 | 少 | 良好 | datetime 境界値（無効月/日/時）の 400 検証が不完全 |
| POST `/api/replay/pause` | 少 | **なし** | 要改善 | Paused 状態で pause（べき等性）の検証なし |
| POST `/api/replay/resume` | 少 | **なし** | 要改善 | Playing 状態で resume（べき等性）の検証なし |
| POST `/api/replay/step-forward` | 十分 | **なし** | 要改善 | Live モード中の step-forward 動作未検証 |
| POST `/api/replay/step-backward` | 十分 | **なし** | 要改善 | start_time 下限での clamp 値が端点一致するか未検証 |
| POST `/api/replay/speed` | 十分 | **なし** | 良好 | - |
| POST `/api/app/save` | 少 | **なし** | 要改善 | ディスク書き込み失敗時の動作未検証 |
| GET `/api/auth/tachibana/status` | 少 | **なし** | 良好 | - |
| GET `/api/pane/list` | 十分 | **なし** | 良好 | 全ペイン削除後の空配列 `[]` 確認なし |
| GET `/api/pane/chart-snapshot` | 少 | 少 | **要改善** | oldest_ts/bar_count の具体値検証が s15-TC-01 のみ。PEND 残存 |
| POST `/api/pane/split` | 十分 | 少 | 良好 | - |
| POST `/api/pane/close` | 少 | 少 | 要改善 | 最後のペイン close（pane 数 0）の動作未検証 |
| POST `/api/pane/set-ticker` | 十分 | 少 | 良好 | 不正 ticker format（":" なし）の 400 検証なし |
| POST `/api/pane/set-timeframe` | 少 | **なし** | **要改善** | 不正 timeframe（"M999", ""）の validation TC なし |
| GET `/api/notification/list` | 少 | **なし** | 良好 | - |
| POST `/api/sidebar/select-ticker` | 少 | 少 | 要改善 | kind="InvalidKind" の 400 検証なし |
| POST `/api/sidebar/open-order-pane` | 少 | 少 | 要改善 | kind 省略時のデフォルト動作未検証 |
| POST `/api/replay/order` | 少 | 少 | **要改善** | qty=0・負値・NaN の 400 検証なし |
| GET `/api/replay/portfolio` | 少 | 少 | **要改善** | cash/equity/positions 個別フィールドの値検証なし |
| GET `/api/replay/state` | 少 | **なし** | 要改善 | current_time_ms 以外のフィールド検証なし |
| GET `/api/replay/orders` | 少 | **なし** | **要改善** | order status（"pending"/"filled"）の複数件検証なし |
| GET `/api/app/screenshot` | 少 | **なし** | 要改善 | ファイルサイズ・PNG フォーマット検証なし |
| **全エンドポイント共通（HTTP 500 パス）** | — | **なし** | **要改善** | ① `{"error":"App channel closed"}`: iced channel 輻輳時に返る（`replay_api.rs` の `sender.send().await.is_err()` パス）。② `{"error":"No response from app"}`: ReplySender drop 時に返る（`reply_rx.await` エラーパス）。どちらも E2E で未検証。運用上の重大障害パスであり、fault injection での検証を検討する |

### 2-2. Tachibana / Binance 非対称領域

| 領域 | Binance | Tachibana | 非対称性 |
|---|---|---|---|
| 基本ライフサイクル（toggle/play/pause/resume/speed） | s1（15 TC・完全） | **なし** | Tachibana の基本フロー全欠落 |
| Step 離散化検証 | s11（M1/M5/H1 全タイムフレーム） | なし（s20 で D1 のみ） | Tachibana は D1 以外のタイムフレームが未検証 |
| chart-snapshot | s15（5 TC） | s19（5 TC） | ほぼ対称。祝日データの bar_count 検証が s29 に分散 |
| エラー境界 | s8 + s17（16 TC） | s21 | Binance 側が 2 倍のスクリプトで重複（統合余地あり） |
| 耐久テスト | s18（3 TC） | s22 | Tachibana 版は読了未完だが対称と推定 |
| 仮想注文フルサイクル | s34/s35/s40 | **なし** | Tachibana 環境での virtual order は全未検証 |

---

## 3. 追加すべきテスト項目

### 3-1. PEND TC のうち実装済みで即実行可能なもの

以下 5 件はいずれも「PEND フラグ・probe check を削除して即実行」が対処。アクションが共通なため列挙する。

- `s15_chart_snapshot.sh:37-40` — "未実装なら SKIP" probe 残存。`/api/pane/chart-snapshot` は `replay_api.rs` に実装済み
- `s19_tachibana_chart_snapshot.sh:83-86` — 同上（DEV 環境必須のまま）
- `s12_pre_start_history.sh:79` — "chart-snapshot 実装待ち" コメント。s15 のロジックを移植して完成させる
- `s16_replay_resilience.sh:77` — "データ不足時 PEND"。range を `-72h/-1h` に拡大して retry
- `s20_tachibana_replay_resilience.sh:101` — "Playing 到達せず（データ不足推定）"。range を `-2400h/-24h` に拡大して試行

### 3-2. 現状テストが検出できないバグパターンと追加 TC

> 新規スクリプト名は s41〜 の空き番号を使用し、既存スクリプトとの prefix 衝突を避ける。

| バグパターン | 現 TC の限界 | 追加すべき TC |
|---|---|---|
| toggle → play 高速切替時のバッファ競合（Race condition） | 各操作間に 1 秒以上の sleep が入っている | **E2E テストでの再現不可**。`replay_api.rs` の HTTP サーバーはシリアル処理のため、HTTP 層で同時リクエストを発行しても連続実行に変換される。Race condition は `iced` の `update()` Message キュー内に存在し、unit test / integration test で検証すること（E2E では検出不能と明記）。`s41` は "連続高速発行でクラッシュしないか" の **安定性テスト** として再定義する。 |
| StepForward が複数バー同時前進（離散化失敗） | delta が STEP_M1 の倍数かのみ検証。2 バー（2×STEP_M1）も pass | s11 に TC 追加 `s11_discrete_monotonicity`: 10 回連続 StepForward の差分が毎回 exactly STEP_M1 になることを検証 |
| start_time clamp が端点一致しない | TC-S12-01 は "start_time の後退禁止" のみ確認 | s12 に TC 追加 `s12a_start_clamp_exact`: STEP_M1 の半分だけ後退 → current_time == start_time（exact match） |
| range 値が toggle で上書きされる | s1 で range_start/end の存在のみ確認 | `s50_range_value_identity.sh`: play で range 設定 → toggle Live → toggle Replay → range_start の値が元の値と一致するか比較 |
| min_step が Binance+Tachibana 混在時に D1 に引き摺られる | Binance のみの s4/s6 で検証済み | `s51_multi_source_step_precision.sh`: Binance M1 + Tachibana D1 ペインで StepForward delta が 60000（M1）のままか確認 |
| Order 約定タイミングの off-by-one | TC-S40-C が「最大 5 回」という曖昧条件 | `s42_fill_timing_exact.sh`: step 1 回目で未約定、step 2 回目で約定を exact に検証（T+1 約定保証） |
| pane 全削除後の pane/list が空配列 | pane 削除テストがあるが、削除後の list 確認なし | s17 に TC 追加: split → close × 1 → `GET /api/pane/list` が `[]` を返すことを検証 |
| `/api/replay/orders` の件数変化 | 注文後の pending 確認のみ | `s43_orders_lifecycle.sh`: **仕様確認を先行タスクとする**（filled 後に orders から消えるのか filled として残るのか未定義）。仕様確定後に TC を実装 |

### 3-3. 境界値・異常系テスト（ファイル単位で整理）

**`s8_error_boundary.sh` に追加:**
```bash
# datetime パーサ境界（POST /api/replay/play）
"2026-02-30 10:00"  # 存在しない日         → 400 を期待
"2026-04-10 25:00"  # 時が 24 超           → 400 を期待
"2026-13-01 09:00"  # 月が 13              → 400 を期待
"2024-02-29 10:00"  # うるう年 2/29（有効） → 200 を期待

# pane_id edge case（POST /api/pane/set-ticker）
pane_id=""           # empty string → 400 を期待
pane_id="not-a-uuid" # 不正 UUID   → 404 を期待

# set-timeframe validation（POST /api/pane/set-timeframe）
timeframe="M999"     # 不正 timeframe → 400 を期待
timeframe=""         # 空文字         → 400 を期待
```

**`s34_virtual_order_basic.sh` に追加:**
```bash
# qty パラメータ境界値（POST /api/replay/order）
qty=0             # zero     → 400 か no-op か仕様確認が先決
qty=-1            # negative → 400 を期待
qty=0.00000001    # 最小 fractional → 実装依存（確認要）
```

---

## 4. テスト設計上の問題

### 4-1. sleep による時間依存（flaky になりやすい箇所）

| スクリプト:行番号 | 問題 | 推奨対策 |
|---|---|---|
| s1_basic_lifecycle.sh:87-91 | `sleep 3` で "1〜100 bar 前進" を期待。ネットワーク遅延時に 0 bar で fail | `wait_for_time_advance` で動的ポーリングに変更 |
| s3_autoplay.sh:90 | `sleep 10` で auto-play 待機。Binance フェッチ遅延時に fail | `wait_playing 30` に統一 |
| s9_speed_step.sh:63-64 | `sleep 5` で "5 秒に 1〜500 bar" を期待。CPU スロットル時にずれる | `wait_for_time_advance` で delta 期待値ベース検証に変更 |
| s16_replay_resilience.sh:84-100 | `sleep 0.5` × 60 loop = 最大 30 秒固定で UTC 0:00 越えを待機 | `wait_for_time_advance` で midnight 突破まで動的待機 |
| s18_endurance.sh:63-80 | StepForward × 500 に `sleep 0.3` 固定。ネットワーク遅延で延伸 | curl 成功まで retry loop（最大 2 秒 / 回）に変更 |
| common_helpers.sh:38-44 | `start_app` が 30 秒固定 wait。遅いマシンで fail | `/api/replay/status` を heartbeat として polling（最大 60 秒）に変更 |

### 4-2. 前のスクリプトの状態に暗黙依存しているテスト

| 問題 | ファイル | 詳細 |
|---|---|---|
| Tachibana keyring セッションの前提 | s19_tachibana_chart_snapshot.sh, s20_tachibana_replay_resilience.sh | セッション作成が s14 にあるが、s19/s20 単体実行時はセッション存在を前提としている。独立実行すると DEV_USER_ID/DEV_PASSWORD でセッション作成ロジックが各スクリプトに分散 |
| relative time 依存の range 指定 | s3_autoplay.sh:20-21 | `utc_offset -3`（現在時刻 -3h）で range 指定。深夜実行時に前日データを含む可能性があり、bar_count 期待値がずれる可能性 |

### 4-3. assert が弱い TC

| スクリプト:行番号 | 問題 | 推奨修正 |
|---|---|---|
| s8_error_boundary.sh:40-50 | `curl -o /dev/null -w "%{http_code}"` で 400/404 のステータスのみ確認。エラーメッセージの format・`error` フィールド存在を検証していない | `-s` でレスポンス body を取得し、`jq '.error // empty'` で `error` フィールド存在を確認 |
| s17_error_boundary.sh:40-46 | UUID フォーマット違反の `pane_id` は `parse_split_command` が `BadRequest(400)` を返す（HTTP ステータスは正しい）。問題は「UUID フォーマットは有効だが存在しない `pane_id`」を渡した場合に `route()` が 200 を返し、**app 層の "not found" エラーが正常応答と区別されない**こと（`handle_pane_api` が常に HTTP 200 でラップするため） | 存在しない UUID を `pane_id` に指定して `pane/split` を実行し、レスポンス body に `"error"` キーが存在することを確認する TC を追加。将来的に `handle_pane_api` が正しいステータスを返すようになれば HTTP 404 を期待するよう更新する |
| s11_bar_step_discrete.sh:34-37 | `BigInt('$DELTA') % BigInt('60000') === 0n` で倍数確認のみ。2 バー同時前進（120000）も pass | delta === 60000（完全一致）を検証 |
| s4_multi_pane_binance.sh:81-83 | `advance_within "$CT1" "$CT2" "$STEP_M1" 100` で "1〜100 bar 前進" 確認。下限（0 bar）の検出なし | `advance_at_least "$CT1" "$CT2" "$STEP_M1" 1` を追加（最低 1 bar 前進を保証） |
| s5_tachibana_mixed.sh:63-66 | inject-master 送信後に `M_OK=true` のみ確認。マスターレコードがペイン UI に反映されたか未検証 | `GET /api/pane/chart-snapshot` で bar_count が注入前後で変化したか確認 |
| s7_mid_replay_pane.sh:95-97 | `streams_ready=true` の boolean のみ確認。実際のデータが available（klines 非空）かは未検証 | `chart-snapshot` の `bar_count > 0` を合わせて確認 |
| s40_virtual_order_fill_cycle.sh | open_positions の件数確認が `>= 1` のみ（推定）。複数 open がバグでも PASS | `open_positions.length === 1` の exact match を検証 |
| common_helpers.sh:23 | `jqn` が "null" 文字列を返す。JSON null と文字列 "null" の区別なし。`jqn` は `common_helpers.sh` 全体で広く使われており、影響が全スクリプトに及ぶ cross-cutting な問題 | `.field // "MISSING"` パターンで null を明示的に区別するか、`jq -e` で exit code を利用。**影響範囲が広いため、修正前に `grep -r jqn tests/e2e_scripts/` で呼び出し箇所を列挙してから対処する** |
| `replay_api.rs:534-541`（`body_opt_str_field`）が影響する TC | **仕様決定済み**: `"kind": null` を送ると `body_opt_str_field` がフィールド省略と同一視し `None` を返す動作は正式仕様（JSON 慣例）。`body_str_field`（必須フィールド）は null を 400 で拒否するが、省略可フィールドが null を `None` とするのは意図的な非対称。`parse_sidebar_select_ticker` が現在の唯一の影響先 | 仕様を docstring に明記済み（`replay_api.rs:533-535`）。ユニットテスト `opt_str_field_null_equals_omission` で仕様を固定済み。E2E テスト追加は不要（unit test で十分） |

---

## 優先度マトリクス

| 優先度 | 影響範囲 | 改善項目 |
|---|---|---|
| **高** | CI 信頼性 | s15/s19 の PEND フラグ削除（実装済み API のテストを即実行） |
| **高** | バグ検出力 | s8 の assertion 強化（HTTP ステータスのみ → body の `error` フィールド検証）。**s17 との統合前に先行実施** |
| **高** | バグ検出力 | s17 の assertion 強化（同上）。**強化後に s8 へ統合** |
| **高** | CI 安定性 | s1:87-91, s9:63-64 の `sleep N` を `wait_for_time_advance` に置換 |
| **高** | 保守性 | Tachibana セッション初期化の helper 関数化（s14/s19/s20 の重複解消） |
| **中** | バグ検出力 | `s41_stability_burst.sh` 追加（toggle/play 連続発行のクラッシュ安定性検証。Race condition 検出は iced unit test で対応） |
| **中** | バグ検出力 | s11 に `discrete_monotonicity` TC 追加（StepForward の 1 バー厳密検証） |
| **中** | カバレッジ | s8 に datetime 境界値テスト追加（"2026-02-30", "2026-13-01" など） |
| **中** | カバレッジ | s8 に `GET /api/pane/set-timeframe` validation TC 追加（"M999", "" の 400 検証） |
| **中** | カバレッジ | Tachibana 基本ライフサイクルスクリプト新規作成（s1 相当） |
| **中** | 保守性 | s1+s2+s3 を統合（実行時間 30% 短縮推定） |
| **中** | 保守性 | s18 を 3 つに分割（s18a/s18b/s18c） |
| **中** | 保守性 | common_helpers.sh の `jqn` null 問題修正（影響範囲を `grep -r jqn` で先に確認してから対処） |
| **低** | バグ検出力 | `s42_fill_timing_exact.sh` 追加（約定タイミング exact 検証） |
| **低** | バグ検出力 | `s51_multi_source_step_precision.sh` 追加（Binance+Tachibana 混在の min_step） |
| **低** | カバレッジ | s34 に qty=0・負値の `POST /api/replay/order` validation TC 追加 |
| **低** | カバレッジ | `POST /api/replay/pause` べき等性・`POST /api/replay/resume` べき等性テスト |
| **低（仕様確認後）** | カバレッジ | `s43_orders_lifecycle.sh` — `/api/replay/orders` の status 遷移検証。filled 後の挙動（orders から消えるか残るか）の仕様を先に確定させること |

---

## 注記

- 本レポートは `tests/e2e_scripts/` および `src/replay_api.rs` の静的読み込みのみに基づく。スクリプトの実行・動的解析は行っていない。
- PEND TC の "実装済み" 判定は `src/replay_api.rs` のルーティング実装を照合した結果。実際の動作確認はスクリプト実行で要検証。
- s21_tachibana_error_boundary, s22_tachibana_endurance, s26〜s33, s35, s37, s39 の詳細は分析対象に含まれているが、上記 gap 分析は主要スクリプト（s1〜s20, s34, s36, s40）を重点的に分析した結果である。
