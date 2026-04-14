use crate::gate::{evaluate_gate, GateResult};
use crate::seqlock::SeqLock;
use crate::spsc::RingBuffer;
use crate::types::{CorePosition, EngineParams, SymbolBuf, TradeEvent, TradeEventKind};
use rust_decimal::Decimal;
use ssm_core::Side;

// ---------------------------------------------------------------------------
// CoreEngine
// ---------------------------------------------------------------------------

/// Per-core execution engine. Owns exactly one position for one symbol.
///
/// No `HashMap`, no `String`, no allocation, no async, no mutex.
/// All methods are synchronous and allocation-free after construction.
///
/// # Architecture
///
/// Each `CoreEngine` runs on a dedicated thread (or is called from a
/// single-threaded context). It reads strategy parameters from a shared
/// `SeqLock` and pushes trade events through a lock-free `RingBuffer`
/// to the controller.
pub struct CoreEngine {
    position: CorePosition,
    cached_params: EngineParams,
    last_param_seq: u64,
    symbol: SymbolBuf,
}

impl CoreEngine {
    /// Create a new engine for the given symbol with no open position.
    pub fn new(symbol: SymbolBuf, initial_params: EngineParams) -> Self {
        Self {
            position: CorePosition::empty(symbol),
            cached_params: initial_params,
            last_param_seq: 0,
            symbol,
        }
    }

    /// Hot-path tick handler: update cached parameters and mark-to-market.
    ///
    /// Called on every price tick. The seqlock read is ~1ns on cache hit
    /// (99.6% of ticks when params change every ~256 ticks).
    #[inline]
    pub fn on_tick(
        &mut self,
        price: Decimal,
        seqlock: &SeqLock<EngineParams>,
        _ring: &RingBuffer<TradeEvent>,
    ) {
        // 1. Check if parameters changed (~1ns on cache hit)
        seqlock.read_if_changed(&mut self.last_param_seq, &mut self.cached_params);

        // 2. Update unrealized PnL
        if self.position.is_open() {
            self.mark_to_market(price);
        }
    }

    /// Signal handler: evaluate gate, apply fill if allowed, push event.
    ///
    /// Returns `GateResult::Open` if the trade was executed,
    /// `GateResult::Blocked` if rejected by risk limits.
    ///
    /// Gate evaluation is only applied for position-increasing orders.
    /// Reducing or closing an existing position always passes the gate
    /// because it decreases risk exposure.
    pub fn on_signal(
        &mut self,
        side: Side,
        quantity: Decimal,
        price: Decimal,
        timestamp: i64,
        ring: &RingBuffer<TradeEvent>,
    ) -> GateResult {
        // Position-reducing orders bypass the gate (they decrease risk)
        let is_reducing = self.position.is_open() && self.position.side != side;

        let gate = if is_reducing {
            GateResult::Open
        } else {
            let current_exposure = self.position.quantity * self.position.entry_price;
            evaluate_gate(
                side,
                self.cached_params.permissions,
                self.position.quantity,
                current_exposure,
                &self.cached_params,
            )
        };

        if gate.is_open() {
            let event = self.apply_fill(side, quantity, price, timestamp);
            // Push event to controller. If ring is full, we silently drop —
            // the controller will detect the gap via sequence numbers.
            let _ = ring.push(event);
        }

        gate
    }

