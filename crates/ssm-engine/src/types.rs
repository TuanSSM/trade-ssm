use rust_decimal::Decimal;
use ssm_core::Side;
use std::fmt;

// ---------------------------------------------------------------------------
// SymbolBuf — fixed-size symbol, no heap allocation
// ---------------------------------------------------------------------------

/// Fixed-size symbol buffer. Fits all Binance futures symbols (max ~12 chars).
/// 17 bytes total: 16 data + 1 length. No heap allocation.
#[derive(Clone, Copy, PartialEq, Eq, Hash)]
#[repr(C)]
pub struct SymbolBuf {
    bytes: [u8; 16],
    len: u8,
}

impl SymbolBuf {
    /// Maximum symbol length in bytes.
    pub const MAX_LEN: usize = 16;

    /// Create from a string slice. Returns `None` if longer than 16 bytes.
    pub fn new(s: &str) -> Option<Self> {
        if s.len() > Self::MAX_LEN {
            return None;
        }
        let mut bytes = [0u8; 16];
        bytes[..s.len()].copy_from_slice(s.as_bytes());
        Some(Self {
            bytes,
            len: s.len() as u8,
        })
    }

    /// View as a UTF-8 string slice.
    pub fn as_str(&self) -> &str {
        // SAFETY: We only construct from valid UTF-8 (&str) in from_str.
        unsafe { std::str::from_utf8_unchecked(&self.bytes[..self.len as usize]) }
    }

    /// Returns true if the symbol is empty.
    pub fn is_empty(&self) -> bool {
        self.len == 0
    }
}

impl fmt::Debug for SymbolBuf {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "SymbolBuf({:?})", self.as_str())
    }
}

impl fmt::Display for SymbolBuf {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

// ---------------------------------------------------------------------------
// CorePosition — Copy position, no heap
// ---------------------------------------------------------------------------

/// Position owned by a single `CoreEngine`. All fields are `Copy` — no heap.
///
/// This mirrors `ssm_core::Position` but uses `SymbolBuf` instead of `String`
/// so it can live on the hot path without allocation.
#[derive(Clone, Copy, Debug)]
#[repr(C)]
pub struct CorePosition {
    pub symbol: SymbolBuf,
    pub side: Side,
    pub entry_price: Decimal,
    pub quantity: Decimal,
    pub unrealized_pnl: Decimal,
    pub realized_pnl: Decimal,
    pub leverage: u32,
    pub opened_at: i64,
}

impl CorePosition {
    /// An empty (no position) state for the given symbol.
    pub fn empty(symbol: SymbolBuf) -> Self {
        Self {
            symbol,
            side: Side::Buy,
            entry_price: Decimal::ZERO,
            quantity: Decimal::ZERO,
            unrealized_pnl: Decimal::ZERO,
            realized_pnl: Decimal::ZERO,
            leverage: 1,
            opened_at: 0,
        }
    }

    /// Returns true if a position is open (quantity > 0).
    pub fn is_open(&self) -> bool {
        self.quantity > Decimal::ZERO
    }

    /// Reset to no position, preserving realized pnl.
    pub fn reset(&mut self) {
        self.quantity = Decimal::ZERO;
        self.entry_price = Decimal::ZERO;
        self.unrealized_pnl = Decimal::ZERO;
        self.opened_at = 0;
    }

    /// Convert to `ssm_core::Position` (allocates String for symbol).
    /// Use on the controller (cold) path only.
    pub fn to_position(&self) -> ssm_core::Position {
        ssm_core::Position {
            symbol: self.symbol.as_str().to_string(),
            side: self.side,
            entry_price: self.entry_price,
            quantity: self.quantity,
            unrealized_pnl: self.unrealized_pnl,
            realized_pnl: self.realized_pnl,
            leverage: self.leverage,
            opened_at: self.opened_at,
        }
    }

