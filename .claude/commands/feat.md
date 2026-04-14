---
allowed-tools: Bash, Read, Write, Edit, Glob, Grep, Agent
---

## Feature Development

Scaffold and implement a new feature. Usage: `/feat <description>`

**Feature:** $ARGUMENTS

1. Identify target crate(s) from feature description
2. Check existing patterns in target crate (read lib.rs, mod.rs)
3. Implement following project conventions:
   - `Decimal` for prices/volumes
   - Pure functions for indicators (no I/O, no async)
   - `Strategy` trait for strategies, `AIModel` trait for models
   - Structured errors in library crates
4. Add tests in `#[cfg(test)]` module
5. If indicator: add anti-repainting test
6. If strategy: implement `Strategy` trait (see CLAUDE.md scaffold)
7. If AI model: implement `AIModel` trait (see CLAUDE.md scaffold)
8. If ssm-engine: all hot-path types must be Copy, no heap allocation, use branchless gate pattern
9. Run `just ci` to validate
9. Create branch `feat/<short-name>` if not on one
