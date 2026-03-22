---
allowed-tools: Bash, Read, Write, Edit, Glob, Grep, Agent
---

## Development Context

Activate development mode for trade-ssm.

**Key rules:**
- `Decimal` for prices/volumes, `f64` only in AI feature vectors
- `anyhow::Result` in binaries, domain errors in libraries
- All I/O async (tokio), indicators sync pure functions
- Never signal on forming candle (anti-repainting)
- One test file per module, inline `#[cfg(test)]`
- `tracing` for logging with field-level context

**Quick commands:** `just ci` (validate) | `just test` | `just lint` | `just fmt`

**Crate to edit:** determine from $ARGUMENTS or ask.

**Dependency graph:**
```
core ← exchange ← indicators ← notify
                 ← strategy   ← execution
                 ← ai
```
