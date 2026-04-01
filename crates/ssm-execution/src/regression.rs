use rust_decimal::Decimal;
use serde::{Deserialize, Serialize};

use crate::backtest::BacktestResult;

// ---------------------------------------------------------------------------
// Thresholds
// ---------------------------------------------------------------------------

/// Thresholds for regression detection.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RegressionThresholds {
    /// Max allowed drop in win rate (absolute, e.g. 0.05 = 5%).
    pub max_winrate_drop: f64,
    /// Max allowed drop in profit factor (absolute, e.g. 0.5).
    pub max_profit_factor_drop: Decimal,
    /// Max allowed increase in max drawdown percent (absolute, e.g. 5.0%).
    pub max_drawdown_increase: Decimal,
    /// Max allowed drop in Sharpe ratio (absolute).
    pub max_sharpe_drop: f64,
    /// Min required total trades (regression if fewer).
    pub min_total_trades: usize,
}

impl Default for RegressionThresholds {
    fn default() -> Self {
        Self {
            max_winrate_drop: 0.05,
            max_profit_factor_drop: Decimal::new(5, 1), // 0.5
            max_drawdown_increase: Decimal::from(5),    // 5%
            max_sharpe_drop: 0.3,
            min_total_trades: 5,
        }
    }
}

// ---------------------------------------------------------------------------
// Report types
// ---------------------------------------------------------------------------

