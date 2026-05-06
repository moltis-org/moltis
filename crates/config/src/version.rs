/// Runtime version of Moltis.
///
/// When the `MOLTIS_VERSION` environment variable is set at **compile time**
/// (e.g. by CI injecting `MOLTIS_VERSION=20260311.01`), that value is used.
/// Otherwise falls back to `CARGO_PKG_VERSION` so local dev builds still
/// report *something* useful.
pub const VERSION: &str = match option_env!("MOLTIS_VERSION") {
    Some(v) => v,
    None => env!("CARGO_PKG_VERSION"),
};

/// `true` when built without an explicit `MOLTIS_VERSION`, i.e. a local dev
/// build from source. Used to suppress the update banner for developers.
pub const IS_DEV_BUILD: bool = option_env!("MOLTIS_VERSION").is_none();
