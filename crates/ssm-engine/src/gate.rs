use crate::types::{EngineParams, PermissionFlags};
use rust_decimal::Decimal;
use ssm_core::Side;

// ---------------------------------------------------------------------------
// GateResult
// ---------------------------------------------------------------------------

/// Result of a risk gate evaluation.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[repr(u8)]
pub enum GateResult {
    /// The trade is allowed.
    Open = 1,
    /// The trade is blocked by risk limits.
    Blocked = 0,
}

impl GateResult {
    /// Convert from a boolean pass/fail condition.
    #[inline]
    pub fn from_pass(pass: bool) -> Self {
        if pass {
            Self::Open
        } else {
            Self::Blocked
        }
    }

    /// Returns true if the gate is open.
    #[inline]
    pub fn is_open(self) -> bool {
        matches!(self, Self::Open)
    }
}

// ---------------------------------------------------------------------------
// Branchless helpers
// ---------------------------------------------------------------------------

/// Branchless Decimal less-than: returns 1 if `a < b`, 0 otherwise.
#[inline]
pub fn decimal_lt(a: Decimal, b: Decimal) -> u32 {
    u32::from(a < b)
}

/// Branchless bool-to-u32 conversion.
#[inline]
pub fn bool_gate(b: bool) -> u32 {
    u32::from(b)
}

// ---------------------------------------------------------------------------
// Gate evaluation
// ---------------------------------------------------------------------------

/// Evaluate the buy gate using arithmetic composition of conditions.
///
/// The gate is open iff ALL of:
/// - `BUY_ALLOWED` permission flag is set
/// - `circuit_breaker` is not active
/// - `current_quantity + order_quantity` <= `max_position_size`
/// - `current_exposure` < `max_exposure`
///
/// Instead of if/else chains, we multiply boolean conditions as u32 values.
/// A zero product means at least one condition failed.
#[inline]
pub fn evaluate_buy_gate(
    permissions: u32,
    current_quantity: Decimal,
    current_exposure: Decimal,
    params: &EngineParams,
) -> GateResult {
    let has_perm = bool_gate(permissions & PermissionFlags::BUY_ALLOWED != 0);
    let no_breaker = bool_gate(!params.circuit_breaker);
    let below_size = decimal_lt(current_quantity, params.max_position_size);
    let below_exposure = decimal_lt(current_exposure, params.max_exposure);

    let pass = has_perm * no_breaker * below_size * below_exposure;
    GateResult::from_pass(pass > 0)
}

/// Evaluate the sell gate using arithmetic composition of conditions.
///
/// Mirrors `evaluate_buy_gate` with `SELL_ALLOWED` permission.
#[inline]
pub fn evaluate_sell_gate(
    permissions: u32,
    current_quantity: Decimal,
    current_exposure: Decimal,
    params: &EngineParams,
) -> GateResult {
    let has_perm = bool_gate(permissions & PermissionFlags::SELL_ALLOWED != 0);
    let no_breaker = bool_gate(!params.circuit_breaker);
    let below_size = decimal_lt(current_quantity, params.max_position_size);
    let below_exposure = decimal_lt(current_exposure, params.max_exposure);

    let pass = has_perm * no_breaker * below_size * below_exposure;
    GateResult::from_pass(pass > 0)
}

