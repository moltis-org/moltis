//! Live onboarding service that backs the `wizard.*` RPC methods.

use std::{path::PathBuf, sync::Mutex};

use {
    serde_json::{Value, json},
    tracing::info,
};

use moltis_config::{AgentIdentity, MoltisConfig, UserProfile};

use crate::{
    Context, Result,
    state::{WizardState, WizardStep},
};

/// Live onboarding service backed by a `WizardState` and config persistence.
pub struct LiveOnboardingService {
    state: Mutex<Option<WizardState>>,
    config_path: PathBuf,
}

impl LiveOnboardingService {
    pub fn new(config_path: PathBuf) -> Self {
        Self {
            state: Mutex::new(None),
            config_path,
        }
    }

    /// Save config to the service's config path.
    fn save(&self, config: &MoltisConfig) -> Result<()> {
        moltis_config::loader::save_config_to_path(&self.config_path, config)
            .context("failed to save onboarding config")?;
        Ok(())
    }

    /// Check whether onboarding has been completed.
    ///
    /// Returns `true` when the `.onboarded` sentinel file exists in the data
    /// directory (written after the wizard finishes) **or** the
    /// `SKIP_ONBOARDING` environment variable is set to a non-empty value.
    /// Pre-existing identity/user data alone no longer auto-skips.
    fn is_already_onboarded(&self) -> bool {
        if std::env::var("SKIP_ONBOARDING")
            .ok()
            .is_some_and(|v| !v.is_empty())
        {
            return true;
        }
        onboarded_sentinel().exists()
    }

    /// Mark onboarding as complete by writing the sentinel file.
    fn mark_onboarded(&self) {
        let path = onboarded_sentinel();
        let _ = std::fs::write(&path, "");
    }

    /// Start the wizard. Returns current step info.
    ///
    /// If `force` is true, the wizard starts even when already onboarded,
    /// allowing the user to reconfigure their identity.
    pub fn wizard_start(&self, force: bool) -> Value {
        if !force && self.is_already_onboarded() {
            return json!({
                "onboarded": true,
                "step": "done",
                "prompt": "Already onboarded!",
            });
        }

        let mut ws = WizardState::new();

        // Pre-populate from existing config so the user can keep values.
        if self.config_path.exists()
            && let Ok(cfg) = moltis_config::loader::load_config(&self.config_path)
        {
            ws.identity = cfg.identity;
            ws.user = cfg.user;
        }
        if let Some(file_identity) = moltis_config::load_identity() {
            merge_identity(&mut ws.identity, &file_identity);
        }
        if let Some(file_user) = moltis_config::load_user() {
            merge_user(&mut ws.user, &file_user);
        }

        let resp = step_response(&ws);
        *self.state.lock().unwrap_or_else(|e| e.into_inner()) = Some(ws);
        resp
    }

    /// Advance the wizard with user input.
    pub fn wizard_next(&self, input: &str) -> Result<Value> {
        let mut guard = self.state.lock().unwrap_or_else(|e| e.into_inner());
        let ws = guard.as_mut().context("no active wizard session")?;
        ws.advance(input);

        if ws.is_done() {
            // Merge into existing config or create new one.
            let mut config = if self.config_path.exists() {
                moltis_config::loader::load_config(&self.config_path).unwrap_or_default()
            } else {
                MoltisConfig::default()
            };
            config.identity = ws.identity.clone();
            config.user = ws.user.clone();
            self.save(&config).context("failed to save config")?;
            moltis_config::save_identity(&ws.identity).context("failed to save IDENTITY.md")?;
            moltis_config::save_user(&ws.user).context("failed to save USER.md")?;
            self.mark_onboarded();

            let resp = json!({
                "step": "done",
                "prompt": ws.prompt(),
                "done": true,
                "identity": {
                    "name": config.identity.name,
                    "emoji": config.identity.emoji,
                    "theme": config.identity.theme,
                },
                "user": {
                    "name": config.user.name,
                    "timezone": config.user.timezone,
                },
            });
            *guard = None;
            return Ok(resp);
        }

        Ok(step_response(ws))
    }

    /// Cancel an active wizard session.
    pub fn wizard_cancel(&self) {
        *self.state.lock().unwrap_or_else(|e| e.into_inner()) = None;
    }

