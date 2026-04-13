# テスト用 saved-state.json テンプレート集

**配置先**: `$APPDATA/flowsurface/saved-state.json`  
**注意**: Windows では `/tmp/` は使えない。`C:/tmp/` を使用すること。

## テストデータの選び方

| 項目 | 推奨値 | 理由 |
|------|--------|------|
| ティッカー | `BinanceLinear:BTCUSDT` | データ豊富。`fetch_trades_batched()` は Binance のみ対応 |
| タイムフレーム | `M1` | 本数が多いが fetch_klines は自動ページングで取得 |
| `timezone` | `"UTC"` | API の unix ms との照合が容易 |
| `trade_fetch_enabled` | `false` | ライブ trades フェッチを止めてノイズ削減 |
| ペイン数 | 最小限 | フェッチ対象減でテスト高速化 |
| リプレイ日時 | 過去 24-48h 以内 | Binance API からデータ取得可能な範囲 |
| リプレイ範囲 | テスト目的による | 短い(1-3h)ほど高速。制限なし |

---

## 1. 最小構成（KlineChart 1枚）

フェッチが速く、基本動作テストに最適:

```json
{
  "layout_manager": {
    "layouts": [{
      "name": "E2E-Test",
      "dashboard": {
        "pane": {
          "KlineChart": {
            "layout": { "splits": [0.78], "autoscale": "FitToVisible" },
            "kind": "Candles",
            "stream_type": [{ "Kline": { "ticker": "BinanceLinear:BTCUSDT", "timeframe": "M1" } }],
            "settings": { "tick_multiply": null, "visual_config": null, "selected_basis": { "Time": "M1" } },
            "indicators": ["Volume"],
            "link_group": "A"
          }
        },
        "popout": []
      }
    }],
    "active_layout": "E2E-Test"
  },
  "timezone": "UTC",
  "trade_fetch_enabled": false,
  "size_in_quote_ccy": "Base"
}
```

## 2. 最小構成 + リプレイ復元テスト

```json
{
  "layout_manager": {
    "layouts": [{
      "name": "E2E-Test",
      "dashboard": {
        "pane": {
          "KlineChart": {
            "layout": { "splits": [0.78], "autoscale": "FitToVisible" },
            "kind": "Candles",
            "stream_type": [{ "Kline": { "ticker": "BinanceLinear:BTCUSDT", "timeframe": "M1" } }],
            "settings": { "tick_multiply": null, "visual_config": null, "selected_basis": { "Time": "M1" } },
            "indicators": ["Volume"],
            "link_group": "A"
          }
        },
        "popout": []
      }
    }],
    "active_layout": "E2E-Test"
  },
  "timezone": "UTC",
  "trade_fetch_enabled": false,
  "size_in_quote_ccy": "Base",
  "replay": {
    "mode": "replay",
    "range_start": "2026-04-10 09:00",
    "range_end": "2026-04-10 15:00"
  }
}
```

## 3. マルチペイン構成（KlineChart + TimeAndSales + Ladder）

複数ペインの連動・ストリーム接続テスト用:

```json
{
  "layout_manager": {
    "layouts": [{
      "name": "E2E-MultiPane",
      "dashboard": {
        "pane": {
          "Split": {
            "axis": "Vertical", "ratio": 0.5,
            "a": {
              "KlineChart": {
                "layout": { "splits": [0.78], "autoscale": "FitToVisible" },
                "kind": "Candles",
                "stream_type": [{ "Kline": { "ticker": "BinanceLinear:BTCUSDT", "timeframe": "M1" } }],
                "settings": { "tick_multiply": null, "visual_config": null, "selected_basis": { "Time": "M1" } },
                "indicators": ["Volume"],
                "link_group": "A"
              }
            },
            "b": {
              "Split": {
                "axis": "Horizontal", "ratio": 0.5,
                "a": {
                  "TimeAndSales": {
                    "stream_type": [{ "Trades": { "ticker": "BinanceLinear:BTCUSDT" } }],
                    "settings": { "tick_multiply": null, "visual_config": null, "selected_basis": { "Time": "MS100" } },
                    "link_group": "A"
                  }
                },
                "b": {
                  "Ladder": {
                    "stream_type": [
                      { "Depth": { "ticker": "BinanceLinear:BTCUSDT", "depth_aggr": "Client", "push_freq": "ServerDefault" } },
                      { "Trades": { "ticker": "BinanceLinear:BTCUSDT" } }
                    ],
                    "settings": { "tick_multiply": 5, "visual_config": null, "selected_basis": { "Time": "MS100" } },
                    "link_group": "A"
                  }
                }
              }
            }
          }
        },
        "popout": []
      }
    }],
    "active_layout": "E2E-MultiPane"
  },
  "timezone": "UTC",
  "trade_fetch_enabled": false,
  "size_in_quote_ccy": "Base"
}
```

