use anyhow::Result;
use serde::{Deserialize, Serialize};
use ssm_core::{Candle, Signal};

/// Analyzed candle data broadcast by a producer.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AnalyzedData {
    pub symbol: String,
    pub timeframe: String,
    pub candle: Candle,
    pub indicators: std::collections::HashMap<String, f64>,
    pub signals: Vec<Signal>,
    pub timestamp: i64,
}

/// Producer mode: analyzes candles and publishes to NATS.
pub struct Producer {
    publisher: crate::publisher::Publisher,
    symbol: String,
}

impl Producer {
    pub fn new(publisher: crate::publisher::Publisher, symbol: String) -> Self {
        Self { publisher, symbol }
    }

    /// Publish analyzed data for consumers.
    pub async fn publish_analysis(&self, data: &AnalyzedData) -> Result<()> {
        self.publisher.publish(&self.topic(), data).await
    }

    /// Topic for this producer's broadcasts.
    pub fn topic(&self) -> String {
        producer_topic(&self.symbol)
    }
}

/// Consumer mode: subscribes to producer data and optionally applies own strategy.
pub struct Consumer {
    subscriber: crate::subscriber::Subscriber,
    producers: Vec<String>, // producer symbols to subscribe to
}

impl Consumer {
    pub fn new(subscriber: crate::subscriber::Subscriber, producers: Vec<String>) -> Self {
        Self {
            subscriber,
            producers,
        }
    }

    /// Subscribe to all configured producers.
    pub async fn subscribe(&self, tx: tokio::sync::mpsc::Sender<AnalyzedData>) -> Result<()> {
        for symbol in &self.producers {
            let topic = producer_topic(symbol);
            self.subscriber.subscribe_typed(&topic, tx.clone()).await?;
        }
        Ok(())
    }

    /// Topics to subscribe to.
    pub fn topics(&self) -> Vec<String> {
        self.producers.iter().map(|s| producer_topic(s)).collect()
    }
}

/// Topics for producer/consumer.
pub fn producer_topic(symbol: &str) -> String {
    format!("ssm.producer.{}", symbol.to_lowercase())
}

#[cfg(test)]
mod tests {
    use super::*;
    use rust_decimal::Decimal;

    fn sample_candle() -> Candle {
        Candle {
            open_time: 1000,
            open: Decimal::from(100),
            high: Decimal::from(110),
            low: Decimal::from(90),
            close: Decimal::from(105),
            volume: Decimal::from(1000),
            close_time: 2000,
            quote_volume: Decimal::from(100000),
            trades: 50,
            taker_buy_volume: Decimal::from(600),
            taker_sell_volume: Decimal::from(400),
        }
    }

    #[test]
    fn analyzed_data_serialization_roundtrip() {
        let data = AnalyzedData {
            symbol: "BTCUSDT".to_string(),
            timeframe: "15m".to_string(),
            candle: sample_candle(),
            indicators: {
                let mut m = std::collections::HashMap::new();
                m.insert("cvd".to_string(), 42.5);
                m.insert("rsi".to_string(), 65.0);
                m
            },
            signals: vec![],
            timestamp: 1234567890,
        };

        let json = serde_json::to_string(&data).expect("serialize");
        let deser: AnalyzedData = serde_json::from_str(&json).expect("deserialize");

        assert_eq!(deser.symbol, "BTCUSDT");
        assert_eq!(deser.timeframe, "15m");
        assert_eq!(deser.timestamp, 1234567890);
        assert_eq!(deser.indicators.len(), 2);
        assert_eq!(deser.indicators["cvd"], 42.5);
        assert_eq!(deser.indicators["rsi"], 65.0);
    }

    #[test]
    fn producer_topic_format() {
        assert_eq!(producer_topic("BTCUSDT"), "ssm.producer.btcusdt");
        assert_eq!(producer_topic("ETHUSDT"), "ssm.producer.ethusdt");
        assert_eq!(producer_topic("btcusdt"), "ssm.producer.btcusdt");
    }

    #[test]
    fn producer_topic_method_returns_correct_topic() {
        // We cannot construct a real Publisher without a NATS connection,
        // so we test the topic generation via the free function directly,
        // which Producer::topic() delegates to.
        let topic = producer_topic("SOLUSDT");
        assert_eq!(topic, "ssm.producer.solusdt");
    }

    #[test]
    fn consumer_topics_returns_correct_topics() {
        // Test topic generation for multiple producers.
        let symbols = ["BTCUSDT".to_string(), "ETHUSDT".to_string()];
        let topics: Vec<String> = symbols.iter().map(|s| producer_topic(s)).collect();
        assert_eq!(topics.len(), 2);
        assert_eq!(topics[0], "ssm.producer.btcusdt");
        assert_eq!(topics[1], "ssm.producer.ethusdt");
    }

    #[test]
    fn producer_topic_mixed_case() {
        assert_eq!(producer_topic("BtCuSdT"), "ssm.producer.btcusdt");
    }

    #[test]
    fn producer_topic_empty_symbol() {
        assert_eq!(producer_topic(""), "ssm.producer.");
    }
}
