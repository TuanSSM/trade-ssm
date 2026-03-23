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
}