## 4. マルチレイアウト構成（レイアウト管理テスト用）

```json
{
  "layout_manager": {
    "layouts": [
      {
        "name": "Layout-A",
        "dashboard": {
          "pane": {
            "KlineChart": {
              "layout": { "splits": [0.78], "autoscale": "FitToVisible" },
              "kind": "Candles",
              "stream_type": [{ "Kline": { "ticker": "BinanceLinear:BTCUSDT", "timeframe": "M1" } }],
              "settings": { "tick_multiply": null, "visual_config": null, "selected_basis": { "Time": "M1" } },
              "indicators": ["Volume"],
              "link_group": "A"
            }
          },
          "popout": []
        }
      },
      {
        "name": "Layout-B",
        "dashboard": {
          "pane": {
            "KlineChart": {
              "layout": { "splits": [0.78], "autoscale": "FitToVisible" },
              "kind": "Candles",
              "stream_type": [{ "Kline": { "ticker": "BinanceLinear:ETHUSDT", "timeframe": "M5" } }],
              "settings": { "tick_multiply": null, "visual_config": null, "selected_basis": { "Time": "M5" } },
              "indicators": ["Volume"],
              "link_group": "A"
            }
          },
          "popout": []
        }
      }
    ],
    "active_layout": "Layout-A"
  },
  "timezone": "UTC",
  "trade_fetch_enabled": false,
  "size_in_quote_ccy": "Base"
}
```

## 5. マルチペイン + リプレイ（回帰テスト用）

kline/trades 分離修正の回帰確認用（BTCUSDT KlineChart + ETHUSDT KlineChart + BTCUSDT TimeAndSales）:

```json
{
  "layout_manager": {
    "layouts": [{
      "name": "E2E-MultiPane-Replay",
      "dashboard": {
        "pane": {
          "Split": {
            "axis": "Vertical", "ratio": 0.33,
            "a": {
              "KlineChart": {
                "layout": { "splits": [0.78], "autoscale": "FitToVisible" },
                "kind": "Candles",
                "stream_type": [{ "Kline": { "ticker": "BinanceLinear:BTCUSDT", "timeframe": "M1" } }],
                "settings": { "tick_multiply": null, "visual_config": null, "selected_basis": { "Time": "M1" } },
                "indicators": ["Volume"],
                "link_group": "A"
              }
            },
            "b": {
              "Split": {
                "axis": "Vertical", "ratio": 0.5,
                "a": {
                  "KlineChart": {
                    "layout": { "splits": [0.78], "autoscale": "FitToVisible" },
                    "kind": "Candles",
                    "stream_type": [{ "Kline": { "ticker": "BinanceLinear:ETHUSDT", "timeframe": "M1" } }],
                    "settings": { "tick_multiply": null, "visual_config": null, "selected_basis": { "Time": "M1" } },
                    "indicators": ["Volume"],
                    "link_group": "B"
                  }
                },
                "b": {
                  "TimeAndSales": {
                    "stream_type": [{ "Trades": { "ticker": "BinanceLinear:BTCUSDT" } }],
                    "settings": { "tick_multiply": null, "visual_config": null, "selected_basis": { "Time": "MS100" } },
                    "link_group": "A"
                  }
                }
              }
            }
          }
        },
        "popout": []
      }
    }],
    "active_layout": "E2E-MultiPane-Replay"
  },
  "timezone": "UTC",
  "trade_fetch_enabled": false,
  "size_in_quote_ccy": "Base",
  "replay": {
    "mode": "replay",
    "range_start": "YYYY-MM-DD HH:MM",
    "range_end": "YYYY-MM-DD HH:MM"
  }
}
```

**注意**: `range_start` / `range_end` は過去 24-48h 以内、12 時間レンジを推奨。
