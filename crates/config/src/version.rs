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
