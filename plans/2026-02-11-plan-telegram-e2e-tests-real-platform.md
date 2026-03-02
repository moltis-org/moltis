# Telegram E2E Tests Against Real Platform

## Goal

Write true end-to-end tests for the Telegram integration that exercise the
real Telegram Bot API, ensuring the integration never silently breaks.

## Architecture

```
┌─────────────┐     real Telegram API     ┌──────────────┐
│  Test user   │ ◄──────────────────────► │   Telegram    │
│ (MTProto     │                          │   servers     │
│  client)     │                          └──────┬───────┘
└─────────────┘                                  │
                                                 │ Bot API polling
                                                 ▼
                                          ┌──────────────┐
                                          │ Moltis       │
                                          │ gateway      │
                                          │ (under test) │
                                          └──────────────┘
```

Two real actors:

1. **The moltis bot** — a real Telegram bot running inside the gateway
2. **A test user** — a real Telegram account driven programmatically via MTProto

The test user sends messages to the bot, the bot processes them through
moltis, and the test user asserts on the response.

## Dependencies

Add `grammers` as a dev-dependency to `crates/telegram/`:

```toml
# Root Cargo.toml [workspace.dependencies]
grammers-client = "0.7"
grammers-session = "0.6"

# crates/telegram/Cargo.toml [dev-dependencies]
grammers-client = { workspace = true }
grammers-session = { workspace = true }
```

## Test User Client Harness

File: `crates/telegram/tests/support/test_user.rs`

```rust
use grammers_client::{Client, Config};
use grammers_session::Session;

struct TestUser {
    client: Client,
    bot_username: String,
}

impl TestUser {
    async fn connect() -> Result<Self> {
        let session = Session::load_file_or_create("test_session")?;
        let client = Client::connect(Config {
            session,
            api_id: env::var("TELEGRAM_API_ID")?.parse()?,
            api_hash: env::var("TELEGRAM_API_HASH")?,
            params: Default::default(),
        }).await?;

        Ok(Self {
            client,
            bot_username: env::var("TELEGRAM_BOT_USERNAME")?,
        })
    }

    /// Send a message to the bot and wait for a reply.
    async fn send_and_expect_reply(
        &self,
        text: &str,
        timeout: Duration,
    ) -> Result<String> {
        let bot = self.client.resolve_username(&self.bot_username).await?
            .ok_or_else(|| anyhow!("bot not found"))?;

        self.client.send_message(&bot, text.into()).await?;

        let deadline = Instant::now() + timeout;
        while Instant::now() < deadline {
            let updates = self.client.next_updates().await?;
            for update in updates {
                if let Some(msg) = update.as_new_message() {
                    if msg.sender_id() == bot.id() {
                        return Ok(msg.text().to_string());
                    }
                }
            }
        }
        anyhow::bail!("no reply within {timeout:?}")
    }
}
```

## Test Scenarios

File: `crates/telegram/tests/e2e_real.rs`

All tests use `#[ignore]` so they only run explicitly via `cargo test -- --ignored`.

### 1. Text message roundtrip

Send a text message, verify the bot responds.

```rust
#[tokio::test]
#[ignore]
async fn telegram_text_roundtrip() {
    let user = TestUser::connect().await.unwrap();
    let gateway = start_test_gateway_with_telegram().await;

    let reply = user
        .send_and_expect_reply("Say exactly: pong", Duration::from_secs(30))
        .await
        .unwrap();

    assert!(reply.contains("pong"));
    gateway.shutdown().await;
}
```

### 2. Slash command /new

```rust
#[tokio::test]
#[ignore]
async fn telegram_slash_new_clears_session() {
    let user = TestUser::connect().await.unwrap();
    let gateway = start_test_gateway_with_telegram().await;

    let reply = user
        .send_and_expect_reply("/new", Duration::from_secs(10))
        .await
        .unwrap();

    assert!(reply.to_lowercase().contains("new")
        || reply.to_lowercase().contains("session")
        || reply.to_lowercase().contains("cleared"));

    gateway.shutdown().await;
}
```

### 3. Access control — unauthorized user gets OTP challenge

```rust
#[tokio::test]
#[ignore]
async fn telegram_unauthorized_user_gets_otp_challenge() {
    let user = TestUser::connect().await.unwrap();
    // Gateway with empty allowlist + allowlist policy
    let gateway = start_test_gateway_with_telegram_locked().await;

    let reply = user
        .send_and_expect_reply("hello", Duration::from_secs(10))
        .await
        .unwrap();

    // Should get OTP challenge, NOT a normal LLM response
    assert!(reply.contains("verification") || reply.contains("code"));
    // Critically: the message must NOT contain the actual 6-digit code
    let digit_sequences: Vec<&str> = reply
        .split(|c: char| !c.is_ascii_digit())
        .filter(|s| s.len() >= 6)
        .collect();
    assert!(digit_sequences.is_empty(), "OTP message must not leak the code");

    gateway.shutdown().await;
}
```

### 4. Error always responds (never silent)

```rust
#[tokio::test]
#[ignore]
async fn telegram_error_still_responds() {
    let user = TestUser::connect().await.unwrap();
    // Gateway with no LLM provider configured (will fail to dispatch)
    let gateway = start_test_gateway_no_provider().await;

    let reply = user
        .send_and_expect_reply("hello", Duration::from_secs(15))
        .await;

    // Must get SOME response, even if it's an error — never silent
    assert!(reply.is_ok(), "bot must always respond, even on error");

    gateway.shutdown().await;
}
```

