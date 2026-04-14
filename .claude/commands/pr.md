---
allowed-tools: Bash, Read, Glob, Grep, Agent
---

## PR Review Context

Review PR $ARGUMENTS against trade-ssm standards.

**Checklist:**
1. Run `gh pr view $ARGUMENTS --json title,body,files` and `gh pr diff $ARGUMENTS`
2. Verify CI status: `gh pr checks $ARGUMENTS`
3. Check for violations:
   - f64 used for prices/volumes (must be rust_decimal::Decimal)
   - Anti-repainting: signals on forming candle, mutable lookback
   - Missing tests for new public functions
   - Crate boundary violations (check dependency graph in CLAUDE.md)
4. Verify `just ci` passes on the PR branch
5. Post summary with approve/request-changes recommendation

**Crate boundaries:** ssm-core←nothing | ssm-engine←core | ssm-exchange←core | ssm-indicators←core | ssm-notify←core,indicators | ssm-execution←core | ssm-strategy←core,indicators | ssm-ai←core,indicators

6. ssm-engine hot-path violations: heap allocation, non-Copy types, branching gates
