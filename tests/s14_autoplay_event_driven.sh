#!/usr/bin/env bash
# s14_autoplay_event_driven.sh — スイート S14: Auto-play タイムアウト廃止
#
# 検証シナリオ:
#   TC-01/02: DEV AUTO-LOGIN → keyring セッション保存 → 再起動後セッション復元
#             → pending_auto_play=true のまま → マスター取得完了後 Playing 到達
#   TC-03: keyring セッションなし → pending_auto_play クリア → Playing にならない
#   TC-04: PEND（マスター遅延シミュレーションは real API では再現不可）
#
# 仕様根拠:
#   docs/replay_header.md §5.1 — auto-play event-driven（タイムアウト廃止・マスター完了待ち）
#
# フィクスチャ: TachibanaSpot:7203 D1, Tachibana セッション必須（DEV AUTO-LOGIN）
#   ビルド: cargo build（debug）
#   前提条件: DEV_USER_ID / DEV_PASSWORD 環境変数設定済み
set -euo pipefail
source "$(dirname "$0")/common_helpers.sh"

# 本番データモードでは debug ビルドを使用
EXE="${FLOWSURFACE_EXE_DEBUG:-$REPO_ROOT/target/debug/flowsurface.exe}"

# 環境変数チェック
if [ -z "${DEV_USER_ID:-}" ] || [ -z "${DEV_PASSWORD:-}" ]; then
  echo "  SKIP: DEV_USER_ID / DEV_PASSWORD が未設定 — Tachibana live テストをスキップします"; exit 0
fi

echo "=== S14: Auto-play タイムアウト廃止 ==="

# headless は Tachibana keyring 操作（persist_session / delete_session）が不可能なため全 TC を PEND
if is_headless; then
  pend "TC-S14-01" "headless は Tachibana keyring 操作不可"
  pend "TC-S14-02" "headless は Tachibana keyring 操作不可"
  pend "TC-S14-03a" "headless は Tachibana keyring 操作不可"
  pend "TC-S14-03b" "headless は Tachibana keyring 操作不可"
  pend "TC-S14-04" "headless は Tachibana keyring 操作不可"
  print_summary
  exit 0
fi

backup_state
trap 'stop_app; restore_state' EXIT ERR

START=$(utc_offset -120)
END=$(utc_offset -24)

write_tachibana_state() {
  cat > "$DATA_DIR/saved-state.json" <<EOF
{
  "layout_manager":{"layouts":[{"name":"S14","dashboard":{"pane":{
    "KlineChart":{
      "layout":{"splits":[0.78],"autoscale":"FitToVisible"},"kind":"Candles",
      "stream_type":[{"Kline":{"ticker":"TachibanaSpot:7203","timeframe":"D1"}}],
      "settings":{"tick_multiply":null,"visual_config":null,"selected_basis":{"Time":"D1"}},
      "indicators":[],"link_group":"A"
    }
  },"popout":[]}}],"active_layout":"S14"},
  "timezone":"UTC","trade_fetch_enabled":false,"size_in_quote_ccy":"Base",
  "replay":{"mode":"replay","range_start":"$START","range_end":"$END"}
}
EOF
}

# DEV AUTO-LOGIN でセッションを keyring に保存するユーティリティ
# ログイン成功後にアプリが persist_session() を呼ぶため、アプリ停止後も keyring に残る
persist_session_to_keyring() {
  write_tachibana_state
  start_app
  echo "  waiting for Tachibana session (DEV AUTO-LOGIN)..."
  if ! wait_tachibana_session 120; then
    echo "  ERROR: Tachibana session not established after 120s"
    return 1
  fi
  echo "  Tachibana session established, stopping app (session persisted to keyring)..."
  stop_app
}

# keyring からセッションを削除するユーティリティ
# /api/test/tachibana/delete-persisted-session は debug ビルドで利用可能
delete_session_from_keyring() {
  write_tachibana_state
  start_app
  curl -s -X POST "$API/test/tachibana/delete-persisted-session" > /dev/null
  stop_app
}

# ── 事前準備: セッションを keyring に保存 ───────────────────────────────────
echo "  [準備] DEV AUTO-LOGIN でセッションを keyring に保存..."
persist_session_to_keyring
echo "  [準備] 完了"

# ===== TC-S14-01 / TC-S14-02: keyring セッション復元 → Playing 到達 =====
write_tachibana_state
start_app
# ↑ try_restore_session() がキーリングのセッションを復元
# → pending_auto_play = true のまま

