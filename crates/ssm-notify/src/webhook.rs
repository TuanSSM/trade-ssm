use anyhow::{Context, Result};
use serde::Serialize;
use std::collections::HashMap;

/// Generic webhook notifier — sends HTTP POST on trade events.
pub struct WebhookNotifier {
    url: String,
    client: reqwest::Client,
    headers: HashMap<String, String>,
    template: Option<String>,
    max_retries: u32,
}

impl WebhookNotifier {
    pub fn new(url: String) -> Self {
        Self {
            url,
            client: reqwest::Client::new(),
            headers: HashMap::new(),
            template: None,
            max_retries: 3,
        }
    }

    pub fn with_header(mut self, key: String, value: String) -> Self {
        self.headers.insert(key, value);
        self
    }

    pub fn with_template(mut self, template: String) -> Self {
        self.template = Some(template);
        self
    }

    pub fn with_retries(mut self, retries: u32) -> Self {
        self.max_retries = retries;
        self
    }

    /// Send a notification with template variable substitution.
    ///
    /// If a template is configured, variables are substituted into it and the
    /// result is sent as JSON `{"text": "..."}`. Otherwise, the variables map
    /// itself is serialized as the JSON body.
    pub async fn send(&self, variables: &HashMap<String, String>) -> Result<()> {
        let payload = if let Some(ref tpl) = self.template {
            let rendered = render_template(tpl, variables);
            serde_json::json!({ "text": rendered })
        } else {
            serde_json::to_value(variables).context("Failed to serialize variables")?
        };

        self.post_json(&payload).await
    }

    /// Send raw JSON payload.
    pub async fn send_json<T: Serialize>(&self, payload: &T) -> Result<()> {
        let value = serde_json::to_value(payload).context("Failed to serialize JSON payload")?;
        self.post_json(&value).await
    }

    async fn post_json(&self, payload: &serde_json::Value) -> Result<()> {
        let mut last_err: Option<anyhow::Error> = None;

        for attempt in 0..=self.max_retries {
            if attempt > 0 {
                tracing::warn!("Webhook retry {attempt}/{}", self.max_retries);
            }

            let mut req = self.client.post(&self.url).json(payload);
            for (k, v) in &self.headers {
                req = req.header(k.as_str(), v.as_str());
            }

            match req.send().await {
                Ok(resp) => {
                    let status = resp.status();
                    if status.is_success() {
                        tracing::info!("Webhook sent successfully");
                        return Ok(());
                    }
                    let body = resp.text().await.unwrap_or_default();
                    last_err = Some(anyhow::anyhow!("Webhook returned {status}: {body}"));
                }
                Err(e) => {
                    last_err = Some(anyhow::anyhow!(e).context("Webhook request failed"));
                }
            }
        }

        Err(last_err.unwrap_or_else(|| anyhow::anyhow!("Webhook failed with no attempts")))
    }
}

/// Discord webhook notifier — uses rich embed format.
pub struct DiscordNotifier {
    webhook_url: String,
    client: reqwest::Client,
    username: Option<String>,
    max_retries: u32,
}

#[derive(Debug, Clone, Serialize)]
pub struct DiscordEmbed {
    pub title: String,
    pub description: String,
    pub color: u32,
    pub fields: Vec<DiscordField>,
}

#[derive(Debug, Clone, Serialize)]
pub struct DiscordField {
    pub name: String,
    pub value: String,
    pub inline: bool,
}

impl DiscordNotifier {
    pub fn new(webhook_url: String) -> Self {
        Self {
            webhook_url,
            client: reqwest::Client::new(),
            username: None,
            max_retries: 3,
        }
    }

    pub fn with_username(mut self, username: String) -> Self {
        self.username = Some(username);
        self
    }

    pub fn with_retries(mut self, retries: u32) -> Self {
        self.max_retries = retries;
        self
    }

    /// Send a Discord embed message.
    pub async fn send_embed(&self, embed: DiscordEmbed) -> Result<()> {
        let mut payload = serde_json::json!({
            "embeds": [embed],
        });
        if let Some(ref username) = self.username {
            payload["username"] = serde_json::json!(username);
        }
        self.post_json(&payload).await
    }

    /// Send a simple text message.
    pub async fn send_message(&self, content: &str) -> Result<()> {
        let mut payload = serde_json::json!({
            "content": content,
        });
        if let Some(ref username) = self.username {
            payload["username"] = serde_json::json!(username);
        }
        self.post_json(&payload).await
    }

    async fn post_json(&self, payload: &serde_json::Value) -> Result<()> {
        let mut last_err: Option<anyhow::Error> = None;

        for attempt in 0..=self.max_retries {
            if attempt > 0 {
                tracing::warn!("Discord webhook retry {attempt}/{}", self.max_retries);
            }

            match self
                .client
                .post(&self.webhook_url)
                .json(payload)
                .send()
                .await
            {
                Ok(resp) => {
                    let status = resp.status();
                    if status.is_success() || status.as_u16() == 204 {
                        tracing::info!("Discord webhook sent successfully");
                        return Ok(());
                    }
                    let body = resp.text().await.unwrap_or_default();
                    last_err = Some(anyhow::anyhow!("Discord webhook returned {status}: {body}"));
                }
                Err(e) => {
                    last_err = Some(anyhow::anyhow!(e).context("Discord webhook request failed"));
                }
            }
        }

        Err(last_err.unwrap_or_else(|| anyhow::anyhow!("Discord webhook failed with no attempts")))
    }
}

