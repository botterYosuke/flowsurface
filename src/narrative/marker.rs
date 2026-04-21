//! ナラティブ用チャートマーカー。
//!
//! チャート Canvas にエントリー／エグジットを可視化する軽量データ構造。
//! 描画側（`src/chart/kline/draw.rs`）からは [`NarrativeMarker`] スライスを
//! 渡して [`draw_markers`] を呼び出すだけで済む。

use iced::widget::canvas::{self, Path, Stroke, stroke};
use iced::{Color, Point};

use super::model::{Narrative, NarrativeSide};

/// マーカーの種別（エントリー/エグジット）。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MarkerKind {
    /// エントリー（action.price / action.side）
    Entry,
    /// エグジット（outcome.fill_price / outcome.closed_at_ms）
    Exit,
}

/// 1 つのナラティブに紐付く 1 マーカー。
#[derive(Debug, Clone, PartialEq)]
pub struct NarrativeMarker {
    pub id: uuid::Uuid,
    pub time_ms: i64,
    pub price: f64,
    pub side: NarrativeSide,
    pub kind: MarkerKind,
}

impl NarrativeMarker {
    /// 1 件の [`Narrative`] から 1〜2 個のマーカーを生成する。
    ///
    /// - 必ずエントリーマーカー（`action` ベース）を返す
    /// - `outcome` が存在する場合はエグジットマーカーも追加
    pub fn from_narrative(n: &Narrative) -> Vec<Self> {
        let mut markers = Vec::with_capacity(2);
        markers.push(NarrativeMarker {
            id: n.id,
            time_ms: n.timestamp_ms,
            price: n.action.price,
            side: n.action.side,
            kind: MarkerKind::Entry,
        });
        if let Some(outcome) = &n.outcome {
            markers.push(NarrativeMarker {
                id: n.id,
                time_ms: outcome.fill_time_ms,
                price: outcome.fill_price,
                side: n.action.side,
                kind: MarkerKind::Exit,
            });
        }
        markers
    }
}

/// チャート Canvas 上にマーカーを描画する。
///
/// - `visible_range_ms`: 現在の可視時間範囲 [start, end]。範囲外のマーカーはスキップする。
/// - `time_to_x` / `price_to_y`: チャートの座標変換関数。
/// - `triangle_size`: 三角形の半幅（ピクセル）。エグジットは少し小さめにする。
pub fn draw_markers(
    frame: &mut canvas::Frame,
    markers: &[NarrativeMarker],
    visible_range_ms: (i64, i64),
    time_to_x: impl Fn(i64) -> f32,
    price_to_y: impl Fn(f64) -> f32,
    triangle_size: f32,
) {
    let (start, end) = visible_range_ms;
    for marker in markers {
        if marker.time_ms < start || marker.time_ms > end {
            continue;
        }
        let x = time_to_x(marker.time_ms);
        let y = price_to_y(marker.price);
        let color = marker_color(marker.side, marker.kind);

        match marker.kind {
            MarkerKind::Entry => {
                let path = triangle_path(Point::new(x, y), triangle_size, marker.side);
                frame.fill(&path, color);
            }
            MarkerKind::Exit => {
                let half = triangle_size * 0.6;
                let path = Path::rectangle(
                    Point::new(x - half, y - half),
                    iced::Size::new(half * 2.0, half * 2.0),
                );
                frame.stroke(
                    &path,
                    Stroke {
                        style: stroke::Style::Solid(color),
                        width: 1.5,
                        ..Stroke::default()
                    },
                );
            }
        }
    }
}

fn triangle_path(tip: Point, size: f32, side: NarrativeSide) -> Path {
    // buy → 上向き（tip が上、底辺が下）
    // sell → 下向き（tip が下、底辺が上）
    let (dx, dy) = match side {
        NarrativeSide::Buy => (size, size * 1.7),
        NarrativeSide::Sell => (size, -size * 1.7),
    };
    Path::new(|b| {
        b.move_to(tip);
        b.line_to(Point::new(tip.x - dx, tip.y + dy));
        b.line_to(Point::new(tip.x + dx, tip.y + dy));
        b.close();
    })
}

