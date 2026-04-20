// 銘柄同期統合テスト
//
// チャートペイン → 注文入力パネルへの銘柄同期ロジックを検証する。
// 特定されたバグ: order_handler.rs の sync_issue_to_order_entry() が
// panel.update() の戻り値（Action::FetchHoldings）を破棄していた。

use flowsurface::screen::dashboard::panel::order_entry::{Action, Message, OrderEntryPanel, Side};

fn panel_with(issue_code: &str) -> OrderEntryPanel {
    let mut p = OrderEntryPanel::new();
    p.issue_code = issue_code.to_string();
    p.issue_name = "テスト銘柄".to_string();
    p.qty = "100".to_string();
    p
}

// ── 基本同期 ─────────────────────────────────────────────────────────────────

#[test]
fn sync_issue_sets_issue_code_and_name() {
    let mut panel = OrderEntryPanel::new();
    panel.update(Message::SyncIssue {
        issue_code: "7203".to_string(),
        issue_name: "トヨタ自動車".to_string(),
        tick_size: None,
    });
    assert_eq!(panel.issue_code, "7203");
    assert_eq!(panel.issue_name, "トヨタ自動車");
}

#[test]
fn sync_issue_updates_tick_size() {
    let mut panel = OrderEntryPanel::new();
    panel.update(Message::SyncIssue {
        issue_code: "7203".to_string(),
        issue_name: "トヨタ自動車".to_string(),
        tick_size: Some(5.0),
    });
    assert_eq!(panel.tick_size, Some(5.0));
}

#[test]
fn sync_issue_clears_tick_size_when_none() {
    let mut panel = panel_with("7203");
    panel.tick_size = Some(1.0);
    panel.update(Message::SyncIssue {
        issue_code: "7203".to_string(),
        issue_name: "トヨタ自動車".to_string(),
        tick_size: None,
    });
    assert!(panel.tick_size.is_none());
}

// ── 銘柄切り替え時の holdings リセット ───────────────────────────────────────

#[test]
fn sync_to_different_issue_resets_holdings() {
    let mut panel = panel_with("7203");
    panel.holdings = Some(500);

    panel.update(Message::SyncIssue {
        issue_code: "6758".to_string(), // ソニーに切り替え
        issue_name: "ソニーグループ".to_string(),
        tick_size: None,
    });

    assert!(
        panel.holdings.is_none(),
        "銘柄変更時は holdings をリセットすべき"
    );
}

#[test]
fn sync_to_same_issue_preserves_holdings() {
    let mut panel = panel_with("7203");
    panel.holdings = Some(200);

    panel.update(Message::SyncIssue {
        issue_code: "7203".to_string(), // 同じ銘柄
        issue_name: "トヨタ自動車".to_string(),
        tick_size: Some(1.0),
    });

    assert_eq!(
        panel.holdings,
        Some(200),
        "同一銘柄なら holdings を保持すべき"
    );
}

// ── Action::FetchHoldings — 売りモードでの保有株数取得 ───────────────────────

#[test]
fn sync_to_different_issue_in_sell_mode_returns_fetch_holdings() {
    let mut panel = panel_with("7203");
    panel.side = Side::Sell;

    let action = panel.update(Message::SyncIssue {
        issue_code: "6758".to_string(),
        issue_name: "ソニーグループ".to_string(),
        tick_size: None,
    });

    assert!(
        matches!(
            action,
            Some(Action::FetchHoldings { ref issue_code }) if issue_code == "6758"
        ),
        "銘柄変更 + 売りモードで FetchHoldings を返すべき: {action:?}"
    );
}

#[test]
fn sync_to_different_issue_in_buy_mode_returns_no_action() {
    let mut panel = panel_with("7203");
    // デフォルトは買いモード

    let action = panel.update(Message::SyncIssue {
        issue_code: "6758".to_string(),
        issue_name: "ソニーグループ".to_string(),
        tick_size: None,
    });

    assert!(action.is_none(), "買いモードでは FetchHoldings を返さない");
}

#[test]
fn sync_to_same_issue_in_sell_mode_does_not_fetch_holdings() {
    let mut panel = panel_with("7203");
    panel.side = Side::Sell;
    panel.holdings = Some(100);

    let action = panel.update(Message::SyncIssue {
        issue_code: "7203".to_string(), // 同じ銘柄
        issue_name: "トヨタ自動車".to_string(),
        tick_size: None,
    });

    assert!(
        action.is_none(),
        "同一銘柄の場合は FetchHoldings を返さない（holdings は保持）"
    );
    assert_eq!(panel.holdings, Some(100));
}

#[test]
fn sync_empty_issue_code_in_sell_mode_does_not_fetch_holdings() {
    let mut panel = panel_with("7203");
    panel.side = Side::Sell;

    let action = panel.update(Message::SyncIssue {
        issue_code: "".to_string(), // 銘柄未選択
        issue_name: "".to_string(),
        tick_size: None,
    });

    assert!(
        action.is_none(),
        "銘柄コードが空なら FetchHoldings を返さない"
    );
}

// ── 複数回同期 ────────────────────────────────────────────────────────────────

#[test]
fn sequential_syncs_update_correctly() {
    let mut panel = OrderEntryPanel::new();
    panel.side = Side::Sell;

    // 1回目: 7203 に同期
    let action1 = panel.update(Message::SyncIssue {
        issue_code: "7203".to_string(),
        issue_name: "トヨタ自動車".to_string(),
        tick_size: Some(1.0),
    });
    assert!(matches!(action1, Some(Action::FetchHoldings { .. })));
    assert_eq!(panel.issue_code, "7203");

    // 仮に holdings が取得されたとする
    panel.holdings = Some(300);

    // 2回目: 6758 に切り替え
    let action2 = panel.update(Message::SyncIssue {
        issue_code: "6758".to_string(),
        issue_name: "ソニーグループ".to_string(),
        tick_size: Some(5.0),
    });
    assert!(
        matches!(action2, Some(Action::FetchHoldings { ref issue_code }) if issue_code == "6758")
    );
    assert!(
        panel.holdings.is_none(),
        "銘柄切り替えで holdings がリセットされるべき"
    );
    assert_eq!(panel.issue_code, "6758");
    assert_eq!(panel.tick_size, Some(5.0));
}
