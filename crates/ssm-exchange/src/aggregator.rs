use rust_decimal::Decimal;
use ssm_core::{Candle, Side, Trade};

/// In-candle trade aggregator (aggr.trade-inspired).
///
/// Accumulates individual trades into a forming candle,
/// tracking buy/sell volume separately for CVD computation.
pub struct TradeAggregator {
    symbol: String,
    interval_ms: i64,
    current: Option<FormingCandle>,
    closed: Vec<Candle>,
}

struct FormingCandle {
    open_time: i64,
    open: Decimal,
    high: Decimal,
    low: Decimal,
    close: Decimal,
    volume: Decimal,
    quote_volume: Decimal,
    trades: u64,
    taker_buy_volume: Decimal,
    taker_sell_volume: Decimal,
}

impl TradeAggregator {
    pub fn new(symbol: &str, interval_ms: i64) -> Self {
        Self {
            symbol: symbol.to_string(),
            interval_ms,
            current: None,
            closed: Vec::new(),
        }
    }

    /// Ingest a single trade tick. Returns a closed candle if the interval boundary was crossed.
    pub fn ingest(&mut self, trade: &Trade) -> Option<Candle> {
        let candle_start = (trade.timestamp / self.interval_ms) * self.interval_ms;
        let mut closed = None;

        // Check if we need to close the current candle
        if let Some(ref forming) = self.current {
            if candle_start > forming.open_time {
                closed = Some(self.close_candle());
            }
        }

        // Update or start forming candle
        match &mut self.current {
            Some(c) => {
                if trade.price > c.high {
                    c.high = trade.price;
                }
                if trade.price < c.low {
                    c.low = trade.price;
                }
                c.close = trade.price;
                c.volume += trade.quantity;
                c.quote_volume += trade.price * trade.quantity;
                c.trades += 1;
                match trade.side {
                    Side::Buy => c.taker_buy_volume += trade.quantity,
                    Side::Sell => c.taker_sell_volume += trade.quantity,
                }
            }
            None => {
                self.current = Some(FormingCandle {
                    open_time: candle_start,
                    open: trade.price,
                    high: trade.price,
                    low: trade.price,
                    close: trade.price,
                    volume: trade.quantity,
                    quote_volume: trade.price * trade.quantity,
                    trades: 1,
                    taker_buy_volume: if trade.side == Side::Buy {
                        trade.quantity
                    } else {
                        Decimal::ZERO
                    },
                    taker_sell_volume: if trade.side == Side::Sell {
                        trade.quantity
                    } else {
                        Decimal::ZERO
                    },
                });
            }
        }

        closed
    }

    /// Get all closed candles so far.
    pub fn closed_candles(&self) -> &[Candle] {
        &self.closed
    }

    /// Drain closed candles, returning ownership.
    pub fn drain_closed(&mut self) -> Vec<Candle> {
        std::mem::take(&mut self.closed)
    }

