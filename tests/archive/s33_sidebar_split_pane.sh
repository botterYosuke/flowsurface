#!/usr/bin/env bash
# s33_sidebar_split_pane.sh — S33: sidebar/select-ticker + kind 指定によるペイン分割テスト
#
# 検証シナリオ:
#   TC-A: kind=KlineChart で ETHUSDT を選択 → ペイン数が 2 になる
#   TC-B: 新ペインの ticker が ETHUSDT である
#   TC-C: 元ペインの ticker は BTCUSDT のまま（上書きされていない）
#   TC-D: エラー通知が出ていない
#   TC-E: 2 回目の split（SOLUSDT, kind=KlineChart）→ ペイン数が 3 になる
#
# 仕様根拠:
#   docs/replay_header.md §9.1 — Sidebar::TickerSelected + kind 指定によるペイン分割フロー
#   kind=KlineChart → init_focused_pane 経路（フォーカスペインを上書きせず Horizontal Split）
#
# フィクスチャ: BinanceLinear:BTCUSDT M1, auto-play (UTC[-3h, -1h])
set -euo pipefail
source "$(dirname "$0")/common_helpers.sh"

echo "=== S33: sidebar/select-ticker + kind 指定によるペイン分割テスト ==="
backup_state
trap 'stop_app; restore_state' EXIT ERR

# ── フィクスチャ: 単一ペイン BinanceLinear:BTCUSDT M1 ────────────────────────
START=$(utc_offset -3)
END=$(utc_offset -1)
PRIMARY=$(primary_ticker)
SECONDARY=$(secondary_ticker)
TERTIARY=$(tertiary_ticker)
SEC_SYMBOL="${SECONDARY##*:}"
TER_SYMBOL="${TERTIARY##*:}"
PRI_SYMBOL="${PRIMARY##*:}"
setup_single_pane "$PRIMARY" "M1" "$START" "$END"
echo "  fixture: $PRIMARY M1, replay $START → $END"

# ── アプリ起動 ────────────────────────────────────────────────────────────────
start_app

# autoplay で Playing に到達するまで待機
if ! wait_status "Playing" 60; then
  fail "S33-precond" "Playing 到達せず（timeout）"
  print_summary
  exit 1
fi

# ── 初期ペイン ID 取得 ──────────────────────────────────────────────────────
PANES=$(curl -s "$API/pane/list")
PANE0=$(node -e "const ps=(JSON.parse(process.argv[1]).panes||[]); console.log(ps[0]?ps[0].id:'');" "$PANES")
if [ -z "$PANE0" ]; then
  fail "S33-precond" "初期ペイン ID 取得失敗"
  print_summary
  exit 1
fi
echo "  PANE0=$PANE0"

# ── TC-A: kind=KlineChart で secondary を選択 → ペイン数 2 ─────────────────────
echo ""
echo "── TC-A: kind=KlineChart で $SECONDARY を選択 → ペイン数 2"
api_post /api/sidebar/select-ticker \
  "{\"pane_id\":\"$PANE0\",\"ticker\":\"$SECONDARY\",\"kind\":\"KlineChart\"}" \
  > /dev/null

if wait_for_pane_count 2 15; then
  pass "TC-A: kind=KlineChart → ペイン数 2"
else
  ACTUAL_COUNT=$(node -e "console.log((JSON.parse(process.argv[1]).panes||[]).length);" \
    "$(curl -s "$API/pane/list")")
  fail "TC-A" "15 秒以内に pane count が 2 にならなかった (actual=$ACTUAL_COUNT)"
  print_summary
  exit 1
fi

# ── TC-B / TC-C: 新・旧ペインの ticker 確認 ──────────────────────────────────
echo ""
echo "── TC-B/TC-C: ペイン ticker 確認"
PANES_AFTER=$(curl -s "$API/pane/list")

# 新ペイン（PANE0 以外）を特定
NEW_PANE=$(node -e "
  const ps = (JSON.parse(process.argv[1]).panes || []);
  const p = ps.find(x => x.id !== '$PANE0');
  console.log(p ? p.id : '');
" "$PANES_AFTER")
echo "  NEW_PANE=$NEW_PANE"
if [ -z "$NEW_PANE" ]; then
  fail "TC-B" "新ペイン ID 取得失敗"
  print_summary
  exit 1
fi

# TC-B: 新ペインの ticker が ETHUSDT
NEW_TICKER=$(node -e "
  const ps = (JSON.parse(process.argv[1]).panes || []);
  const p = ps.find(x => x.id === '$NEW_PANE');
  console.log(p ? (p.ticker || 'null') : 'not_found');
" "$PANES_AFTER")
echo "  new pane ticker=$NEW_TICKER"
if echo "$NEW_TICKER" | grep -qi "$SEC_SYMBOL"; then
  pass "TC-B: 新ペインの ticker に $SEC_SYMBOL が含まれる (=$NEW_TICKER)"
else
  fail "TC-B" "新ペイン ticker=$NEW_TICKER (expected to contain $SEC_SYMBOL)"
fi

# TC-C: 元ペインの ticker が BTCUSDT のまま
ORIG_TICKER=$(node -e "
  const ps = (JSON.parse(process.argv[1]).panes || []);
  const p = ps.find(x => x.id === '$PANE0');
  console.log(p ? (p.ticker || 'null') : 'not_found');
" "$PANES_AFTER")
echo "  orig pane ticker=$ORIG_TICKER"
if echo "$ORIG_TICKER" | grep -qi "$PRI_SYMBOL"; then
  pass "TC-C: 元ペインの ticker は $PRI_SYMBOL のまま (=$ORIG_TICKER)"
else
  fail "TC-C" "元ペイン ticker=$ORIG_TICKER (expected to contain $PRI_SYMBOL — 上書きされている)"
fi

# ── TC-D: エラー通知が出ていない ─────────────────────────────────────────────
echo ""
echo "── TC-D: エラー通知なし確認"
NOTIFS=$(curl -s "$API/notification/list")
ERROR_COUNT=$(node -e "
  const ns = (JSON.parse(process.argv[1]).notifications || []);
  console.log(ns.filter(n => n.level === 'error').length);
" "$NOTIFS")
echo "  error notification count=$ERROR_COUNT"
[ "$ERROR_COUNT" = "0" ] \
  && pass "TC-D: エラー通知 0 件" \
  || fail "TC-D" "エラー通知が $ERROR_COUNT 件発生"

# ── TC-E: 2 回目の split（tertiary, kind=KlineChart）→ ペイン数 3 ─────────────
echo ""
echo "── TC-E: 2 回目 split $TERTIARY → ペイン数 3"
api_post /api/sidebar/select-ticker \
  "{\"pane_id\":\"$PANE0\",\"ticker\":\"$TERTIARY\",\"kind\":\"KlineChart\"}" \
  > /dev/null

if wait_for_pane_count 3 15; then
  pass "TC-E: 2 回目 kind=KlineChart ($TERTIARY) → ペイン数 3"
else
  ACTUAL_COUNT=$(node -e "console.log((JSON.parse(process.argv[1]).panes||[]).length);" \
    "$(curl -s "$API/pane/list")")
  fail "TC-E" "15 秒以内に pane count が 3 にならなかった (actual=$ACTUAL_COUNT)"
fi

print_summary
[ $FAIL -eq 0 ]
