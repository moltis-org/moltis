use std::sync::Arc;

use {
    async_trait::async_trait,
    secrecy::{ExposeSecret, Secret},
    tracing::{info, warn},
};

use crate::auth;

use crate::server::CoreStartupProfile;

pub(super) fn gateway_credentials_allowed(
    profile: CoreStartupProfile,
    network: &moltis_tools::sandbox::NetworkPolicy,
) -> bool {
    !profile.is_headless() && *network != moltis_tools::sandbox::NetworkPolicy::Blocked
}

pub(super) struct CredentialEnvVarProvider {
    pub(super) store: Arc<auth::CredentialStore>,
    pub(super) gateway_url: Option<String>,
    pub(super) sandbox_api_key: Option<Secret<String>>,
}

#[async_trait]
impl moltis_tools::exec::EnvVarProvider for CredentialEnvVarProvider {
    async fn get_env_vars(&self) -> Vec<(String, Secret<String>)> {
        let mut vars = match self.store.get_all_env_values().await {
            Ok(values) => values
                .into_iter()
                .filter(|(key, _)| !key.starts_with("__MOLTIS_"))
                .map(|(key, value)| (key, Secret::new(value)))
                .collect(),
            Err(error) => {
                warn!(error = %error, "failed to load runtime env overrides for tools");
                Vec::new()
            },
        };

        if let Some(ref url) = self.gateway_url {
            vars.push(("MOLTIS_GATEWAY_URL".into(), Secret::new(url.clone())));
        }
        if let Some(ref key) = self.sandbox_api_key {
            vars.push((
                "MOLTIS_API_KEY".into(),
                Secret::new(key.expose_secret().clone()),
            ));
        }

        vars
    }
}

/// Label every sandbox API key is minted under.
const SANDBOX_API_KEY_LABEL: &str = "sandbox-ctl";

/// Env var the current sandbox API key is cached in.
///
/// Versioned, and that version bump is the whole point. The v1 name cached a
/// key minted with `operator.write`, and this function returns the cached key
/// before it ever looks at the scope list - so narrowing the mint on its own
/// would have fixed fresh installs and left every existing one holding a
/// read+write key forever. A new name means an install that has only the v1
/// entry falls through to the mint path exactly once, on its next startup,
/// with no operator action anywhere.
const SANDBOX_API_KEY_ENV: &str = "__MOLTIS_SANDBOX_API_KEY_V2";

/// The pre-rotation name. Read only to notice it and retire what it points at.
const SANDBOX_API_KEY_ENV_V1: &str = "__MOLTIS_SANDBOX_API_KEY";

pub(super) async fn ensure_sandbox_api_key(store: &auth::CredentialStore) -> Option<String> {
    // A read failure is not "there is no cached key". `if let Ok(..)` fell
    // through on any store error straight into retire-and-mint, which revokes
    // the key every running sandbox is still holding over a transient sqlite
    // hiccup. Give up for this startup instead: a sandbox without
    // `MOLTIS_API_KEY` loses `moltis-ctl`, while a revoked live key breaks
    // every sandbox already up.
    let vals = match store.get_all_env_values().await {
        Ok(vals) => vals,
        Err(e) => {
            warn!(error = %e, "failed to read credential store, leaving the sandbox API key alone");
            return None;
        },
    };
    if let Some((_, key)) = vals.iter().find(|(k, _)| k == SANDBOX_API_KEY_ENV) {
        return Some(key.clone());
    }

    // Snapshot the keys to retire *before* minting, and retire only these.
    //
    // Retiring first and by label revoked whatever carried the label at that
    // moment, which on two gateways sharing one database is not necessarily an
    // old key. Both miss the cache, one mints, the other's retirement pass
    // lists that brand-new key and revokes it, and the first gateway then
    // injects a dead credential into every sandbox it starts. A key minted
    // after this line cannot be in this list, so no startup can revoke another
    // startup's fresh key.
    let superseded = superseded_sandbox_key_ids(store).await;

    let scopes = auth::SANDBOX_API_KEY_SCOPES
        .iter()
        .map(|scope| (*scope).to_string())
        .collect::<Vec<_>>();
    let (new_id, raw_key) = match store
        .create_api_key(SANDBOX_API_KEY_LABEL, Some(&scopes))
        .await
    {
        Ok(minted) => minted,
        Err(e) => {
            warn!(error = %e, "failed to create sandbox API key");
            return None;
        },
    };

    // Publishing the cache entry is the atomic step that picks the winner:
    // the env var key is UNIQUE, so exactly one concurrent startup inserts it.
    match store
        .set_env_var_if_absent(SANDBOX_API_KEY_ENV, &raw_key)
        .await
    {
        Ok(true) => {
            retire_superseded_sandbox_keys(store, &superseded).await;
            info!(scopes = ?scopes, "created sandbox-ctl API key for moltis-ctl");
            Some(raw_key)
        },
        Ok(false) => {
            // Another startup published first. Its key is the one every
            // sandbox will be handed, so ours was never live anywhere: revoke
            // it rather than leave a valid spare credential in the store, and
            // retire nothing - the winner owns that.
            if let Err(e) = store.revoke_api_key(new_id).await {
                warn!(error = %e, id = new_id, "failed to revoke the losing sandbox API key");
            }
            info!("a concurrent startup published the sandbox key first, adopting it");
            match store.get_all_env_values().await {
                Ok(vals) => vals
                    .into_iter()
                    .find(|(k, _)| k == SANDBOX_API_KEY_ENV)
                    .map(|(_, v)| v)
                    .or_else(|| {
                        warn!("the published sandbox API key vanished before it could be read");
                        None
                    }),
                Err(e) => {
                    warn!(error = %e, "failed to re-read the published sandbox API key");
                    None
                },
            }
        },
        Err(e) => {
            // The key is valid, it just is not cached, so this boot works and
            // the next one mints again. Retire nothing: without a published
            // cache entry there is no winner, and revoking here would be the
            // very race this ordering exists to avoid.
            warn!(error = %e, "failed to persist sandbox API key");
            Some(raw_key)
        },
    }
}

