use crate::core::CoreEngine;
use crate::gate::GateResult;
use crate::seqlock::SeqLock;
use crate::spsc::RingBuffer;
use crate::types::{CorePosition, EngineParams, SymbolBuf, TradeEvent};
use rust_decimal::Decimal;
use ssm_core::Side;

/// Default SPSC ring buffer capacity per core.
const DEFAULT_RING_CAPACITY: usize = 4096;

// ---------------------------------------------------------------------------
// CoreSlot
// ---------------------------------------------------------------------------

/// A slot holding one core engine and its associated SPSC ring buffer.
pub struct CoreSlot {
    pub symbol: SymbolBuf,
    pub engine: CoreEngine,
    pub ring: RingBuffer<TradeEvent>,
}

// ---------------------------------------------------------------------------
// Controller
// ---------------------------------------------------------------------------

/// Manages multiple `CoreEngine` instances, publishes parameters via `SeqLock`,
/// and drains trade events from each core's SPSC ring.
///
/// The controller is the **cold path** — allocations and I/O are acceptable here.
/// It runs on its own thread or tokio task, separate from the execution cores.
pub struct Controller {
    cores: Vec<CoreSlot>,
    params: SeqLock<EngineParams>,
    current_params: EngineParams,
}

impl Controller {
    /// Create a new controller with the given initial parameters.
    pub fn new(params: EngineParams) -> Self {
        Self {
            cores: Vec::new(),
            params: SeqLock::new(params),
            current_params: params,
        }
    }

    /// Add a new execution core for the given symbol.
    ///
    /// Returns the core index on success, or an error if the symbol is already
    /// registered or invalid.
    pub fn add_core(&mut self, symbol: &str) -> Result<usize, &'static str> {
        let sym = SymbolBuf::new(symbol).ok_or("symbol too long (max 16 bytes)")?;

        if self.cores.iter().any(|c| c.symbol == sym) {
            return Err("symbol already registered");
        }