    /// Apply a fill to the position. Returns the resulting `TradeEvent`.
    ///
    /// Logic mirrors `PositionTracker::apply_fill` from `ssm-execution`
    /// but without `HashMap`, `String` allocation, or `Vec` operations.
    pub fn apply_fill(
        &mut self,
        side: Side,
        quantity: Decimal,
        price: Decimal,
        timestamp: i64,
    ) -> TradeEvent {
        if !self.position.is_open() {
            // New position
            self.position.side = side;
            self.position.entry_price = price;
            self.position.quantity = quantity;
            self.position.unrealized_pnl = Decimal::ZERO;
            self.position.opened_at = timestamp;
            return TradeEvent {
                kind: TradeEventKind::PositionOpened,
                symbol: self.symbol,
                side,
                price,
                quantity,
                realized_pnl: Decimal::ZERO,
                timestamp,
            };
        }

        if self.position.side == side {
            // Adding to position — weighted average entry
            let total_cost = self.position.entry_price * self.position.quantity + price * quantity;
            self.position.quantity += quantity;
            if self.position.quantity > Decimal::ZERO {
                self.position.entry_price = total_cost / self.position.quantity;
            }
            return TradeEvent {
                kind: TradeEventKind::PositionIncreased,
                symbol: self.symbol,
                side,
                price,
                quantity,
                realized_pnl: Decimal::ZERO,
                timestamp,
            };
        }

        // Reducing/closing/flipping position
        let pnl_per_unit = match self.position.side {
            Side::Buy => price - self.position.entry_price,
            Side::Sell => self.position.entry_price - price,
        };

        let close_qty = quantity.min(self.position.quantity);
        let realized = pnl_per_unit * close_qty;
        self.position.realized_pnl += realized;
        self.position.quantity -= close_qty;

        let remainder = quantity - close_qty;

        if self.position.quantity <= Decimal::ZERO && remainder > Decimal::ZERO {
            // Position flip: close old, open new on opposite side
            self.position.side = side;
            self.position.entry_price = price;
            self.position.quantity = remainder;
            self.position.unrealized_pnl = Decimal::ZERO;
            self.position.opened_at = timestamp;
            TradeEvent {
                kind: TradeEventKind::PositionClosed,
                symbol: self.symbol,
                side,
                price,
                quantity: close_qty,
                realized_pnl: realized,
                timestamp,
            }
        } else if self.position.quantity <= Decimal::ZERO {
            // Full close
            self.position.reset();
            TradeEvent {
                kind: TradeEventKind::PositionClosed,
                symbol: self.symbol,
                side,
                price,
                quantity: close_qty,
                realized_pnl: realized,
                timestamp,
            }
        } else {
            // Partial close
            TradeEvent {
                kind: TradeEventKind::PositionReduced,
                symbol: self.symbol,
                side,
                price,
                quantity: close_qty,
                realized_pnl: realized,
                timestamp,
            }
        }
    }

    /// Update unrealized pnl based on current market price.
    #[inline]
    pub fn mark_to_market(&mut self, price: Decimal) {
        if !self.position.is_open() {
            return;
        }
        self.position.unrealized_pnl = match self.position.side {
            Side::Buy => (price - self.position.entry_price) * self.position.quantity,
            Side::Sell => (self.position.entry_price - price) * self.position.quantity,
        };
    }

    /// Read-only access to the current position.
    pub fn position(&self) -> &CorePosition {
        &self.position
    }

    /// Returns true if a position is open.
    pub fn has_position(&self) -> bool {
        self.position.is_open()
    }

    /// The symbol this core is assigned to.
    pub fn symbol(&self) -> &SymbolBuf {
        &self.symbol
    }

