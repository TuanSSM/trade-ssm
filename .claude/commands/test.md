---
allowed-tools: Bash, Read, Grep, Glob
---

## Test Context

Run and analyze tests. Usage: `/test [crate-name|all]`

**Scope:** $ARGUMENTS (default: all)

1. Run: `cargo test --workspace` (or `cargo test -p $ARGUMENTS` if specific crate)
2. On failure: read failing test, trace to source, suggest fix
3. Coverage check: list public functions in modified files missing test coverage
4. Anti-repainting verification for indicator tests:
   - Values at `[0..N]` must not change when candle `N+1` added
   - `analyze_cvd()` must be pure: same input = same output
5. Report: passed/failed/ignored counts per crate
