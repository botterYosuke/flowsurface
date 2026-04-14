#!/usr/bin/env bash
# s14_autoplay_event_driven.sh — スイート S14: Auto-play タイムアウト廃止
# ビルド要件: cargo build --release --features e2e-mock
#
# 設計:
#   - TC-01/02: keyring にセッションを事前に保存 → 起動時 try_restore_session() 成功
#     → pending_auto_play=true のまま → inject-master で auto-play を発火させる
#   - TC-03: keyring セッションなし → pending_auto_play クリア → Playing にならない
#   - TC-04: inject-master(空) + inject-master(正規) の 2 段階で Playing 到達
set -euo pipefail
source "$(dirname "$0")/common_helpers.sh"

echo "=== S14: Auto-play タイムアウト廃止 ==="
backup_state
trap 'stop_app; restore_state' EXIT ERR

START=$(utc_offset -4)
END=$(utc_offset -2)
MID_MS=$(node -e "console.log(Date.now() - 3*3600*1000)")

MASTER=$(cat <<'MEOF'
{"records":[{"sIssueCode":"7203","sIssueNameEizi":"Toyota Motor","sCLMID":"CLMIssueMstKabu"}]}
MEOF
)

DAILY_BODY=$(node -e "
  const t = $MID_MS;
  console.log(JSON.stringify({
    issue_code: '7203',
    klines: [
      {time: t - 86400000, open: 3000, high: 3100, low: 2900, close: 3050, volume: 500000},
      {time: t,            open: 3050, high: 3150, low: 2950, close: 3100, volume: 600000}
    ]
  }));
")

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

# セッションを keyring に保存するユーティリティ
persist_session_to_keyring() {
  write_tachibana_state
  start_app
  curl -s -X POST "$API/test/tachibana/inject-session" > /dev/null
  curl -s -X POST "$API/test/tachibana/persist-session" > /dev/null
  stop_app
}

# keyring からセッションを削除するユーティリティ
delete_session_from_keyring() {
  write_tachibana_state
  start_app
  curl -s -X POST "$API/test/tachibana/delete-persisted-session" > /dev/null
  stop_app
}

# ── 事前準備: セッションを keyring に保存 ───────────────────────────────────
echo "  [準備] keyring にダミーセッションを保存..."
persist_session_to_keyring
echo "  [準備] 完了"

# ===== TC-S14-01 / TC-S14-02: 35 秒遅延 inject でも Playing 到達 =====
write_tachibana_state
start_app
# ↑ try_restore_session() がキーリングの e2e-mock セッションを復元
# → pending_auto_play = true のまま（SessionRestoreResult(None) 経路に入らない）

echo "  inject なしで 35 秒待機中（旧 30s タイムアウトが発火するはずだった時間帯）..."
ELAPSED=0
PREMATURE_PLAY=false
while [ $ELAPSED -lt 35 ]; do
  STATUS=$(jqn "$(curl -s "$API/replay/status")" "d.status")
  if [ "$STATUS" = "Playing" ]; then
    fail "TC-S14-01-pre" "inject なしで Playing になった (elapsed=${ELAPSED}s)"
    PREMATURE_PLAY=true
    break
  fi
  sleep 5
  ELAPSED=$((ELAPSED + 5))
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
  && pass "TC-S14-02: 35s 経過後も timed out トーストなし" \
  || fail "TC-S14-02" "timed out トースト発見（旧実装の挙動）"

# TC-S14-01: セッション + daily history + master 注入後に Playing 到達
curl -s -X POST "$API/test/tachibana/inject-session" > /dev/null
curl -s -X POST "$API/test/tachibana/inject-daily-history" \
  -H "Content-Type: application/json" -d "$DAILY_BODY" > /dev/null
curl -s -X POST "$API/test/tachibana/inject-master" \
  -H "Content-Type: application/json" -d "$MASTER" > /dev/null

if $PREMATURE_PLAY; then
  pend "TC-S14-01" "inject なしで Playing になったため前提条件未達"
elif wait_playing 60; then
  pass "TC-S14-01: 35s 遅延後に inject → Playing 到達（タイムアウトなし）"
else
  fail "TC-S14-01" "Playing に到達せず（60 秒タイムアウト）"
fi

stop_app

# ===== TC-S14-03: セッションなし → Playing にならず待機系 info トーストが出る =====
echo "  [TC-S14-03] keyring セッションを削除してセッションなし状態でテスト..."
delete_session_from_keyring

write_tachibana_state
start_app
# ↑ try_restore_session() → None → SessionRestoreResult(None) → on_session_unavailable()
#   → pending_auto_play=false + Toast::info("Replay auto-play was deferred: please log in to resume")

echo "  セッションなしで 15 秒待機中..."
sleep 15
STATUS=$(jqn "$(curl -s "$API/replay/status")" "d.status")
[ "$STATUS" != "Playing" ] \
  && pass "TC-S14-03a: セッションなし → Playing でない (status=${STATUS:-none})" \
  || fail "TC-S14-03a" "Playing になった（セッションなしなのに）"

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

stop_app

# ===== TC-S14-04: 2 回目の inject-master で Playing 到達（マスター遅延模擬）=====
echo "  [TC-S14-04] keyring にセッションを再保存..."
persist_session_to_keyring

write_tachibana_state
start_app
# ↑ keyring セッション復元 → pending_auto_play=true

# inject-session のみ（master は遅らせる）
curl -s -X POST "$API/test/tachibana/inject-session" > /dev/null
echo "  inject-session 完了。10 秒後に空 master 注入..."
sleep 10

# 1 回目: 空リスト（ticker 7203 が見つからず stream 解決失敗を模擬）
curl -s -X POST "$API/test/tachibana/inject-master" \
  -H "Content-Type: application/json" -d '{"records":[]}' > /dev/null
sleep 5
STATUS=$(jqn "$(curl -s "$API/replay/status")" "d.status")
[ "$STATUS" != "Playing" ] \
  && pass "TC-S14-04-pre: 空 master では Playing にならない (status=${STATUS:-none})" \
  || fail "TC-S14-04-pre" "空 master で Playing になった"

# 2 回目: daily history + 正規データ → Playing 到達
curl -s -X POST "$API/test/tachibana/inject-daily-history" \
  -H "Content-Type: application/json" -d "$DAILY_BODY" > /dev/null
curl -s -X POST "$API/test/tachibana/inject-master" \
  -H "Content-Type: application/json" -d "$MASTER" > /dev/null

if wait_playing 60; then
  pass "TC-S14-04: 2 回目 inject-master 後に Playing 到達（マスター遅延模擬）"
else
  fail "TC-S14-04" "2 回目 inject-master 後も Playing に到達せず"
fi

# クリーンアップ: keyring セッション削除
delete_session_from_keyring

print_summary
[ $FAIL -eq 0 ]