### 5. Streaming response (edit-in-place)

```rust
#[tokio::test]
#[ignore]
async fn telegram_streaming_delivers_final_response() {
    let user = TestUser::connect().await.unwrap();
    // Gateway with streaming enabled (edit-in-place mode)
    let gateway = start_test_gateway_with_telegram().await;

    let reply = user
        .send_and_expect_reply(
            "Write a haiku about testing",
            Duration::from_secs(30),
        )
        .await
        .unwrap();

    // Response should be non-empty and not an error
    assert!(!reply.is_empty());
    assert!(!reply.to_lowercase().contains("error"));

    gateway.shutdown().await;
}
```

### Future scenarios (lower priority)

- Voice message transcription (requires sending an actual voice file)
- Photo attachment handling
- Group mention mode (requires a test group)
- `/model` inline keyboard interaction
- `/sessions` session switching
- Conflict detection (two instances polling same token)

## Gateway Test Harness

Helper to start a real moltis gateway with Telegram configured:

```rust
async fn start_test_gateway_with_telegram() -> TestGateway {
    let config_dir = tempdir().unwrap();
    let data_dir = tempdir().unwrap();

    // Write minimal config with Telegram enabled
    let config = format!(r#"
[channels.telegram.test-bot]
token = "{}"
dm_policy = "open"
stream_mode = "edit_in_place"
"#, env::var("TELEGRAM_BOT_TOKEN").unwrap());

    fs::write(config_dir.path().join("moltis.toml"), config).unwrap();

    // Seed identity so we skip onboarding
    fs::write(data_dir.path().join("IDENTITY.md"), "Test Bot").unwrap();
    fs::write(data_dir.path().join("USER.md"), "").unwrap();

    // Start gateway on random port
    let port = find_free_port();
    let handle = tokio::spawn(async move {
        moltis_gateway::run(GatewayConfig {
            port,
            config_dir: config_dir.path().to_path_buf(),
            data_dir: data_dir.path().to_path_buf(),
            ..Default::default()
        }).await
    });

    // Wait for gateway to be ready
    wait_for_health(port).await;

    TestGateway { handle, port, _config_dir: config_dir, _data_dir: data_dir }
}
```

## CI Setup

### Secrets needed

| Secret | Source |
|--------|--------|
| `TELEGRAM_BOT_TOKEN` | From @BotFather — dedicated test bot |
| `TELEGRAM_BOT_USERNAME` | Bot username (without @) |
| `TELEGRAM_API_ID` | From https://my.telegram.org — test user app |
| `TELEGRAM_API_HASH` | Same |
| `TELEGRAM_SESSION` | Pre-authenticated session file (base64-encoded) |

### Recommended: Use Telegram's test environment

Telegram provides test DCs at `149.154.167.40:443`. Benefits:
- Create throwaway accounts with test phone numbers (`99966XXXXX`)
- No real phone number needed
- Isolated from production
- `grammers` supports custom DC addresses

### Workflow

```yaml
# .github/workflows/e2e-telegram.yml
name: Telegram E2E
on:
  schedule:
    - cron: '0 6 * * 1'   # Weekly Monday 6am UTC
  workflow_dispatch:        # Manual trigger

jobs:
  telegram-e2e:
    runs-on: ubuntu-latest
    if: github.repository == 'moltis-org/moltis'
    timeout-minutes: 10
    steps:
      - uses: actions/checkout@v4

      - name: Install Rust
        uses: dtolnay/rust-toolchain@stable

      - name: Restore session file
        run: echo "${{ secrets.TELEGRAM_SESSION }}" | base64 -d > test_session

      - name: Run Telegram E2E tests
        run: cargo test --package moltis-telegram -- --ignored --test-threads=1
        env:
          TELEGRAM_BOT_TOKEN: ${{ secrets.TELEGRAM_BOT_TOKEN }}
          TELEGRAM_BOT_USERNAME: ${{ secrets.TELEGRAM_BOT_USERNAME }}
          TELEGRAM_API_ID: ${{ secrets.TELEGRAM_API_ID }}
          TELEGRAM_API_HASH: ${{ secrets.TELEGRAM_API_HASH }}
```

Run weekly + manual trigger. Not on every PR (flaky due to network, rate limits).

## LLM non-determinism strategy

Real LLM responses are non-deterministic. Options:

1. **Controlled prompts** — `"Say exactly: pong"` forces specific output
2. **Structural assertions** — response is non-empty, arrives within timeout,
   doesn't contain "error"
3. **Mock LLM in gateway** — keep Telegram real but use a canned LLM provider
   that returns fixed responses. Best of both worlds: tests real Telegram
   transport while making assertions deterministic.

Option 3 is recommended for most scenarios. Only use real LLM for smoke tests.

## Implementation order

1. Add `grammers` workspace dependencies
2. Build `TestUser` harness (`tests/support/test_user.rs`)
3. Build `TestGateway` harness (start gateway with temp config)
4. Write `telegram_text_roundtrip` test first (proves the whole stack works)
5. Add access control / OTP test
6. Add error-always-responds test
7. Set up CI workflow with secrets
8. Add remaining scenarios incrementally

## Complementary: keep mock-based tests too

The real-platform tests run weekly and catch integration regressions. Keep
the existing mock-based unit tests in `crates/telegram/src/handlers.rs` for
fast feedback on every PR — they cover logic; the real tests cover transport.
