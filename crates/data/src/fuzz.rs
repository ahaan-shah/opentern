//! Fuzzing what the network hands us: every parser of a provider's reply,
//! fed mutated replies (`opentern_core::fuzzing`). Any panic is a bug: a
//! malformed or hostile reply must never take a data thread down.

#[cfg(test)]
mod tests {
    use crate::feeds::HotKind;
    use opentern_core::fuzzing::{json_target, text_target};
    use serde_json::json;

    fn ny() -> jiff::tz::TimeZone {
        jiff::tz::TimeZone::get("America/New_York").unwrap()
    }

    #[test]
    fn exchange_replies() {
        json_target(
            "binance klines",
            json!([[
                1700000000000i64,
                "1",
                "2",
                "0.5",
                "1.5",
                "10",
                1700000059999i64,
                "15",
                3,
                "5",
                "7",
                "0"
            ]]),
            |v| {
                let _ = crate::binance::parse_klines(v);
            },
        );
        json_target(
            "binance ws",
            json!({"e":"kline","E":1,"s":"BTCUSDT","k":{"t":1700000000000i64,"o":"1","h":"2","l":"0.5","c":"1.5","v":"10","x":false}}),
            |v| {
                let _ = crate::binance::parse_ws_kline(v);
            },
        );
        json_target(
            "bybit klines",
            json!({"retCode":0,"retMsg":"OK","result":{"list":[["1700003600000","2","4","1","3","20","0"]]}}),
            |v| {
                let _ = crate::bybit::parse_klines(v);
            },
        );
        json_target(
            "new york fed effr",
            json!({"refRates":[
                {"effectiveDate":"2026-11-24","type":"EFFR","percentRate":3.88,"targetRateFrom":3.75,"targetRateTo":4.0,"revisionIndicator":""},
                {"effectiveDate":"2000-07-03","type":"EFFR","percentRate":7.03,"targetRateFrom":6.5,"revisionIndicator":"r"}
            ]}),
            |v| {
                let days = crate::nyfed::parse(v);
                let _ = crate::nyfed::bars(&days);
            },
        );
        json_target(
            "bybit ws",
            json!({"topic":"kline.60.BTCUSDT","data":[{"start":1700000000000i64,"open":"1","close":"2","high":"3","low":"0.5","volume":"7"}]}),
            |v| {
                let _ = crate::bybit::parse_ws(v);
            },
        );
        json_target(
            "hyperliquid candles",
            json!([{"t":1700000000000i64,"T":1700003599999i64,"s":"BTC","i":"1h","o":"1","c":"2","h":"3","l":"0.5","v":"7","n":4}]),
            |v| {
                let _ = crate::hyperliquid::parse_candles(v);
            },
        );
        json_target(
            "hyperliquid ws",
            json!({"channel":"candle","data":{"t":1700000000000i64,"o":"1","c":"2","h":"3","l":"0.5","v":"7"}}),
            |v| {
                let _ = crate::hyperliquid::parse_ws(v);
            },
        );
        json_target(
            "hyperliquid meta",
            json!([{"universe":[{"name":"BTC","szDecimals":5}]},[{"funding":"0.0000125","markPx":"97000"}]]),
            |v| {
                let _ = crate::hyperliquid::parse_perps(v, "USDC");
                let _ = crate::hyperliquid::parse_funding(v, "BTC");
            },
        );
        json_target(
            "okx candles",
            json!({"code":"0","msg":"","data":[["1700003600000","2","4","1","3","20","0","0","1"]]}),
            |v| {
                let _ = crate::okx::parse_candles_at(v, 5);
                let _ = crate::okx::parse_candles_at(v, 6);
            },
        );
        json_target(
            "okx ws",
            json!({"arg":{"channel":"candle1H","instId":"BTC-USDT"},"data":[["1700000000000","1","2","0.5","1.5","9","0","0","0"]]}),
            |v| {
                let _ = crate::okx::parse_ws(v);
                let _ = crate::okx::parse_ws_swap(v);
            },
        );
        json_target(
            "coinbase candles",
            json!([[1700000000, 0.5, 2.0, 1.0, 1.5, 10.0]]),
            |v| {
                let _ = crate::coinbase::parse_candles(v);
            },
        );
        json_target(
            "coinbase match",
            json!({"type":"match","price":"64210.5","size":"0.01","time":"2023-11-14T22:13:20.123456Z"}),
            |v| {
                let _ = crate::coinbase::parse_match(v);
            },
        );
        json_target(
            "kraken ohlc",
            json!({"error":[],"result":{"XXBTZUSD":[[1700000000,"1","2","0.5","1.5","1.2","7.5",3]],"last":1700000000}}),
            |v| {
                let _ = crate::kraken::parse_ohlc(v);
            },
        );
        json_target(
            "kraken ws",
            json!([
                42,
                ["1700000000.0", "1700000060.0", "1", "2", "0.5", "1.5", "1.2", "7.5", 3],
                "ohlc-1",
                "XBT/USD"
            ]),
            |v| {
                let _ = crate::kraken::parse_ws(v);
            },
        );
    }

