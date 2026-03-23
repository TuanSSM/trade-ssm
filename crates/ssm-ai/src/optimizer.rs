use serde::{Deserialize, Serialize};
use ssm_core::{AIAction, Candle};
use std::collections::HashMap;

use crate::config::RlConfig;
use crate::env::{Observation, TradingEnv};
use crate::metrics::EpisodeMetrics;

/// A parameter range for optimization.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum ParamRange {
    Float { min: f64, max: f64, steps: usize },
    Int { min: usize, max: usize, step: usize },
}

/// Defines which parameters to search over.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SearchSpace {
    pub params: Vec<(String, ParamRange)>,
}

/// Objective function selector.
#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
pub enum Objective {
    TotalReturn,
    SharpeRatio,
    ProfitFactor,
    WinRate,
}

impl Objective {
    pub fn parse(s: &str) -> Option<Self> {
        match s {
            "TotalReturn" => Some(Objective::TotalReturn),
            "SharpeRatio" => Some(Objective::SharpeRatio),
            "ProfitFactor" => Some(Objective::ProfitFactor),
            "WinRate" => Some(Objective::WinRate),
            _ => None,
        }
    }

    pub fn extract(&self, metrics: &EpisodeMetrics) -> f64 {
        match self {
            Objective::TotalReturn => metrics.total_return_pct,
            Objective::SharpeRatio => metrics.sharpe_ratio,
            Objective::ProfitFactor => {
                if metrics.profit_factor.is_infinite() {
                    f64::MAX
                } else {
                    metrics.profit_factor
                }
            }
            Objective::WinRate => metrics.win_rate,
        }
    }
}

/// A single trial result from optimization.
#[derive(Debug, Clone, Serialize)]
pub struct TrialResult {
    pub trial_id: usize,
    pub params: HashMap<String, f64>,
    pub metrics: EpisodeMetrics,
    pub objective: f64,
}

/// Generate parameter combinations for grid search (cartesian product).
pub fn grid_search(space: &SearchSpace) -> Vec<HashMap<String, f64>> {
    let mut param_values: Vec<(String, Vec<f64>)> = Vec::new();

    for (name, range) in &space.params {
        let values = match range {
            ParamRange::Float { min, max, steps } => {
                if *steps <= 1 {
                    vec![*min]
                } else {
                    (0..*steps)
                        .map(|i| min + (max - min) * (i as f64 / (*steps - 1) as f64))
                        .collect()
                }
            }
            ParamRange::Int { min, max, step } => {
                let mut vals = Vec::new();
                let mut v = *min;
                while v <= *max {
                    vals.push(v as f64);
                    v += step;
                }
                vals
            }
        };
        param_values.push((name.clone(), values));
    }

    // Cartesian product
    let mut combinations = vec![HashMap::new()];
    for (name, values) in &param_values {
        let mut new_combinations = Vec::new();
        for combo in &combinations {
            for &val in values {
                let mut new_combo = combo.clone();
                new_combo.insert(name.clone(), val);
                new_combinations.push(new_combo);
            }
        }
        combinations = new_combinations;
    }

    combinations
}

/// Generate random parameter combinations using a simple LCG PRNG.
pub fn random_search(space: &SearchSpace, n_trials: usize, seed: u64) -> Vec<HashMap<String, f64>> {
    let mut rng = LcgRng::new(seed);
    let mut results = Vec::with_capacity(n_trials);

    for _ in 0..n_trials {
        let mut params = HashMap::new();
        for (name, range) in &space.params {
            let val = match range {
                ParamRange::Float { min, max, .. } => {
                    let r = rng.next_f64();
                    min + (max - min) * r
                }
                ParamRange::Int { min, max, step } => {
                    let range_size = (max - min) / step + 1;
                    let idx = (rng.next_f64() * range_size as f64) as usize;
                    (min + idx * step) as f64
                }
            };
            params.insert(name.clone(), val);
        }
        results.push(params);
    }

    results
}

/// Simple LCG random number generator (no external deps).
struct LcgRng {
    state: u64,
}

impl LcgRng {
    fn new(seed: u64) -> Self {
        Self {
            state: seed.wrapping_add(1),
        }
    }

    fn next_u64(&mut self) -> u64 {
        // LCG parameters from Numerical Recipes
        self.state = self
            .state
            .wrapping_mul(6_364_136_223_846_793_005)
            .wrapping_add(1);
        self.state
    }

    fn next_f64(&mut self) -> f64 {
        (self.next_u64() >> 11) as f64 / (1u64 << 53) as f64
    }
}