/// The ids of the sandbox keys that exist right now.
///
/// Captured before a new key is minted, so that the retirement pass can name
/// exactly the keys this startup is replacing instead of "whatever carries the
/// label when I get around to it" - which is how a concurrent startup's fresh
/// key ended up in the revoke list.
async fn superseded_sandbox_key_ids(store: &auth::CredentialStore) -> Vec<i64> {
    match store.list_api_keys().await {
        Ok(keys) => keys
            .iter()
            .filter(|e| e.label == SANDBOX_API_KEY_LABEL)
            .map(|e| e.id)
            .collect(),
        Err(e) => {
            warn!(error = %e, "failed to list API keys while rotating the sandbox key");
            Vec::new()
        },
    }
}

/// Revoke the keys `superseded_sandbox_key_ids` named, and drop the v1 cache entry.
///
/// Revoked rather than merely orphaned: the raw v1 key was handed to every
/// sandbox that ever ran, so leaving the row live would leave a read+write
/// credential valid on the gateway for anything that kept a copy. Best effort
/// throughout - a failure here must not stop the narrower key being minted,
/// because a startup that gives up leaves the old key in place, which is the
/// state this is here to end.
///
/// Only ever called by the startup that won the cache slot, and only with ids
/// it observed before minting.
async fn retire_superseded_sandbox_keys(store: &auth::CredentialStore, superseded: &[i64]) {
    for &id in superseded {
        if let Err(e) = store.revoke_api_key(id).await {
            warn!(error = %e, id, "failed to revoke old sandbox API key");
        } else {
            info!(id, "revoked pre-rotation sandbox-ctl API key");
        }
    }

    match store.list_env_vars().await {
        Ok(vars) => {
            for var in vars.iter().filter(|v| v.key == SANDBOX_API_KEY_ENV_V1) {
                if let Err(e) = store.delete_env_var(var.id).await {
                    warn!(error = %e, "failed to drop the pre-rotation sandbox key env var");
                }
            }
        },
        Err(e) => warn!(error = %e, "failed to list env vars while rotating the sandbox key"),
    }
}

#[cfg(test)]
#[allow(clippy::expect_used)]
mod tests {
    use super::*;

    use sqlx::SqlitePool;

    async fn memory_store() -> auth::CredentialStore {
        let pool = SqlitePool::connect("sqlite::memory:")
            .await
            .expect("in-memory sqlite");
        auth::CredentialStore::new(pool)
            .await
            .expect("credential store")
    }

    async fn sandbox_key_scopes(
        store: &auth::CredentialStore,
        raw_key: &str,
    ) -> Option<Vec<String>> {
        store
            .verify_api_key(raw_key)
            .await
            .expect("verify")
            .map(|verification| verification.scopes)
    }

