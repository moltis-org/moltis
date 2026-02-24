/// Auto-migrate config from legacy schemas.
pub fn migrate_if_needed(_config: &mut serde_json::Value) -> crate::Result<bool> {
    todo!("detect old schema version and apply migrations")
}