    /// Return the current wizard status.
    pub fn wizard_status(&self) -> Value {
        let guard = self.state.lock().unwrap_or_else(|e| e.into_inner());
        let onboarded = self.is_already_onboarded();
        match guard.as_ref() {
            Some(ws) => json!({
                "active": true,
                "step": ws.step,
                "onboarded": onboarded,
            }),
            None => json!({
                "active": false,
                "onboarded": onboarded,
            }),
        }
    }

    /// Update identity fields by merging partial JSON into the existing config.
    ///
    /// Accepts: `{name?, emoji?, theme?, soul?, user_name?, user_timezone?}`
    /// Also accepts `"creature"` and `"vibe"` as backward-compat aliases for `"theme"`.
    pub fn identity_update(&self, params: Value) -> Result<Value> {
        let mut config = if self.config_path.exists() {
            moltis_config::loader::load_config(&self.config_path).unwrap_or_default()
        } else {
            MoltisConfig::default()
        };
        let mut identity = config.identity.clone();
        if let Some(file_identity) = moltis_config::load_identity() {
            merge_identity(&mut identity, &file_identity);
        }
        let mut user = config.user.clone();
        if let Some(file_user) = moltis_config::load_user() {
            merge_user(&mut user, &file_user);
        }

        /// Extract an optional non-empty string from JSON, mapping `""` to `None`.
        fn str_field(params: &Value, key: &str) -> Option<Option<String>> {
            params
                .get(key)
                .and_then(|v| v.as_str())
                .map(|v| (!v.is_empty()).then(|| v.to_string()))
        }

        /// Extract optional timezone field, mapping:
        /// - missing key => None (no-op)
        /// - empty string => Some(None) (clear timezone)
        /// - valid IANA timezone => Some(Some(Timezone))
        /// - invalid timezone => None (ignore)
        fn timezone_field(params: &Value, key: &str) -> Option<Option<moltis_config::Timezone>> {
            let raw = params.get(key).and_then(|v| v.as_str())?;
            let trimmed = raw.trim();
            if trimmed.is_empty() {
                return Some(None);
            }
            trimmed.parse::<moltis_config::Timezone>().ok().map(Some)
        }

        /// Extract optional location field from either `user_location` or `location`.
        ///
        /// Accepted shape:
        /// `{ latitude: f64, longitude: f64, place?: String }`.
        /// - Missing key => None (no-op)
        /// - null => Some(None) (clear location)
        /// - invalid payload => None (ignore)
        fn location_field(params: &Value) -> Option<Option<moltis_config::GeoLocation>> {
            let raw = params
                .get("user_location")
                .or_else(|| params.get("location"))?;

            if raw.is_null() {
                return Some(None);
            }

            let latitude = raw.get("latitude").and_then(|v| v.as_f64())?;
            let longitude = raw.get("longitude").and_then(|v| v.as_f64())?;
            let place = raw
                .get("place")
                .and_then(|v| v.as_str())
                .map(|v| v.to_string());

            Some(Some(moltis_config::GeoLocation::now(
                latitude, longitude, place,
            )))
        }

        if let Some(v) = str_field(&params, "name") {
            identity.name = v;
        }
        if let Some(v) = str_field(&params, "emoji") {
            identity.emoji = v;
        }
        if let Some(v) = str_field(&params, "theme") {
            identity.theme = v;
        }
        // Backward compat: accept creature/vibe and compose into theme.
        if let Some(creature) = str_field(&params, "creature") {
            let vibe = str_field(&params, "vibe").flatten();
            identity.theme = match (vibe, creature) {
                (Some(v), Some(c)) => Some(format!("{v} {c}")),
                (Some(v), None) => Some(v),
                (None, Some(c)) => Some(c),
                (None, None) => None,
            };
        } else if let Some(vibe) = str_field(&params, "vibe") {
            identity.theme = vibe;
        }
        if let Some(v) = params.get("soul") {
            let soul = if v.is_null() {
                None
            } else {
                v.as_str().map(|s| s.to_string())
            };
            moltis_config::save_soul(soul.as_deref()).context("failed to save soul")?;
        }
        if let Some(v) = str_field(&params, "user_name") {
            user.name = v;
        }
        if let Some(v) =
            timezone_field(&params, "user_timezone").or_else(|| timezone_field(&params, "timezone"))
        {
            user.timezone = v;
        }
        if let Some(v) = location_field(&params) {
            user.location = v;
        }

        config.identity = identity.clone();
        config.user = user.clone();

        self.save(&config)?;
        moltis_config::save_identity(&identity).context("failed to save identity")?;
        moltis_config::save_user(&user).context("failed to save user")?;

        // Mark onboarding complete once both names are present.
        if identity.name.is_some() && user.name.is_some() {
            self.mark_onboarded();
        }

        Ok(json!({
            "name": identity.name,
            "emoji": identity.emoji,
            "theme": identity.theme,
            "soul": moltis_config::load_soul(),
            "user_name": user.name,
            "user_timezone": user.timezone.as_ref().map(|tz| tz.name()),
            "user_location": user.location.as_ref().map(|loc| json!({
                "latitude": loc.latitude,
                "longitude": loc.longitude,
                "place": loc.place,
                "updated_at": loc.updated_at,
            })),
        }))
    }

