pub use moltis_auth::*;

/// Generate a random 6-digit numeric setup code.
pub fn generate_setup_code() -> String {
    use rand::Rng;
    rand::rng().random_range(100_000..1_000_000).to_string()
}
