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
}
