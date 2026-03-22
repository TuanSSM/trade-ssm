---
allowed-tools: Bash, Read, Grep, Glob
---

## Release Management

Create and validate a release. Usage: `/release <version>` (e.g., `/release 0.3.0`)

**Version:** $ARGUMENTS

1. Validate semver format matches `v*.*.*`
2. Ensure on `main` branch, clean working tree
3. Run `just ci` — must pass
4. Check all `Cargo.toml` workspace member versions are consistent
5. Verify no TODO/FIXME/HACK in crates (warn, don't block)
6. Create annotated tag: `git tag -a v$ARGUMENTS -m "Release v$ARGUMENTS"`
7. Show release checklist:
   - [ ] CI green on main
   - [ ] Docker builds (`just docker-build`)
   - [ ] Changelog reviewed
   - [ ] Tag pushed: `git push origin v$ARGUMENTS`
   - [ ] release.yml workflow triggers automatically
