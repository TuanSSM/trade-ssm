use ssm_core::Candle;

/// Episode sampler for training diversity.
///
/// Instead of always training from candle 0, randomly samples windows
/// of candles for each training episode.
pub struct EpisodeSampler {
    min_length: usize,
    max_length: usize,
}

impl EpisodeSampler {
    pub fn new(min_length: usize, max_length: usize) -> Self {
        Self {
            min_length: min_length.max(10),
            max_length,
        }
    }

    /// Sample a random window of candles for a training episode.
    ///
    /// Returns a slice of the input candles with random start and length.
    pub fn sample<'a>(&self, candles: &'a [Candle], seed: u64) -> &'a [Candle] {
        if candles.len() <= self.min_length {
            return candles;
        }

        let mut rng = LcgRng::new(seed);

        let max_len = self.max_length.min(candles.len());
        let min_len = self.min_length.min(max_len);
        let length = if max_len > min_len {
            min_len + ((rng.next_f64() * (max_len - min_len) as f64) as usize)
        } else {
            min_len
        };

        let max_start = candles.len() - length;
        let start = if max_start > 0 {
            (rng.next_f64() * max_start as f64) as usize
        } else {
            0
        };

        &candles[start..start + length]
    }

    /// Generate multiple episode windows for batch training.
    pub fn sample_batch<'a>(
        &self,
        candles: &'a [Candle],
        n_episodes: usize,
        base_seed: u64,
    ) -> Vec<&'a [Candle]> {
        (0..n_episodes)
            .map(|i| self.sample(candles, base_seed.wrapping_add(i as u64)))
            .collect()
    }
}

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

#[cfg(test)]
mod tests {
    use super::*;
    use rust_decimal::Decimal;

    fn candle_at(i: usize) -> Candle {
        let p = Decimal::from(100 + i as u64);
        Candle {
            open_time: i as i64 * 60_000,
            open: p,
            high: p,
            low: p,
            close: p,
            volume: Decimal::from(100),
            close_time: i as i64 * 60_000 + 59_999,
            quote_volume: Decimal::ZERO,
            trades: 10,
            taker_buy_volume: Decimal::from(50),
            taker_sell_volume: Decimal::from(50),
        }
    }

    #[test]
    fn sample_within_bounds() {
        let candles: Vec<_> = (0..1000).map(candle_at).collect();
        let sampler = EpisodeSampler::new(50, 200);

        for seed in 0..100 {
            let window = sampler.sample(&candles, seed);
            assert!(window.len() >= 50);
            assert!(window.len() <= 200);
        }
    }

    #[test]
    fn sample_deterministic() {
        let candles: Vec<_> = (0..1000).map(candle_at).collect();
        let sampler = EpisodeSampler::new(50, 200);

        let a = sampler.sample(&candles, 42);
        let b = sampler.sample(&candles, 42);
        assert_eq!(a.len(), b.len());
        assert_eq!(a[0].open_time, b[0].open_time);
    }

    #[test]
    fn sample_batch_produces_diverse_windows() {
        let candles: Vec<_> = (0..1000).map(candle_at).collect();
        let sampler = EpisodeSampler::new(50, 200);

        let batch = sampler.sample_batch(&candles, 10, 42);
        assert_eq!(batch.len(), 10);

        // Different seeds → different start times (with high probability)
        let starts: Vec<_> = batch.iter().map(|w| w[0].open_time).collect();
        let unique: std::collections::HashSet<_> = starts.iter().collect();
        assert!(unique.len() > 1, "batch should have diverse start times");
    }

    #[test]
    fn small_dataset_returns_all() {
        let candles: Vec<_> = (0..5).map(candle_at).collect();
        let sampler = EpisodeSampler::new(50, 200);

        let window = sampler.sample(&candles, 42);
        assert_eq!(window.len(), 5);
    }

    #[test]
    fn min_length_clamped_to_ten() {
        // min_length of 1 should be clamped to 10
        let sampler = EpisodeSampler::new(1, 200);
        let candles: Vec<_> = (0..100).map(candle_at).collect();
        for seed in 0..50 {
            let window = sampler.sample(&candles, seed);
            assert!(window.len() >= 10, "min_length should be clamped to 10");
        }
    }

    #[test]
    fn min_equals_max_length() {
        let sampler = EpisodeSampler::new(50, 50);
        let candles: Vec<_> = (0..1000).map(candle_at).collect();
        for seed in 0..20 {
            let window = sampler.sample(&candles, seed);
            assert_eq!(window.len(), 50, "fixed length should always be 50");
        }
    }

    #[test]
    fn sample_batch_correct_count() {
        let candles: Vec<_> = (0..1000).map(candle_at).collect();
        let sampler = EpisodeSampler::new(50, 200);
        let batch = sampler.sample_batch(&candles, 5, 42);
        assert_eq!(batch.len(), 5);
    }

    #[test]
    fn sample_batch_zero_episodes() {
        let candles: Vec<_> = (0..100).map(candle_at).collect();
        let sampler = EpisodeSampler::new(50, 200);
        let batch = sampler.sample_batch(&candles, 0, 42);
        assert!(batch.is_empty());
    }
}
