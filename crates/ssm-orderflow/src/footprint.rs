use rust_decimal::Decimal;
use serde::{Deserialize, Serialize};
use ssm_core::Trade;
use std::collections::BTreeMap;

/// A single row in a footprint chart — bid/ask volume at a price level.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FootprintRow {
    pub price_level: Decimal,
    pub bid_volume: Decimal,
    pub ask_volume: Decimal,
    pub delta: Decimal,
}

/// Footprint chart for a single candle — volume bucketed by price level.
///
/// Tracks bid (sell) and ask (buy) volume at each price level to show
/// where aggressive buying/selling occurred.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FootprintCandle {
    pub open_time: i64,
    pub close_time: i64,
    pub tick_size: Decimal,
    pub rows: Vec<FootprintRow>,
    pub total_bid_volume: Decimal,
    pub total_ask_volume: Decimal,
    pub total_delta: Decimal,
}

/// Builds a footprint candle from a sequence of trades within one candle interval.
pub struct FootprintBuilder {
    tick_size: Decimal,
    open_time: i64,
    close_time: i64,
    levels: BTreeMap<Decimal, (Decimal, Decimal)>, // price_level -> (bid_vol, ask_vol)
}

impl FootprintBuilder {
    pub fn new(tick_size: Decimal, open_time: i64, close_time: i64) -> Self {
        Self {
            tick_size,
            open_time,
            close_time,
            levels: BTreeMap::new(),
        }
    }

    /// Bucket a trade price to its footprint level.
    fn bucket(&self, price: Decimal) -> Decimal {
        if self.tick_size.is_zero() {
            return price;
        }
        (price / self.tick_size).floor() * self.tick_size
    }

    /// Add a trade to the footprint.
    pub fn add_trade(&mut self, trade: &Trade) {
        let level = self.bucket(trade.price);
        let entry = self
            .levels
            .entry(level)
            .or_insert((Decimal::ZERO, Decimal::ZERO));
        match trade.side {
            ssm_core::Side::Buy => entry.1 += trade.quantity, // ask (aggressive buy)
            ssm_core::Side::Sell => entry.0 += trade.quantity, // bid (aggressive sell)
        }
    }

