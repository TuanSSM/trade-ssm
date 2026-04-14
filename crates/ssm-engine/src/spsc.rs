use std::cell::UnsafeCell;
use std::mem::MaybeUninit;
use std::sync::atomic::{AtomicUsize, Ordering};

// ---------------------------------------------------------------------------
// Cache-line padding
// ---------------------------------------------------------------------------

/// Pads the inner value to a full cache line (64 bytes) to prevent
/// false sharing between producer and consumer indices.
#[repr(align(64))]
struct CachePadded<T> {
    value: T,
}

impl<T> CachePadded<T> {
    fn new(value: T) -> Self {
        Self { value }
    }
}

// ---------------------------------------------------------------------------
// RingBuffer
// ---------------------------------------------------------------------------

/// Bounded lock-free single-producer single-consumer ring buffer.
///
/// # Safety contract
///
/// - Exactly one thread may call `push` (the producer).
/// - Exactly one thread may call `pop` (the consumer).
/// - `len`, `is_empty`, and `capacity` are safe to call from any thread.
///
/// Violating the single-producer or single-consumer invariant is undefined
/// behavior. The type does **not** enforce this at compile time — it is the
/// caller's responsibility.
pub struct RingBuffer<T> {
    head: CachePadded<AtomicUsize>,
    tail: CachePadded<AtomicUsize>,
    buffer: Box<[UnsafeCell<MaybeUninit<T>>]>,
    mask: usize,
}

// SAFETY: The ring buffer is designed for single-producer, single-consumer
// across threads. The atomic head/tail provide the necessary synchronization.
// The UnsafeCell<MaybeUninit<T>> slots are only accessed by one side at a time:
// the producer writes to slots between tail..head, the consumer reads from
// slots between head..tail (modulo capacity).
unsafe impl<T: Send> Send for RingBuffer<T> {}
unsafe impl<T: Send> Sync for RingBuffer<T> {}

impl<T> RingBuffer<T> {
    /// Create a new ring buffer. Capacity is rounded up to the next power of two.
    ///
    /// # Panics
    ///
    /// Panics if `capacity` is 0.
    pub fn new(capacity: usize) -> Self {
        assert!(capacity > 0, "RingBuffer capacity must be > 0");
        let capacity = capacity.next_power_of_two();
        let buffer: Vec<UnsafeCell<MaybeUninit<T>>> = (0..capacity)
            .map(|_| UnsafeCell::new(MaybeUninit::uninit()))
            .collect();
        Self {
            head: CachePadded::new(AtomicUsize::new(0)),
            tail: CachePadded::new(AtomicUsize::new(0)),
            buffer: buffer.into_boxed_slice(),
            mask: capacity - 1,
        }
    }

    /// Push a value into the ring buffer (producer-only).
    ///
    /// Returns `Ok(())` on success, or `Err(value)` if the buffer is full.
    ///
    /// # Safety invariant
    ///
    /// Only one thread may call this method.
    pub fn push(&self, value: T) -> Result<(), T> {
        let head = self.head.value.load(Ordering::Relaxed);
        let tail = self.tail.value.load(Ordering::Acquire);

        if head.wrapping_sub(tail) >= self.buffer.len() {
            return Err(value);
        }

        let slot = head & self.mask;
        // SAFETY: We are the sole producer. The slot at `head` is not currently
        // being read by the consumer because `head - tail < capacity` (checked above)
        // and the consumer only reads up to `head` (loaded with Acquire).
        unsafe {
            (*self.buffer[slot].get()).write(value);
        }

        self.head
            .value
            .store(head.wrapping_add(1), Ordering::Release);
        Ok(())
    }