    #[tokio::test]
    async fn existing_install_with_a_read_write_sandbox_key_is_rotated_to_read_only() {
        // An install from before the fix: a sandbox key minted with
        // operator.write, already cached under the v1 env name. Nothing about
        // it is touched by an operator - the next startup has to retire it on
        // its own, or narrowing the mint only ever helps fresh installs.
        let store = memory_store().await;
        let legacy_scopes = vec!["operator.read".to_string(), "operator.write".to_string()];
        let (_legacy_id, legacy_key) = store
            .create_api_key("sandbox-ctl", Some(&legacy_scopes))
            .await
            .expect("legacy key");
        store
            .set_env_var("__MOLTIS_SANDBOX_API_KEY", &legacy_key)
            .await
            .expect("cache legacy key");

        let key = ensure_sandbox_api_key(&store).await.expect("sandbox key");

        assert_eq!(
            sandbox_key_scopes(&store, &key).await,
            Some(
                auth::SANDBOX_API_KEY_SCOPES
                    .iter()
                    .map(|scope| (*scope).to_string())
                    .collect::<Vec<_>>()
            ),
            "the key handed to the sandbox must carry the minted scopes"
        );
        assert!(
            !key.eq(&legacy_key),
            "the read+write key must not be handed out again"
        );
        assert!(
            sandbox_key_scopes(&store, &legacy_key).await.is_none(),
            "the read+write key must be revoked, not merely unused"
        );
    }

    #[tokio::test]
    async fn the_rotated_sandbox_key_is_stable_across_startups() {
        let store = memory_store().await;
        let first = ensure_sandbox_api_key(&store).await.expect("first key");
        let second = ensure_sandbox_api_key(&store).await.expect("second key");
        assert_eq!(
            first, second,
            "a settled install must not re-mint every boot"
        );
        assert_eq!(
            store
                .list_api_keys()
                .await
                .expect("list")
                .iter()
                .filter(|entry| entry.label == "sandbox-ctl")
                .count(),
            1,
            "re-minting on every startup would leak a key per boot"
        );
    }

    #[tokio::test]
    async fn a_startup_never_revokes_a_key_minted_after_its_own_snapshot() {
        // The concurrent-rotation bug, reduced to its ordering. Two gateways
        // share a credential database and both miss the V2 cache. Gateway B
        // looks at the store, then gateway A completes a whole rotation, then
        // B gets around to retiring. The old pass retired by label at that
        // later moment, so it revoked A's brand-new key and left A injecting a
        // dead credential into every sandbox it started.
        let store = memory_store().await;

        // B looks first: nothing carries the label yet.
        let b_snapshot = superseded_sandbox_key_ids(&store).await;
        assert!(
            b_snapshot.is_empty(),
            "nothing exists yet, so B has nothing of its own to retire"
        );

        // A mints, publishes and hands its key to its sandboxes.
        let a_key = ensure_sandbox_api_key(&store).await.expect("gateway A key");

        // B now retires, with the list it captured before A existed.
        retire_superseded_sandbox_keys(&store, &b_snapshot).await;

        assert!(
            sandbox_key_scopes(&store, &a_key).await.is_some(),
            "the key A handed to its sandboxes must still be valid"
        );
    }

    #[tokio::test]
    async fn a_losing_startup_adopts_the_published_key_instead_of_its_own() {
        // Both startups mint, only one can publish. The loser must hand its
        // sandboxes the published key - two live keys under one cache entry
        // means whichever gateway retires next revokes a credential that is in
        // use somewhere.
        let store = memory_store().await;

        let (first, second) = tokio::join!(
            ensure_sandbox_api_key(&store),
            ensure_sandbox_api_key(&store),
        );
        let first = first.expect("first startup key");
        let second = second.expect("second startup key");

        assert_eq!(
            first, second,
            "both startups must end up handing out the same key"
        );
        assert!(
            sandbox_key_scopes(&store, &first).await.is_some(),
            "and that key must not have been revoked by the other startup"
        );
        assert_eq!(
            store
                .list_api_keys()
                .await
                .expect("list")
                .iter()
                .filter(|entry| entry.label == SANDBOX_API_KEY_LABEL)
                .count(),
            1,
            "a key minted and then lost must be revoked, not left live as a spare"
        );
    }

    #[test]
    fn headless_profiles_never_create_gateway_credentials() {
        assert!(!gateway_credentials_allowed(
            CoreStartupProfile::Headless,
            &moltis_tools::sandbox::NetworkPolicy::Trusted,
        ));
        assert!(!gateway_credentials_allowed(
            CoreStartupProfile::Headless,
            &moltis_tools::sandbox::NetworkPolicy::Bypass,
        ));
    }
}
