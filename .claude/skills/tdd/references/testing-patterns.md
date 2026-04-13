# Testing Patterns

## Table of Contents
- [Arrange-Act-Assert](#arrange-act-assert)
- [Test Naming](#test-naming)
- [Near-Miss Negative Tests](#near-miss-negative-tests)
- [Behavior Over Internals](#behavior-over-internals)
- [Test Smells](#test-smells)
- [Pre-Run Sanity Check](#pre-run-sanity-check)

## Arrange-Act-Assert

Keep tests readable:

```rust
#[test]
fn rejects_empty_email() {
    // Arrange
    let input = FormData { email: "".to_string() };

    // Act
    let result = submit_form(input);

    // Assert
    assert_eq!(result, Err(FormError::EmailRequired));
}
```

If the test mixes many concerns, split it.

Async tests require `#[tokio::test]` (or the relevant async runtime attribute):

```rust
#[tokio::test]
async fn fetches_user_by_id() {
    // Arrange
    let repo = InMemoryUserRepo::new();
    repo.insert(User { id: 1, name: "Alice".into() });

    // Act
    let result = repo.find_by_id(1).await;

    // Assert
    assert_eq!(result.unwrap().name, "Alice");
}
```

## Test Naming

Use `snake_case` function names that describe user-observable behavior:
- `creates_task_with_pending_status`
- `returns_error_without_authentication`
- `rejects_title_that_exceeds_max_length`

Avoid:
- `it_works`
- `test1`
- names that describe implementation details instead of behavior

Group related tests in a `mod tests` block:

```rust
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn creates_task_with_pending_status() { ... }

    #[test]
    fn rejects_empty_title() { ... }
}
```

## Near-Miss Negative Tests

Add rejection tests for values that are almost valid:
- boundary value minus one (e.g. `max_len - 1` vs `max_len`)
- structurally plausible but semantically wrong input
- missing required field
- expired or stale state
- off-by-one at range boundaries

These catch the bugs happy-path tests miss.

```rust
#[test]
fn rejects_title_at_exactly_max_length_plus_one() {
    let title = "a".repeat(MAX_TITLE_LEN + 1);
    let result = validate_title(&title);
    assert!(matches!(result, Err(ValidationError::TooLong)));
}

#[test]
fn accepts_title_at_exactly_max_length() {
    let title = "a".repeat(MAX_TITLE_LEN);
    let result = validate_title(&title);
    assert!(result.is_ok());
}
```

## Behavior Over Internals

Ask: "What does the caller or user observe?"

Prefer:
- return values (`assert_eq!(result, Ok(...))`)
- `Result` / `Option` variants (`assert!(matches!(result, Err(MyError::NotFound)))`)
- observable state after a mutation (`assert_eq!(order.status(), Status::Shipped)`)
- HTTP status codes and response bodies in integration tests

Avoid asserting on:
- private fields or internal counters
- internal state containers not part of the public API
- implementation-only helper details
- call counts on mocks when the return value already proves correctness

## Test Smells

Common smells:

| Smell | Sign |
|-------|------|
| Giant setup | `let ... = ...; let ... = ...;` fills half the test |
| Dependent tests | Test B panics when Test A is skipped |
| Mock-heavy | More mock config than real logic |
| Asserting internals | `assert_eq!(obj.internal_vec.len(), 3)` |
| No assertion | Test compiles and passes but proves nothing |
| Lying name | `fn creates_user()` but test also deletes something |

If a test is hard to understand, it is not helping TDD.

## Pre-Run Sanity Check

Before running the suite:
- no test depends on execution order (Rust runs tests in parallel by default)
- no test uses `thread::sleep` where condition-based polling belongs
- no test mocks the module it is supposed to verify
- assertions target behavior, not structure
- `#[cfg(test)]` gates are in place so test helpers never ship in release builds

If any check fails, fix the test before trusting the result.

Run with:
```bash
cargo test            # all tests
cargo test foo        # tests whose name contains "foo"
cargo test -- --nocapture   # show println! output
```
