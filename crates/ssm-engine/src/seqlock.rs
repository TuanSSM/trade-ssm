use std::cell::UnsafeCell;
use std::sync::atomic::{fence, AtomicU64, Ordering};

// ---------------------------------------------------------------------------
// SeqLock
// ---------------------------------------------------------------------------

/// Single-writer, multi-reader lock-free parameter cache.
///
/// The writer publishes new data by incrementing a sequence number (odd during
/// write, even when stable). Readers detect torn reads by comparing the
/// sequence before and after reading.
///
/// The key optimization is `read_if_changed`: the reader caches the last seen
/// sequence number and only performs the full memcpy when the sequence changes.
/// On a typical trading workload, parameters change every ~256 ticks, giving
/// a ~99.6% cache hit rate where each check costs ~1ns (single atomic load).
///
/// # Safety contract
///
/// - Exactly one thread may call `write` (the writer).
/// - Any number of threads may call `read` or `read_if_changed`.
pub struct SeqLock<T: Copy> {
    sequence: AtomicU64,
    data: UnsafeCell<T>,
}

// SAFETY: SeqLock is designed for single-writer, multi-reader across threads.
// The sequence number provides the synchronization guarantee. Readers never
// see a torn write because they validate the sequence before and after reading.
// T: Copy ensures no drop logic is needed for partial reads.
unsafe impl<T: Copy + Send> Send for SeqLock<T> {}
unsafe impl<T: Copy + Send + Sync> Sync for SeqLock<T> {}

impl<T: Copy> SeqLock<T> {
    /// Create a new `SeqLock` with an initial value.
    /// Sequence starts at 0 (even = stable).
    pub fn new(initial: T) -> Self {
        Self {
            sequence: AtomicU64::new(0),
            data: UnsafeCell::new(initial),
        }
    }

    /// Write a new value (writer-only).
    ///
    /// Increments the sequence to odd (writing), stores the data, then
    /// increments to even (stable). Uses release ordering to ensure the
    /// data write is visible before the final sequence update.
    ///
    /// # Safety invariant
    ///
    /// Only one thread may call this method.
    pub fn write(&self, value: T) {
        let seq = self.sequence.load(Ordering::Relaxed);
        // Set sequence to odd (write in progress)
        self.sequence.store(seq.wrapping_add(1), Ordering::Relaxed);
        fence(Ordering::Release);

        // SAFETY: We are the sole writer. No other thread writes to data.
        // Readers that see the odd sequence will retry, so a torn read is
        // harmless (it will be detected and discarded).
        unsafe {
            self.data.get().write(value);
        }

        fence(Ordering::Release);
        // Set sequence to even (write complete)
        self.sequence.store(seq.wrapping_add(2), Ordering::Release);
    }

    /// Read the current value (may spin if a write is in progress).
    ///
    /// Spins until a consistent read is obtained (sequence is even and
    /// unchanged across the read).
    pub fn read(&self) -> T {
        loop {
            let seq1 = self.sequence.load(Ordering::Acquire);
            if seq1 & 1 != 0 {
                // Write in progress, spin
                std::hint::spin_loop();
                continue;
            }

            // SAFETY: The sequence is even, so no write is in progress.
            // We read the data and verify the sequence hasn't changed.
            let value = unsafe { *self.data.get() };

            fence(Ordering::Acquire);
            let seq2 = self.sequence.load(Ordering::Relaxed);

            if seq1 == seq2 {
                return value;
            }
            // Sequence changed during read — retry
            std::hint::spin_loop();
        }
    }

    /// Fast-path read: only copies data when the sequence has changed.
    ///
    /// The caller maintains `last_seq` and `cached` across calls.
    /// On each call:
    /// - If the sequence hasn't changed since `last_seq`, returns `false`
    ///   immediately (~1ns — single atomic load).
    /// - If the sequence has changed, reads the new value into `cached`,
    ///   updates `last_seq`, and returns `true`.
    ///
    /// Typical cache hit rate: ~99.6% when parameters change every ~256 ticks.
    pub fn read_if_changed(&self, last_seq: &mut u64, cached: &mut T) -> bool {
        let current_seq = self.sequence.load(Ordering::Acquire);

        if *last_seq == current_seq {
            return false;
        }

        // Sequence changed — do a full consistent read
        let value = self.read();
        *cached = value;
        *last_seq = self.sequence.load(Ordering::Relaxed);
        true
    }