    /// Update SOUL.md in the workspace root.
    pub fn identity_update_soul(&self, soul: Option<String>) -> Result<Value> {
        moltis_config::save_soul(soul.as_deref()).context("failed to save soul")?;
        Ok(json!({}))
    }

    /// Read identity from the config file (for `agent.identity.get`).
    pub fn identity_get(&self) -> moltis_config::ResolvedIdentity {
        let id = if self.config_path.exists()
            && let Ok(cfg) = moltis_config::loader::load_config(&self.config_path)
        {
            info!(
                config_path = %self.config_path.display(),
                config_name = ?cfg.identity.name,
                config_theme = ?cfg.identity.theme,
                "identity_get: loaded config"
            );
            moltis_config::resolve_identity_from_config(&cfg)
        } else {
            info!(
                config_path = %self.config_path.display(),
                "identity_get: config not found, using defaults"
            );
            moltis_config::resolve_identity_from_config(&MoltisConfig::default())
        };
        info!(
            resolved_name = %id.name,
            resolved_emoji = ?id.emoji,
            resolved_theme = ?id.theme,
            resolved_user_name = ?id.user_name,
            "identity_get: resolved identity"
        );
        id
    }
}

/// Path to the `.onboarded` sentinel file in the data directory.
fn onboarded_sentinel() -> PathBuf {
    moltis_config::data_dir().join(".onboarded")
}

fn merge_identity(dst: &mut AgentIdentity, src: &AgentIdentity) {
    if src.name.is_some() {
        dst.name = src.name.clone();
    }
    if src.emoji.is_some() {
        dst.emoji = src.emoji.clone();
    }
    if src.theme.is_some() {
        dst.theme = src.theme.clone();
    }
}

fn merge_user(dst: &mut UserProfile, src: &UserProfile) {
    if src.name.is_some() {
        dst.name = src.name.clone();
    }
    if src.timezone.is_some() {
        dst.timezone = src.timezone.clone();
    }
    if src.location.is_some() {
        dst.location = src.location.clone();
    }
}

fn step_response(ws: &WizardState) -> Value {
    json!({
        "step": ws.step,
        "prompt": ws.prompt(),
        "done": ws.step == WizardStep::Done,
        "onboarded": false,
        "current": current_value(ws),
    })
}

/// Returns the current (pre-populated) value for the active step, if any.
fn current_value(ws: &WizardState) -> Option<&str> {
    use WizardStep::*;
    match ws.step {
        UserName => ws.user.name.as_deref(),
        AgentName => ws.identity.name.as_deref(),
        AgentEmoji => ws.identity.emoji.as_deref(),
        AgentTheme => ws.identity.theme.as_deref(),
        _ => None,
    }
}

#[allow(clippy::unwrap_used, clippy::expect_used)]
#[cfg(test)]
mod tests {
    use {super::*, std::io::Write};

    struct TestDataDirState {
        _data_dir: Option<PathBuf>,
    }

