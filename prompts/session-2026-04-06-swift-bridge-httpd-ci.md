## Summary

- Investigated GitHub Actions run `24011266296`, job `70023185728`.
- Root cause was `moltis-swift-bridge` test `httpd_start_and_stop` aborting with `SIGSEGV` after reporting `ok`, which pointed to teardown rather than request handling.
- Fixed the embedded HTTP server shutdown path so stop/shutdown waits for the spawned Axum server task to finish before the test process exits.

## Code changes

- `crates/swift-bridge/src/lib.rs`
  - `HttpdHandle` now stores the server `JoinHandle`.
  - Added `stop_httpd_handle(...)` helper to send the shutdown signal and await task completion.
  - `moltis_stop_httpd()` and `moltis_shutdown()` now take the handle out of the global mutex, drop the lock, and perform a synchronous join on the bridge runtime.

## Validation

- `cargo +nightly-2025-11-30 fmt --all -- --check`
- `cargo +nightly-2025-11-30 test -p moltis-swift-bridge httpd_start_and_stop -- --nocapture` repeated 5 times
- `cargo +nightly-2025-11-30 clippy -p moltis-swift-bridge --all-features --tests -- -D warnings`
- `cargo +nightly-2025-11-30 test -p moltis-swift-bridge`
