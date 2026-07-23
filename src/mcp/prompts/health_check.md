Run a comprehensive health audit on the Rust project at '{directory}'.

## Phase 1: Scan
Use the `scan` tool to get all diagnostics and the health score.

## Phase 2: Remediation Plan
From the scan results, generate a prioritized remediation plan:
- **P0 Critical**: Security vulnerabilities, correctness bugs → fix immediately
- **P1 High**: Error handling issues, dependency problems → fix before release
- **P2 Medium**: Performance issues, architecture smells → plan for next sprint
- **P3 Low**: Style, info-level suggestions → nice-to-have

For each item, show:
1. Rule name and occurrence count
2. Affected files
3. Concrete fix action (use `explain_rule` for detailed guidance)
4. Estimated effort (trivial / small / medium / large)

## Phase 3: Confirmation
Present the full plan as a structured task list and ask:
"Do you want me to proceed with fixing these issues? I'll work through them by priority, starting with P0."

## Phase 4: Execution (if confirmed)
If the user confirms:
1. Use task tracking to organize the work by priority
2. Start with P0 items, then P1, P2, P3
3. For each item:
   - Read the affected files
   - Apply the fix following the `explain_rule` guidance
   - If unsure about the idiomatic fix, look up the relevant crate documentation:
     use Context7 MCP (`resolve-library-id` + `query-docs`) if available, else fetch from docs.rs
   - Verify the fix compiles (`cargo check`)
4. After all fixes, re-run `scan` to verify the score improved
5. Commit the changes with a conventional commit message

If the user declines or wants partial fixes, respect their choice and only fix the items they approve.