use anyhow::{Context, Result};
use rust_decimal::prelude::ToPrimitive;
use ssm_indicators::cvd::{CvdAnalysis, CvdTrend};
use ssm_indicators::liquidations::{LiquidationBias, LiquidationSummary};

pub struct TelegramBot {
    client: reqwest::Client,
    token: String,
    chat_id: String,
}

impl TelegramBot {
    pub fn from_env() -> Result<Self> {
        Ok(Self {
            client: reqwest::Client::new(),
            token: std::env::var("TELEGRAM_BOT_TOKEN")
                .context("TELEGRAM_BOT_TOKEN env var required")?,
            chat_id: std::env::var("TELEGRAM_CHAT_ID")
                .context("TELEGRAM_CHAT_ID env var required")?,
        })
    }

    pub async fn send_message(&self, text: &str) -> Result<()> {
        let url = format!("https://api.telegram.org/bot{}/sendMessage", self.token);
        let resp = self
            .client
            .post(&url)
            .json(&serde_json::json!({
                "chat_id": self.chat_id,
                "text": text,
                "parse_mode": "Markdown"
            }))
            .send()
            .await
            .context("Failed to send Telegram message")?;

        let status = resp.status();
        if !status.is_success() {
            let body = resp.text().await.unwrap_or_default();
            anyhow::bail!("Telegram API returned {status}: {body}");
        }

        tracing::info!("Telegram message sent");
        Ok(())
    }
}

pub fn format_report(
    symbol: &str,
    interval: &str,
    cvd: &CvdAnalysis,
    liq: &LiquidationSummary,
) -> String {
    let cvd_icon = match cvd.trend {
        CvdTrend::Bullish => "🟢",
        CvdTrend::Bearish => "🔴",
        CvdTrend::Neutral => "⚪",
    };
    let liq_icon = match liq.bias {
        LiquidationBias::LongsLiquidated => "🔴",
        LiquidationBias::ShortsLiquidated => "🟢",
        LiquidationBias::Balanced => "⚪",
    };

    let mut msg = format!(
        "*{symbol} {interval} — trade-ssm*\n\n\
         {cvd_icon} *CVD ({} candles):* {}\n\
         Total: `{:.4}`\n\n\
         {liq_icon} *Liquidations:* {}\n\
         Longs rekt: {} (${:.0})\n\
         Shorts rekt: {} (${:.0})\n",
        cvd.window_size,
        cvd.trend,
        cvd.total_cvd.to_f64().unwrap_or(0.0),
        liq.bias,
        liq.total_long_liquidations,
        liq.total_long_value.to_f64().unwrap_or(0.0),
        liq.total_short_liquidations,
        liq.total_short_value.to_f64().unwrap_or(0.0),
    );

    if liq.by_tier.iter().any(|t| t.long_count + t.short_count > 0) {
        msg.push_str("\n*Tiers:*\n");
        for t in &liq.by_tier {
            let total = t.long_count + t.short_count;
            if total > 0 {
                let val = (t.long_value + t.short_value).to_f64().unwrap_or(0.0);
                msg.push_str(&format!(
                    "  {} — {} orders (${val:.0})\n",
                    t.tier.label(),
                    total
                ));
            }
        }
    }

    msg.push_str(&format!(
        "\n_{}_",
        chrono::Utc::now().format("%Y-%m-%d %H:%M UTC")
    ));
    msg
}

#[cfg(test)]
mod tests {
    use super::*;
    use rust_decimal::Decimal;
    use ssm_indicators::liquidations::TierSummary;

    #[test]
    fn format_report_includes_symbol() {
        let cvd = CvdAnalysis {
            deltas: vec![Decimal::from(10)],
            cumulative: vec![Decimal::from(10)],
            total_cvd: Decimal::from(10),
            trend: CvdTrend::Bullish,
            window_size: 1,
        };
        let liq = LiquidationSummary {
            total_long_liquidations: 0,
            total_short_liquidations: 0,
            total_long_value: Decimal::ZERO,
            total_short_value: Decimal::ZERO,
            by_tier: vec![],
            bias: LiquidationBias::Balanced,
        };
        let report = format_report("BTCUSDT", "15m", &cvd, &liq);
        assert!(report.contains("BTCUSDT"));
        assert!(report.contains("BULLISH"));
    }

