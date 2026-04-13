---
name: tdd
description: "Internal skill. Use cc10x-router for all development tasks."
allowed-tools: Read Grep Glob Bash Write Edit Agent
user-invocable: false
---

# TDD Orchestrator

Coordinates the test-writer and implementer agents to enforce strict Red-Green-Refactor.

## Agent Roles

| Agent | Phase | Responsibility |
|-------|-------|----------------|
| **test-writer** | RED | Write one failing test, confirm it fails for the right reason |
| **implementer** | GREEN + REFACTOR | Write minimal code to pass, refactor, verify coverage |

## Iron Law

```
NO PRODUCTION CODE WITHOUT A FAILING TEST FIRST
```

Write code before the test? Delete it. Start over. No exceptions.

## Vertical Slicing (CRITICAL)

```
WRONG — horizontal (all tests then all code):
  RED:   test1, test2, test3, test4, test5
  GREEN: impl1, impl2, impl3, impl4, impl5

RIGHT — vertical (one feature at a time):
  RED -> GREEN: test1 -> impl1
  RED -> GREEN: test2 -> impl2
  RED -> GREEN: test3 -> impl3
```

Do NOT bulk-write tests. One cycle at a time.

## Per-Feature Cycle

```
┌─────────────────────────────────────────────────────────┐
│  1. Invoke test-writer                                  │
│     → writes one failing test                           │
│     → confirms exit 1 (runtime failure, not compile)   │
│     → reports RED result                                │
│                                                         │
│  2. Invoke implementer                                  │
│     → receives RED report                               │
│     → writes minimal code                               │
│     → confirms exit 0 for target test                   │
│     → confirms full suite still green                   │
│     → refactors (tests stay green)                      │
│     → checks 80%+ coverage                             │
│     → reports GREEN + REFACTOR result                   │
│                                                         │
│  3. Repeat for next behavior                            │
└─────────────────────────────────────────────────────────┘
```

## How to Invoke Agents

Use the Agent tool with `subagent_type` matching the agent name.

**Invoke test-writer:**
```
Agent(subagent_type="test-writer", prompt="Write the next failing test for: [behavior description]. Context: [relevant files/types].")
```

**Invoke implementer:**
```
Agent(subagent_type="implementer", prompt="RED report received: test `[name]` fails with [reason] at [path:line]. Make it pass.")
```

## Escalation

| Situation | Action |
|-----------|--------|
| test-writer cannot describe the failure | Ask human partner to clarify behavior |
| implementer cannot pass without breaking 3+ tests | Stop, report to human partner |
| Task needs real APIs / browser / stress proof | Read `references/integration-and-live-proof.md` |
| "Skip TDD just this once" | Delete code. Start over. |

## Rationalization Table

| Excuse | Reality |
|--------|---------|
| "Too simple to test" | Simple code breaks. Test takes 30 seconds. |
| "I'll test after" | Tests passing immediately prove nothing. |
| "Already manually tested" | Ad-hoc ≠ systematic. No record, can't re-run. |
| "Deleting X hours is wasteful" | Sunk cost fallacy. Keeping unverified code is debt. |
| "Keep as reference, write tests first" | You'll adapt it. That's testing after. Delete means delete. |
| "TDD will slow me down" | TDD faster than debugging. |

## Completion Checklist

Before marking any feature done:

- [ ] Every new function/method has a test that was watched to fail
- [ ] Each failure was at runtime, not compile error, for the right reason
- [ ] Minimal code was written per cycle
- [ ] All tests pass: `cargo test`
- [ ] Output pristine: no warnings, `cargo clippy` clean
- [ ] 80%+ line coverage: `cargo tarpaulin`

Cannot check all boxes? You skipped TDD. Start over.

## Test Contracts Across Agents

The test file IS the contract. Planner defines behavior as test names and assertions. Builder writes tests first, implements to green. Reviewer re-runs — pass means contract fulfilled.

Do not duplicate the contract in prose. If the test file expresses the requirement, the test file is the requirement.