    /// Pop a value from the ring buffer (consumer-only).
    ///
    /// Returns `Some(value)` if available, or `None` if empty.
    ///
    /// # Safety invariant
    ///
    /// Only one thread may call this method.
    pub fn pop(&self) -> Option<T> {
        let tail = self.tail.value.load(Ordering::Relaxed);
        let head = self.head.value.load(Ordering::Acquire);

        if tail == head {
            return None;
        }

        let slot = tail & self.mask;
        // SAFETY: We are the sole consumer. The slot at `tail` was written by
        // the producer (head > tail, guaranteed by the Acquire load above).
        // The producer will not overwrite this slot until we advance `tail`
        // and `head` wraps around, which requires the buffer to be fully
        // drained first.
        let value = unsafe { (*self.buffer[slot].get()).assume_init_read() };

        self.tail
            .value
            .store(tail.wrapping_add(1), Ordering::Release);
        Some(value)
    }

    /// Approximate number of items in the buffer.
    ///
    /// Safe to call from any thread, but the value may be stale.
    pub fn len(&self) -> usize {
        let head = self.head.value.load(Ordering::Relaxed);
        let tail = self.tail.value.load(Ordering::Relaxed);
        head.wrapping_sub(tail)
    }

    /// Returns true if the buffer appears empty.
    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    /// The actual capacity (always a power of two).
    pub fn capacity(&self) -> usize {
        self.buffer.len()
    }
}