    /// Create from `ssm_core::Position`. Returns `None` if symbol too long.
    pub fn from_position(pos: &ssm_core::Position) -> Option<Self> {
        Some(Self {
            symbol: SymbolBuf::new(&pos.symbol)?,
            side: pos.side,
            entry_price: pos.entry_price,
            quantity: pos.quantity,
            unrealized_pnl: pos.unrealized_pnl,
            realized_pnl: pos.realized_pnl,
            leverage: pos.leverage,
            opened_at: pos.opened_at,
        })
    }
}

// ---------------------------------------------------------------------------
// TradeEvent — fixed-size event for SPSC ring
// ---------------------------------------------------------------------------

/// Kind of trade event pushed from core to controller.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[repr(u8)]
pub enum TradeEventKind {
    PositionOpened = 0,
    PositionIncreased = 1,
    PositionReduced = 2,
    PositionClosed = 3,
    GateRejected = 4,
}

/// Fixed-size event pushed from execution core to controller via SPSC ring.
/// No heap allocation.
#[derive(Clone, Copy, Debug)]
#[repr(C)]
pub struct TradeEvent {
    pub kind: TradeEventKind,
    pub symbol: SymbolBuf,
    pub side: Side,
    pub price: Decimal,
    pub quantity: Decimal,
    pub realized_pnl: Decimal,
    pub timestamp: i64,
}

// ---------------------------------------------------------------------------
// EngineParams — cache-line-aligned parameter block
// ---------------------------------------------------------------------------

/// Permission bitflags for gate evaluation.
pub struct PermissionFlags;

impl PermissionFlags {
    pub const BUY_ALLOWED: u32 = 1 << 0;
    pub const SELL_ALLOWED: u32 = 1 << 1;
    pub const ALL: u32 = Self::BUY_ALLOWED | Self::SELL_ALLOWED;
    pub const NONE: u32 = 0;
}

/// Parameters published by the controller, read by cores via `SeqLock`.
///
/// Fixed-size, `Copy`, cache-line-aligned. Fits in ~96 bytes.
#[derive(Clone, Copy, Debug)]
#[repr(C, align(64))]
pub struct EngineParams {
    /// Permission bitfield (see `PermissionFlags`).
    pub permissions: u32,
    /// Maximum position size per symbol (in base asset).
    pub max_position_size: Decimal,
    /// Maximum total exposure (in quote currency).
    pub max_exposure: Decimal,
    /// Maximum drawdown percentage before circuit breaker.
    pub max_drawdown_pct: Decimal,
    /// Fixed fractional position sizing.
    pub position_size_fraction: Decimal,
    /// Circuit breaker active flag.
    pub circuit_breaker: bool,
}

impl Default for EngineParams {
    fn default() -> Self {
        Self {
            permissions: PermissionFlags::ALL,
            max_position_size: Decimal::from(10),
            max_exposure: Decimal::from(1_000_000),
            max_drawdown_pct: Decimal::new(10, 2),
            position_size_fraction: Decimal::new(2, 2),
            circuit_breaker: false,
        }
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn symbol_buf_from_str_valid() {
        let s = SymbolBuf::new("BTCUSDT").unwrap();
        assert_eq!(s.as_str(), "BTCUSDT");
        assert!(!s.is_empty());
    }

    #[test]
    fn symbol_buf_from_str_max_length() {
        let s = SymbolBuf::new("1234567890123456").unwrap();
        assert_eq!(s.as_str(), "1234567890123456");
    }

    #[test]
    fn symbol_buf_from_str_too_long() {
        assert!(SymbolBuf::new("12345678901234567").is_none());
    }

    #[test]
    fn symbol_buf_from_str_empty() {
        let s = SymbolBuf::new("").unwrap();
        assert!(s.is_empty());
        assert_eq!(s.as_str(), "");
    }

    #[test]
    fn symbol_buf_display() {
        let s = SymbolBuf::new("ETHUSDT").unwrap();
        assert_eq!(format!("{s}"), "ETHUSDT");
    }

    #[test]
    fn symbol_buf_equality() {
        let a = SymbolBuf::new("BTCUSDT").unwrap();
        let b = SymbolBuf::new("BTCUSDT").unwrap();
        let c = SymbolBuf::new("ETHUSDT").unwrap();
        assert_eq!(a, b);
        assert_ne!(a, c);
    }

    #[test]
    fn core_position_empty() {
        let sym = SymbolBuf::new("BTCUSDT").unwrap();
        let pos = CorePosition::empty(sym);
        assert!(!pos.is_open());
        assert_eq!(pos.quantity, Decimal::ZERO);
    }

    #[test]
    fn core_position_is_open() {
        let sym = SymbolBuf::new("BTCUSDT").unwrap();
        let mut pos = CorePosition::empty(sym);
        pos.quantity = Decimal::from(1);
        assert!(pos.is_open());
    }

    #[test]
    fn core_position_reset_preserves_realized_pnl() {
        let sym = SymbolBuf::new("BTCUSDT").unwrap();
        let mut pos = CorePosition::empty(sym);
        pos.quantity = Decimal::from(1);
        pos.realized_pnl = Decimal::from(500);
        pos.reset();
        assert!(!pos.is_open());
        assert_eq!(pos.realized_pnl, Decimal::from(500));
    }

    #[test]
    fn core_position_round_trip() {
        let sym = SymbolBuf::new("BTCUSDT").unwrap();
        let core = CorePosition {
            symbol: sym,
            side: Side::Buy,
            entry_price: Decimal::from(50000),
            quantity: Decimal::from(2),
            unrealized_pnl: Decimal::from(1000),
            realized_pnl: Decimal::ZERO,
            leverage: 5,
            opened_at: 1_234_567_890,
        };
        let std_pos = core.to_position();
        assert_eq!(std_pos.symbol, "BTCUSDT");
        assert_eq!(std_pos.entry_price, Decimal::from(50000));
        assert_eq!(std_pos.leverage, 5);

        let back = CorePosition::from_position(&std_pos).unwrap();
        assert_eq!(back.symbol, sym);
        assert_eq!(back.entry_price, core.entry_price);
    }

    #[test]
    fn core_position_from_position_too_long_symbol() {
        let pos = ssm_core::Position {
            symbol: "A".repeat(17),
            side: Side::Buy,
            entry_price: Decimal::ZERO,
            quantity: Decimal::ZERO,
            unrealized_pnl: Decimal::ZERO,
            realized_pnl: Decimal::ZERO,
            leverage: 1,
            opened_at: 0,
        };
        assert!(CorePosition::from_position(&pos).is_none());
    }

    #[test]
    fn engine_params_default() {
        let params = EngineParams::default();
        assert_eq!(params.permissions, PermissionFlags::ALL);
        assert!(!params.circuit_breaker);
        assert_eq!(params.max_position_size, Decimal::from(10));
    }

    #[test]
    fn engine_params_is_copy() {
        let a = EngineParams::default();
        let b = a;
        assert_eq!(a.permissions, b.permissions);
    }

    #[test]
    fn trade_event_kind_values() {
        assert_eq!(TradeEventKind::PositionOpened as u8, 0);
        assert_eq!(TradeEventKind::GateRejected as u8, 4);
    }
}