/// Apply a parameter set to an RlConfig, returning a modified config.
pub fn apply_params(base: &RlConfig, params: &HashMap<String, f64>) -> RlConfig {
    let mut config = base.clone();

    for (name, &value) in params {
        match name.as_str() {
            "fee_rate" => config.env.fee_rate = value,
            "slippage_rate" => config.env.slippage_rate = value,
            "initial_balance" => config.env.initial_balance = value,
            "position_size_pct" => config.env.position_size_pct = value,
            "hold_penalty_threshold" => config.reward.hold_penalty_threshold = value as usize,
            "hold_penalty_rate" => config.reward.hold_penalty_rate = value,
            "invalid_action_penalty" => config.reward.invalid_action_penalty = value,
            "close_penalty_threshold" => config.reward.close_penalty_threshold = value as usize,
            "close_penalty_rate" => config.reward.close_penalty_rate = value,
            "win_bonus" => config.reward.win_bonus = value,
            "drawdown_penalty_rate" => config.reward.drawdown_penalty_rate = value,
            _ => {
                tracing::warn!(param = name, "unknown parameter in search space");
            }
        }
    }

    config
}

/// Run a single trial: create env with config, step through with a policy, return metrics.
pub fn run_trial(
    candles: &[Candle],
    config: &RlConfig,
    steps_per_year: f64,
    policy: &dyn Fn(&Observation) -> AIAction,
) -> EpisodeMetrics {
    let mut env =
        TradingEnv::with_config(candles.to_vec(), config.env.clone(), config.reward.clone());
    let mut obs = env.reset();

    while !obs.done {
        let action = policy(&obs);
        let (new_obs, _) = env.step(action);
        obs = new_obs;
    }

    env.episode_metrics(steps_per_year)
}

/// Run optimization: test multiple parameter configurations and return sorted results.
pub fn optimize(
    candles: &[Candle],
    base_config: &RlConfig,
    param_sets: &[HashMap<String, f64>],
    objective: Objective,
    steps_per_year: f64,
    policy: &dyn Fn(&Observation) -> AIAction,
) -> Vec<TrialResult> {
    let mut results: Vec<TrialResult> = param_sets
        .iter()
        .enumerate()
        .map(|(id, params)| {
            let config = apply_params(base_config, params);
            let metrics = run_trial(candles, &config, steps_per_year, policy);
            let obj_value = objective.extract(&metrics);
            TrialResult {
                trial_id: id,
                params: params.clone(),
                metrics,
                objective: obj_value,
            }
        })
        .collect();

    results.sort_by(|a, b| {
        b.objective
            .partial_cmp(&a.objective)
            .unwrap_or(std::cmp::Ordering::Equal)
    });
    results
}

#[cfg(test)]
mod tests {
    use super::*;
    use rust_decimal::Decimal;
    use std::str::FromStr;

    fn candle_price(price: &str) -> Candle {
        let p = Decimal::from_str(price).unwrap();
        Candle {
            open_time: 0,
            open: p,
            high: p,
            low: p,
            close: p,
            volume: Decimal::from(100),
            close_time: 0,
            quote_volume: Decimal::ZERO,
            trades: 10,
            taker_buy_volume: Decimal::from(50),
            taker_sell_volume: Decimal::from(50),
        }
    }

    #[test]
    fn grid_search_generates_correct_count() {
        let space = SearchSpace {
            params: vec![
                (
                    "fee_rate".into(),
                    ParamRange::Float {
                        min: 0.0,
                        max: 0.001,
                        steps: 2,
                    },
                ),
                (
                    "hold_penalty_threshold".into(),
                    ParamRange::Int {
                        min: 10,
                        max: 30,
                        step: 10,
                    },
                ),
            ],
        };
        let combos = grid_search(&space);
        // 2 float steps * 3 int values (10, 20, 30) = 6
        assert_eq!(combos.len(), 6);
    }

    #[test]
    fn random_search_seeded_deterministic() {
        let space = SearchSpace {
            params: vec![(
                "fee_rate".into(),
                ParamRange::Float {
                    min: 0.0,
                    max: 1.0,
                    steps: 10,
                },
            )],
        };
        let a = random_search(&space, 5, 42);
        let b = random_search(&space, 5, 42);
        for (x, y) in a.iter().zip(b.iter()) {
            assert!((x["fee_rate"] - y["fee_rate"]).abs() < f64::EPSILON);
        }
    }

    #[test]
    fn apply_params_modifies_config() {
        let base = RlConfig::default();
        let mut params = HashMap::new();
        params.insert("fee_rate".into(), 0.005);
        params.insert("hold_penalty_threshold".into(), 30.0);

        let modified = apply_params(&base, &params);
        assert!((modified.env.fee_rate - 0.005).abs() < f64::EPSILON);
        assert_eq!(modified.reward.hold_penalty_threshold, 30);
    }

    #[test]
    fn run_trial_returns_valid_metrics() {
        let candles: Vec<Candle> = (0..50)
            .map(|i| {
                let price = format!("{}", 100 + i % 10);
                candle_price(&price)
            })
            .collect();

        let config = RlConfig::default();
        // Simple policy: always neutral
        let metrics = run_trial(&candles, &config, 35040.0, &|_obs| AIAction::Neutral);

        assert_eq!(metrics.total_trades, 0);
        assert!((metrics.initial_balance - 10_000.0).abs() < f64::EPSILON);
    }

