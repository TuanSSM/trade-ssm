---
allowed-tools: Bash, Read, Grep, Glob
---

## Pre-commit Validation

Run full validation before committing staged changes.

1. `cargo fmt --check` — formatting
2. `cargo clippy --workspace --all-targets -- -D warnings` — lints
3. `cargo test --workspace` — all tests pass
4. Scan staged `.rs` files for:
   - `f64` in price/volume/order paths (must use `Decimal`)
   - Signals on `candles.last()` or `candles[candles.len()-1]` (anti-repainting)
   - Missing `#[cfg(test)]` modules in modified files with new pub functions
5. In `crates/ssm-engine/src/core.rs` and `gate.rs`:
   - No `String`, `Vec`, `HashMap`, `Box`, `Arc`, `Mutex` usage
   - No `async` functions
   - Gate functions use `bool_gate()` and `decimal_lt()` helpers
6. Report pass/fail with specific file:line for violations