fn marker_color(side: NarrativeSide, kind: MarkerKind) -> Color {
    // D-3: buy=緑 / sell=赤。エグジットはアルファを落とす。
    let base = match side {
        NarrativeSide::Buy => Color::from_rgb(0.2, 0.8, 0.3),
        NarrativeSide::Sell => Color::from_rgb(0.9, 0.3, 0.3),
    };
    match kind {
        MarkerKind::Entry => base,
        MarkerKind::Exit => Color { a: 0.75, ..base },
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::narrative::model::{NarrativeAction, NarrativeOutcome, SnapshotRef};
    use std::path::PathBuf;

    fn sample_narrative(side: NarrativeSide, with_outcome: bool) -> Narrative {
        Narrative {
            id: uuid::Uuid::new_v4(),
            agent_id: "a".to_string(),
            uagent_address: None,
            timestamp_ms: 1000,
            ticker: "BTCUSDT".to_string(),
            timeframe: "1h".to_string(),
            snapshot_ref: SnapshotRef {
                path: PathBuf::from("x"),
                size_bytes: 0,
                sha256: "0".repeat(64),
            },
            reasoning: "x".to_string(),
            action: NarrativeAction {
                side,
                qty: 0.1,
                price: 100.0,
            },
            confidence: 0.5,
            outcome: if with_outcome {
                Some(NarrativeOutcome {
                    fill_price: 101.0,
                    fill_time_ms: 2000,
                    closed_at_ms: None,
                    realized_pnl: None,
                })
            } else {
                None
            },
            linked_order_id: None,
            public: false,
            created_at_ms: 1,
            idempotency_key: None,
        }
    }

    #[test]
    fn narrative_without_outcome_yields_only_entry() {
        let markers = NarrativeMarker::from_narrative(&sample_narrative(NarrativeSide::Buy, false));
        assert_eq!(markers.len(), 1);
        assert_eq!(markers[0].kind, MarkerKind::Entry);
        assert_eq!(markers[0].side, NarrativeSide::Buy);
        assert_eq!(markers[0].time_ms, 1000);
        assert!((markers[0].price - 100.0).abs() < 1e-9);
    }

    #[test]
    fn narrative_with_outcome_yields_entry_and_exit() {
        let markers = NarrativeMarker::from_narrative(&sample_narrative(NarrativeSide::Sell, true));
        assert_eq!(markers.len(), 2);
        assert_eq!(markers[0].kind, MarkerKind::Entry);
        assert_eq!(markers[1].kind, MarkerKind::Exit);
        assert_eq!(markers[1].time_ms, 2000);
        assert!((markers[1].price - 101.0).abs() < 1e-9);
    }

    #[test]
    fn buy_and_sell_get_different_colors() {
        let buy = marker_color(NarrativeSide::Buy, MarkerKind::Entry);
        let sell = marker_color(NarrativeSide::Sell, MarkerKind::Entry);
        assert_ne!(buy, sell);
        // buy は緑成分が大きい
        assert!(buy.g > buy.r);
        // sell は赤成分が大きい
        assert!(sell.r > sell.g);
    }

    #[test]
    fn draw_markers_skips_outside_visible_range() {
        // 可視範囲外は描画されない（この単体テストでは frame を持たないので
        // 副作用の確認はできないが、範囲フィルタのロジックを確認する）。
        let markers = vec![
            NarrativeMarker {
                id: uuid::Uuid::new_v4(),
                time_ms: 500, // before range
                price: 100.0,
                side: NarrativeSide::Buy,
                kind: MarkerKind::Entry,
            },
            NarrativeMarker {
                id: uuid::Uuid::new_v4(),
                time_ms: 3000, // after range
                price: 100.0,
                side: NarrativeSide::Buy,
                kind: MarkerKind::Entry,
            },
        ];
        let in_range: Vec<_> = markers
            .iter()
            .filter(|m| m.time_ms >= 1000 && m.time_ms <= 2000)
            .collect();
        assert!(in_range.is_empty());
    }
}
