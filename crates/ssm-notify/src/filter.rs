use rust_decimal::Decimal;

/// Configuration for filtering notifications.
#[derive(Debug, Clone, Default)]
pub struct NotificationFilter {
    /// Minimum absolute profit to trigger a notification. None = no filter.
    pub min_profit: Option<Decimal>,
    /// Minimum confidence threshold for signal notifications. None = no filter.
    pub min_confidence: Option<f64>,
    /// Quiet hours (UTC). Notifications suppressed between start and end hour.
    /// e.g., (22, 6) means quiet from 22:00 to 06:00 UTC.
    pub quiet_hours: Option<(u32, u32)>,
    /// Cooldown in seconds between notifications. Prevents spam.
    pub cooldown_secs: Option<u64>,
    /// Which actions to notify on. Empty = all actions.
    pub enabled_actions: Vec<String>,
}

/// Event data for filter evaluation.
pub struct NotificationEvent {
    pub action: String,
    pub profit: Option<Decimal>,
    pub confidence: Option<f64>,
    pub timestamp_ms: i64,
}

impl NotificationFilter {
    /// Create a new filter with all checks disabled (everything passes).
    pub fn new() -> Self {
        Self::default()
    }

    /// Set minimum profit threshold.
    pub fn with_min_profit(mut self, min: Decimal) -> Self {
        self.min_profit = Some(min);
        self
    }

    /// Set minimum confidence threshold.
    pub fn with_min_confidence(mut self, min: f64) -> Self {
        self.min_confidence = Some(min);
        self
    }

    /// Set quiet hours (UTC).
    pub fn with_quiet_hours(mut self, start: u32, end: u32) -> Self {
        self.quiet_hours = Some((start, end));
        self
    }

    /// Set cooldown between notifications.
    pub fn with_cooldown(mut self, secs: u64) -> Self {
        self.cooldown_secs = Some(secs);
        self
    }

    /// Set which actions trigger notifications.
    pub fn with_actions(mut self, actions: Vec<String>) -> Self {
        self.enabled_actions = actions;
        self
    }

    /// Check if a notification should be sent for this event.
    pub fn should_notify(
        &self,
        event: &NotificationEvent,
        last_notification_ms: Option<i64>,
    ) -> bool {
        // Check action filter
        if !self.enabled_actions.is_empty()
            && !self.enabled_actions.iter().any(|a| a == &event.action)
        {
            return false;
        }

        // Check profit threshold
        if let (Some(min), Some(profit)) = (self.min_profit, event.profit) {
            if profit.abs() < min {
                return false;
            }
        }

        // Check confidence threshold
        if let (Some(min), Some(conf)) = (self.min_confidence, event.confidence) {
            if conf < min {
                return false;
            }
        }

        // Check quiet hours
        if let Some((start, end)) = self.quiet_hours {
            let hour = {
                let secs = event.timestamp_ms / 1000;
                let dt = chrono::DateTime::from_timestamp(secs, 0);
                dt.map(|d| d.format("%H").to_string().parse::<u32>().unwrap_or(0))
                    .unwrap_or(0)
            };
            if start <= end {
                // e.g., quiet 2-6: suppress if hour >= 2 && hour < 6
                if hour >= start && hour < end {
                    return false;
                }
            } else {
                // e.g., quiet 22-6: suppress if hour >= 22 || hour < 6
                if hour >= start || hour < end {
                    return false;
                }
            }
        }

        // Check cooldown
        if let (Some(cooldown), Some(last)) = (self.cooldown_secs, last_notification_ms) {
            let elapsed_ms = event.timestamp_ms - last;
            if elapsed_ms < (cooldown as i64 * 1000) {
                return false;
            }
        }

        true
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use rust_decimal_macros::dec;

    fn make_event(
        action: &str,
        profit: Option<Decimal>,
        confidence: Option<f64>,
        hour: u32,
    ) -> NotificationEvent {
        // Create a timestamp for the given UTC hour on an arbitrary day
        let ts = chrono::NaiveDate::from_ymd_opt(2026, 1, 1)
            .unwrap()
            .and_hms_opt(hour, 30, 0)
            .unwrap()
            .and_utc()
            .timestamp_millis();
        NotificationEvent {
            action: action.to_string(),
            profit,
            confidence,
            timestamp_ms: ts,
        }
    }

    #[test]
    fn default_filter_allows_everything() {
        let filter = NotificationFilter::new();
        let event = make_event("EnterLong", Some(dec!(100)), Some(0.9), 12);
        assert!(filter.should_notify(&event, None));
    }

    #[test]
    fn min_profit_filters_small_trades() {
        let filter = NotificationFilter::new().with_min_profit(dec!(50));
        let small = make_event("ExitLong", Some(dec!(10)), None, 12);
        let big = make_event("ExitLong", Some(dec!(100)), None, 12);
        assert!(!filter.should_notify(&small, None));
        assert!(filter.should_notify(&big, None));
    }

    #[test]
    fn quiet_hours_suppresses_notifications() {
        let filter = NotificationFilter::new().with_quiet_hours(22, 6);
        let night = make_event("EnterLong", None, None, 23);
        let early = make_event("EnterLong", None, None, 3);
        let day = make_event("EnterLong", None, None, 14);
        assert!(!filter.should_notify(&night, None));
        assert!(!filter.should_notify(&early, None));
        assert!(filter.should_notify(&day, None));
    }

    #[test]
    fn cooldown_prevents_spam() {
        let filter = NotificationFilter::new().with_cooldown(60);
        let event = make_event("EnterLong", None, None, 12);
        let last = event.timestamp_ms - 30_000; // 30 seconds ago
        assert!(!filter.should_notify(&event, Some(last)));
        let old_last = event.timestamp_ms - 120_000; // 2 minutes ago
        assert!(filter.should_notify(&event, Some(old_last)));
    }

    #[test]
    fn action_filter_limits_notifications() {
        let filter =
            NotificationFilter::new().with_actions(vec!["EnterLong".into(), "EnterShort".into()]);
        let enter = make_event("EnterLong", None, None, 12);
        let exit = make_event("ExitLong", None, None, 12);
        assert!(filter.should_notify(&enter, None));
        assert!(!filter.should_notify(&exit, None));
    }

    #[test]
    fn min_confidence_filters() {
        let filter = NotificationFilter::new().with_min_confidence(0.8);
        let low = make_event("EnterLong", None, Some(0.5), 12);
        let high = make_event("EnterLong", None, Some(0.9), 12);
        assert!(!filter.should_notify(&low, None));
        assert!(filter.should_notify(&high, None));
    }
}