/// Result of a regression comparison.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RegressionReport {
    pub passed: bool,
    pub violations: Vec<RegressionViolation>,
    pub improvements: Vec<String>,
    pub baseline_summary: MetricsSummary,
    pub current_summary: MetricsSummary,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RegressionViolation {
    pub metric: String,
    pub baseline_value: String,
    pub current_value: String,
    pub threshold: String,
    pub severity: Severity,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum Severity {
    Warning,
    Critical,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MetricsSummary {
    pub total_trades: usize,
    pub win_rate: f64,
    pub profit_factor: Decimal,
    pub max_drawdown_pct: Decimal,
    pub sharpe_ratio: f64,
    pub total_profit_pct: Decimal,
}

// ---------------------------------------------------------------------------
// From impl
// ---------------------------------------------------------------------------

impl From<&BacktestResult> for MetricsSummary {
    fn from(r: &BacktestResult) -> Self {
        Self {
            total_trades: r.total_trades,
            win_rate: r.win_rate,
            profit_factor: r.profit_factor,
            max_drawdown_pct: r.max_drawdown_pct,
            sharpe_ratio: r.sharpe_ratio,
            total_profit_pct: r.total_profit_pct,
        }
    }
}

// ---------------------------------------------------------------------------
// Detection logic
// ---------------------------------------------------------------------------

/// Compare two backtest results and detect regressions.
///
/// Returns a [`RegressionReport`] summarising every metric that crossed a
/// threshold, every metric that improved, and an overall `passed` flag.
/// `passed` is `true` when no `Critical` violations exist (warnings are
/// informational only).
pub fn detect_regression(
    baseline: &BacktestResult,
    current: &BacktestResult,
    thresholds: &RegressionThresholds,
) -> RegressionReport {
    let mut violations: Vec<RegressionViolation> = Vec::new();
    let mut improvements: Vec<String> = Vec::new();

    // --- Win rate ---
    let winrate_drop = baseline.win_rate - current.win_rate;
    if winrate_drop > thresholds.max_winrate_drop {
        violations.push(RegressionViolation {
            metric: "win_rate".to_string(),
            baseline_value: format!("{:.4}", baseline.win_rate),
            current_value: format!("{:.4}", current.win_rate),
            threshold: format!("max drop {:.4}", thresholds.max_winrate_drop),
            severity: Severity::Critical,
        });
    } else if current.win_rate > baseline.win_rate {
        improvements.push(format!(
            "win_rate improved: {:.4} -> {:.4}",
            baseline.win_rate, current.win_rate
        ));
    }

    // --- Profit factor ---
    let pf_drop = baseline.profit_factor - current.profit_factor;
    if pf_drop > thresholds.max_profit_factor_drop {
        violations.push(RegressionViolation {
            metric: "profit_factor".to_string(),
            baseline_value: baseline.profit_factor.to_string(),
            current_value: current.profit_factor.to_string(),
            threshold: format!("max drop {}", thresholds.max_profit_factor_drop),
            severity: Severity::Critical,
        });
    } else if current.profit_factor > baseline.profit_factor {
        improvements.push(format!(
            "profit_factor improved: {} -> {}",
            baseline.profit_factor, current.profit_factor
        ));
    }

    // --- Max drawdown (increase = regression) ---
    let dd_increase = current.max_drawdown_pct - baseline.max_drawdown_pct;
    if dd_increase > thresholds.max_drawdown_increase {
        violations.push(RegressionViolation {
            metric: "max_drawdown_pct".to_string(),
            baseline_value: baseline.max_drawdown_pct.to_string(),
            current_value: current.max_drawdown_pct.to_string(),
            threshold: format!("max increase {}%", thresholds.max_drawdown_increase),
            severity: Severity::Critical,
        });
    } else if current.max_drawdown_pct < baseline.max_drawdown_pct {
        improvements.push(format!(
            "max_drawdown_pct improved: {} -> {}",
            baseline.max_drawdown_pct, current.max_drawdown_pct
        ));
    }

    // --- Sharpe ratio ---
    let sharpe_drop = baseline.sharpe_ratio - current.sharpe_ratio;
    if sharpe_drop > thresholds.max_sharpe_drop {
        violations.push(RegressionViolation {
            metric: "sharpe_ratio".to_string(),
            baseline_value: format!("{:.4}", baseline.sharpe_ratio),
            current_value: format!("{:.4}", current.sharpe_ratio),
            threshold: format!("max drop {:.4}", thresholds.max_sharpe_drop),
            severity: Severity::Warning,
        });
    } else if current.sharpe_ratio > baseline.sharpe_ratio {
        improvements.push(format!(
            "sharpe_ratio improved: {:.4} -> {:.4}",
            baseline.sharpe_ratio, current.sharpe_ratio
        ));
    }

    // --- Minimum trades ---
    if current.total_trades < thresholds.min_total_trades {
        violations.push(RegressionViolation {
            metric: "total_trades".to_string(),
            baseline_value: baseline.total_trades.to_string(),
            current_value: current.total_trades.to_string(),
            threshold: format!("min {}", thresholds.min_total_trades),
            severity: Severity::Warning,
        });
    } else if current.total_trades > baseline.total_trades {
        improvements.push(format!(
            "total_trades increased: {} -> {}",
            baseline.total_trades, current.total_trades
        ));
    }

    // --- Total profit ---
    if current.total_profit_pct > baseline.total_profit_pct {
        improvements.push(format!(
            "total_profit_pct improved: {} -> {}",
            baseline.total_profit_pct, current.total_profit_pct
        ));
    }

    let passed = violations.iter().all(|v| v.severity != Severity::Critical);

    RegressionReport {
        passed,
        violations,
        improvements,
        baseline_summary: MetricsSummary::from(baseline),
        current_summary: MetricsSummary::from(current),
    }
}

// ===========================================================================
// Tests
// ===========================================================================

#[cfg(test)]
mod tests {
    use super::*;
    use crate::backtest::BacktestResult;
    use rust_decimal::Decimal;

    fn make_result(
        total_trades: usize,
        winning_trades: usize,
        win_rate: f64,
        profit_factor: Decimal,
        max_drawdown_pct: Decimal,
        sharpe_ratio: f64,
        total_profit_pct: Decimal,
    ) -> BacktestResult {
        let losing_trades = total_trades - winning_trades;
        BacktestResult {
            total_trades,
            winning_trades,
            losing_trades,
            win_rate,
            total_profit: Decimal::ZERO,
            total_profit_pct,
            avg_profit: Decimal::ZERO,
            avg_duration_candles: 0.0,
            best_trade: Decimal::ZERO,
            worst_trade: Decimal::ZERO,
            max_drawdown: Decimal::ZERO,
            max_drawdown_pct,
            max_drawdown_duration: 0,
            sharpe_ratio,
            sortino_ratio: 0.0,
            profit_factor,
            final_balance: Decimal::from(10_000),
            trades: vec![],
        }
    }

    fn good_baseline() -> BacktestResult {
        make_result(
            20,
            14,
            0.70,
            Decimal::new(18, 1), // 1.8
            Decimal::new(8, 0),  // 8%
            1.5,
            Decimal::new(120, 1), // 12%
        )
    }

    // -----------------------------------------------------------------------
    // 1. No regression when results are identical
    // -----------------------------------------------------------------------
    #[test]
    fn test_no_regression_identical() {
        let baseline = good_baseline();
        let current = baseline.clone();
        let thresholds = RegressionThresholds::default();

        let report = detect_regression(&baseline, &current, &thresholds);

        assert!(report.passed);
        assert!(
            report.violations.is_empty(),
            "expected no violations, got: {:?}",
            report.violations
        );
    }

    // -----------------------------------------------------------------------
    // 2. Win rate regression → Critical
    // -----------------------------------------------------------------------
    #[test]
    fn test_winrate_regression_critical() {
        let baseline = good_baseline();
        // Drop win rate by 0.10 (> default threshold of 0.05)
        let current = make_result(
            20,
            12,
            0.60,
            Decimal::new(18, 1),
            Decimal::new(8, 0),
            1.5,
            Decimal::new(120, 1),
        );
        let thresholds = RegressionThresholds::default();

        let report = detect_regression(&baseline, &current, &thresholds);

        assert!(!report.passed);
        let v = report
            .violations
            .iter()
            .find(|v| v.metric == "win_rate")
            .expect("win_rate violation missing");
        assert_eq!(v.severity, Severity::Critical);
    }

    // -----------------------------------------------------------------------
    // 3. Profit factor regression → Critical
    // -----------------------------------------------------------------------
    #[test]
    fn test_profit_factor_regression_critical() {
        let baseline = good_baseline();
        // Drop profit factor from 1.8 to 0.9 (drop of 0.9 > default 0.5)
        let current = make_result(
            20,
            14,
            0.70,
            Decimal::new(9, 1), // 0.9
            Decimal::new(8, 0),
            1.5,
            Decimal::new(120, 1),
        );
        let thresholds = RegressionThresholds::default();

        let report = detect_regression(&baseline, &current, &thresholds);

        assert!(!report.passed);
        let v = report
            .violations
            .iter()
            .find(|v| v.metric == "profit_factor")
            .expect("profit_factor violation missing");
        assert_eq!(v.severity, Severity::Critical);
    }

    // -----------------------------------------------------------------------
    // 4. Drawdown increase regression → Critical
    // -----------------------------------------------------------------------
    #[test]
    fn test_drawdown_regression_critical() {
        let baseline = good_baseline();
        // Increase max drawdown from 8% to 15% (increase of 7 > default 5)
        let current = make_result(
            20,
            14,
            0.70,
            Decimal::new(18, 1),
            Decimal::from(15), // 15%
            1.5,
            Decimal::new(120, 1),
        );
        let thresholds = RegressionThresholds::default();

        let report = detect_regression(&baseline, &current, &thresholds);

        assert!(!report.passed);
        let v = report
            .violations
            .iter()
            .find(|v| v.metric == "max_drawdown_pct")
            .expect("max_drawdown_pct violation missing");
        assert_eq!(v.severity, Severity::Critical);
    }

    // -----------------------------------------------------------------------
    // 5. Sharpe drop → Warning (not Critical, still passes)
    // -----------------------------------------------------------------------
    #[test]
    fn test_sharpe_regression_warning_only() {
        let baseline = good_baseline();
        // Drop Sharpe from 1.5 to 1.0 (drop of 0.5 > default 0.3)
        let current = make_result(
            20,
            14,
            0.70,
            Decimal::new(18, 1),
            Decimal::new(8, 0),
            1.0,
            Decimal::new(120, 1),
        );
        let thresholds = RegressionThresholds::default();

        let report = detect_regression(&baseline, &current, &thresholds);

        // Only a Warning → should still pass
        assert!(report.passed, "sharpe warning should not fail the report");
        let v = report
            .violations
            .iter()
            .find(|v| v.metric == "sharpe_ratio")
            .expect("sharpe_ratio violation missing");
        assert_eq!(v.severity, Severity::Warning);
    }

    // -----------------------------------------------------------------------
    // 6. Multiple violations at once
    // -----------------------------------------------------------------------
    #[test]
    fn test_multiple_violations() {
        let baseline = good_baseline();
        // Worsen every metric beyond every threshold simultaneously
        let current = make_result(
            2, // below min_total_trades=5 → Warning
            1,
            0.50,               // win_rate drop 0.20 → Critical
            Decimal::new(5, 1), // profit_factor 0.5, drop of 1.3 → Critical
            Decimal::from(20),  // drawdown 20%, increase of 12 → Critical
            0.5,                // sharpe drop 1.0 → Warning
            Decimal::ZERO,
        );
        let thresholds = RegressionThresholds::default();

        let report = detect_regression(&baseline, &current, &thresholds);

        assert!(!report.passed);
        assert!(
            report.violations.len() >= 4,
            "expected at least 4 violations"
        );

        let critical_count = report
            .violations
            .iter()
            .filter(|v| v.severity == Severity::Critical)
            .count();
        let warning_count = report
            .violations
            .iter()
            .filter(|v| v.severity == Severity::Warning)
            .count();
        assert!(critical_count >= 3);
        assert!(warning_count >= 1);
    }

    // -----------------------------------------------------------------------
    // 7. Improvements detected alongside regressions
    // -----------------------------------------------------------------------
    #[test]
    fn test_improvements_detected_alongside_regressions() {
        let baseline = good_baseline();
        // Win rate drops (Critical), but drawdown improves and profit improves
        let current = make_result(
            20,
            12,
            0.60,                 // win_rate regression
            Decimal::new(22, 1),  // profit_factor improved
            Decimal::new(4, 0),   // drawdown improved (4% vs 8%)
            2.0,                  // sharpe improved
            Decimal::new(200, 1), // total_profit_pct improved
        );
        let thresholds = RegressionThresholds::default();

        let report = detect_regression(&baseline, &current, &thresholds);

        assert!(!report.passed); // win_rate Critical
        assert!(
            !report.improvements.is_empty(),
            "expected improvements to be reported"
        );
        // Drawdown, profit_factor, sharpe, total_profit_pct all improved
        let has_dd_improvement = report
            .improvements
            .iter()
            .any(|s| s.contains("max_drawdown_pct"));
        assert!(has_dd_improvement, "drawdown improvement should be noted");
    }

    // -----------------------------------------------------------------------
    // 8. Default thresholds are sensible
    // -----------------------------------------------------------------------
    #[test]
    fn test_default_thresholds_sensible() {
        let t = RegressionThresholds::default();

        // win rate: 5% absolute drop threshold
        assert!((t.max_winrate_drop - 0.05).abs() < f64::EPSILON);
        // profit factor: 0.5 drop
        assert_eq!(t.max_profit_factor_drop, Decimal::new(5, 1));
        // drawdown: 5% increase
        assert_eq!(t.max_drawdown_increase, Decimal::from(5));
        // Sharpe: 0.3 drop
        assert!((t.max_sharpe_drop - 0.3).abs() < f64::EPSILON);
        // minimum trades
        assert_eq!(t.min_total_trades, 5);
    }

    // -----------------------------------------------------------------------
    // 9. Min trades violation → Warning (still passes unless combined Critical)
    // -----------------------------------------------------------------------
    #[test]
    fn test_min_trades_violation_warning() {
        let baseline = good_baseline();
        // Everything else is fine, only trade count is below minimum
        let current = make_result(
            3, // below min of 5
            2,
            0.66,
            Decimal::new(18, 1),
            Decimal::new(8, 0),
            1.5,
            Decimal::new(120, 1),
        );
        let thresholds = RegressionThresholds::default();

        let report = detect_regression(&baseline, &current, &thresholds);

        // Warning only → passes
        assert!(report.passed);
        let v = report
            .violations
            .iter()
            .find(|v| v.metric == "total_trades")
            .expect("total_trades violation missing");
        assert_eq!(v.severity, Severity::Warning);
    }

    // -----------------------------------------------------------------------
    // 10. MetricsSummary correctly mirrors BacktestResult fields
    // -----------------------------------------------------------------------
    #[test]
    fn test_metrics_summary_from_backtest_result() {
        let r = good_baseline();
        let summary = MetricsSummary::from(&r);

        assert_eq!(summary.total_trades, r.total_trades);
        assert!((summary.win_rate - r.win_rate).abs() < f64::EPSILON);
        assert_eq!(summary.profit_factor, r.profit_factor);
        assert_eq!(summary.max_drawdown_pct, r.max_drawdown_pct);
        assert!((summary.sharpe_ratio - r.sharpe_ratio).abs() < f64::EPSILON);
        assert_eq!(summary.total_profit_pct, r.total_profit_pct);
    }
}
