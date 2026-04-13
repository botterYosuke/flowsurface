# Test Data And Mocks

## Table of Contents
- [Factory Pattern](#factory-pattern)
- [Mock Only Boundaries](#mock-only-boundaries)
- [Common Boundary Mocks](#common-boundary-mocks)
- [Environment And Time](#environment-and-time)
- [Mock Quality Gate](#mock-quality-gate)

## Factory Pattern

Prefer small builder or helper functions over repeated struct literals:

```rust
fn make_user(overrides: impl FnOnce(&mut User)) -> User {
    let mut user = User {
        id: 1,
        name: "Alice".to_string(),
        email: "alice@example.com".to_string(),
        role: Role::User,
    };
    overrides(&mut user);
    user
}

#[test]
fn rejects_admin_role_on_self_registration() {
    let user = make_user(|u| u.role = Role::Admin);
    let result = register(user);
    assert!(matches!(result, Err(RegistrationError::ForbiddenRole)));
}
```

Or use `Default` + struct update syntax for simpler cases:

```rust
#[cfg(test)]
impl Default for User {
    fn default() -> Self {
        User {
            id: 1,
            name: "Alice".to_string(),
            email: "alice@example.com".to_string(),
            role: Role::User,
        }
    }
}

#[test]
fn creates_user_with_pending_status() {
    let user = User { email: "bob@example.com".to_string(), ..User::default() };
    // ...
}
```

This keeps each test focused on the field that matters.

## Mock Only Boundaries

Mock:
- HTTP clients (outbound network calls)
- database connections when isolation requires it
- system clocks / time
- third-party service SDKs

Do not mock:
- your own core business logic
- internal collaborators you control
- the very module under test

If you must mock everything to write the test, the design probably needs work.

### Preferred Rust Mocking Approaches

| Need | Approach |
|------|----------|
| Trait-based mocking | Define a trait, provide a test-only `InMemory*` or `Fake*` impl |
| HTTP boundary | [`wiremock`](https://crates.io/crates/wiremock) or `httpmock` |
| Auto-generated mocks | [`mockall`](https://crates.io/crates/mockall) |
| Arbitrary fake data | [`fake`](https://crates.io/crates/fake) crate |

**Prefer hand-written fakes over `mockall`** when the fake is simple — fakes are easier to read and maintain.

## Common Boundary Mocks

Typical examples in a Rust project:

```rust
// Trait that wraps the real HTTP client
trait MarketDataClient: Send + Sync {
    async fn fetch_price(&self, symbol: &str) -> Result<f64, ClientError>;
}

// In-memory fake for tests
struct FakeMarketDataClient {
    prices: HashMap<String, f64>,
}

impl MarketDataClient for FakeMarketDataClient {
    async fn fetch_price(&self, symbol: &str) -> Result<f64, ClientError> {
        self.prices
            .get(symbol)
            .copied()
            .ok_or(ClientError::NotFound)
    }
}
```

For HTTP-level mocking (testing the client itself):

```rust
use wiremock::{MockServer, Mock, ResponseTemplate};
use wiremock::matchers::{method, path};

#[tokio::test]
async fn returns_error_on_404_response() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/price/AAPL"))
        .respond_with(ResponseTemplate::new(404))
        .mount(&server)
        .await;

    let client = HttpMarketDataClient::new(&server.uri());
    let result = client.fetch_price("AAPL").await;

    assert!(matches!(result, Err(ClientError::NotFound)));
}
```

Keep mocks thin. The point is to isolate the boundary, not recreate the system.

## Environment And Time

### Environment Variables

Use `std::env` and clean up after the test. For parallel-safe env mutation, prefer
a mutex guard or a crate like [`temp-env`](https://crates.io/crates/temp-env):

```rust
#[test]
fn reads_api_key_from_env() {
    temp_env::with_var("API_KEY", Some("test-key"), || {
        let config = Config::from_env().unwrap();
        assert_eq!(config.api_key, "test-key");
    });
}
```

Never set env vars directly in parallel tests — they are process-global and will
cause flaky failures.

### Time

Inject the clock as a dependency rather than calling `SystemTime::now()` or
`Utc::now()` directly:

```rust
trait Clock: Send + Sync {
    fn now(&self) -> DateTime<Utc>;
}

struct SystemClock;
impl Clock for SystemClock {
    fn now(&self) -> DateTime<Utc> { Utc::now() }
}

struct FakeClock(DateTime<Utc>);
impl Clock for FakeClock {
    fn now(&self) -> DateTime<Utc> { self.0 }
}

#[test]
fn token_expired_after_one_hour() {
    let issued_at = Utc::now();
    let clock = FakeClock(issued_at + Duration::hours(2));
    let token = Token::new(issued_at);

    assert!(token.is_expired(&clock));
}
```

For `tokio`-based time, use `tokio::time::pause()` and `tokio::time::advance()`:

```rust
#[tokio::test]
async fn retries_after_delay() {
    tokio::time::pause();
    let handle = tokio::spawn(retry_with_backoff(operation));
    tokio::time::advance(Duration::from_secs(5)).await;
    let result = handle.await.unwrap();
    assert!(result.is_ok());
}
```

## Mock Quality Gate

Reconsider the design when:
- mock setup is longer than the test body
- the mock defines more behavior than the production code path
- the assertion proves the mock was called but not that behavior changed
- you are implementing the same fake logic in every test file

If the fake is useful across many tests, promote it to a shared `tests/helpers/`
or a `#[cfg(test)]` module and import it. The test should still teach you
something real about the system.
