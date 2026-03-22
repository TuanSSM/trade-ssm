use anyhow::{Context, Result};

pub struct TelegramBot {
    client: reqwest::Client,
    token: String,
    chat_id: String,
}

impl TelegramBot {
    pub fn from_env() -> Result<Self> {
        let token =
            std::env::var("TELEGRAM_BOT_TOKEN").context("TELEGRAM_BOT_TOKEN env var required")?;
        let chat_id =
            std::env::var("TELEGRAM_CHAT_ID").context("TELEGRAM_CHAT_ID env var required")?;

        Ok(Self {
            client: reqwest::Client::new(),
            token,
            chat_id,
        })
    }

    /// Send a markdown-formatted message to the configured chat
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
            anyhow::bail!("Telegram API returned {}: {}", status, body);
        }

        tracing::info!("Telegram message sent successfully");
        Ok(())
    }
}

/// Format a CVD + liquidation report for Telegram
pub fn format_report(
    symbol: &str,
    interval: &str,
    cvd: &crate::indicators::cvd::CvdAnalysis,
    liq: &crate::indicators::liquidations::LiquidationSummary,
) -> String {
    use rust_decimal::prelude::ToPrimitive;

    let cvd_emoji = match cvd.trend {
        crate::indicators::cvd::CvdTrend::Bullish => "🟢",
        crate::indicators::cvd::CvdTrend::Bearish => "🔴",
        crate::indicators::cvd::CvdTrend::Neutral => "⚪",
    };

    let liq_emoji = match liq.bias {
        crate::indicators::liquidations::LiquidationBias::LongsLiquidated => "🔴",
        crate::indicators::liquidations::LiquidationBias::ShortsLiquidated => "🟢",
        crate::indicators::liquidations::LiquidationBias::Balanced => "⚪",
    };

    let mut msg = format!(
        "*{} {} — trade-ssm Report*\n\n\
         {} *CVD (last {} candles):* {}\n\
         Total CVD: `{:.4}`\n\n\
         {} *Liquidations:* {}\n\
         Longs rekt: {} (${:.0})\n\
         Shorts rekt: {} (${:.0})\n",
        symbol,
        interval,
        cvd_emoji,
        cvd.window_size,
        cvd.trend,
        cvd.total_cvd.to_f64().unwrap_or(0.0),
        liq_emoji,
        liq.bias,
        liq.total_long_liquidations,
        liq.total_long_value.to_f64().unwrap_or(0.0),
        liq.total_short_liquidations,
        liq.total_short_value.to_f64().unwrap_or(0.0),
    );

    // Add tier breakdown for non-empty tiers
    let has_tiers = liq.by_tier.iter().any(|t| t.long_count + t.short_count > 0);
    if has_tiers {
        msg.push_str("\n*Liquidation Tiers:*\n");
        for tier in &liq.by_tier {
            let total = tier.long_count + tier.short_count;
            if total > 0 {
                let total_val = tier.long_value + tier.short_value;
                msg.push_str(&format!(
                    "  {} — {} orders (${:.0})\n",
                    tier.tier.label(),
                    total,
                    total_val.to_f64().unwrap_or(0.0),
                ));
            }
        }
    }

    let now = chrono::Utc::now().format("%Y-%m-%d %H:%M UTC");
    msg.push_str(&format!("\n_{}_", now));

    msg
}