    #[test]
    fn books_and_lists() {
        json_target(
            "book",
            json!({"lastUpdateId":1,"bids":[["100.5","2.0"],["100.4","1.5"]],"asks":[["100.6","0.7"]]}),
            |v| {
                crate::depth::fuzz_books(v);
            },
        );
        json_target(
            "binance 24h",
            json!([{"symbol":"BTCUSDT","lastPrice":"60000","quoteVolume":"1e9","priceChangePercent":"1.2"}]),
            |v| {
                for k in [HotKind::Gainers, HotKind::Losers, HotKind::Volume] {
                    let _ = crate::feeds::parse_binance_24h(v, k);
                }
            },
        );
        json_target(
            "yahoo screener",
            json!({"finance":{"result":[{"quotes":[{"symbol":"NVDA","regularMarketPrice":186.6,"regularMarketChangePercent":1.2,"regularMarketVolume":1e8,"exchange":"NMS","shortName":"NVIDIA"}]}]}}),
            |v| {
                let _ = crate::feeds::parse_yahoo_screener(v);
            },
        );
        json_target(
            "yahoo news",
            json!({"news":[{"title":"A","publisher":"B","link":"https://x.example/a","providerPublishTime":1700000000}]}),
            |v| {
                let _ = crate::feeds::parse_news(v);
            },
        );
        json_target(
            "trading days",
            json!([{"symbol":"BTCUSDT","openPrice":"60000","lastPrice":"61000"}]),
            |v| {
                let _ = crate::overview::parse_days(v);
            },
        );
    }

    #[test]
    fn yahoo_replies() {
        json_target(
            "yahoo chart",
            json!({"chart":{"result":[{"meta":{"regularMarketPrice":186.6,"regularMarketTime":1700000000,"exchangeTimezoneName":"America/New_York","currentTradingPeriod":{"regular":{"start":1699970000,"end":1699993400}}},
                "timestamp":[1699970000,1699970060],"indicators":{"quote":[{"open":[1.0,null],"high":[2.0,2.1],"low":[0.5,0.4],"close":[1.5,null],"volume":[10,0]}],"adjclose":[{"adjclose":[1.5,1.6]}]},
                "events":{"dividends":{"1":{"amount":0.2,"date":1699970000}},"splits":{"2":{"numerator":2,"denominator":1,"date":1699970000}}}}],"error":null}}),
            |v| {
                let _ = crate::yahoo::parse_chart(v, true);
                let _ = crate::yahoo::parse_chart(v, false);
                let _ = crate::yahoo::parse_events(v);
            },
        );
        json_target(
            "yahoo spark",
            json!({"AAPL":{"timestamp":[1,2,3],"close":[1.0,null,3.0],"fulldayPrice":3.1,"fulldayChangePercent":-0.4,"chartPreviousClose":2.9}}),
            |v| {
                let _ = crate::overview::parse_spark(v);
            },
        );
        json_target(
            "yahoo quotes",
            json!({"finance":{"result":[{"quotes":[{"symbol":"NVDA","marketCap":4.5e12,"regularMarketPrice":186.6,"exchange":"NMS"}]}]}}),
            |v| {
                let _ = crate::overview::parse_quotes(v);
            },
        );
        json_target(
            "fundamentals",
            json!({"quoteSummary":{"result":[{"price":{"marketCap":{"raw":3e12},"currency":"USD","regularMarketPrice":{"raw":190.0}},
                "summaryDetail":{"trailingPE":{"raw":30.0}},"defaultKeyStatistics":{"floatShares":{"raw":1.5e10},"enterpriseValue":{"raw":3.1e12}},
                "financialData":{"ebitda":{"raw":1.2e11},"totalRevenue":{"raw":4e11},"profitMargins":{"raw":0.25},"financialCurrency":"USD"}}],"error":null}}),
            |v| {
                let _ = crate::fundamentals::parse(v);
            },
        );
        json_target(
            "earnings",
            json!({"timeseries":{"result":[{"meta":{"type":["quarterlyEarnings"]},"timestamp":[1700000000],"quarterlyEarnings":[{"asOfDate":"2023-11-14","reportedValue":{"raw":1.2}}]}]},
                "quoteSummary":{"result":[{"calendarEvents":{"earnings":{"earningsDate":[{"raw":1700000000}],"earningsAverage":{"raw":1.1}}}}]},
                "data":{"earningsSurpriseTable":{"rows":[{"dateReported":"11/14/2023","eps":"1.2","consensusForecast":"1.1"}]}}}),
            |v| {
                let tz = ny();
                let _ = crate::earnings::parse_history(v, &tz);
                let _ = crate::earnings::parse_next(v, &tz, 0);
                let _ = crate::earnings::parse_nasdaq(v, &tz);
            },
        );
    }

