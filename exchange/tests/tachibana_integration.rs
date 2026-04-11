//! 立花証券 API 統合テスト
//!
//! 実際のデモAPIに接続してテストする。
//! 環境変数 TACHIBANA_USER_ID, TACHIBANA_PASSWORD が必要。
//!
//! 実行方法:
//!   cargo test --package flowsurface-exchange --test tachibana_integration -- --nocapture

use flowsurface_exchange::adapter::tachibana::{
    BASE_URL_DEMO, fetch_all_master, login, master_record_to_ticker_info,
};

fn get_credentials() -> Option<(String, String)> {
    // 環境変数を優先し、なければ .env ファイルからフォールバック
    let mut user_id = std::env::var("TACHIBANA_USER_ID").unwrap_or_default();
    let mut password = std::env::var("TACHIBANA_PASSWORD").unwrap_or_default();

    if (user_id.is_empty() || password.is_empty())
        && let Ok(content) = std::fs::read_to_string(
            std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
                .parent()
                .unwrap()
                .join(".env"),
        )
    {
        for line in content.lines() {
            let line = line.trim();
            if line.is_empty() || line.starts_with('#') {
                continue;
            }
            if let Some((key, value)) = line.split_once('=') {
                match key.trim() {
                    "TACHIBANA_USER_ID" => user_id = value.trim().to_string(),
                    "TACHIBANA_PASSWORD" => password = value.trim().to_string(),
                    _ => {}
                }
            }
        }
    }

    if user_id.is_empty() || password.is_empty() {
        return None;
    }
    Some((user_id, password))
}

#[tokio::test]
async fn test_login_and_fetch_master() {
    let (user_id, password) = match get_credentials() {
        Some(creds) => creds,
        None => {
            eprintln!("SKIP: TACHIBANA_USER_ID / TACHIBANA_PASSWORD が設定されていません");
            return;
        }
    };

    let client = reqwest::Client::new();

    // Step 1: ログイン
    eprintln!("=== ログイン開始 ===");
    let session = login(&client, BASE_URL_DEMO, user_id, password)
        .await
        .expect("ログインに失敗しました");

    eprintln!("ログイン成功!");
    eprintln!("  url_request: {}", session.url_request);
    eprintln!("  url_master:  {}", session.url_master);
    eprintln!("  url_price:   {}", session.url_price);
    eprintln!("  url_event:   {}", session.url_event);

    // Step 2: 銘柄マスタダウンロード
    eprintln!("\n=== 銘柄マスタダウンロード開始 ===");
    let records = fetch_all_master(&client, &session)
        .await
        .expect("マスタダウンロードに失敗しました");

    eprintln!("取得レコード数: {}", records.len());
    assert!(
        records.len() > 100,
        "銘柄マスタが100件未満: {} 件しかありません",
        records.len()
    );

    // Step 3: 最初の10件を表示
    eprintln!("\n=== 銘柄マスタ (先頭10件) ===");
    for (i, record) in records.iter().take(10).enumerate() {
        eprintln!(
            "  [{}] {} {} ({})",
            i + 1,
            record.issue_code,
            record.issue_name_short,
            record.issue_name_english,
        );
    }

    // Step 4: TickerInfo への変換テスト
    eprintln!("\n=== TickerInfo 変換テスト ===");
    let mut converted = 0;
    let mut failed = 0;
    for record in &records {
        match master_record_to_ticker_info(record) {
            Some((ticker, info)) => {
                if converted < 5 {
                    eprintln!(
                        "  {:?} => min_qty={:?}, min_tick={:?}",
                        ticker, info.min_qty, info.min_ticksize,
                    );
                }
                converted += 1;
            }
            None => {
                failed += 1;
            }
        }
    }
    eprintln!("変換成功: {} 件, 変換失敗: {} 件", converted, failed);
    assert!(
        converted > 100,
        "TickerInfo 変換成功が100件未満: {} 件",
        converted
    );

    // Step 5: 特定銘柄の存在確認（トヨタ自動車 7203）
    let toyota = records.iter().find(|r| r.issue_code == "7203");
    assert!(
        toyota.is_some(),
        "トヨタ自動車 (7203) が銘柄マスタに存在するべき"
    );
    if let Some(t) = toyota {
        eprintln!("\n=== トヨタ自動車 (7203) ===");
        eprintln!("  名称: {}", t.issue_name);
        eprintln!("  略称: {}", t.issue_name_short);
        eprintln!("  カナ: {}", t.issue_name_kana);
        eprintln!("  英名: {}", t.issue_name_english);
        eprintln!("  市場: {}", t.primary_market);
        eprintln!("  業種: {} ({})", t.sector_name, t.sector_code);
    }

    eprintln!("\n=== 全テスト完了 ===");
}