    /// Build the final footprint candle.
    pub fn build(self) -> FootprintCandle {
        let mut total_bid = Decimal::ZERO;
        let mut total_ask = Decimal::ZERO;

        let rows: Vec<FootprintRow> = self
            .levels
            .into_iter()
            .map(|(price_level, (bid_vol, ask_vol))| {
                total_bid += bid_vol;
                total_ask += ask_vol;
                FootprintRow {
                    price_level,
                    bid_volume: bid_vol,
                    ask_volume: ask_vol,
                    delta: ask_vol - bid_vol,
                }
            })
            .collect();

        FootprintCandle {
            open_time: self.open_time,
            close_time: self.close_time,
            tick_size: self.tick_size,
            rows,
            total_bid_volume: total_bid,
            total_ask_volume: total_ask,
            total_delta: total_ask - total_bid,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::str::FromStr;

    fn trade(price: &str, qty: &str, side: ssm_core::Side) -> Trade {
        Trade {
            symbol: "BTCUSDT".into(),
            price: Decimal::from_str(price).unwrap(),
            quantity: Decimal::from_str(qty).unwrap(),
            side,
            timestamp: 1000,
            is_liquidation: false,
        }
    }

    #[test]
    fn footprint_basic() {
        let mut builder = FootprintBuilder::new(Decimal::from(100), 0, 59999);
        builder.add_trade(&trade("50050", "1.0", ssm_core::Side::Buy));
        builder.add_trade(&trade("50050", "0.5", ssm_core::Side::Sell));
        builder.add_trade(&trade("50150", "2.0", ssm_core::Side::Buy));

        let fp = builder.build();
        assert_eq!(fp.rows.len(), 2);
        assert_eq!(fp.total_ask_volume, Decimal::from_str("3.0").unwrap());
        assert_eq!(fp.total_bid_volume, Decimal::from_str("0.5").unwrap());
        assert!(fp.total_delta > Decimal::ZERO); // net buying
    }

    #[test]
    fn bucket_rounding() {
        let builder = FootprintBuilder::new(Decimal::from(100), 0, 0);
        assert_eq!(builder.bucket(Decimal::from(50_050)), Decimal::from(50_000));
        assert_eq!(builder.bucket(Decimal::from(50_100)), Decimal::from(50_100));
        assert_eq!(builder.bucket(Decimal::from(50_199)), Decimal::from(50_100));
    }

    #[test]
    fn empty_footprint() {
        let builder = FootprintBuilder::new(Decimal::from(100), 0, 0);
        let fp = builder.build();
        assert!(fp.rows.is_empty());
        assert_eq!(fp.total_delta, Decimal::ZERO);
    }

    #[test]
    fn zero_tick_size_uses_exact_price() {
        let mut builder = FootprintBuilder::new(Decimal::ZERO, 0, 59999);
        builder.add_trade(&trade("50050", "1.0", ssm_core::Side::Buy));
        builder.add_trade(&trade("50051", "2.0", ssm_core::Side::Buy));

        let fp = builder.build();
        // Each exact price is its own level
        assert_eq!(fp.rows.len(), 2);
    }

    #[test]
    fn single_trade_footprint() {
        let mut builder = FootprintBuilder::new(Decimal::from(10), 100, 200);
        builder.add_trade(&trade("50005", "3.5", ssm_core::Side::Sell));

        let fp = builder.build();
        assert_eq!(fp.rows.len(), 1);
        assert_eq!(fp.total_bid_volume, Decimal::from_str("3.5").unwrap());
        assert_eq!(fp.total_ask_volume, Decimal::ZERO);
        assert_eq!(fp.total_delta, Decimal::from_str("-3.5").unwrap()); // negative delta (net selling)
        assert_eq!(fp.open_time, 100);
        assert_eq!(fp.close_time, 200);
    }

    #[test]
    fn multiple_trades_same_level_aggregate() {
        let mut builder = FootprintBuilder::new(Decimal::from(100), 0, 0);
        // All bucket to 50000
        builder.add_trade(&trade("50010", "1.0", ssm_core::Side::Buy));
        builder.add_trade(&trade("50020", "2.0", ssm_core::Side::Buy));
        builder.add_trade(&trade("50030", "0.5", ssm_core::Side::Sell));

        let fp = builder.build();
        assert_eq!(fp.rows.len(), 1); // all in same bucket
        assert_eq!(fp.rows[0].ask_volume, Decimal::from_str("3.0").unwrap());
        assert_eq!(fp.rows[0].bid_volume, Decimal::from_str("0.5").unwrap());
        assert_eq!(fp.rows[0].delta, Decimal::from_str("2.5").unwrap());
    }

    #[test]
    fn footprint_rows_sorted_by_price() {
        let mut builder = FootprintBuilder::new(Decimal::from(100), 0, 0);
        builder.add_trade(&trade("50200", "1.0", ssm_core::Side::Buy));
        builder.add_trade(&trade("50000", "1.0", ssm_core::Side::Buy));
        builder.add_trade(&trade("50100", "1.0", ssm_core::Side::Buy));

        let fp = builder.build();
        assert_eq!(fp.rows.len(), 3);
        // BTreeMap ensures sorted order
        assert!(fp.rows[0].price_level < fp.rows[1].price_level);
        assert!(fp.rows[1].price_level < fp.rows[2].price_level);
    }

    #[test]
    fn mixed_buy_sell_same_level_delta_correct() {
        // Multiple buys and sells at the same price level
        let mut builder = FootprintBuilder::new(Decimal::from(100), 0, 0);
        builder.add_trade(&trade("50050", "2.0", ssm_core::Side::Buy));
        builder.add_trade(&trade("50050", "3.0", ssm_core::Side::Sell));
        builder.add_trade(&trade("50050", "1.5", ssm_core::Side::Buy));
        builder.add_trade(&trade("50050", "0.5", ssm_core::Side::Sell));

        let fp = builder.build();
        assert_eq!(fp.rows.len(), 1);
        // ask (buy) = 2.0 + 1.5 = 3.5, bid (sell) = 3.0 + 0.5 = 3.5
        assert_eq!(fp.rows[0].ask_volume, Decimal::from_str("3.5").unwrap());
        assert_eq!(fp.rows[0].bid_volume, Decimal::from_str("3.5").unwrap());
        assert_eq!(fp.rows[0].delta, Decimal::ZERO);
        assert_eq!(fp.total_delta, Decimal::ZERO);
    }

    #[test]
    fn bucket_with_fractional_tick_size() {
        // tick_size = 0.5
        let builder = FootprintBuilder::new(Decimal::from_str("0.5").unwrap(), 0, 0);
        assert_eq!(builder.bucket(Decimal::from_str("100.3").unwrap()), Decimal::from_str("100.0").unwrap());
        assert_eq!(builder.bucket(Decimal::from_str("100.5").unwrap()), Decimal::from_str("100.5").unwrap());
        assert_eq!(builder.bucket(Decimal::from_str("100.9").unwrap()), Decimal::from_str("100.5").unwrap());
    }

    #[test]
    fn total_volumes_match_sum_of_rows() {
        let mut builder = FootprintBuilder::new(Decimal::from(100), 0, 0);
        builder.add_trade(&trade("50050", "1.0", ssm_core::Side::Buy));
        builder.add_trade(&trade("50150", "2.0", ssm_core::Side::Sell));
        builder.add_trade(&trade("50250", "3.0", ssm_core::Side::Buy));

        let fp = builder.build();
        let sum_bid: Decimal = fp.rows.iter().map(|r| r.bid_volume).sum();
        let sum_ask: Decimal = fp.rows.iter().map(|r| r.ask_volume).sum();
        assert_eq!(fp.total_bid_volume, sum_bid);
        assert_eq!(fp.total_ask_volume, sum_ask);
        assert_eq!(fp.total_delta, sum_ask - sum_bid);
    }

    #[test]
    fn zero_range_candle_single_level() {
        // All trades at exactly the same price
        let mut builder = FootprintBuilder::new(Decimal::from(10), 0, 0);
        builder.add_trade(&trade("50000", "1.0", ssm_core::Side::Buy));
        builder.add_trade(&trade("50000", "2.0", ssm_core::Side::Sell));
        builder.add_trade(&trade("50000", "0.5", ssm_core::Side::Buy));

        let fp = builder.build();
        assert_eq!(fp.rows.len(), 1);
        assert_eq!(fp.rows[0].ask_volume, Decimal::from_str("1.5").unwrap());
        assert_eq!(fp.rows[0].bid_volume, Decimal::from_str("2.0").unwrap());
        assert_eq!(fp.rows[0].delta, Decimal::from_str("-0.5").unwrap());
    }

    #[test]
    fn very_wide_range_many_levels() {
        // Trades spread across a wide range with small tick size
        let mut builder = FootprintBuilder::new(Decimal::from(100), 0, 0);
        builder.add_trade(&trade("40000", "1.0", ssm_core::Side::Buy));
        builder.add_trade(&trade("45000", "1.0", ssm_core::Side::Buy));
        builder.add_trade(&trade("50000", "1.0", ssm_core::Side::Sell));
        builder.add_trade(&trade("55000", "1.0", ssm_core::Side::Sell));
        builder.add_trade(&trade("60000", "1.0", ssm_core::Side::Buy));

        let fp = builder.build();
        // 5 different price levels (each 5000 apart, tick=100 so distinct buckets)
        assert_eq!(fp.rows.len(), 5);
        assert_eq!(fp.total_ask_volume, Decimal::from(3)); // 3 buys
        assert_eq!(fp.total_bid_volume, Decimal::from(2)); // 2 sells
        assert_eq!(fp.total_delta, Decimal::from(1));
    }

    #[test]
    fn very_small_tick_size() {
        // tick_size = 0.01
        let ts = Decimal::from_str("0.01").unwrap();
        let builder = FootprintBuilder::new(ts, 0, 0);
        assert_eq!(builder.bucket(Decimal::from_str("100.456").unwrap()), Decimal::from_str("100.45").unwrap());
        assert_eq!(builder.bucket(Decimal::from_str("100.001").unwrap()), Decimal::from_str("100.00").unwrap());
        assert_eq!(builder.bucket(Decimal::from_str("100.999").unwrap()), Decimal::from_str("100.99").unwrap());
    }

    #[test]
    fn very_large_tick_size() {
        // tick_size = 1000
        let builder = FootprintBuilder::new(Decimal::from(1000), 0, 0);
        assert_eq!(builder.bucket(Decimal::from(50500)), Decimal::from(50000));
        assert_eq!(builder.bucket(Decimal::from(50999)), Decimal::from(50000));
        assert_eq!(builder.bucket(Decimal::from(51000)), Decimal::from(51000));
    }

    #[test]
    fn all_sells_negative_total_delta() {
        let mut builder = FootprintBuilder::new(Decimal::from(100), 0, 0);
        builder.add_trade(&trade("50000", "5.0", ssm_core::Side::Sell));
        builder.add_trade(&trade("50100", "3.0", ssm_core::Side::Sell));

        let fp = builder.build();
        assert_eq!(fp.total_ask_volume, Decimal::ZERO);
        assert_eq!(fp.total_bid_volume, Decimal::from(8));
        assert_eq!(fp.total_delta, Decimal::from(-8));
        for row in &fp.rows {
            assert!(row.delta <= Decimal::ZERO);
        }
    }

    #[test]
    fn all_buys_positive_total_delta() {
        let mut builder = FootprintBuilder::new(Decimal::from(100), 0, 0);
        builder.add_trade(&trade("50000", "4.0", ssm_core::Side::Buy));
        builder.add_trade(&trade("50100", "6.0", ssm_core::Side::Buy));

        let fp = builder.build();
        assert_eq!(fp.total_bid_volume, Decimal::ZERO);
        assert_eq!(fp.total_ask_volume, Decimal::from(10));
        assert_eq!(fp.total_delta, Decimal::from(10));
        for row in &fp.rows {
            assert!(row.delta >= Decimal::ZERO);
        }
    }

    #[test]
    fn tick_size_one_exact_prices() {
        // tick_size=1: each integer price is its own level
        let mut builder = FootprintBuilder::new(Decimal::from(1), 0, 0);
        builder.add_trade(&trade("100", "1.0", ssm_core::Side::Buy));
        builder.add_trade(&trade("101", "1.0", ssm_core::Side::Buy));
        builder.add_trade(&trade("100", "0.5", ssm_core::Side::Sell));

        let fp = builder.build();
        assert_eq!(fp.rows.len(), 2);
        // Level 100: ask=1.0, bid=0.5
        assert_eq!(fp.rows[0].price_level, Decimal::from(100));
        assert_eq!(fp.rows[0].ask_volume, Decimal::from_str("1.0").unwrap());
        assert_eq!(fp.rows[0].bid_volume, Decimal::from_str("0.5").unwrap());
    }

    #[test]
    fn open_and_close_times_preserved() {
        let builder = FootprintBuilder::new(Decimal::from(100), 1234567890, 1234567950);
        let fp = builder.build();
        assert_eq!(fp.open_time, 1234567890);
        assert_eq!(fp.close_time, 1234567950);
        assert_eq!(fp.tick_size, Decimal::from(100));
    }
}