    #[test]
    fn keyed_provider_replies() {
        json_target(
            "alpaca",
            json!({"bars":[{"t":"2024-01-02T14:30:00Z","o":1.0,"h":2.0,"l":0.5,"c":1.5,"v":10.0}],
                "trades":[{"T":"t","S":"AAPL","p":185.5,"s":100,"t":"2024-01-02T14:30:01.5Z"}]}),
            |v| {
                let tz = ny();
                let _ = crate::alpaca::parse_bars(v, true, &tz);
                let _ = crate::alpaca::parse_bars(v, false, &tz);
                let _ = crate::alpaca::parse_trades(v);
                let _ = crate::alpaca::parse_trades(&v["trades"]);
            },
        );
        json_target(
            "oanda",
            json!({"candles":[{"time":"2024-01-02T14:00:00.000000000Z","complete":true,"volume":10,"mid":{"o":"1.09","h":"1.10","l":"1.08","c":"1.095"}}],
                "type":"PRICE","bids":[{"price":"1.09440"}],"asks":[{"price":"1.09460"}]}),
            |v| {
                let _ = crate::oanda::parse_candles(v);
                let _ = crate::oanda::parse_price(v);
            },
        );
        json_target(
            "upstox",
            json!({"status":"success","data":{"candles":[["2025-01-02T09:15:00+05:30",1,2,0.5,1.5,10,0]],"NSE_EQ:RELIANCE":{"last_price":1255.5,"volume":123456}}}),
            |v| {
                let _ = crate::upstox::parse_candles(v, true);
                let _ = crate::upstox::parse_candles(v, false);
                let _ = crate::upstox::parse_quote(v);
                let mut out = std::collections::HashMap::new();
                crate::upstox::parse_master(v, &mut out);
            },
        );
        json_target(
            "ibkr",
            json!({"symbol":"AAPL","data":[{"o":185.0,"c":186.0,"h":187.0,"l":184.0,"v":1000.0,"t":1704205800000i64}],
                "list":[{"conid":265598,"31":"C185.50","87_raw":1200000.0}]}),
            |v| {
                let tz = ny();
                let _ = crate::ibkr::parse_history(v, true, &tz);
                let _ = crate::ibkr::parse_history(v, false, &tz);
                let _ = crate::ibkr::parse_conid(&v["list"]);
                let _ = crate::ibkr::parse_snapshot(&v["list"]);
            },
        );
    }

    #[test]
    fn text_replies() {
        let rss = r#"<?xml version="1.0"?><rss><channel><item><title>Bitcoin rises &amp; falls</title><link>https://x.example/a?b=1&amp;c=2</link>
            <pubDate>Tue, 14 Nov 2023 22:13:20 GMT</pubDate><source url="https://s.example">Src</source><description><![CDATA[<p>hi</p>]]></description></item></channel></rss>"#;
        text_target("rss", rss, |s| {
            let _ = crate::feeds::parse_rss(s, "X");
            let _ = crate::feeds::parse_rss(s, "Google News");
        });
        text_target(
            "sp500 list",
            "Symbol,Security,GICS Sector,GICS Sub-Industry,Exchange\nAAPL,Apple Inc.,Information Technology,\"Hardware, Storage\",NASDAQ\n",
            |s| {
                let _ = crate::overview::parse_sp500(s);
            },
        );
        text_target(
            "nifty500 list",
            "Company Name,Industry,Symbol,Series,ISIN Code\nReliance Industries Ltd.,Oil Gas,RELIANCE,EQ,INE002A01018\n",
            |s| {
                let _ = crate::overview::parse_nifty500(s);
            },
        );
        text_target("time", "2024-01-02T14:30:01.123456789Z", |s| {
            let _ = crate::provider::parse_time(s);
        });
        // Formulas come from the user and from imported watchlists.
        text_target("formula", "(BINANCE:BTCUSDT + NASDAQ:AAPL * 2) / 3 - NSE:M&M", |s| {
            let _ = crate::expr::parse(s);
            let _ = crate::expr::is_expr(s);
        });
    }

    #[test]
    fn deep_formulas_dont_overflow_the_stack() {
        // A quarter of Windows' 1 MB main thread: room for its larger
        // debug frames.
        std::thread::Builder::new()
            .stack_size(1 << 18)
            .spawn(|| {
                let deep = format!("{}BINANCE:BTCUSDT{}", "(".repeat(50_000), ")".repeat(50_000));
                let _ = crate::expr::parse(&deep);
                let _ = crate::expr::parse(&"-".repeat(50_000));
                let _ = crate::expr::parse(&"A + ".repeat(50_000));
                // At the cap: a 1,000-member index (`X*w + …`) parses, renders
                // and is freed without running out of stack.
                let big = format!("{}A*2", "A*2 + ".repeat(999));
                let (e, legs) = crate::expr::parse(&big).expect("a long sum parses");
                assert_eq!(legs.len(), 1);
                let _ = crate::expr::render(&e, &|i| format!("x{i}"));
                assert_eq!(e.eval(&[1.0]), 2000.0);
                drop(e);
            })
            .unwrap()
            .join()
            .unwrap();
    }
}