/// Template variable substitution: replaces `{pair}`, `{profit}`, `{action}`, etc.
pub fn render_template(template: &str, variables: &HashMap<String, String>) -> String {
    let mut result = template.to_string();
    for (key, value) in variables {
        result = result.replace(&format!("{{{key}}}"), value);
    }
    result
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn template_rendering_substitutes_all_variables() {
        let tpl = "Trade {action} on {pair} with profit {profit}";
        let mut vars = HashMap::new();
        vars.insert("action".to_string(), "BUY".to_string());
        vars.insert("pair".to_string(), "BTCUSDT".to_string());
        vars.insert("profit".to_string(), "1.5%".to_string());

        let result = render_template(tpl, &vars);
        assert_eq!(result, "Trade BUY on BTCUSDT with profit 1.5%");
    }

    #[test]
    fn template_rendering_leaves_unknown_variables_unchanged() {
        let tpl = "Trade {action} on {pair} with {unknown}";
        let mut vars = HashMap::new();
        vars.insert("action".to_string(), "SELL".to_string());
        vars.insert("pair".to_string(), "ETHUSDT".to_string());

        let result = render_template(tpl, &vars);
        assert_eq!(result, "Trade SELL on ETHUSDT with {unknown}");
    }

    #[test]
    fn template_rendering_handles_empty_variables() {
        let tpl = "Signal: {action}";
        let vars = HashMap::new();

        let result = render_template(tpl, &vars);
        assert_eq!(result, "Signal: {action}");
    }

    #[test]
    fn webhook_notifier_builds_correctly() {
        let notifier = WebhookNotifier::new("https://example.com/hook".to_string());
        assert_eq!(notifier.url, "https://example.com/hook");
        assert_eq!(notifier.max_retries, 3);
        assert!(notifier.headers.is_empty());
        assert!(notifier.template.is_none());
    }

    #[test]
    fn discord_notifier_builds_correctly() {
        let notifier = DiscordNotifier::new("https://discord.com/api/webhooks/123".to_string());
        assert_eq!(notifier.webhook_url, "https://discord.com/api/webhooks/123");
        assert_eq!(notifier.max_retries, 3);
        assert!(notifier.username.is_none());
    }

    #[test]
    fn discord_embed_serialization_produces_correct_json() {
        let embed = DiscordEmbed {
            title: "Trade Alert".to_string(),
            description: "BTC long entry".to_string(),
            color: 0x00FF00,
            fields: vec![
                DiscordField {
                    name: "Pair".to_string(),
                    value: "BTCUSDT".to_string(),
                    inline: true,
                },
                DiscordField {
                    name: "Action".to_string(),
                    value: "EnterLong".to_string(),
                    inline: true,
                },
            ],
        };

        let json = serde_json::to_value(&embed).unwrap();
        assert_eq!(json["title"], "Trade Alert");
        assert_eq!(json["description"], "BTC long entry");
        assert_eq!(json["color"], 0x00FF00);
        assert_eq!(json["fields"][0]["name"], "Pair");
        assert_eq!(json["fields"][0]["value"], "BTCUSDT");
        assert!(json["fields"][0]["inline"].as_bool().unwrap());
        assert_eq!(json["fields"][1]["name"], "Action");
        assert_eq!(json["fields"][1]["value"], "EnterLong");
        assert!(json["fields"][1]["inline"].as_bool().unwrap());
    }

    #[test]
    fn retry_count_configuration_works() {
        let notifier = WebhookNotifier::new("https://example.com".to_string()).with_retries(5);
        assert_eq!(notifier.max_retries, 5);

        let discord = DiscordNotifier::new("https://discord.com/api/webhooks/123".to_string())
            .with_retries(10);
        assert_eq!(discord.max_retries, 10);
    }

    #[test]
    fn header_configuration_works() {
        let notifier = WebhookNotifier::new("https://example.com".to_string())
            .with_header("Authorization".to_string(), "Bearer token123".to_string())
            .with_header("X-Custom".to_string(), "value".to_string());

        assert_eq!(notifier.headers.len(), 2);
        assert_eq!(notifier.headers["Authorization"], "Bearer token123");
        assert_eq!(notifier.headers["X-Custom"], "value");
    }

    #[test]
    fn webhook_with_template_configuration() {
        let notifier = WebhookNotifier::new("https://example.com".to_string())
            .with_template("{action} on {pair}".to_string());
        assert_eq!(notifier.template.as_deref(), Some("{action} on {pair}"));
    }

    #[test]
    fn discord_with_username_configuration() {
        let notifier = DiscordNotifier::new("https://discord.com/api/webhooks/123".to_string())
            .with_username("TradeBot".to_string());
        assert_eq!(notifier.username.as_deref(), Some("TradeBot"));
    }
}
