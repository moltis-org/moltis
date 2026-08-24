use moltis_config::schema::HeartbeatConfig;

/// Apply a `heartbeat.update` payload to the configuration already in effect.
///
/// `HeartbeatConfig` is `#[serde(default)]`, so deserializing a payload on its
/// own resets every omitted key to its default. An explicit `null` still clears
/// an optional field.
pub(super) fn apply_heartbeat_patch(
    current: &HeartbeatConfig,
    patch: &serde_json::Value,
) -> Result<HeartbeatConfig, serde_json::Error> {
    let mut merged = serde_json::to_value(current)?;
    overlay_json(&mut merged, patch);
    serde_json::from_value(merged)
}

/// Overlay `patch` onto `base`, recursing into objects so a partial nested
/// object updates the keys it carries instead of replacing the whole thing.
fn overlay_json(base: &mut serde_json::Value, patch: &serde_json::Value) {
    match (base, patch) {
        (serde_json::Value::Object(base), serde_json::Value::Object(patch)) => {
            for (key, value) in patch {
                overlay_json(
                    base.entry(key.clone()).or_insert(serde_json::Value::Null),
                    value,
                );
            }
        },
        (base, patch) => *base = patch.clone(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn update_leaves_fields_the_payload_omits_alone() -> anyhow::Result<()> {
        let current = HeartbeatConfig {
            wake_cooldown: "1h".into(),
            agent_id: Some("night-shift".into()),
            ..Default::default()
        };

        // The settings form has no input for these fields, so neither key is in
        // its payload.
        let saved = apply_heartbeat_patch(
            &current,
            &serde_json::json!({
                "enabled": true,
                "every": "15m",
                "ack_max_chars": 300,
                "deliver": false,
                "sandbox_enabled": true,
                "active_hours": {"start": "07:00", "end": "23:00", "timezone": "local"},
            }),
        )?;

        assert_eq!(saved.every, "15m");
        assert_eq!(saved.wake_cooldown, "1h");
        assert_eq!(saved.agent_id.as_deref(), Some("night-shift"));
        Ok(())
    }

    #[test]
    fn partial_active_hours_moves_only_the_keys_it_carries() -> anyhow::Result<()> {
        let current = HeartbeatConfig {
            active_hours: moltis_config::schema::ActiveHoursConfig {
                start: "09:00".into(),
                end: "21:00".into(),
                timezone: "UTC".into(),
            },
            ..Default::default()
        };

        let saved = apply_heartbeat_patch(
            &current,
            &serde_json::json!({"active_hours": {"start": "07:00"}}),
        )?;

        assert_eq!(saved.active_hours.start, "07:00");
        assert_eq!(saved.active_hours.end, "21:00");
        assert_eq!(saved.active_hours.timezone, "UTC");
        Ok(())
    }

    #[test]
    fn explicit_null_still_clears_an_optional_field() -> anyhow::Result<()> {
        let current = HeartbeatConfig {
            model: Some("anthropic/claude-sonnet-4".into()),
            ..Default::default()
        };

        let saved = apply_heartbeat_patch(&current, &serde_json::json!({"model": null}))?;

        assert!(saved.model.is_none());
        Ok(())
    }
}