    fn close_candle(&mut self) -> Candle {
        let c = self.current.take().unwrap();
        let candle = Candle {
            open_time: c.open_time,
            open: c.open,
            high: c.high,
            low: c.low,
            close: c.close,
            volume: c.volume,
            close_time: c.open_time + self.interval_ms - 1,
            quote_volume: c.quote_volume,
            trades: c.trades,
            taker_buy_volume: c.taker_buy_volume,
            taker_sell_volume: c.taker_sell_volume,
        };
        self.closed.push(candle.clone());

        tracing::debug!(
            symbol = %self.symbol,
            open_time = c.open_time,
            trades = c.trades,
            volume = %c.volume,
            "candle closed"
        );

        candle
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::str::FromStr;

    fn trade(price: &str, qty: &str, side: Side, ts: i64) -> Trade {
        Trade {
            symbol: "BTCUSDT".into(),
            price: Decimal::from_str(price).unwrap(),
            quantity: Decimal::from_str(qty).unwrap(),
            side,
            timestamp: ts,
            is_liquidation: false,
        }
    }

    #[test]
    fn single_trade_no_close() {
        let mut agg = TradeAggregator::new("BTCUSDT", 60_000); // 1 minute
        let t = trade("50000", "1.0", Side::Buy, 60_000);
        assert!(agg.ingest(&t).is_none());
    }

    #[test]
    fn candle_closes_on_interval_boundary() {
        let mut agg = TradeAggregator::new("BTCUSDT", 60_000);

        // Trades in first minute
        agg.ingest(&trade("50000", "1.0", Side::Buy, 60_000));
        agg.ingest(&trade("50100", "0.5", Side::Sell, 60_500));
        agg.ingest(&trade("49900", "2.0", Side::Buy, 61_000));

        // Trade in second minute — triggers close
        let closed = agg.ingest(&trade("50200", "1.0", Side::Buy, 120_000));
        assert!(closed.is_some());

        let c = closed.unwrap();
        assert_eq!(c.open, Decimal::from_str("50000").unwrap());
        assert_eq!(c.high, Decimal::from_str("50100").unwrap());
        assert_eq!(c.low, Decimal::from_str("49900").unwrap());
        assert_eq!(c.close, Decimal::from_str("49900").unwrap());
        assert_eq!(c.trades, 3);
        assert_eq!(c.taker_buy_volume, Decimal::from_str("3.0").unwrap());
        assert_eq!(c.taker_sell_volume, Decimal::from_str("0.5").unwrap());
    }

    #[test]
    fn drain_returns_and_clears() {
        let mut agg = TradeAggregator::new("BTCUSDT", 60_000);
        agg.ingest(&trade("100", "1", Side::Buy, 0));
        agg.ingest(&trade("101", "1", Side::Buy, 60_000)); // closes first candle

        let drained = agg.drain_closed();
        assert_eq!(drained.len(), 1);
        assert!(agg.closed_candles().is_empty());
    }

    #[test]
    fn ohlcv_accuracy() {
        let mut agg = TradeAggregator::new("BTCUSDT", 60_000);
        agg.ingest(&trade("100", "1", Side::Buy, 0)); // open
        agg.ingest(&trade("110", "1", Side::Buy, 1000)); // high
        agg.ingest(&trade("90", "1", Side::Sell, 2000)); // low
        agg.ingest(&trade("105", "1", Side::Buy, 3000)); // close

        // Next interval triggers close
        let c = agg.ingest(&trade("106", "1", Side::Buy, 60_000)).unwrap();
        assert_eq!(c.open, Decimal::from(100));
        assert_eq!(c.high, Decimal::from(110));
        assert_eq!(c.low, Decimal::from(90));
        assert_eq!(c.close, Decimal::from(105));
        assert_eq!(c.volume, Decimal::from(4));
    }

    #[test]
    fn test_multiple_candle_closes() {
        let mut agg = TradeAggregator::new("BTCUSDT", 60_000);

        // Interval 0: [0, 60_000)
        agg.ingest(&trade("100", "1", Side::Buy, 0));
        agg.ingest(&trade("101", "1", Side::Buy, 30_000));

        // Interval 1: [60_000, 120_000) — closes interval 0
        let c1 = agg.ingest(&trade("102", "1", Side::Buy, 60_000));
        assert!(c1.is_some());
        let c1 = c1.unwrap();
        assert_eq!(c1.open_time, 0);
        assert_eq!(c1.trades, 2);

        agg.ingest(&trade("103", "1", Side::Sell, 90_000));

        // Interval 2: [120_000, 180_000) — closes interval 1
        let c2 = agg.ingest(&trade("104", "1", Side::Buy, 120_000));
        assert!(c2.is_some());
        let c2 = c2.unwrap();
        assert_eq!(c2.open_time, 60_000);
        assert_eq!(c2.trades, 2);

        agg.ingest(&trade("105", "1", Side::Buy, 150_000));

        // Interval 3: [180_000, 240_000) — closes interval 2
        let c3 = agg.ingest(&trade("106", "1", Side::Buy, 180_000));
        assert!(c3.is_some());
        let c3 = c3.unwrap();
        assert_eq!(c3.open_time, 120_000);
        assert_eq!(c3.trades, 2);

        // Verify closed_candles accumulated all 3
        assert_eq!(agg.closed_candles().len(), 3);
    }

    #[test]
    fn test_buy_sell_volume_tracking() {
        let mut agg = TradeAggregator::new("BTCUSDT", 60_000);

        // 3 buys
        agg.ingest(&trade("100", "1.0", Side::Buy, 0));
        agg.ingest(&trade("101", "2.0", Side::Buy, 1000));
        agg.ingest(&trade("102", "0.5", Side::Buy, 2000));

        // 2 sells
        agg.ingest(&trade("99", "1.5", Side::Sell, 3000));
        agg.ingest(&trade("98", "3.0", Side::Sell, 4000));

        // Close the candle
        let c = agg.ingest(&trade("100", "1", Side::Buy, 60_000)).unwrap();

        assert_eq!(c.taker_buy_volume, Decimal::from_str("3.5").unwrap()); // 1.0 + 2.0 + 0.5
        assert_eq!(c.taker_sell_volume, Decimal::from_str("4.5").unwrap()); // 1.5 + 3.0
        assert_eq!(c.volume, Decimal::from_str("8.0").unwrap()); // 3.5 + 4.5
        assert_eq!(c.trades, 5);
    }

    #[test]
    fn test_close_time_calculation() {
        let interval_ms: i64 = 60_000;
        let mut agg = TradeAggregator::new("BTCUSDT", interval_ms);

        agg.ingest(&trade("100", "1", Side::Buy, 0));
        let c = agg.ingest(&trade("101", "1", Side::Buy, 60_000)).unwrap();

        // close_time should be open_time + interval_ms - 1
        assert_eq!(c.close_time, c.open_time + interval_ms - 1);
        assert_eq!(c.close_time, 59_999);
    }

    #[test]
    fn test_quote_volume_accumulation() {
        let mut agg = TradeAggregator::new("BTCUSDT", 60_000);

        // Trade 1: price=100, qty=2 → quote_volume = 200
        agg.ingest(&trade("100", "2", Side::Buy, 0));
        // Trade 2: price=150, qty=3 → quote_volume = 450
        agg.ingest(&trade("150", "3", Side::Sell, 1000));
        // Trade 3: price=200, qty=1 → quote_volume = 200
        agg.ingest(&trade("200", "1", Side::Buy, 2000));

        let c = agg.ingest(&trade("100", "1", Side::Buy, 60_000)).unwrap();

        // Expected quote_volume = 200 + 450 + 200 = 850
        assert_eq!(c.quote_volume, Decimal::from(850));
    }

    #[test]
    fn test_aggregator_different_intervals() {
        // 5-minute interval = 300_000 ms
        let mut agg = TradeAggregator::new("BTCUSDT", 300_000);

        // Trades within first 5-minute window [0, 300_000)
        agg.ingest(&trade("100", "1", Side::Buy, 0));
        agg.ingest(&trade("105", "1", Side::Buy, 60_000)); // 1 min in
        agg.ingest(&trade("95", "1", Side::Sell, 120_000)); // 2 min in
        agg.ingest(&trade("110", "1", Side::Buy, 240_000)); // 4 min in

        // No close yet — still in first 5-minute window
        assert!(agg.closed_candles().is_empty());

        // Trade at 300_000 starts new window, closing the first
        let c = agg.ingest(&trade("108", "1", Side::Buy, 300_000)).unwrap();
        assert_eq!(c.open_time, 0);
        assert_eq!(c.close_time, 299_999); // open_time + 300_000 - 1
        assert_eq!(c.open, Decimal::from(100));
        assert_eq!(c.high, Decimal::from(110));
        assert_eq!(c.low, Decimal::from(95));
        assert_eq!(c.close, Decimal::from(110));
        assert_eq!(c.trades, 4);
    }
}
