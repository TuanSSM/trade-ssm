/// Experience replay buffer for off-policy RL training.
///
/// Stores (state, action, reward, next_state, done) transitions
/// and supports uniform and prioritized sampling.
///
/// A single experience transition.
#[derive(Debug, Clone)]
pub struct Transition {
    pub state: Vec<f64>,
    pub action: Vec<f64>,
    pub reward: f64,
    pub next_state: Vec<f64>,
    pub done: bool,
    /// TD-error priority (for prioritized replay).
    pub priority: f64,
}

/// Uniform replay buffer with optional priority support.
pub struct ReplayBuffer {
    capacity: usize,
    buffer: Vec<Transition>,
    position: usize,
    full: bool,
}

impl ReplayBuffer {
    pub fn new(capacity: usize) -> Self {
        Self {
            capacity,
            buffer: Vec::with_capacity(capacity),
            position: 0,
            full: false,
        }
    }

    pub fn len(&self) -> usize {
        if self.full {
            self.capacity
        } else {
            self.position
        }
    }

    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    /// Push a transition into the buffer (circular).
    pub fn push(&mut self, transition: Transition) {
        if self.buffer.len() < self.capacity {
            self.buffer.push(transition);
        } else {
            self.buffer[self.position] = transition;
        }
        self.position = (self.position + 1) % self.capacity;
        if self.position == 0 && self.buffer.len() == self.capacity {
            self.full = true;
        }
    }

    /// Sample a batch uniformly at random using a simple LCG PRNG.
    pub fn sample(&self, batch_size: usize, seed: u64) -> Vec<&Transition> {
        let len = self.len();
        if len == 0 || batch_size == 0 {
            return vec![];
        }

        let mut rng = LcgRng::new(seed);
        let actual_batch = batch_size.min(len);

        (0..actual_batch)
            .map(|_| {
                let idx = (rng.next_f64() * len as f64) as usize % len;
                &self.buffer[idx]
            })
            .collect()
    }

    /// Sample prioritized: higher-priority transitions are more likely to be sampled.
    pub fn sample_prioritized(
        &self,
        batch_size: usize,
        seed: u64,
        alpha: f64,
    ) -> Vec<(usize, &Transition)> {
        let len = self.len();
        if len == 0 || batch_size == 0 {
            return vec![];
        }

        let mut rng = LcgRng::new(seed);

        // Compute priorities
        let priorities: Vec<f64> = self.buffer[..len]
            .iter()
            .map(|t| (t.priority.abs() + 1e-6).powf(alpha))
            .collect();
        let total: f64 = priorities.iter().sum();

        let actual_batch = batch_size.min(len);
        let mut result = Vec::with_capacity(actual_batch);

        for _ in 0..actual_batch {
            let threshold = rng.next_f64() * total;
            let mut cumulative = 0.0;
            for (idx, &p) in priorities.iter().enumerate() {
                cumulative += p;
                if cumulative >= threshold {
                    result.push((idx, &self.buffer[idx]));
                    break;
                }
            }
        }

        result
    }

    /// Update priority for a specific transition.
    pub fn update_priority(&mut self, index: usize, new_priority: f64) {
        if index < self.len() {
            self.buffer[index].priority = new_priority;
        }
    }
}

/// Simple LCG random number generator.
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

    fn make_transition(reward: f64) -> Transition {
        Transition {
            state: vec![1.0, 2.0],
            action: vec![0.5],
            reward,
            next_state: vec![3.0, 4.0],
            done: false,
            priority: reward.abs(),
        }
    }

    #[test]
    fn push_and_len() {
        let mut buf = ReplayBuffer::new(100);
        assert_eq!(buf.len(), 0);
        assert!(buf.is_empty());

        buf.push(make_transition(1.0));
        assert_eq!(buf.len(), 1);
        assert!(!buf.is_empty());
    }

    #[test]
    fn circular_buffer() {
        let mut buf = ReplayBuffer::new(3);
        for i in 0..5 {
            buf.push(make_transition(i as f64));
        }
        assert_eq!(buf.len(), 3);
    }

    #[test]
    fn sample_uniform() {
        let mut buf = ReplayBuffer::new(100);
        for i in 0..50 {
            buf.push(make_transition(i as f64));
        }

        let batch = buf.sample(10, 42);
        assert_eq!(batch.len(), 10);
    }

    #[test]
    fn sample_deterministic() {
        let mut buf = ReplayBuffer::new(100);
        for i in 0..50 {
            buf.push(make_transition(i as f64));
        }

        let a = buf.sample(10, 42);
        let b = buf.sample(10, 42);

        for (x, y) in a.iter().zip(b.iter()) {
            assert!((x.reward - y.reward).abs() < f64::EPSILON);
        }
    }

    #[test]
    fn sample_empty_buffer() {
        let buf = ReplayBuffer::new(100);
        let batch = buf.sample(10, 42);
        assert!(batch.is_empty());
    }

    #[test]
    fn prioritized_sampling() {
        let mut buf = ReplayBuffer::new(100);
        // Use extreme priority difference
        for _ in 0..10 {
            buf.push(make_transition(0.001)); // very low priority
        }
        buf.push(make_transition(1000.0)); // very high priority

        let samples = buf.sample_prioritized(50, 42, 2.0);
        assert!(!samples.is_empty());
        // With alpha=2.0 and 1000x priority difference, high-priority should appear
        let high_count = samples.iter().filter(|(_, t)| t.reward > 500.0).count();
        assert!(
            high_count > 0,
            "high priority item should be sampled at least once"
        );
    }

    #[test]
    fn update_priority() {
        let mut buf = ReplayBuffer::new(10);
        buf.push(make_transition(1.0));
        buf.update_priority(0, 999.0);
        assert!((buf.buffer[0].priority - 999.0).abs() < f64::EPSILON);
    }
}
