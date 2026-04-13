# Integration And Live Proof

## Table of Contents
- [Choose The Right Verification Depth](#choose-the-right-verification-depth)
- [When Unit Tests Are Enough](#when-unit-tests-are-enough)
- [When To Escalate To Integration Tests](#when-to-escalate-to-integration-tests)
- [When To Escalate To E2E Or Live Harness Proof](#when-to-escalate-to-e2e-or-live-harness-proof)
- [Live-Proof Triggers](#live-proof-triggers)
- [Rust Integration Test Layout](#rust-integration-test-layout)
- [Coordination With CC10X Live Verification](#coordination-with-cc10x-live-verification)

## Choose The Right Verification Depth

TDD starts with unit or focused behavior tests, but it does not end there when
the risk profile is higher.

Think in layers:
- unit tests prove local behavior
- integration tests prove boundary wiring
- E2E or live harness proof proves the system truth the user cares about

## When Unit Tests Are Enough

Unit tests in `#[cfg(test)]` inline modules are usually enough for:
- pure logic (parsers, calculators, state machines)
- isolated validation
- small refactors with unchanged public signatures

## When To Escalate To Integration Tests

Escalate when the change crosses a real boundary:
- HTTP handler to service layer
- service to database (SQLite, Postgres, etc.)
- async task producer to consumer
- cache or background side effects
- serialization round-trips (JSON, bincode, etc.)

The test should exercise the real collaboration, not just the local branch.

In Rust, integration tests live in the `tests/` directory and link against the
compiled crate as an external consumer:

```
tests/
  order_workflow.rs      ← tests the public API end-to-end
  replay_api.rs          ← tests HTTP routes with a real server
```

Run integration tests only:
```bash
cargo test --test order_workflow
```

Run everything:
```bash
cargo test
```

## When To Escalate To E2E Or Live Harness Proof

Escalate beyond integration when the user or accepted plan needs confidence in:
- seeded workflows (full DB + server boot)
- real HTTP API calls (not `wiremock`)
- background job or timer orchestration
- cross-process side effects
- load or stress behavior

At that point, unit tests are necessary but not sufficient.

For this project, E2E tests use the shell harness in `tests/`:

```bash
bash tests/e2e_replay_api.sh
```

## Live-Proof Triggers

Treat these as strong signals to escalate:
- "production-like"
- "real data"
- "real API calls"
- "connect all the dots"
- "seed the database"
- "stress test"
- "boot the actual process"

When these appear, do not claim the work is verified with unit tests alone.

## Rust Integration Test Layout

```
src/
  lib.rs              ← public API surface
  replay/mod.rs       ← internal modules
tests/
  e2e_replay_api.sh   ← shell-level E2E harness
  *.rs                ← Rust integration tests (link crate as external)
```

Rust integration test files can share helpers via a `tests/common/mod.rs`:

```rust
// tests/common/mod.rs
pub fn start_test_server() -> TestServer { ... }
pub fn seed_orders(db: &Db, n: usize) { ... }

// tests/order_workflow.rs
mod common;

#[tokio::test]
async fn complete_order_flow() {
    let server = common::start_test_server().await;
    // ...
}
```

Keep integration test helpers in `tests/common/`, not in `src/`, so they never
ship in release builds.

## Coordination With CC10X Live Verification

If the plan requires live proof:
- read the live-verification strategy reference in the CC10X planning skill
- use the harness commands and proof scripts defined there
- confirm the server process is fully booted before issuing requests

The TDD cycle still matters. Live proof is the outer confidence ring, not a
replacement for test-first discipline.