    #[test]
    fn objective_extraction() {
        let metrics = EpisodeMetrics {
            initial_balance: 10_000.0,
            final_balance: 11_000.0,
            equity_curve: vec![],
            total_return_pct: 10.0,
            buy_and_hold_return_pct: 5.0,
            alpha: 5.0,
            max_drawdown_pct: 3.0,
            sharpe_ratio: 1.5,
            sortino_ratio: 2.0,
            total_trades: 10,
            winning_trades: 7,
            losing_trades: 3,
            win_rate: 0.7,
            profit_factor: 2.5,
            avg_win: 200.0,
            avg_loss: 100.0,
            largest_win: 500.0,
            largest_loss: 300.0,
            avg_hold_duration: 5.0,
            total_fees_paid: 10.0,
        };

        assert!((Objective::TotalReturn.extract(&metrics) - 10.0).abs() < f64::EPSILON);
        assert!((Objective::SharpeRatio.extract(&metrics) - 1.5).abs() < f64::EPSILON);
        assert!((Objective::WinRate.extract(&metrics) - 0.7).abs() < f64::EPSILON);
        assert!((Objective::ProfitFactor.extract(&metrics) - 2.5).abs() < f64::EPSILON);
    }

    #[test]
    fn optimize_sorts_by_objective() {
        let candles: Vec<Candle> = (0..20).map(|_| candle_price("100")).collect();
        let base = RlConfig::default();

        let param_sets = vec![
            {
                let mut m = HashMap::new();
                m.insert("fee_rate".into(), 0.001);
                m
            },
            {
                let mut m = HashMap::new();
                m.insert("fee_rate".into(), 0.0);
                m
            },
        ];

        let results = optimize(
            &candles,
            &base,
            &param_sets,
            Objective::TotalReturn,
            35040.0,
            &|_| AIAction::Neutral,
        );

        assert_eq!(results.len(), 2);
        // Results should be sorted descending by objective
        assert!(results[0].objective >= results[1].objective);
    }

    #[test]
    fn objective_parse_all_variants() {
        assert!(matches!(Objective::parse("TotalReturn"), Some(Objective::TotalReturn)));
        assert!(matches!(Objective::parse("SharpeRatio"), Some(Objective::SharpeRatio)));
        assert!(matches!(Objective::parse("ProfitFactor"), Some(Objective::ProfitFactor)));
        assert!(matches!(Objective::parse("WinRate"), Some(Objective::WinRate)));
        assert!(Objective::parse("Invalid").is_none());
        assert!(Objective::parse("").is_none());
    }

    #[test]
    fn grid_search_single_param_single_step() {
        let space = SearchSpace {
            params: vec![(
                "fee_rate".into(),
                ParamRange::Float {
                    min: 0.001,
                    max: 0.001,
                    steps: 1,
                },
            )],
        };
        let combos = grid_search(&space);
        assert_eq!(combos.len(), 1);
        assert!((combos[0]["fee_rate"] - 0.001).abs() < f64::EPSILON);
    }

    #[test]
    fn grid_search_empty_space() {
        let space = SearchSpace { params: vec![] };
        let combos = grid_search(&space);
        // Empty space should produce one combo (the empty combination)
        assert_eq!(combos.len(), 1);
        assert!(combos[0].is_empty());
    }

    #[test]
    fn random_search_values_in_range() {
        let space = SearchSpace {
            params: vec![(
                "fee_rate".into(),
                ParamRange::Float {
                    min: 0.0,
                    max: 1.0,
                    steps: 10,
                },
            )],
        };
        let results = random_search(&space, 100, 42);
        assert_eq!(results.len(), 100);
        for r in &results {
            let val = r["fee_rate"];
            assert!(val >= 0.0 && val <= 1.0, "value {val} out of range [0, 1]");
        }
    }

    #[test]
    fn apply_params_unknown_param_ignored() {
        let base = RlConfig::default();
        let mut params = HashMap::new();
        params.insert("nonexistent_param".into(), 999.0);
        let modified = apply_params(&base, &params);
        // Should not change any defaults
        assert!((modified.env.fee_rate - 0.0).abs() < f64::EPSILON);
    }

    #[test]
    fn objective_profit_factor_infinite() {
        let metrics = EpisodeMetrics {
            initial_balance: 10_000.0,
            final_balance: 11_000.0,
            equity_curve: vec![],
            total_return_pct: 10.0,
            buy_and_hold_return_pct: 5.0,
            alpha: 5.0,
            max_drawdown_pct: 0.0,
            sharpe_ratio: 0.0,
            sortino_ratio: 0.0,
            total_trades: 1,
            winning_trades: 1,
            losing_trades: 0,
            win_rate: 1.0,
            profit_factor: f64::INFINITY,
            avg_win: 1000.0,
            avg_loss: 0.0,
            largest_win: 1000.0,
            largest_loss: 0.0,
            avg_hold_duration: 5.0,
            total_fees_paid: 0.0,
        };
        // Infinite profit factor should be converted to f64::MAX
        let val = Objective::ProfitFactor.extract(&metrics);
        assert_eq!(val, f64::MAX);
    }
}
