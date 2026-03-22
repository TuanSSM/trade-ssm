---
allowed-tools: Bash, Read, Write, Edit, Glob, Grep
---

## Hotfix Workflow

Emergency fix with minimal blast radius. Usage: `/hotfix <issue-description>`

**Issue:** $ARGUMENTS

**Constraints:**
- Change as few files as possible
- No refactoring — fix only
- No new dependencies
- Must pass `just ci`

1. Identify root cause from description
2. Find affected code (minimal scope)
3. Implement fix — smallest possible change
4. Add regression test
5. Run `just ci`
6. Create branch `hotfix/<short-name>` from main if needed
7. Prepare: `git add <changed-files> && git commit -m "fix: <description>"`
8. Show: ready to push + create PR, or tag for immediate release