echo "  セッション復元待機なしで 35 秒経過を確認（旧 30s タイムアウトが発火しないことを検証）..."
ELAPSED=0
PREMATURE_PLAY=false
while [ $ELAPSED -lt 35 ]; do
  STATUS=$(jqn "$(curl -s "$API/replay/status")" "d.status")
  if [ "$STATUS" = "Playing" ]; then
    # real API ではマスター取得が速く完了して Playing になる場合もある
    echo "  INFO: Playing 到達 (elapsed=${ELAPSED}s) — マスター取得完了"
    PREMATURE_PLAY=true
    break
  fi
  sleep 1
  ELAPSED=$((ELAPSED + 1))
done

# TC-S14-02: 35 秒時点で timed out トーストがないことを確認
NOTIFS=$(curl -s "$API/notification/list")
HAS_TIMEOUT=$(node -e "
  const ns = (JSON.parse(process.argv[1]).notifications || []);
  console.log(ns.some(n =>
    (n.body  && n.body.toLowerCase().includes('timed out')) ||
    (n.title && n.title.toLowerCase().includes('timed out'))
  ) ? 'true' : 'false');
" "$NOTIFS")
[ "$HAS_TIMEOUT" = "false" ] \
  && pass "TC-S14-02: 35s 経過後も timed out トーストなし（タイムアウト廃止確認）" \
  || fail "TC-S14-02" "timed out トースト発見（旧実装の挙動）"

# TC-S14-01: keyring セッション復元後に Playing 到達（マスター取得完了で自動発火）
if $PREMATURE_PLAY; then
  pass "TC-S14-01: keyring セッション復元 → マスター取得完了 → Playing 到達"
elif wait_playing 120; then
  pass "TC-S14-01: keyring セッション復元 → Playing 到達（120s 以内）"
else
  fail "TC-S14-01" "Playing に到達せず（120 秒タイムアウト）"
fi

stop_app

# ===== TC-S14-03: セッションなし → Playing にならず待機系 info トーストが出る =====
echo "  [TC-S14-03] keyring セッションを削除してセッションなし状態でテスト..."
delete_session_from_keyring

write_tachibana_state
start_app
# ↑ try_restore_session() → None → SessionRestoreResult(None) → on_session_unavailable()
#   → pending_auto_play=false + Toast::info("Replay auto-play was deferred: please log in to resume")

# TC-S14-03b: toast は DEFAULT_TIMEOUT=8s でタイムアウト → 3s 以内に確認
echo "  セッションなしで 3 秒待機中（SessionRestoreResult 処理待ち）..."
sleep 3
NOTIFS=$(curl -s "$API/notification/list")
HAS_WAIT_INFO=$(node -e "
  const ns = (JSON.parse(process.argv[1]).notifications || []);
  const KEYWORDS = ['waiting', 'session', 'login', 'pending', 'tachibana', 'deferred', '待機', 'ログイン'];
  const found = ns.some(n =>
    n.level === 'info' && (
      (n.body  && KEYWORDS.some(k => n.body.toLowerCase().includes(k))) ||
      (n.title && KEYWORDS.some(k => n.title.toLowerCase().includes(k)))
    )
  );
  console.log(found ? 'true' : 'false');
" "$NOTIFS")
NOTIFS_TEXT=$(node -e "
  const d=JSON.parse(process.argv[1]);
  const ns=d.notifications||[];
  console.log(ns.map(n=>n.level+':'+n.body).join(' | ') || '(none)');
" "$NOTIFS")
[ "$HAS_WAIT_INFO" = "true" ] \
  && pass "TC-S14-03b: 待機系 info トーストあり" \
  || fail "TC-S14-03b" "待機系 info トーストなし。通知一覧: $NOTIFS_TEXT"

# TC-S14-03a: 15 秒経過後も Playing でないことを確認
echo "  さらに 12 秒待機中（合計 15 秒）..."
sleep 12
STATUS=$(jqn "$(curl -s "$API/replay/status")" "d.status")
[ "$STATUS" != "Playing" ] \
  && pass "TC-S14-03a: セッションなし → Playing でない (status=${STATUS:-none})" \
  || fail "TC-S14-03a" "Playing になった（セッションなしなのに）"

stop_app

# ===== TC-S14-04: マスター遅延シミュレーション（PEND） =====
# real API ではマスターを「空→正規」と段階的に注入する操作が不可能なため PEND とする
pend "TC-S14-04" "マスター遅延シミュレーションは real API 環境では再現不可（e2e-mock 専用シナリオ）"

# クリーンアップ: keyring セッション削除
delete_session_from_keyring

print_summary
[ $FAIL -eq 0 ]