impl<T> Drop for RingBuffer<T> {
    fn drop(&mut self) {
        // Drop any remaining items in the buffer.
        while self.pop().is_some() {}
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn push_pop_single_item() {
        let ring = RingBuffer::new(4);
        assert!(ring.push(42u32).is_ok());
        assert_eq!(ring.pop(), Some(42));
        assert_eq!(ring.pop(), None);
    }

    #[test]
    fn push_pop_multiple() {
        let ring = RingBuffer::new(4);
        ring.push(1u32).unwrap();
        ring.push(2).unwrap();
        ring.push(3).unwrap();
        assert_eq!(ring.pop(), Some(1));
        assert_eq!(ring.pop(), Some(2));
        assert_eq!(ring.pop(), Some(3));
        assert_eq!(ring.pop(), None);
    }

    #[test]
    fn push_full_returns_err() {
        let ring = RingBuffer::new(2); // rounds to 2
        ring.push(1u32).unwrap();
        ring.push(2).unwrap();
        assert_eq!(ring.push(3), Err(3));
    }

    #[test]
    fn pop_empty_returns_none() {
        let ring = RingBuffer::<u32>::new(4);
        assert_eq!(ring.pop(), None);
    }

    #[test]
    fn capacity_rounds_to_power_of_two() {
        assert_eq!(RingBuffer::<u32>::new(3).capacity(), 4);
        assert_eq!(RingBuffer::<u32>::new(5).capacity(), 8);
        assert_eq!(RingBuffer::<u32>::new(8).capacity(), 8);
        assert_eq!(RingBuffer::<u32>::new(1).capacity(), 1);
    }

    #[test]
    fn wraparound_produces_correct_order() {
        let ring = RingBuffer::new(4);
        // Fill and drain several times to force wraparound
        for round in 0u32..10 {
            let base = round * 4;
            ring.push(base).unwrap();
            ring.push(base + 1).unwrap();
            ring.push(base + 2).unwrap();
            ring.push(base + 3).unwrap();
            assert_eq!(ring.pop(), Some(base));
            assert_eq!(ring.pop(), Some(base + 1));
            assert_eq!(ring.pop(), Some(base + 2));
            assert_eq!(ring.pop(), Some(base + 3));
        }
    }

    #[test]
    fn fill_and_drain() {
        let ring = RingBuffer::new(8);
        for i in 0u32..8 {
            ring.push(i).unwrap();
        }
        assert_eq!(ring.len(), 8);
        assert_eq!(ring.push(99), Err(99));

        for i in 0u32..8 {
            assert_eq!(ring.pop(), Some(i));
        }
        assert!(ring.is_empty());
    }

    #[test]
    fn interleaved_push_pop() {
        let ring = RingBuffer::new(4);
        ring.push(1u32).unwrap();
        ring.push(2).unwrap();
        assert_eq!(ring.pop(), Some(1));
        ring.push(3).unwrap();
        ring.push(4).unwrap();
        assert_eq!(ring.pop(), Some(2));
        assert_eq!(ring.pop(), Some(3));
        assert_eq!(ring.pop(), Some(4));
        assert_eq!(ring.pop(), None);
    }

    #[test]
    fn len_accuracy() {
        let ring = RingBuffer::new(8);
        assert_eq!(ring.len(), 0);
        ring.push(1u32).unwrap();
        assert_eq!(ring.len(), 1);
        ring.push(2).unwrap();
        assert_eq!(ring.len(), 2);
        ring.pop();
        assert_eq!(ring.len(), 1);
    }

    #[test]
    fn is_empty_check() {
        let ring = RingBuffer::new(4);
        assert!(ring.is_empty());
        ring.push(1u32).unwrap();
        assert!(!ring.is_empty());
        ring.pop();
        assert!(ring.is_empty());
    }

    #[test]
    #[should_panic(expected = "capacity must be > 0")]
    fn zero_capacity_panics() {
        let _ring = RingBuffer::<u32>::new(0);
    }

    #[test]
    fn push_pop_trade_event() {
        use crate::types::{SymbolBuf, TradeEvent, TradeEventKind};
        use rust_decimal::Decimal;
        use ssm_core::Side;

        let ring = RingBuffer::new(4);
        let event = TradeEvent {
            kind: TradeEventKind::PositionOpened,
            symbol: SymbolBuf::new("BTCUSDT").unwrap(),
            side: Side::Buy,
            price: Decimal::from(50000),
            quantity: Decimal::from(1),
            realized_pnl: Decimal::ZERO,
            timestamp: 1_234_567_890,
        };
        ring.push(event).unwrap();
        let popped = ring.pop().unwrap();
        assert_eq!(popped.kind, TradeEventKind::PositionOpened);
        assert_eq!(popped.price, Decimal::from(50000));
    }

    #[test]
    fn cross_thread_fifo_order() {
        use std::sync::Arc;

        let ring = Arc::new(RingBuffer::new(1024));
        let ring_producer = Arc::clone(&ring);
        let ring_consumer = Arc::clone(&ring);
        let count = 10_000u32;

        let producer = std::thread::spawn(move || {
            for i in 0..count {
                while ring_producer.push(i).is_err() {
                    std::hint::spin_loop();
                }
            }
        });

        let consumer = std::thread::spawn(move || {
            let mut received = Vec::with_capacity(count as usize);
            while received.len() < count as usize {
                if let Some(val) = ring_consumer.pop() {
                    received.push(val);
                } else {
                    std::hint::spin_loop();
                }
            }
            received
        });

        producer.join().unwrap();
        let received = consumer.join().unwrap();
        let expected: Vec<u32> = (0..count).collect();
        assert_eq!(received, expected);
    }

    #[test]
    fn drop_cleans_up_remaining_items() {
        use std::sync::atomic::{AtomicUsize, Ordering};
        use std::sync::Arc;

        #[derive(Debug)]
        struct DropCounter(Arc<AtomicUsize>);
        impl Drop for DropCounter {
            fn drop(&mut self) {
                self.0.fetch_add(1, Ordering::Relaxed);
            }
        }

        let drop_count = Arc::new(AtomicUsize::new(0));

        {
            let ring = RingBuffer::new(4);
            ring.push(DropCounter(Arc::clone(&drop_count))).unwrap();
            ring.push(DropCounter(Arc::clone(&drop_count))).unwrap();
            ring.push(DropCounter(Arc::clone(&drop_count))).unwrap();
            // Drop ring with 3 items still in it
        }
        assert_eq!(drop_count.load(Ordering::Relaxed), 3);
    }
}
