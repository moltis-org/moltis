/// Runtime version of Moltis-Mini.
///
/// Reads the version from `.mini-version` file at compile time.
/// Format: `{UPSTREAM_VERSION}-mini.{NN}` (e.g., `20260426.01-mini.01`)
///
/// When the `MOLTIS_VERSION` environment variable is set at **compile time**
/// (e.g. by CI), that value takes precedence. Otherwise, reads from the
/// `.mini-version` file. Falls back to `CARGO_PKG_VERSION` for local dev builds.
pub const VERSION: &str = match option_env!("MOLTIS_VERSION") {
    Some(v) => v,
    None => match option_env!("MOLTIS_MINI_VERSION") {
        Some(v) => v,
        None => env!("CARGO_PKG_VERSION"),
    },
};

/// `true` when built without an explicit `MOLTIS_VERSION`, i.e. a local dev
/// build from source. Used to suppress the update banner for developers.
pub const IS_DEV_BUILD: bool = option_env!("MOLTIS_VERSION").is_none();