        let index = self.cores.len();
        self.cores.push(CoreSlot {
            symbol: sym,
            engine: CoreEngine::new(sym, self.current_params),
            ring: RingBuffer::new(DEFAULT_RING_CAPACITY),
        });
        Ok(index)
    }

    /// Update parameters for all cores. Writes to the shared `SeqLock`.
    pub fn update_params(&mut self, params: EngineParams) {
        self.current_params = params;
        self.params.write(params);
    }

    /// Activate or deactivate the circuit breaker across all cores.
    pub fn set_circuit_breaker(&mut self, active: bool) {
        self.current_params.circuit_breaker = active;
        self.params.write(self.current_params);
    }

    /// Set permission flags for all cores.
    pub fn set_permissions(&mut self, permissions: u32) {
        self.current_params.permissions = permissions;
        self.params.write(self.current_params);
    }

    /// Process a price tick for a specific symbol.
    ///
    /// Calls `on_tick` on the matching core, which updates the seqlock cache
    /// and mark-to-market.
    pub fn tick(&mut self, symbol: &SymbolBuf, price: Decimal) {
        if let Some(slot) = self.cores.iter_mut().find(|c| c.symbol == *symbol) {
            slot.engine.on_tick(price, &self.params, &slot.ring);
        }
    }

    /// Process a signal for a specific symbol.
    ///
    /// Routes to the correct core and returns the gate result.
    /// Returns `None` if the symbol is not registered.
    pub fn signal(
        &mut self,
        symbol: &SymbolBuf,
        side: Side,
        quantity: Decimal,
        price: Decimal,
        timestamp: i64,
    ) -> Option<GateResult> {
        let slot = self.cores.iter_mut().find(|c| c.symbol == *symbol)?;
        Some(
            slot.engine
                .on_signal(side, quantity, price, timestamp, &slot.ring),
        )
    }

    /// Drain all trade events from all cores into a new Vec.
    pub fn drain_events(&self) -> Vec<TradeEvent> {
        let mut events = Vec::new();
        self.drain_events_into(&mut events);
        events
    }

    /// Drain all trade events from all cores into an existing buffer.
    pub fn drain_events_into(&self, buffer: &mut Vec<TradeEvent>) {
        for slot in &self.cores {
            while let Some(event) = slot.ring.pop() {
                buffer.push(event);
            }
        }
    }

    /// Mark all positions to market with the given prices.
    pub fn mark_to_market_all(&mut self, prices: &[(SymbolBuf, Decimal)]) {
        for (symbol, price) in prices {
            if let Some(slot) = self.cores.iter_mut().find(|c| c.symbol == *symbol) {
                slot.engine.mark_to_market(*price);
            }
        }
    }

    /// Snapshot of all current positions.
    pub fn positions(&self) -> Vec<CorePosition> {
        self.cores.iter().map(|c| *c.engine.position()).collect()
    }

    /// Get a reference to a core slot by symbol.
    pub fn get_core(&self, symbol: &SymbolBuf) -> Option<&CoreSlot> {
        self.cores.iter().find(|c| c.symbol == *symbol)
    }

    /// Number of registered cores.
    pub fn core_count(&self) -> usize {
        self.cores.len()
    }

    /// Access the shared `SeqLock` (for external readers).
    pub fn params_seqlock(&self) -> &SeqLock<EngineParams> {
        &self.params
    }

    /// Current parameters as seen by the controller.
    pub fn current_params(&self) -> &EngineParams {
        &self.current_params
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::{PermissionFlags, TradeEventKind};

    fn default_params() -> EngineParams {
        EngineParams::default()
    }

    #[test]
    fn add_core() {
        let mut ctrl = Controller::new(default_params());
        let idx = ctrl.add_core("BTCUSDT").unwrap();
        assert_eq!(idx, 0);
        assert_eq!(ctrl.core_count(), 1);
    }

    #[test]
    fn add_multiple_cores() {
        let mut ctrl = Controller::new(default_params());
        assert_eq!(ctrl.add_core("BTCUSDT").unwrap(), 0);
        assert_eq!(ctrl.add_core("ETHUSDT").unwrap(), 1);
        assert_eq!(ctrl.add_core("SOLUSDT").unwrap(), 2);
        assert_eq!(ctrl.core_count(), 3);
    }

    #[test]
    fn add_duplicate_core() {
        let mut ctrl = Controller::new(default_params());
        ctrl.add_core("BTCUSDT").unwrap();
        assert_eq!(ctrl.add_core("BTCUSDT"), Err("symbol already registered"));
    }

    #[test]
    fn add_core_too_long_symbol() {
        let mut ctrl = Controller::new(default_params());
        assert_eq!(
            ctrl.add_core("VERYLONGSYMBOLNAME"),
            Err("symbol too long (max 16 bytes)")
        );
    }

    #[test]
    fn update_params_reflected_in_seqlock() {
        let mut ctrl = Controller::new(default_params());
        let mut new_params = default_params();
        new_params.max_position_size = Decimal::from(99);
        ctrl.update_params(new_params);

        let read_params = ctrl.params_seqlock().read();
        assert_eq!(read_params.max_position_size, Decimal::from(99));
    }

    #[test]
    fn signal_routes_to_correct_core() {
        let mut ctrl = Controller::new(default_params());
        ctrl.add_core("BTCUSDT").unwrap();
        ctrl.add_core("ETHUSDT").unwrap();

        let btc = SymbolBuf::new("BTCUSDT").unwrap();
        let eth = SymbolBuf::new("ETHUSDT").unwrap();

        ctrl.signal(
            &btc,
            Side::Buy,
            Decimal::from(1),
            Decimal::from(50000),
            1000,
        );
        ctrl.signal(
            &eth,
            Side::Sell,
            Decimal::from(2),
            Decimal::from(3000),
            1000,
        );

        let btc_pos = ctrl.get_core(&btc).unwrap().engine.position();
        assert_eq!(btc_pos.side, Side::Buy);
        assert_eq!(btc_pos.quantity, Decimal::from(1));

        let eth_pos = ctrl.get_core(&eth).unwrap().engine.position();
        assert_eq!(eth_pos.side, Side::Sell);
        assert_eq!(eth_pos.quantity, Decimal::from(2));
    }

    #[test]
    fn signal_unknown_symbol() {
        let mut ctrl = Controller::new(default_params());
        let unknown = SymbolBuf::new("XYZUSDT").unwrap();
        assert_eq!(
            ctrl.signal(
                &unknown,
                Side::Buy,
                Decimal::from(1),
                Decimal::from(100),
                1000
            ),
            None
        );
    }

    #[test]
    fn drain_events_returns_all() {
        let mut ctrl = Controller::new(default_params());
        ctrl.add_core("BTCUSDT").unwrap();
        ctrl.add_core("ETHUSDT").unwrap();

        let btc = SymbolBuf::new("BTCUSDT").unwrap();
        let eth = SymbolBuf::new("ETHUSDT").unwrap();

        ctrl.signal(
            &btc,
            Side::Buy,
            Decimal::from(1),
            Decimal::from(50000),
            1000,
        );
        ctrl.signal(
            &eth,
            Side::Sell,
            Decimal::from(1),
            Decimal::from(3000),
            1000,
        );

        let events = ctrl.drain_events();
        assert_eq!(events.len(), 2);
        assert!(events
            .iter()
            .any(|e| e.symbol.as_str() == "BTCUSDT" && e.kind == TradeEventKind::PositionOpened));
        assert!(events
            .iter()
            .any(|e| e.symbol.as_str() == "ETHUSDT" && e.kind == TradeEventKind::PositionOpened));
    }

    #[test]
    fn drain_events_empty() {
        let ctrl = Controller::new(default_params());
        assert!(ctrl.drain_events().is_empty());
    }

    #[test]
    fn drain_events_into_reuses_buffer() {
        let mut ctrl = Controller::new(default_params());
        ctrl.add_core("BTCUSDT").unwrap();
        let btc = SymbolBuf::new("BTCUSDT").unwrap();
        ctrl.signal(
            &btc,
            Side::Buy,
            Decimal::from(1),
            Decimal::from(50000),
            1000,
        );

        let mut buf = Vec::new();
        ctrl.drain_events_into(&mut buf);
        assert_eq!(buf.len(), 1);

        // Second drain — no new events
        ctrl.drain_events_into(&mut buf);
        assert_eq!(buf.len(), 1); // nothing new added
    }

    #[test]
    fn tick_all_updates_pnl() {
        let mut ctrl = Controller::new(default_params());
        ctrl.add_core("BTCUSDT").unwrap();
        let btc = SymbolBuf::new("BTCUSDT").unwrap();
        ctrl.signal(
            &btc,
            Side::Buy,
            Decimal::from(1),
            Decimal::from(50000),
            1000,
        );
        ctrl.tick(&btc, Decimal::from(55000));

        let pos = ctrl.get_core(&btc).unwrap().engine.position();
        assert_eq!(pos.unrealized_pnl, Decimal::from(5000));
    }

    #[test]
    fn circuit_breaker_blocks_signals() {
        let mut ctrl = Controller::new(default_params());
        ctrl.add_core("BTCUSDT").unwrap();
        ctrl.set_circuit_breaker(true);

        let btc = SymbolBuf::new("BTCUSDT").unwrap();

        // First need a tick to pick up new params
        ctrl.tick(&btc, Decimal::from(50000));

        let result = ctrl.signal(
            &btc,
            Side::Buy,
            Decimal::from(1),
            Decimal::from(50000),
            1000,
        );
        assert_eq!(result, Some(GateResult::Blocked));
    }

    #[test]
    fn positions_snapshot() {
        let mut ctrl = Controller::new(default_params());
        ctrl.add_core("BTCUSDT").unwrap();
        ctrl.add_core("ETHUSDT").unwrap();

        let btc = SymbolBuf::new("BTCUSDT").unwrap();
        ctrl.signal(
            &btc,
            Side::Buy,
            Decimal::from(1),
            Decimal::from(50000),
            1000,
        );

        let positions = ctrl.positions();
        assert_eq!(positions.len(), 2);
    }

    #[test]
    fn multiple_cores_independent() {
        let mut ctrl = Controller::new(default_params());
        ctrl.add_core("BTCUSDT").unwrap();
        ctrl.add_core("ETHUSDT").unwrap();

        let btc = SymbolBuf::new("BTCUSDT").unwrap();
        let eth = SymbolBuf::new("ETHUSDT").unwrap();

        // Trade BTC
        ctrl.signal(
            &btc,
            Side::Buy,
            Decimal::from(1),
            Decimal::from(50000),
            1000,
        );
        // Close BTC
        ctrl.signal(
            &btc,
            Side::Sell,
            Decimal::from(1),
            Decimal::from(51000),
            2000,
        );

        // ETH should be unaffected
        let eth_pos = ctrl.get_core(&eth).unwrap().engine.position();
        assert!(!eth_pos.is_open());
        assert_eq!(eth_pos.realized_pnl, Decimal::ZERO);
    }

    #[test]
    fn set_permissions_propagates() {
        let mut ctrl = Controller::new(default_params());
        ctrl.add_core("BTCUSDT").unwrap();
        ctrl.set_permissions(PermissionFlags::NONE);

        let btc = SymbolBuf::new("BTCUSDT").unwrap();

        // Tick to pick up params
        ctrl.tick(&btc, Decimal::from(50000));

        let result = ctrl.signal(
            &btc,
            Side::Buy,
            Decimal::from(1),
            Decimal::from(50000),
            1000,
        );
        assert_eq!(result, Some(GateResult::Blocked));
    }

    #[test]
    fn mark_to_market_all() {
        let mut ctrl = Controller::new(default_params());
        ctrl.add_core("BTCUSDT").unwrap();
        ctrl.add_core("ETHUSDT").unwrap();

        let btc = SymbolBuf::new("BTCUSDT").unwrap();
        let eth = SymbolBuf::new("ETHUSDT").unwrap();

        ctrl.signal(
            &btc,
            Side::Buy,
            Decimal::from(1),
            Decimal::from(50000),
            1000,
        );
        ctrl.signal(
            &eth,
            Side::Sell,
            Decimal::from(10),
            Decimal::from(3000),
            1000,
        );

        ctrl.mark_to_market_all(&[(btc, Decimal::from(55000)), (eth, Decimal::from(2800))]);

        let btc_pnl = ctrl
            .get_core(&btc)
            .unwrap()
            .engine
            .position()
            .unrealized_pnl;
        let eth_pnl = ctrl
            .get_core(&eth)
            .unwrap()
            .engine
            .position()
            .unrealized_pnl;
        assert_eq!(btc_pnl, Decimal::from(5000));
        assert_eq!(eth_pnl, Decimal::from(2000));
    }

    #[test]
    fn end_to_end_flow() {
        let mut ctrl = Controller::new(default_params());
        ctrl.add_core("BTCUSDT").unwrap();

        let btc = SymbolBuf::new("BTCUSDT").unwrap();

        // Open long
        let result = ctrl.signal(
            &btc,
            Side::Buy,
            Decimal::from(2),
            Decimal::from(50000),
            1000,
        );
        assert_eq!(result, Some(GateResult::Open));

        // Price moves up
        ctrl.tick(&btc, Decimal::from(55000));
        assert_eq!(
            ctrl.get_core(&btc)
                .unwrap()
                .engine
                .position()
                .unrealized_pnl,
            Decimal::from(10000)
        );

        // Close position
        ctrl.signal(
            &btc,
            Side::Sell,
            Decimal::from(2),
            Decimal::from(55000),
            2000,
        );

        // Drain all events
        let events = ctrl.drain_events();
        assert_eq!(events.len(), 2); // open + close
        assert_eq!(events[0].kind, TradeEventKind::PositionOpened);
        assert_eq!(events[1].kind, TradeEventKind::PositionClosed);
        assert_eq!(events[1].realized_pnl, Decimal::from(10000));
    }
}