    static DATA_DIR_TEST_LOCK: Mutex<TestDataDirState> =
        Mutex::new(TestDataDirState { _data_dir: None });

    #[test]
    fn wizard_round_trip() {
        let _guard = DATA_DIR_TEST_LOCK.lock().unwrap();
        let dir = tempfile::tempdir().unwrap();
        moltis_config::set_data_dir(dir.path().to_path_buf());
        let config_path = dir.path().join("moltis.toml");
        let svc = LiveOnboardingService::new(config_path.clone());

        // Start
        let resp = svc.wizard_start(false);
        assert_eq!(resp["onboarded"], false);
        assert_eq!(resp["step"], "welcome");

        // Advance through all steps
        svc.wizard_next("").unwrap(); // welcome → user_name
        svc.wizard_next("Alice").unwrap(); // → agent_name
        svc.wizard_next("Rex").unwrap(); // → emoji
        svc.wizard_next("\u{1f436}").unwrap(); // → theme
        svc.wizard_next("chill dog").unwrap(); // → confirm
        let done = svc.wizard_next("").unwrap(); // → done

        assert_eq!(done["done"], true);
        assert_eq!(done["identity"]["name"], "Rex");
        assert_eq!(done["user"]["name"], "Alice");

        // Config file should exist
        assert!(config_path.exists());

        // Should report as onboarded now
        let status = svc.wizard_status();
        assert_eq!(status["onboarded"], true);

        assert!(dir.path().join("IDENTITY.md").exists());
        assert!(dir.path().join("USER.md").exists());
        moltis_config::clear_data_dir();
    }

    #[test]
    fn config_data_alone_does_not_skip_onboarding() {
        let _guard = DATA_DIR_TEST_LOCK.lock().unwrap();
        let dir = tempfile::tempdir().unwrap();
        moltis_config::set_data_dir(dir.path().to_path_buf());
        let config_path = dir.path().join("moltis.toml");
        // Write a config with identity and user — but no sentinel file.
        let mut f = std::fs::File::create(&config_path).unwrap();
        writeln!(f, "[identity]\nname = \"Rex\"\n\n[user]\nname = \"Alice\"").unwrap();

        let svc = LiveOnboardingService::new(config_path);
        // Should NOT be onboarded — data alone isn't enough.
        let resp = svc.wizard_start(false);
        assert_eq!(resp["onboarded"], false);
        assert_eq!(resp["step"], "welcome");
        moltis_config::clear_data_dir();
    }

    #[test]
    fn sentinel_file_marks_onboarded() {
        let _guard = DATA_DIR_TEST_LOCK.lock().unwrap();
        let dir = tempfile::tempdir().unwrap();
        moltis_config::set_data_dir(dir.path().to_path_buf());
        let config_path = dir.path().join("moltis.toml");
        // Write sentinel file.
        std::fs::write(dir.path().join(".onboarded"), "").unwrap();

        let svc = LiveOnboardingService::new(config_path);
        let resp = svc.wizard_start(false);
        assert_eq!(resp["onboarded"], true);
        moltis_config::clear_data_dir();
    }

    #[test]
    fn cancel_wizard() {
        let _guard = DATA_DIR_TEST_LOCK.lock().unwrap();
        let dir = tempfile::tempdir().unwrap();
        moltis_config::set_data_dir(dir.path().to_path_buf());
        let svc = LiveOnboardingService::new(dir.path().join("moltis.toml"));
        svc.wizard_start(false);
        assert_eq!(svc.wizard_status()["active"], true);
        svc.wizard_cancel();
        assert_eq!(svc.wizard_status()["active"], false);
        moltis_config::clear_data_dir();
    }