    /// Current sequence number. Even = stable, odd = write in progress.
    pub fn sequence(&self) -> u64 {
        self.sequence.load(Ordering::Relaxed)
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::{EngineParams, PermissionFlags};
    use rust_decimal::Decimal;

    #[test]
    fn initial_read() {
        let lock = SeqLock::new(42u64);
        assert_eq!(lock.read(), 42);
    }

    #[test]
    fn write_then_read() {
        let lock = SeqLock::new(0u64);
        lock.write(99);
        assert_eq!(lock.read(), 99);
    }

    #[test]
    fn multiple_writes_reads() {
        let lock = SeqLock::new(0u32);
        for i in 0u32..100 {
            lock.write(i);
            assert_eq!(lock.read(), i);
        }
    }

    #[test]
    fn read_if_changed_cache_hit() {
        let lock = SeqLock::new(42u64);
        let mut last_seq = 0u64;
        let mut cached = 0u64;

        // First read should detect change (seq 0 != initial last_seq... actually both 0)
        // Sequence starts at 0 and no write happened, so last_seq 0 == current 0
        assert!(!lock.read_if_changed(&mut last_seq, &mut cached));

        // Write something
        lock.write(42);
        // Now seq changed (0 -> 2), should detect
        assert!(lock.read_if_changed(&mut last_seq, &mut cached));
        assert_eq!(cached, 42);

        // Second call with same seq should be cache hit
        assert!(!lock.read_if_changed(&mut last_seq, &mut cached));
    }

    #[test]
    fn read_if_changed_detects_change() {
        let lock = SeqLock::new(0u64);
        let mut last_seq = 0u64;
        let mut cached = 0u64;

        lock.write(10);
        assert!(lock.read_if_changed(&mut last_seq, &mut cached));
        assert_eq!(cached, 10);

        lock.write(20);
        assert!(lock.read_if_changed(&mut last_seq, &mut cached));
        assert_eq!(cached, 20);

        lock.write(30);
        assert!(lock.read_if_changed(&mut last_seq, &mut cached));
        assert_eq!(cached, 30);
    }

    #[test]
    fn read_if_changed_multiple_writes_between_checks() {
        let lock = SeqLock::new(0u64);
        let mut last_seq = 0u64;
        let mut cached = 0u64;

        lock.write(1);
        lock.write(2);
        lock.write(3);

        // Should read the latest value
        assert!(lock.read_if_changed(&mut last_seq, &mut cached));
        assert_eq!(cached, 3);
    }

    #[test]
    fn sequence_increments_by_two_per_write() {
        let lock = SeqLock::new(0u32);
        assert_eq!(lock.sequence(), 0);
        lock.write(1);
        assert_eq!(lock.sequence(), 2);
        lock.write(2);
        assert_eq!(lock.sequence(), 4);
        lock.write(3);
        assert_eq!(lock.sequence(), 6);
    }

    #[test]
    fn with_engine_params() {
        let params = EngineParams::default();
        let lock = SeqLock::new(params);

        let mut new_params = params;
        new_params.permissions = PermissionFlags::NONE;
        new_params.circuit_breaker = true;
        new_params.max_position_size = Decimal::from(5);

        lock.write(new_params);
        let read_params = lock.read();
        assert_eq!(read_params.permissions, PermissionFlags::NONE);
        assert!(read_params.circuit_breaker);
        assert_eq!(read_params.max_position_size, Decimal::from(5));
    }

    #[test]
    fn engine_params_cache_hit_rate() {
        let params = EngineParams::default();
        let lock = SeqLock::new(params);
        let mut last_seq = 0u64;
        let mut cached = params;

        let mut hits = 0u32;
        let total = 1000u32;

        // Simulate: write every 256 reads
        for i in 0..total {
            if i % 256 == 0 && i > 0 {
                let mut p = params;
                p.max_position_size = Decimal::from(i);
                lock.write(p);
            }
            if !lock.read_if_changed(&mut last_seq, &mut cached) {
                hits += 1;
            }
        }

        // Expect high cache hit rate (>99%)
        let hit_rate = f64::from(hits) / f64::from(total);
        assert!(hit_rate > 0.99, "cache hit rate too low: {hit_rate}");
    }

    #[test]
    fn cross_thread_consistency() {
        use std::sync::Arc;

        let lock = Arc::new(SeqLock::new(0u64));
        let writer_lock = Arc::clone(&lock);
        let reader_lock = Arc::clone(&lock);
        let iterations = 100_000u64;

        let writer = std::thread::spawn(move || {
            for i in 1..=iterations {
                writer_lock.write(i);
            }
        });

        let reader = std::thread::spawn(move || {
            let mut last_val = 0u64;
            let mut reads = 0u64;
            while last_val < iterations {
                let val = reader_lock.read();
                // Values must be monotonically non-decreasing
                assert!(val >= last_val, "non-monotonic read: {val} < {last_val}");
                last_val = val;
                reads += 1;
            }
            reads
        });

        writer.join().unwrap();
        let _reads = reader.join().unwrap();
    }
}
