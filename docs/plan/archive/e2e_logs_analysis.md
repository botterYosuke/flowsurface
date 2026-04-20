# E2E ワークフロー ログ エラー集計結果

`C:\Users\sasai\Downloads\logs_65297484727` 内の58件のE2Eテストログから、エラー（`FAIL:` で記録されているもの）の抽出・集計を行いました。

## エラー概要
多くのエラーは「立花証券（Tachibana）APIのセッション確立」や「Replay環境で `Playing` 状態への遷移失敗」によるタイムアウト関連に集中しています。

### 1. Tachibana セッション・ログイン関連エラー (計12件)
主にセッション確立待ちでのタイムアウトやAPIエラーです。

* **Tachibana session not established after 120s** (計5件)
  * `TC-S20-01-pre`
  * `TC-S20-03-pre`
  * `TC-S20-04-pre`
  * `TC-S20-05-pre`
  * `TC-S21-precond`
* **precond — Tachibana セッションが確立されなかった（DEV_USER_ID でのログインに失敗）** (計3件)
* **Tachibana セッション確立せず（120 秒タイムアウト）** (計2件)
  * `TC-S21-04-pre`
  * `TC-S21-05-pre`
* **precond — Tachibana セッション確立失敗** (計1件)
* **Step 3 — orders フィールドが配列でない: {'error': 'API エラー: code=2, message=セッションが切断しました。'}** (計1件)

### 2. Replay Playing 状態への到達エラー・タイムアウト (計9件)
Replayの準備フェーズにおいて、ステータスが `Playing` にならない状態でのタイムアウトです。

* **Playing 到達せず** (計7件)
  * `TC-S11-01-pre`
  * `TC-S11-03-pre`
  * `TC-S11-04-pre`
  * `TC-S11-05-pre`
  * `TC-S11-06-pre`
  * `TC-S12-precond`
  * `TC-S13-precond`
* **TC-S32-01 — Playing 未到達（120 秒タイムアウト）: status=None** (計1件)
* **TC-B — REPLAY Playing に到達せず（60s タイムアウト）** (計1件)

### 3. アサーションエラー / 期待値の不一致 (計1件)
* **TC-S6-04 — diff=60000 (expected 300000)** (計1件)

--- 

> [!NOTE]
> この集計結果は、各テキストファイル(`*.txt`)の出力を解析し、重複する原因ごとにまとめたものです。インフラや環境起因でのタイムアウトが多く発生している傾向が見られます。