    #[test]
    fn identity_update_partial() {
        let _guard = DATA_DIR_TEST_LOCK.lock().unwrap();
        let dir = tempfile::tempdir().unwrap();
        moltis_config::set_data_dir(dir.path().to_path_buf());
        let config_path = dir.path().join("moltis.toml");
        let svc = LiveOnboardingService::new(config_path.clone());

        // Create initial identity
        let res = svc
            .identity_update(json!({
                "name": "Rex",
                "emoji": "\u{1f436}",
                "theme": "chill dog",
                "user_name": "Alice",
                "user_timezone": "America/New_York",
            }))
            .unwrap();
        assert_eq!(res["name"], "Rex");
        assert_eq!(res["user_name"], "Alice");
        assert_eq!(res["user_timezone"], "America/New_York");

        // Partial update: only change theme
        let res = svc
            .identity_update(json!({ "theme": "playful pup" }))
            .unwrap();
        assert_eq!(res["name"], "Rex");
        assert_eq!(res["theme"], "playful pup");
        assert_eq!(res["emoji"], "\u{1f436}");

        // Verify identity_get reflects updates
        let id = svc.identity_get();
        assert_eq!(id.name, "Rex");
        assert_eq!(id.theme.as_deref(), Some("playful pup"));
        assert_eq!(id.user_name.as_deref(), Some("Alice"));
        let user = moltis_config::load_user().expect("load user");
        assert_eq!(
            user.timezone.as_ref().map(|tz| tz.name()),
            Some("America/New_York")
        );

        // Update soul
        let res = svc
            .identity_update(json!({ "soul": "Be helpful." }))
            .unwrap();
        assert_eq!(res["soul"], "Be helpful.");

        // Clear soul with null
        let res = svc.identity_update(json!({ "soul": null })).unwrap();
        assert!(res["soul"].is_null());

        let soul_path = dir.path().join("SOUL.md");
        // save_soul(None) writes an empty file (not deleted) to prevent re-seeding
        assert!(soul_path.exists());
        assert!(std::fs::read_to_string(&soul_path).unwrap().is_empty());

        // Reports as onboarded
        assert_eq!(svc.wizard_status()["onboarded"], true);

        moltis_config::clear_data_dir();
    }

    #[test]
    fn identity_update_location_fields() {
        let _guard = DATA_DIR_TEST_LOCK.lock().unwrap();
        let dir = tempfile::tempdir().unwrap();
        moltis_config::set_data_dir(dir.path().to_path_buf());
        let svc = LiveOnboardingService::new(dir.path().join("moltis.toml"));

        let res = svc
            .identity_update(json!({
                "user_location": {
                    "latitude": 37.7749,
                    "longitude": -122.4194,
                    "place": "San Francisco",
                }
            }))
            .unwrap();

        assert_eq!(res["user_location"]["latitude"], 37.7749);
        assert_eq!(res["user_location"]["longitude"], -122.4194);
        assert_eq!(res["user_location"]["place"], "San Francisco");
        assert!(res["user_location"]["updated_at"].is_number());

        let user = moltis_config::load_user().expect("load user");
        let location = user.location.expect("location should be persisted");
        assert_eq!(location.latitude, 37.7749);
        assert_eq!(location.longitude, -122.4194);
        assert_eq!(location.place.as_deref(), Some("San Francisco"));

        moltis_config::clear_data_dir();
    }

    #[test]
    fn identity_update_location_null_clears_existing_value() {
        let _guard = DATA_DIR_TEST_LOCK.lock().unwrap();
        let dir = tempfile::tempdir().unwrap();
        moltis_config::set_data_dir(dir.path().to_path_buf());
        let svc = LiveOnboardingService::new(dir.path().join("moltis.toml"));

        svc.identity_update(json!({
            "user_location": {
                "latitude": 37.7749,
                "longitude": -122.4194
            }
        }))
        .unwrap();

        let res = svc
            .identity_update(json!({ "user_location": null }))
            .unwrap();
        assert!(res["user_location"].is_null());

        let user = moltis_config::load_user();
        assert!(user.as_ref().and_then(|u| u.location.as_ref()).is_none());

        moltis_config::clear_data_dir();
    }

    #[test]
    fn identity_update_empty_fields() {
        let _guard = DATA_DIR_TEST_LOCK.lock().unwrap();
        let dir = tempfile::tempdir().unwrap();
        moltis_config::set_data_dir(dir.path().to_path_buf());
        let svc = LiveOnboardingService::new(dir.path().join("moltis.toml"));

        // Set name, then clear it
        svc.identity_update(json!({ "name": "Rex" })).unwrap();
        let res = svc.identity_update(json!({ "name": "" })).unwrap();
        assert!(res["name"].is_null());
        moltis_config::clear_data_dir();
    }
}