    #[test]
    fn format_report_shows_tiers() {
        let cvd = CvdAnalysis {
            deltas: vec![],
            cumulative: vec![],
            total_cvd: Decimal::ZERO,
            trend: CvdTrend::Neutral,
            window_size: 0,
        };
        let liq = LiquidationSummary {
            total_long_liquidations: 1,
            total_short_liquidations: 0,
            total_long_value: Decimal::from(50_000),
            total_short_value: Decimal::ZERO,
            by_tier: vec![TierSummary {
                tier: ssm_core::LiquidationTier::Large,
                long_count: 1,
                short_count: 0,
                long_value: Decimal::from(50_000),
                short_value: Decimal::ZERO,
            }],
            bias: LiquidationBias::LongsLiquidated,
        };
        let report = format_report("BTCUSDT", "1h", &cvd, &liq);
        assert!(report.contains("$30K+"));
        assert!(report.contains("1 orders"));
    }

    #[test]
    fn test_format_report_bearish() {
        let cvd = CvdAnalysis {
            deltas: vec![Decimal::from(-10)],
            cumulative: vec![Decimal::from(-10)],
            total_cvd: Decimal::from(-10),
            trend: CvdTrend::Bearish,
            window_size: 1,
        };
        let liq = LiquidationSummary {
            total_long_liquidations: 0,
            total_short_liquidations: 0,
            total_long_value: Decimal::ZERO,
            total_short_value: Decimal::ZERO,
            by_tier: vec![],
            bias: LiquidationBias::Balanced,
        };
        let report = format_report("BTCUSDT", "15m", &cvd, &liq);
        assert!(report.contains("BEARISH"));
        assert!(report.contains("\u{1f534}")); // 🔴
    }

    #[test]
    fn test_format_report_neutral() {
        let cvd = CvdAnalysis {
            deltas: vec![],
            cumulative: vec![],
            total_cvd: Decimal::ZERO,
            trend: CvdTrend::Neutral,
            window_size: 0,
        };
        let liq = LiquidationSummary {
            total_long_liquidations: 0,
            total_short_liquidations: 0,
            total_long_value: Decimal::ZERO,
            total_short_value: Decimal::ZERO,
            by_tier: vec![],
            bias: LiquidationBias::Balanced,
        };
        let report = format_report("BTCUSDT", "15m", &cvd, &liq);
        assert!(report.contains("NEUTRAL"));
    }

    #[test]
    fn test_format_report_shorts_rekt() {
        let cvd = CvdAnalysis {
            deltas: vec![],
            cumulative: vec![],
            total_cvd: Decimal::ZERO,
            trend: CvdTrend::Neutral,
            window_size: 0,
        };
        let liq = LiquidationSummary {
            total_long_liquidations: 0,
            total_short_liquidations: 5,
            total_long_value: Decimal::ZERO,
            total_short_value: Decimal::from(100_000),
            by_tier: vec![],
            bias: LiquidationBias::ShortsLiquidated,
        };
        let report = format_report("BTCUSDT", "15m", &cvd, &liq);
        assert!(report.contains("SHORTS REKT"));
    }

    #[test]
    fn test_format_report_includes_interval() {
        let cvd = CvdAnalysis {
            deltas: vec![],
            cumulative: vec![],
            total_cvd: Decimal::ZERO,
            trend: CvdTrend::Neutral,
            window_size: 0,
        };
        let liq = LiquidationSummary {
            total_long_liquidations: 0,
            total_short_liquidations: 0,
            total_long_value: Decimal::ZERO,
            total_short_value: Decimal::ZERO,
            by_tier: vec![],
            bias: LiquidationBias::Balanced,
        };
        let report = format_report("BTCUSDT", "4h", &cvd, &liq);
        assert!(report.contains("4h"));
    }

    #[test]
    fn test_format_report_empty_tiers() {
        let cvd = CvdAnalysis {
            deltas: vec![],
            cumulative: vec![],
            total_cvd: Decimal::ZERO,
            trend: CvdTrend::Neutral,
            window_size: 0,
        };
        let liq = LiquidationSummary {
            total_long_liquidations: 0,
            total_short_liquidations: 0,
            total_long_value: Decimal::ZERO,
            total_short_value: Decimal::ZERO,
            by_tier: vec![],
            bias: LiquidationBias::Balanced,
        };
        let report = format_report("BTCUSDT", "15m", &cvd, &liq);
        assert!(!report.contains("Tiers:"));
    }

    #[test]
    fn test_format_report_date() {
        let cvd = CvdAnalysis {
            deltas: vec![],
            cumulative: vec![],
            total_cvd: Decimal::ZERO,
            trend: CvdTrend::Neutral,
            window_size: 0,
        };
        let liq = LiquidationSummary {
            total_long_liquidations: 0,
            total_short_liquidations: 0,
            total_long_value: Decimal::ZERO,
            total_short_value: Decimal::ZERO,
            by_tier: vec![],
            bias: LiquidationBias::Balanced,
        };
        let report = format_report("BTCUSDT", "15m", &cvd, &liq);
        assert!(report.contains("UTC"));
    }
}
