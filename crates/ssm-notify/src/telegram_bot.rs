use anyhow::{Context, Result};
use serde::Deserialize;
use std::collections::HashSet;

/// Telegram update from the Bot API.
#[derive(Debug, Deserialize)]
pub struct TelegramUpdate {
    pub update_id: i64,
    pub message: Option<TelegramMessage>,
}

#[derive(Debug, Deserialize)]
pub struct TelegramMessage {
    pub message_id: i64,
    pub chat: TelegramChat,
    pub text: Option<String>,
}

#[derive(Debug, Deserialize)]
pub struct TelegramChat {
    pub id: i64,
}

/// Command parsed from a Telegram message.
#[derive(Debug, Clone, PartialEq)]
pub enum BotCommand {
    Status,
    Profit,
    Balance,
    Daily,
    Weekly,
    Start,
    Stop,
    ForceExit(String), // pair
    Help,
    Unknown(String),
}

impl BotCommand {
    pub fn parse(text: &str) -> Self {
        let parts: Vec<&str> = text.split_whitespace().collect();
        match parts.first().map(|s| s.to_lowercase()).as_deref() {
            Some("/status") => Self::Status,
            Some("/profit") => Self::Profit,
            Some("/balance") => Self::Balance,
            Some("/daily") => Self::Daily,
            Some("/weekly") => Self::Weekly,
            Some("/start") => Self::Start,
            Some("/stop") => Self::Stop,
            Some("/forceexit") => Self::ForceExit(parts.get(1).unwrap_or(&"").to_string()),
            Some("/help") => Self::Help,
            Some(cmd) => Self::Unknown(cmd.to_string()),
            None => Self::Unknown(String::new()),
        }
    }
}

/// Interactive Telegram bot with command handling.
pub struct InteractiveTelegramBot {
    token: String,
    client: reqwest::Client,
    allowed_chats: HashSet<i64>,
    last_update_id: i64,
}

impl InteractiveTelegramBot {
    pub fn new(token: String, allowed_chats: Vec<i64>) -> Self {
        Self {
            token,
            client: reqwest::Client::new(),
            allowed_chats: allowed_chats.into_iter().collect(),
            last_update_id: 0,
        }
    }

    pub fn from_env() -> Result<Self> {
        let token =
            std::env::var("TELEGRAM_BOT_TOKEN").context("TELEGRAM_BOT_TOKEN env var required")?;
        let chat_id: i64 = std::env::var("TELEGRAM_CHAT_ID")
            .context("TELEGRAM_CHAT_ID env var required")?
            .parse()
            .context("TELEGRAM_CHAT_ID must be a valid integer")?;
        Ok(Self::new(token, vec![chat_id]))
    }

    /// Poll for updates (long polling).
    pub async fn get_updates(&mut self) -> Result<Vec<TelegramUpdate>> {
        let url = format!("https://api.telegram.org/bot{}/getUpdates", self.token);
        let resp = self
            .client
            .get(&url)
            .query(&[
                ("offset", (self.last_update_id + 1).to_string()),
                ("timeout", "30".to_string()),
            ])
            .send()
            .await
            .context("Failed to poll Telegram updates")?;

        let status = resp.status();
        if !status.is_success() {
            let body = resp.text().await.unwrap_or_default();
            anyhow::bail!("Telegram getUpdates returned {status}: {body}");
        }

        #[derive(Deserialize)]
        struct ApiResponse {
            result: Vec<TelegramUpdate>,
        }

        let api_resp: ApiResponse = resp
            .json()
            .await
            .context("Failed to parse Telegram response")?;

        if let Some(last) = api_resp.result.last() {
            self.last_update_id = last.update_id;
        }

        Ok(api_resp.result)
    }

    /// Send a reply message.
    pub async fn reply(&self, chat_id: i64, text: &str) -> Result<()> {
        let url = format!("https://api.telegram.org/bot{}/sendMessage", self.token);
        let resp = self
            .client
            .post(&url)
            .json(&serde_json::json!({
                "chat_id": chat_id,
                "text": text,
                "parse_mode": "HTML"
            }))
            .send()
            .await
            .context("Failed to send Telegram reply")?;

        let status = resp.status();
        if !status.is_success() {
            let body = resp.text().await.unwrap_or_default();
            anyhow::bail!("Telegram sendMessage returned {status}: {body}");
        }

        tracing::info!(chat_id, "Telegram reply sent");
        Ok(())
    }

    /// Check if a chat is authorized.
    pub fn is_authorized(&self, chat_id: i64) -> bool {
        self.allowed_chats.contains(&chat_id)
    }

