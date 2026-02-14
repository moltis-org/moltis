//! Live onboarding service that backs the `wizard.*` RPC methods.

use std::{path::PathBuf, sync::Mutex};

use serde_json::{Value, json};

use moltis_config::{AgentIdentity, MoltisConfig, UserProfile};

use crate::state::{WizardState, WizardStep};

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
    fn save(&self, config: &MoltisConfig) -> anyhow::Result<()> {
        moltis_config::loader::save_config_to_path(&self.config_path, config)?;
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
    pub fn wizard_next(&self, input: &str) -> Result<Value, String> {
        let mut guard = self.state.lock().unwrap_or_else(|e| e.into_inner());
        let ws = guard.as_mut().ok_or("no active wizard session")?;
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
            self.save(&config)
                .map_err(|e| format!("failed to save config: {e}"))?;
            if let Err(e) = moltis_config::save_identity(&ws.identity) {
                return Err(format!("failed to save IDENTITY.md: {e}"));
            }
            if let Err(e) = moltis_config::save_user(&ws.user) {
                return Err(format!("failed to save USER.md: {e}"));
            }
            self.mark_onboarded();

            let resp = json!({
                "step": "done",
                "prompt": ws.prompt(),
                "done": true,
                "identity": {
                    "name": config.identity.name,
                    "emoji": config.identity.emoji,
                    "creature": config.identity.creature,
                    "vibe": config.identity.vibe,
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
    /// Accepts: `{name?, emoji?, creature?, vibe?, soul?, user_name?}`
    pub fn identity_update(&self, params: Value) -> anyhow::Result<Value> {
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

        if let Some(v) = str_field(&params, "name") {
            identity.name = v;
        }
        if let Some(v) = str_field(&params, "emoji") {
            identity.emoji = v;
        }
        if let Some(v) = str_field(&params, "creature") {
            identity.creature = v;
        }
        if let Some(v) = str_field(&params, "vibe") {
            identity.vibe = v;
        }
        if let Some(v) = params.get("soul") {
            let soul = if v.is_null() {
                None
            } else {
                v.as_str().map(|s| s.to_string())
            };
            moltis_config::save_soul(soul.as_deref())?;
        }
        if let Some(v) = str_field(&params, "user_name") {
            user.name = v;
        }

        config.identity = identity.clone();
        config.user = user.clone();

        self.save(&config)?;
        moltis_config::save_identity(&identity)?;
        moltis_config::save_user(&user)?;

        // Mark onboarding complete once both names are present.
        if identity.name.is_some() && user.name.is_some() {
            self.mark_onboarded();
        }

        Ok(json!({
            "name": identity.name,
            "emoji": identity.emoji,
            "creature": identity.creature,
            "vibe": identity.vibe,
            "soul": moltis_config::load_soul(),
            "user_name": user.name,
        }))
    }

    /// Update SOUL.md in the workspace root.
    pub fn identity_update_soul(&self, soul: Option<String>) -> anyhow::Result<Value> {
        moltis_config::save_soul(soul.as_deref())?;
        Ok(json!({}))
    }

    /// Read identity from the config file (for `agent.identity.get`).
    pub fn identity_get(&self) -> moltis_config::ResolvedIdentity {
        if self.config_path.exists()
            && let Ok(cfg) = moltis_config::loader::load_config(&self.config_path)
        {
            let mut id = moltis_config::ResolvedIdentity::from_config(&cfg);
            if let Some(file_identity) = moltis_config::load_identity() {
                if let Some(name) = file_identity.name {
                    id.name = name;
                }
                if let Some(emoji) = file_identity.emoji {
                    id.emoji = Some(emoji);
                }
                if let Some(creature) = file_identity.creature {
                    id.creature = Some(creature);
                }
                if let Some(vibe) = file_identity.vibe {
                    id.vibe = Some(vibe);
                }
            }
            if let Some(file_user) = moltis_config::load_user()
                && let Some(name) = file_user.name
            {
                id.user_name = Some(name);
            }
            id.soul = moltis_config::load_soul();
            return id;
        }
        let mut id = moltis_config::ResolvedIdentity::default();
        if let Some(file_identity) = moltis_config::load_identity() {
            if let Some(name) = file_identity.name {
                id.name = name;
            }
            id.emoji = file_identity.emoji;
            id.creature = file_identity.creature;
            id.vibe = file_identity.vibe;
        }
        if let Some(file_user) = moltis_config::load_user() {
            id.user_name = file_user.name;
        }
        id.soul = moltis_config::load_soul();
        id
    }
}

/// Path to the `.onboarded` sentinel file in the data directory.
fn onboarded_sentinel() -> std::path::PathBuf {
    moltis_config::data_dir().join(".onboarded")
}

fn merge_identity(dst: &mut AgentIdentity, src: &AgentIdentity) {
    if src.name.is_some() {
        dst.name = src.name.clone();
    }
    if src.emoji.is_some() {
        dst.emoji = src.emoji.clone();
    }
    if src.creature.is_some() {
        dst.creature = src.creature.clone();
    }
    if src.vibe.is_some() {
        dst.vibe = src.vibe.clone();
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
        AgentCreature => ws.identity.creature.as_deref(),
        AgentVibe => ws.identity.vibe.as_deref(),
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

    static DATA_DIR_TEST_LOCK: std::sync::Mutex<TestDataDirState> =
        std::sync::Mutex::new(TestDataDirState { _data_dir: None });

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
        svc.wizard_next("\u{1f436}").unwrap(); // → creature
        svc.wizard_next("dog").unwrap(); // → vibe
        svc.wizard_next("chill").unwrap(); // → confirm
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
                "creature": "dog",
                "vibe": "chill",
                "user_name": "Alice",
            }))
            .unwrap();
        assert_eq!(res["name"], "Rex");
        assert_eq!(res["user_name"], "Alice");

        // Partial update: only change vibe
        let res = svc.identity_update(json!({ "vibe": "playful" })).unwrap();
        assert_eq!(res["name"], "Rex");
        assert_eq!(res["vibe"], "playful");
        assert_eq!(res["emoji"], "\u{1f436}");

        // Verify identity_get reflects updates
        let id = svc.identity_get();
        assert_eq!(id.name, "Rex");
        assert_eq!(id.vibe.as_deref(), Some("playful"));
        assert_eq!(id.user_name.as_deref(), Some("Alice"));

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