    /// Current cached engine parameters.
    pub fn cached_params(&self) -> &EngineParams {
        &self.cached_params
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::PermissionFlags;

    fn sym() -> SymbolBuf {
        SymbolBuf::new("BTCUSDT").unwrap()
    }

    fn params() -> EngineParams {
        EngineParams::default()
    }

    fn ring() -> RingBuffer<TradeEvent> {
        RingBuffer::new(64)
    }

    #[test]
    fn new_engine_has_no_position() {
        let engine = CoreEngine::new(sym(), params());
        assert!(!engine.has_position());
        assert_eq!(engine.position().quantity, Decimal::ZERO);
    }

    #[test]
    fn signal_buy_opens_position() {
        let mut engine = CoreEngine::new(sym(), params());
        let r = ring();
        let result = engine.on_signal(Side::Buy, Decimal::from(1), Decimal::from(50000), 1000, &r);
        assert_eq!(result, GateResult::Open);
        assert!(engine.has_position());
        assert_eq!(engine.position().side, Side::Buy);
        assert_eq!(engine.position().quantity, Decimal::from(1));
        assert_eq!(engine.position().entry_price, Decimal::from(50000));
    }

    #[test]
    fn signal_sell_opens_short() {
        let mut engine = CoreEngine::new(sym(), params());
        let r = ring();
        let result = engine.on_signal(Side::Sell, Decimal::from(2), Decimal::from(50000), 1000, &r);
        assert_eq!(result, GateResult::Open);
        assert_eq!(engine.position().side, Side::Sell);
        assert_eq!(engine.position().quantity, Decimal::from(2));
    }

    #[test]
    fn signal_blocked_by_permission() {
        let mut p = params();
        p.permissions = PermissionFlags::SELL_ALLOWED; // no BUY
        let mut engine = CoreEngine::new(sym(), p);
        let r = ring();
        let result = engine.on_signal(Side::Buy, Decimal::from(1), Decimal::from(50000), 1000, &r);
        assert_eq!(result, GateResult::Blocked);
        assert!(!engine.has_position());
    }

    #[test]
    fn signal_blocked_by_circuit_breaker() {
        let mut p = params();
        p.circuit_breaker = true;
        let mut engine = CoreEngine::new(sym(), p);
        let r = ring();
        let result = engine.on_signal(Side::Buy, Decimal::from(1), Decimal::from(50000), 1000, &r);
        assert_eq!(result, GateResult::Blocked);
    }

    #[test]
    fn signal_blocked_by_max_position() {
        let mut p = params();
        p.max_position_size = Decimal::from(5);
        let mut engine = CoreEngine::new(sym(), p);
        let r = ring();
        // Open position of 5
        engine.on_signal(Side::Buy, Decimal::from(5), Decimal::from(50000), 1000, &r);
        // Try to add more — blocked because quantity (5) is not < max (5)
        let result = engine.on_signal(Side::Buy, Decimal::from(1), Decimal::from(50000), 2000, &r);
        assert_eq!(result, GateResult::Blocked);
    }

    #[test]
    fn add_to_position_averages_entry() {
        let mut engine = CoreEngine::new(sym(), params());
        let r = ring();
        engine.on_signal(Side::Buy, Decimal::from(1), Decimal::from(100), 1000, &r);
        engine.on_signal(Side::Buy, Decimal::from(1), Decimal::from(200), 2000, &r);
        assert_eq!(engine.position().quantity, Decimal::from(2));
        assert_eq!(engine.position().entry_price, Decimal::from(150));
    }

    #[test]
    fn close_position_calculates_pnl() {
        let mut engine = CoreEngine::new(sym(), params());
        let r = ring();
        engine.on_signal(Side::Buy, Decimal::from(2), Decimal::from(50000), 1000, &r);
        engine.on_signal(Side::Sell, Decimal::from(2), Decimal::from(51000), 2000, &r);
        // Position closed, realized PnL = (51000-50000)*2 = 2000
        assert!(!engine.has_position());
        assert_eq!(engine.position().realized_pnl, Decimal::from(2000));
    }

    #[test]
    fn partial_close() {
        let mut engine = CoreEngine::new(sym(), params());
        let r = ring();
        engine.on_signal(Side::Buy, Decimal::from(10), Decimal::from(50000), 1000, &r);
        engine.on_signal(Side::Sell, Decimal::from(3), Decimal::from(51000), 2000, &r);
        assert!(engine.has_position());
        assert_eq!(engine.position().quantity, Decimal::from(7));
        // PnL: (51000-50000)*3 = 3000
        assert_eq!(engine.position().realized_pnl, Decimal::from(3000));
    }

    #[test]
    fn position_flip_long_to_short() {
        let mut engine = CoreEngine::new(sym(), params());
        let r = ring();
        engine.on_signal(Side::Buy, Decimal::from(2), Decimal::from(50000), 1000, &r);
        engine.on_signal(Side::Sell, Decimal::from(5), Decimal::from(52000), 2000, &r);
        assert!(engine.has_position());
        assert_eq!(engine.position().side, Side::Sell);
        assert_eq!(engine.position().quantity, Decimal::from(3));
        assert_eq!(engine.position().entry_price, Decimal::from(52000));
    }

    #[test]
    fn position_flip_short_to_long() {
        let mut engine = CoreEngine::new(sym(), params());
        let r = ring();
        engine.on_signal(Side::Sell, Decimal::from(2), Decimal::from(50000), 1000, &r);
        engine.on_signal(Side::Buy, Decimal::from(5), Decimal::from(48000), 2000, &r);
        assert_eq!(engine.position().side, Side::Buy);
        assert_eq!(engine.position().quantity, Decimal::from(3));
        assert_eq!(engine.position().entry_price, Decimal::from(48000));
    }

    #[test]
    fn position_flip_preserves_realized_pnl() {
        let mut engine = CoreEngine::new(sym(), params());
        let r = ring();
        engine.on_signal(Side::Buy, Decimal::from(2), Decimal::from(50000), 1000, &r);
        engine.on_signal(Side::Sell, Decimal::from(3), Decimal::from(52000), 2000, &r);
        // Realized from closing long: (52000-50000)*2 = 4000
        assert_eq!(engine.position().realized_pnl, Decimal::from(4000));
    }

    #[test]
    fn mark_to_market_long() {
        let mut engine = CoreEngine::new(sym(), params());
        let r = ring();
        engine.on_signal(Side::Buy, Decimal::from(1), Decimal::from(50000), 1000, &r);
        engine.mark_to_market(Decimal::from(52000));
        assert_eq!(engine.position().unrealized_pnl, Decimal::from(2000));
    }

    #[test]
    fn mark_to_market_short() {
        let mut engine = CoreEngine::new(sym(), params());
        let r = ring();
        engine.on_signal(Side::Sell, Decimal::from(1), Decimal::from(50000), 1000, &r);
        engine.mark_to_market(Decimal::from(48000));
        assert_eq!(engine.position().unrealized_pnl, Decimal::from(2000));
    }

    #[test]
    fn mark_to_market_no_position() {
        let mut engine = CoreEngine::new(sym(), params());
        engine.mark_to_market(Decimal::from(50000));
        assert_eq!(engine.position().unrealized_pnl, Decimal::ZERO);
    }

    #[test]
    fn trade_event_pushed_on_open() {
        let mut engine = CoreEngine::new(sym(), params());
        let r = ring();
        engine.on_signal(Side::Buy, Decimal::from(1), Decimal::from(50000), 1000, &r);
        let event = r.pop().unwrap();
        assert_eq!(event.kind, TradeEventKind::PositionOpened);
        assert_eq!(event.price, Decimal::from(50000));
    }

    #[test]
    fn trade_event_pushed_on_close() {
        let mut engine = CoreEngine::new(sym(), params());
        let r = ring();
        engine.on_signal(Side::Buy, Decimal::from(1), Decimal::from(50000), 1000, &r);
        r.pop(); // consume open event
        engine.on_signal(Side::Sell, Decimal::from(1), Decimal::from(51000), 2000, &r);
        let event = r.pop().unwrap();
        assert_eq!(event.kind, TradeEventKind::PositionClosed);
        assert_eq!(event.realized_pnl, Decimal::from(1000));
    }

    #[test]
    fn trade_event_pushed_on_partial_close() {
        let mut engine = CoreEngine::new(sym(), params());
        let r = ring();
        engine.on_signal(Side::Buy, Decimal::from(10), Decimal::from(50000), 1000, &r);
        r.pop();
        engine.on_signal(Side::Sell, Decimal::from(3), Decimal::from(51000), 2000, &r);
        let event = r.pop().unwrap();
        assert_eq!(event.kind, TradeEventKind::PositionReduced);
    }

    #[test]
    fn trade_event_pushed_on_increase() {
        let mut engine = CoreEngine::new(sym(), params());
        let r = ring();
        engine.on_signal(Side::Buy, Decimal::from(1), Decimal::from(50000), 1000, &r);
        r.pop();
        engine.on_signal(Side::Buy, Decimal::from(1), Decimal::from(51000), 2000, &r);
        let event = r.pop().unwrap();
        assert_eq!(event.kind, TradeEventKind::PositionIncreased);
    }

    #[test]
    fn gate_rejection_no_event_pushed() {
        let mut p = params();
        p.circuit_breaker = true;
        let mut engine = CoreEngine::new(sym(), p);
        let r = ring();
        engine.on_signal(Side::Buy, Decimal::from(1), Decimal::from(50000), 1000, &r);
        assert!(r.is_empty());
    }

    #[test]
    fn on_tick_updates_unrealized_pnl() {
        let mut engine = CoreEngine::new(sym(), params());
        let seqlock = SeqLock::new(params());
        let r = ring();
        engine.on_signal(Side::Buy, Decimal::from(1), Decimal::from(50000), 1000, &r);
        engine.on_tick(Decimal::from(55000), &seqlock, &r);
        assert_eq!(engine.position().unrealized_pnl, Decimal::from(5000));
    }

    #[test]
    fn on_tick_reads_params_from_seqlock() {
        let mut engine = CoreEngine::new(sym(), params());
        let seqlock = SeqLock::new(params());
        let r = ring();

        // Write new params with circuit breaker
        let mut new_params = params();
        new_params.circuit_breaker = true;
        seqlock.write(new_params);

        // Tick should pick up the new params
        engine.on_tick(Decimal::from(50000), &seqlock, &r);
        assert!(engine.cached_params().circuit_breaker);
    }

    #[test]
    fn multiple_fills_accumulate_realized_pnl() {
        let mut engine = CoreEngine::new(sym(), params());
        let r = ring();
        engine.on_signal(Side::Buy, Decimal::from(10), Decimal::from(50000), 1000, &r);
        // Close 3 at 51000
        engine.on_signal(Side::Sell, Decimal::from(3), Decimal::from(51000), 2000, &r);
        // Close 4 at 52000
        engine.on_signal(Side::Sell, Decimal::from(4), Decimal::from(52000), 3000, &r);
        // PnL: (51000-50000)*3 + (52000-50000)*4 = 3000 + 8000 = 11000
        assert_eq!(engine.position().realized_pnl, Decimal::from(11000));
        assert_eq!(engine.position().quantity, Decimal::from(3));
    }

    #[test]
    fn short_position_pnl() {
        let mut engine = CoreEngine::new(sym(), params());
        let r = ring();
        engine.on_signal(Side::Sell, Decimal::from(2), Decimal::from(100), 1000, &r);
        // Close at 90 — profit for short
        engine.on_signal(Side::Buy, Decimal::from(2), Decimal::from(90), 2000, &r);
        // PnL: (100-90)*2 = 20
        assert_eq!(engine.position().realized_pnl, Decimal::from(20));
    }

    #[test]
    fn entry_price_weighted_average_unequal() {
        let mut engine = CoreEngine::new(sym(), params());
        let r = ring();
        engine.on_signal(Side::Buy, Decimal::from(1), Decimal::from(100), 1000, &r);
        engine.on_signal(Side::Buy, Decimal::from(3), Decimal::from(200), 2000, &r);
        // Weighted: (100*1 + 200*3) / 4 = 700/4 = 175
        assert_eq!(engine.position().entry_price, Decimal::from(175));
        assert_eq!(engine.position().quantity, Decimal::from(4));
    }

    #[test]
    fn symbol_accessor() {
        let engine = CoreEngine::new(sym(), params());
        assert_eq!(engine.symbol().as_str(), "BTCUSDT");
    }
}
