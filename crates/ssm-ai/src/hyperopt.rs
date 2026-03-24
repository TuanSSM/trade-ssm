use anyhow::Result;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;

/// A hyperparameter with search range.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HyperParam {
    pub name: String,
    pub param_type: ParamType,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum ParamType {
    Float { min: f64, max: f64, step: f64 },
    Int { min: i64, max: i64, step: i64 },
    Choice(Vec<String>),
}

/// Loss function for optimization.
#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
pub enum LossFunction {
    SharpeRatio,
    MaxDrawdown,
    TotalProfit,
    SortinoRatio,
    WinRate,
}

/// Result of a single hyperopt trial.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HyperoptTrial {
    pub params: HashMap<String, f64>,
    pub loss: f64,
    pub metrics: TrialMetrics,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TrialMetrics {
    pub total_profit: f64,
    pub sharpe_ratio: f64,
    pub max_drawdown: f64,
    pub win_rate: f64,
    pub total_trades: usize,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
pub enum SearchMode {
    Grid,
    Random { n_trials: usize },
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

/// Hyperparameter optimization runner.
pub struct HyperoptRunner {
    params: Vec<HyperParam>,
    loss_fn: LossFunction,
    search_mode: SearchMode,
}

impl HyperoptRunner {
    pub fn new(params: Vec<HyperParam>, loss_fn: LossFunction, search_mode: SearchMode) -> Self {
        Self {
            params,
            loss_fn,
            search_mode,
        }
    }

    /// Run optimization. `eval_fn` takes a parameter set and returns `TrialMetrics`.
    pub fn run<F>(&self, eval_fn: F) -> Result<Vec<HyperoptTrial>>
    where
        F: Fn(&HashMap<String, f64>) -> Result<TrialMetrics>,
    {
        let param_sets = match self.search_mode {
            SearchMode::Grid => self.generate_grid(),
            SearchMode::Random { n_trials } => self.generate_random(n_trials),
        };

        let mut trials: Vec<HyperoptTrial> = param_sets
            .into_iter()
            .map(|params| {
                let metrics = eval_fn(&params)?;
                let loss = self.compute_loss(&metrics);
                Ok(HyperoptTrial {
                    params,
                    loss,
                    metrics,
                })
            })
            .collect::<Result<Vec<_>>>()?;

        // Sort: for MaxDrawdown lower is better (ascending),
        // for everything else higher is better so we negate to sort ascending by loss.
        // Since we store loss as the negated "higher-is-better" values, ascending sort
        // always puts the best trial first.
        trials.sort_by(|a, b| {
            a.loss
                .partial_cmp(&b.loss)
                .unwrap_or(std::cmp::Ordering::Equal)
        });

        Ok(trials)
    }

    /// Get best trial by loss (lowest loss value).
    pub fn best_trial(trials: &[HyperoptTrial]) -> Option<&HyperoptTrial> {
        trials.iter().min_by(|a, b| {
            a.loss
                .partial_cmp(&b.loss)
                .unwrap_or(std::cmp::Ordering::Equal)
        })
    }

    fn compute_loss(&self, metrics: &TrialMetrics) -> f64 {
        match self.loss_fn {
            // Higher is better -> negate for minimization
            LossFunction::SharpeRatio => -metrics.sharpe_ratio,
            LossFunction::SortinoRatio => -metrics.sharpe_ratio, // uses sortino field would be ideal but spec uses sharpe
            LossFunction::TotalProfit => -metrics.total_profit,
            LossFunction::WinRate => -metrics.win_rate,
            // Lower drawdown is better -> use directly
            LossFunction::MaxDrawdown => metrics.max_drawdown,
        }
    }

    fn generate_grid(&self) -> Vec<HashMap<String, f64>> {
        let mut param_values: Vec<(String, Vec<f64>)> = Vec::new();

        for hp in &self.params {
            let values = match &hp.param_type {
                ParamType::Float { min, max, step } => {
                    if *step <= 0.0 {
                        vec![*min]
                    } else {
                        let mut vals = Vec::new();
                        let mut v = *min;
                        while v <= *max + step * 0.5 * f64::EPSILON {
                            vals.push(v);
                            v += step;
                        }
                        // Ensure we don't exceed max due to float drift
                        if let Some(last) = vals.last() {
                            if *last > *max + f64::EPSILON {
                                vals.pop();
                            }
                        }
                        if vals.is_empty() {
                            vec![*min]
                        } else {
                            vals
                        }
                    }
                }
                ParamType::Int { min, max, step } => {
                    let mut vals = Vec::new();
                    let mut v = *min;
                    while v <= *max {
                        vals.push(v as f64);
                        v += step;
                    }
                    if vals.is_empty() {
                        vec![*min as f64]
                    } else {
                        vals
                    }
                }
                ParamType::Choice(choices) => (0..choices.len()).map(|i| i as f64).collect(),
            };
            param_values.push((hp.name.clone(), values));
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

    fn generate_random(&self, n_trials: usize) -> Vec<HashMap<String, f64>> {
        let mut rng = LcgRng::new(42);
        let mut results = Vec::with_capacity(n_trials);

        for _ in 0..n_trials {
            let mut params = HashMap::new();
            for hp in &self.params {
                let val = match &hp.param_type {
                    ParamType::Float { min, max, .. } => {
                        let r = rng.next_f64();
                        min + (max - min) * r
                    }
                    ParamType::Int { min, max, step } => {
                        let range_size = ((max - min) / step + 1) as usize;
                        let idx = (rng.next_f64() * range_size as f64) as i64;
                        (min + idx * step) as f64
                    }
                    ParamType::Choice(choices) => {
                        let idx = (rng.next_f64() * choices.len() as f64) as usize;
                        idx.min(choices.len() - 1) as f64
                    }
                };
                params.insert(hp.name.clone(), val);
            }
            results.push(params);
        }

        results
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn make_params(pairs: &[(&str, ParamType)]) -> Vec<HyperParam> {
        pairs
            .iter()
            .map(|(name, pt)| HyperParam {
                name: name.to_string(),
                param_type: pt.clone(),
            })
            .collect()
    }

    #[test]
    fn grid_search_generates_correct_count() {
        let params = make_params(&[
            (
                "lr",
                ParamType::Float {
                    min: 0.01,
                    max: 0.03,
                    step: 0.01,
                },
            ),
            (
                "epochs",
                ParamType::Int {
                    min: 10,
                    max: 30,
                    step: 10,
                },
            ),
        ]);

        let runner = HyperoptRunner::new(params, LossFunction::SharpeRatio, SearchMode::Grid);
        let combos = runner.generate_grid();
        // lr: 0.01, 0.02, 0.03 = 3 values
        // epochs: 10, 20, 30 = 3 values
        // total: 3 * 3 = 9
        assert_eq!(combos.len(), 9);
    }

    #[test]
    fn random_search_generates_n_trials() {
        let params = make_params(&[(
            "lr",
            ParamType::Float {
                min: 0.0,
                max: 1.0,
                step: 0.1,
            },
        )]);

        let runner = HyperoptRunner::new(
            params,
            LossFunction::TotalProfit,
            SearchMode::Random { n_trials: 25 },
        );
        let result = runner
            .run(|_params| {
                Ok(TrialMetrics {
                    total_profit: 100.0,
                    sharpe_ratio: 1.0,
                    max_drawdown: 5.0,
                    win_rate: 0.6,
                    total_trades: 10,
                })
            })
            .unwrap();

        assert_eq!(result.len(), 25);
    }

    #[test]
    fn best_trial_sharpe_ratio_selects_max() {
        let trials = vec![
            HyperoptTrial {
                params: HashMap::new(),
                loss: -1.5, // sharpe 1.5
                metrics: TrialMetrics {
                    total_profit: 100.0,
                    sharpe_ratio: 1.5,
                    max_drawdown: 5.0,
                    win_rate: 0.6,
                    total_trades: 10,
                },
            },
            HyperoptTrial {
                params: HashMap::new(),
                loss: -2.5, // sharpe 2.5 (better)
                metrics: TrialMetrics {
                    total_profit: 200.0,
                    sharpe_ratio: 2.5,
                    max_drawdown: 3.0,
                    win_rate: 0.7,
                    total_trades: 15,
                },
            },
        ];

        let best = HyperoptRunner::best_trial(&trials).unwrap();
        assert!((best.metrics.sharpe_ratio - 2.5).abs() < f64::EPSILON);
    }

    #[test]
    fn best_trial_max_drawdown_selects_min() {
        let trials = vec![
            HyperoptTrial {
                params: HashMap::new(),
                loss: 10.0, // drawdown 10%
                metrics: TrialMetrics {
                    total_profit: 100.0,
                    sharpe_ratio: 1.0,
                    max_drawdown: 10.0,
                    win_rate: 0.5,
                    total_trades: 10,
                },
            },
            HyperoptTrial {
                params: HashMap::new(),
                loss: 3.0, // drawdown 3% (better)
                metrics: TrialMetrics {
                    total_profit: 80.0,
                    sharpe_ratio: 0.8,
                    max_drawdown: 3.0,
                    win_rate: 0.55,
                    total_trades: 8,
                },
            },
        ];

        let best = HyperoptRunner::best_trial(&trials).unwrap();
        assert!((best.metrics.max_drawdown - 3.0).abs() < f64::EPSILON);
    }

    #[test]
    fn loss_functions_compute_correctly() {
        let metrics = TrialMetrics {
            total_profit: 500.0,
            sharpe_ratio: 2.0,
            max_drawdown: 8.0,
            win_rate: 0.65,
            total_trades: 20,
        };

        let runner_sharpe =
            HyperoptRunner::new(vec![], LossFunction::SharpeRatio, SearchMode::Grid);
        assert!((runner_sharpe.compute_loss(&metrics) - (-2.0)).abs() < f64::EPSILON);

        let runner_dd = HyperoptRunner::new(vec![], LossFunction::MaxDrawdown, SearchMode::Grid);
        assert!((runner_dd.compute_loss(&metrics) - 8.0).abs() < f64::EPSILON);

        let runner_profit =
            HyperoptRunner::new(vec![], LossFunction::TotalProfit, SearchMode::Grid);
        assert!((runner_profit.compute_loss(&metrics) - (-500.0)).abs() < f64::EPSILON);

        let runner_wr = HyperoptRunner::new(vec![], LossFunction::WinRate, SearchMode::Grid);
        assert!((runner_wr.compute_loss(&metrics) - (-0.65)).abs() < f64::EPSILON);
    }

    #[test]
    fn run_grid_search_end_to_end() {
        let params = make_params(&[(
            "x",
            ParamType::Int {
                min: 1,
                max: 3,
                step: 1,
            },
        )]);

        let runner = HyperoptRunner::new(params, LossFunction::TotalProfit, SearchMode::Grid);
        let trials = runner
            .run(|p| {
                let x = p["x"];
                Ok(TrialMetrics {
                    total_profit: x * 100.0,
                    sharpe_ratio: x,
                    max_drawdown: 10.0 / x,
                    win_rate: 0.5,
                    total_trades: 10,
                })
            })
            .unwrap();

        assert_eq!(trials.len(), 3);
        // Best by total profit (highest) should be first (loss is most negative)
        assert!((trials[0].metrics.total_profit - 300.0).abs() < f64::EPSILON);
    }

    #[test]
    fn best_trial_empty_returns_none() {
        let trials: Vec<HyperoptTrial> = vec![];
        assert!(HyperoptRunner::best_trial(&trials).is_none());
    }
}