    /// Format help text listing all commands.
    pub fn help_text() -> &'static str {
        concat!(
            "<b>trade-ssm Bot Commands</b>\n\n",
            "/status - Show bot status\n",
            "/profit - Show profit summary\n",
            "/balance - Show account balance\n",
            "/daily - Show daily profit summary\n",
            "/weekly - Show weekly profit summary\n",
            "/start - Start trading\n",
            "/stop - Stop trading\n",
            "/forceexit &lt;pair&gt; - Force exit a position\n",
            "/help - Show this help message",
        )
    }

    /// Format status response.
    pub fn format_status(running: bool, symbol: &str, mode: &str, uptime_secs: u64) -> String {
        let status_icon = if running { "ON" } else { "OFF" };
        let hours = uptime_secs / 3600;
        let minutes = (uptime_secs % 3600) / 60;
        format!(
            "<b>Status:</b> {status_icon}\n\
             <b>Symbol:</b> {symbol}\n\
             <b>Mode:</b> {mode}\n\
             <b>Uptime:</b> {hours}h {minutes}m"
        )
    }

    /// Format profit response.
    pub fn format_profit(realized: &str, unrealized: &str, trade_count: usize) -> String {
        format!(
            "<b>Profit Summary</b>\n\
             Realized: {realized}\n\
             Unrealized: {unrealized}\n\
             Trades: {trade_count}"
        )
    }

    /// Format daily summary.
    pub fn format_daily(profits: &[(String, String)]) -> String {
        if profits.is_empty() {
            return "<b>Daily Summary</b>\nNo data available.".to_string();
        }
        let mut msg = "<b>Daily Summary</b>\n".to_string();
        for (date, profit) in profits {
            msg.push_str(&format!("{date}: {profit}\n"));
        }
        msg
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_status_command() {
        assert_eq!(BotCommand::parse("/status"), BotCommand::Status);
    }

    #[test]
    fn parse_profit_command() {
        assert_eq!(BotCommand::parse("/profit"), BotCommand::Profit);
    }

    #[test]
    fn parse_balance_command() {
        assert_eq!(BotCommand::parse("/balance"), BotCommand::Balance);
    }

    #[test]
    fn parse_daily_command() {
        assert_eq!(BotCommand::parse("/daily"), BotCommand::Daily);
    }

    #[test]
    fn parse_weekly_command() {
        assert_eq!(BotCommand::parse("/weekly"), BotCommand::Weekly);
    }

    #[test]
    fn parse_start_command() {
        assert_eq!(BotCommand::parse("/start"), BotCommand::Start);
    }

    #[test]
    fn parse_stop_command() {
        assert_eq!(BotCommand::parse("/stop"), BotCommand::Stop);
    }

    #[test]
    fn parse_help_command() {
        assert_eq!(BotCommand::parse("/help"), BotCommand::Help);
    }

    #[test]
    fn parse_forceexit_with_pair() {
        assert_eq!(
            BotCommand::parse("/forceexit BTCUSDT"),
            BotCommand::ForceExit("BTCUSDT".to_string())
        );
    }

    #[test]
    fn parse_forceexit_without_pair() {
        assert_eq!(
            BotCommand::parse("/forceexit"),
            BotCommand::ForceExit(String::new())
        );
    }

    #[test]
    fn parse_unknown_command() {
        assert_eq!(
            BotCommand::parse("/foo"),
            BotCommand::Unknown("/foo".to_string())
        );
    }

    #[test]
    fn parse_empty_string() {
        assert_eq!(BotCommand::parse(""), BotCommand::Unknown(String::new()));
    }

    #[test]
    fn parse_case_insensitive() {
        assert_eq!(BotCommand::parse("/STATUS"), BotCommand::Status);
        assert_eq!(BotCommand::parse("/Profit"), BotCommand::Profit);
        assert_eq!(BotCommand::parse("/HELP"), BotCommand::Help);
    }

    #[test]
    fn is_authorized_checks_allowed_list() {
        let bot = InteractiveTelegramBot::new("token".to_string(), vec![123, 456]);
        assert!(bot.is_authorized(123));
        assert!(bot.is_authorized(456));
        assert!(!bot.is_authorized(789));
    }

    #[test]
    fn is_authorized_empty_list() {
        let bot = InteractiveTelegramBot::new("token".to_string(), vec![]);
        assert!(!bot.is_authorized(123));
    }

    #[test]
    fn help_text_contains_all_commands() {
        let help = InteractiveTelegramBot::help_text();
        assert!(help.contains("/status"));
        assert!(help.contains("/profit"));
        assert!(help.contains("/balance"));
        assert!(help.contains("/daily"));
        assert!(help.contains("/weekly"));
        assert!(help.contains("/start"));
        assert!(help.contains("/stop"));
        assert!(help.contains("/forceexit"));
        assert!(help.contains("/help"));
    }

    #[test]
    fn format_status_running() {
        let status = InteractiveTelegramBot::format_status(true, "BTCUSDT", "paper", 3661);
        assert!(status.contains("ON"));
        assert!(status.contains("BTCUSDT"));
        assert!(status.contains("paper"));
        assert!(status.contains("1h 1m"));
    }

    #[test]
    fn format_status_stopped() {
        let status = InteractiveTelegramBot::format_status(false, "ETHUSDT", "live", 0);
        assert!(status.contains("OFF"));
        assert!(status.contains("ETHUSDT"));
        assert!(status.contains("live"));
        assert!(status.contains("0h 0m"));
    }

    #[test]
    fn format_profit_output() {
        let profit = InteractiveTelegramBot::format_profit("$150.00", "-$20.00", 42);
        assert!(profit.contains("$150.00"));
        assert!(profit.contains("-$20.00"));
        assert!(profit.contains("42"));
        assert!(profit.contains("Profit Summary"));
    }

    #[test]
    fn format_daily_with_data() {
        let profits = vec![
            ("2026-03-23".to_string(), "$50.00".to_string()),
            ("2026-03-24".to_string(), "-$10.00".to_string()),
        ];
        let daily = InteractiveTelegramBot::format_daily(&profits);
        assert!(daily.contains("Daily Summary"));
        assert!(daily.contains("2026-03-23"));
        assert!(daily.contains("$50.00"));
        assert!(daily.contains("2026-03-24"));
        assert!(daily.contains("-$10.00"));
    }

    #[test]
    fn format_daily_empty() {
        let daily = InteractiveTelegramBot::format_daily(&[]);
        assert!(daily.contains("No data available"));
    }
}