/// Evaluate gate for a given side. Dispatches to buy or sell gate.
#[inline]
pub fn evaluate_gate(
    side: Side,
    permissions: u32,
    current_quantity: Decimal,
    current_exposure: Decimal,
    params: &EngineParams,
) -> GateResult {
    match side {
        Side::Buy => evaluate_buy_gate(permissions, current_quantity, current_exposure, params),
        Side::Sell => evaluate_sell_gate(permissions, current_quantity, current_exposure, params),
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    fn default_params() -> EngineParams {
        EngineParams::default()
    }

    #[test]
    fn buy_gate_all_conditions_met() {
        let params = default_params();
        let result = evaluate_buy_gate(
            PermissionFlags::ALL,
            Decimal::from(1),
            Decimal::from(1000),
            &params,
        );
        assert_eq!(result, GateResult::Open);
    }

    #[test]
    fn buy_gate_no_permission() {
        let params = default_params();
        let result = evaluate_buy_gate(
            PermissionFlags::SELL_ALLOWED, // no BUY_ALLOWED
            Decimal::from(1),
            Decimal::from(1000),
            &params,
        );
        assert_eq!(result, GateResult::Blocked);
    }

    #[test]
    fn buy_gate_circuit_breaker() {
        let mut params = default_params();
        params.circuit_breaker = true;
        let result = evaluate_buy_gate(
            PermissionFlags::ALL,
            Decimal::from(1),
            Decimal::from(1000),
            &params,
        );
        assert_eq!(result, GateResult::Blocked);
    }

    #[test]
    fn buy_gate_max_exposure_exceeded() {
        let params = default_params();
        let result = evaluate_buy_gate(
            PermissionFlags::ALL,
            Decimal::from(1),
            Decimal::from(2_000_000), // > 1_000_000
            &params,
        );
        assert_eq!(result, GateResult::Blocked);
    }

    #[test]
    fn buy_gate_max_position_exceeded() {
        let params = default_params();
        let result = evaluate_buy_gate(
            PermissionFlags::ALL,
            Decimal::from(15), // > max_position_size (10)
            Decimal::from(1000),
            &params,
        );
        assert_eq!(result, GateResult::Blocked);
    }

    #[test]
    fn sell_gate_all_conditions_met() {
        let params = default_params();
        let result = evaluate_sell_gate(
            PermissionFlags::ALL,
            Decimal::from(1),
            Decimal::from(1000),
            &params,
        );
        assert_eq!(result, GateResult::Open);
    }

    #[test]
    fn sell_gate_no_permission() {
        let params = default_params();
        let result = evaluate_sell_gate(
            PermissionFlags::BUY_ALLOWED, // no SELL_ALLOWED
            Decimal::from(1),
            Decimal::from(1000),
            &params,
        );
        assert_eq!(result, GateResult::Blocked);
    }

    #[test]
    fn decimal_lt_less() {
        assert_eq!(decimal_lt(Decimal::from(1), Decimal::from(2)), 1);
    }

    #[test]
    fn decimal_lt_equal() {
        assert_eq!(decimal_lt(Decimal::from(2), Decimal::from(2)), 0);
    }

    #[test]
    fn decimal_lt_greater() {
        assert_eq!(decimal_lt(Decimal::from(3), Decimal::from(2)), 0);
    }

    #[test]
    fn bool_gate_true() {
        assert_eq!(bool_gate(true), 1);
    }

    #[test]
    fn bool_gate_false() {
        assert_eq!(bool_gate(false), 0);
    }

    #[test]
    fn gate_with_zero_quantity() {
        let params = default_params();
        let result = evaluate_buy_gate(PermissionFlags::ALL, Decimal::ZERO, Decimal::ZERO, &params);
        assert_eq!(result, GateResult::Open);
    }

    #[test]
    fn gate_at_exact_limit() {
        let mut params = default_params();
        params.max_position_size = Decimal::from(5);
        // quantity == max_position_size → not less than → blocked
        let result = evaluate_buy_gate(
            PermissionFlags::ALL,
            Decimal::from(5),
            Decimal::from(100),
            &params,
        );
        assert_eq!(result, GateResult::Blocked);
    }

    #[test]
    fn gate_just_below_limit() {
        let mut params = default_params();
        params.max_position_size = Decimal::from(5);
        let result = evaluate_buy_gate(
            PermissionFlags::ALL,
            Decimal::new(499, 2), // 4.99
            Decimal::from(100),
            &params,
        );
        assert_eq!(result, GateResult::Open);
    }

    #[test]
    fn gate_no_permissions_blocks_both() {
        let params = default_params();
        assert_eq!(
            evaluate_buy_gate(PermissionFlags::NONE, Decimal::ZERO, Decimal::ZERO, &params),
            GateResult::Blocked
        );
        assert_eq!(
            evaluate_sell_gate(PermissionFlags::NONE, Decimal::ZERO, Decimal::ZERO, &params),
            GateResult::Blocked
        );
    }

    #[test]
    fn evaluate_gate_dispatches_by_side() {
        let params = default_params();
        let buy = evaluate_gate(
            Side::Buy,
            PermissionFlags::ALL,
            Decimal::ZERO,
            Decimal::ZERO,
            &params,
        );
        let sell = evaluate_gate(
            Side::Sell,
            PermissionFlags::ALL,
            Decimal::ZERO,
            Decimal::ZERO,
            &params,
        );
        assert_eq!(buy, GateResult::Open);
        assert_eq!(sell, GateResult::Open);
    }

    #[test]
    fn gate_result_is_open() {
        assert!(GateResult::Open.is_open());
        assert!(!GateResult::Blocked.is_open());
    }
}
