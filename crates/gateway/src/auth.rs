pub use moltis_auth::*;

/// Generate a random 8-character alphanumeric setup code (~48 bits of entropy).
///
/// Uses uppercase + digits only (no ambiguous chars like 0/O, 1/I/L) for easy
/// reading from a terminal.
pub fn generate_setup_code() -> String {
    use rand::RngExt;
    const CHARSET: &[u8] = b"ABCDEFGHJKMNPQRSTUVWXYZ23456789";
    let mut rng = rand::rng();
    (0..8)
        .map(|_| CHARSET[rng.random_range(0..CHARSET.len())] as char)
        .collect()
}
