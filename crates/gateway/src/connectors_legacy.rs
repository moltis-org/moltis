use std::collections::HashSet;

use {
    moltis_connectors::{Account, AccountUpdate, ConnectorKind},
    secrecy::{ExposeSecret, Secret},
    serde_json::{Value, from_value},
    url::Url,
};

use super::{
    AccountCreateRequest, ConnectorManager, ConnectorManagerError, Result, StoredAccountConfigView,
    account_config_value, stored_secret_present,
};

const SOURCE_PREFIX: &str = "config:caldav:";
const MAX_SOURCE_KEY_CHARS: usize = 1_024;

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct ConfiguredCalDavReconciliation {
    pub created: u64,
    pub adopted: u64,
    pub updated: u64,
    pub unchanged: u64,
    pub disabled: u64,
    pub deferred: u64,
    pub invalid: u64,
    pub ambiguous: u64,
}

#[derive(Clone)]
struct DesiredAccount {
    source_key: String,
    name: String,
    server_url: String,
    username: String,
    password: Secret<String>,
    timeout_seconds: u64,
    enabled: bool,
}

impl ConnectorManager {
    pub async fn reconcile_configured_caldav_accounts(
        &self,
        config: &moltis_config::CalDavConfig,
    ) -> Result<ConfiguredCalDavReconciliation> {
        let mut report = ConfiguredCalDavReconciliation::default();
        let mut entries = config.accounts.iter().collect::<Vec<_>>();
        entries.sort_by_key(|(name, _)| *name);
        let mut desired_sources = HashSet::new();

        for (name, legacy) in entries {
            let desired = match desired_account(name, legacy, config.enabled) {
                Ok(desired) => desired,
                Err(_) => {
                    report.invalid += 1;
                    continue;
                },
            };
            desired_sources.insert(desired.source_key.clone());
            match self.reconcile_configured_caldav_account(desired).await {
                Ok(AccountReconciliation::Created) => report.created += 1,
                Ok(AccountReconciliation::Adopted) => report.adopted += 1,
                Ok(AccountReconciliation::Updated) => report.updated += 1,
                Ok(AccountReconciliation::Unchanged) => report.unchanged += 1,
                Ok(AccountReconciliation::Deferred) => report.deferred += 1,
                Ok(AccountReconciliation::Ambiguous) => report.ambiguous += 1,
                Err(_) => report.deferred += 1,
            }
        }

        let accounts = self
            .store
            .list_accounts()
            .await
            .map_err(ConnectorManagerError::from)?;
        for account in accounts.into_iter().filter(|account| {
            account.source_key.as_ref().is_some_and(|source| {
                source.starts_with(SOURCE_PREFIX) && !desired_sources.contains(source)
            })
        }) {
            let has_password = from_value::<StoredAccountConfigView>(account.config.clone())
                .ok()
                .and_then(|stored| stored_secret_present(&stored.password).ok())
                .unwrap_or(true);
            if !account.enabled && !has_password {
                continue;
            }
            if self.disable_configured_account(account).await.is_ok() {
                report.disabled += 1;
            } else {
                report.deferred += 1;
            }
        }
        Ok(report)
    }

    async fn reconcile_configured_caldav_account(
        &self,
        desired: DesiredAccount,
    ) -> Result<AccountReconciliation> {
        if let Some(existing) = self
            .store
            .get_account_by_source_key(ConnectorKind::Caldav, &desired.source_key)
            .await
            .map_err(ConnectorManagerError::from)?
        {
            return self.update_configured_account(existing, desired).await;
        }

        let candidates = self
            .store
            .list_accounts()
            .await
            .map_err(ConnectorManagerError::from)?
            .into_iter()
            .filter(|account| account.kind == ConnectorKind::Caldav && account.source_key.is_none())
            .filter_map(|account| {
                let stored = from_value::<StoredAccountConfigView>(account.config.clone()).ok()?;
                (same_server_url(&stored.server_url, &desired.server_url)
                    && stored.username == desired.username)
                    .then_some(account)
            })
            .collect::<Vec<_>>();

        match candidates.as_slice() {
            [] => {
                self.add_account_with_source(desired.create_request(), Some(desired.source_key))
                    .await?;
                Ok(AccountReconciliation::Created)
            },
            [candidate] => {
                let mut mutation = self.account_mutations.lock().await;
                self.ensure_account_inactive(&candidate.id).await?;
                let candidate = self.account_required(&candidate.id).await?;
                if candidate.source_key.is_some() {
                    return Err(ConnectorManagerError::InvalidInput(
                        "connector account was adopted concurrently".to_owned(),
                    ));
                }
                let stored: StoredAccountConfigView = from_value(candidate.config)
                    .map_err(|error| super::internal(error, "deserialize CalDAV account"))?;
                if !same_server_url(&stored.server_url, &desired.server_url)
                    || stored.username != desired.username
                {
                    return Err(ConnectorManagerError::InvalidInput(
                        "connector account changed before adoption".to_owned(),
                    ));
                }
                let mut config = account_config_value(
                    desired.server_url,
                    desired.username,
                    Value::String(desired.password.expose_secret().to_owned()),
                    desired.timeout_seconds,
                    stored.allow_insecure_http,
                    stored.allow_private_network,
                );
                self.prepare_secret_for_storage(&candidate.id, &mut config)
                    .await?;
                self.store
                    .adopt_account(&candidate.id, &desired.source_key, AccountUpdate {
                        name: desired.name,
                        config,
                        enabled: desired.enabled,
                    })
                    .await
                    .map_err(ConnectorManagerError::from)?;
                mutation.revision = mutation.revision.wrapping_add(1);
                Ok(AccountReconciliation::Adopted)
            },
            _ => Ok(AccountReconciliation::Ambiguous),
        }
    }

    async fn update_configured_account(
        &self,
        existing: Account,
        desired: DesiredAccount,
    ) -> Result<AccountReconciliation> {
        let mut mutation = self.account_mutations.lock().await;
        self.ensure_account_inactive(&existing.id).await?;
        let existing = self.account_required(&existing.id).await?;
        let stored: StoredAccountConfigView = from_value(existing.config.clone())
            .map_err(|error| super::internal(error, "deserialize configured CalDAV account"))?;
        let metadata_changed = existing.name != desired.name
            || existing.enabled != desired.enabled
            || stored.server_url != desired.server_url
            || stored.username != desired.username
            || stored.timeout_seconds != desired.timeout_seconds;
        let has_stored_password = stored_secret_present(&stored.password)?;
        let runtime = self.runtime_account_config(&existing.id).await;
        let password_changed = !has_stored_password
            || runtime.as_ref().is_ok_and(|runtime| {
                runtime.password.expose_secret() != desired.password.expose_secret()
            });
        if !metadata_changed && !password_changed {
            return Ok(if runtime.is_ok() {
                AccountReconciliation::Unchanged
            } else {
                AccountReconciliation::Deferred
            });
        }

        let password = if password_changed {
            Value::String(desired.password.expose_secret().to_owned())
        } else {
            stored.password
        };
        let mut config = account_config_value(
            desired.server_url,
            desired.username,
            password,
            desired.timeout_seconds,
            stored.allow_insecure_http,
            stored.allow_private_network,
        );
        self.prepare_secret_for_storage(&existing.id, &mut config)
            .await?;
        self.store
            .update_account(&existing.id, AccountUpdate {
                name: desired.name,
                config,
                enabled: desired.enabled,
            })
            .await
            .map_err(ConnectorManagerError::from)?;
        mutation.revision = mutation.revision.wrapping_add(1);
        Ok(AccountReconciliation::Updated)
    }

    async fn disable_configured_account(&self, account: Account) -> Result<()> {
        let mut mutation = self.account_mutations.lock().await;
        self.ensure_account_inactive(&account.id).await?;
        let latest = self.account_required(&account.id).await?;
        let stored: StoredAccountConfigView = from_value(latest.config)
            .map_err(|error| super::internal(error, "deserialize configured CalDAV account"))?;
        let config = account_config_value(
            stored.server_url,
            stored.username,
            Value::String(String::new()),
            stored.timeout_seconds,
            stored.allow_insecure_http,
            stored.allow_private_network,
        );
        self.store
            .update_account(&account.id, AccountUpdate {
                name: account.name,
                config,
                enabled: false,
            })
            .await
            .map_err(ConnectorManagerError::from)?;
        mutation.revision = mutation.revision.wrapping_add(1);
        Ok(())
    }
}

impl DesiredAccount {
    fn create_request(&self) -> AccountCreateRequest {
        AccountCreateRequest {
            kind: ConnectorKind::Caldav,
            channel_type: None,
            channel_account_id: None,
            himalaya_account_name: None,
            himalaya_backend: None,
            name: self.name.clone(),
            server_url: self.server_url.clone(),
            username: self.username.clone(),
            password: self.password.clone(),
            timeout_seconds: self.timeout_seconds,
            allow_insecure_http: false,
            allow_private_network: false,
            enabled: self.enabled,
        }
    }
}

fn desired_account(
    name: &str,
    account: &moltis_config::CalDavAccountConfig,
    enabled: bool,
) -> Result<DesiredAccount> {
    if name.trim().is_empty() {
        return Err(ConnectorManagerError::InvalidInput(
            "configured CalDAV account name must not be empty".to_owned(),
        ));
    }
    let source_key = format!("{SOURCE_PREFIX}{name}");
    if source_key.chars().count() > MAX_SOURCE_KEY_CHARS {
        return Err(ConnectorManagerError::InvalidInput(
            "configured CalDAV account name is too long".to_owned(),
        ));
    }
    let server_url = moltis_caldav::discovery::resolve_base_url(
        account.provider.as_deref(),
        account.url.as_deref(),
    )
    .ok_or_else(|| {
        ConnectorManagerError::InvalidInput(
            "configured CalDAV account requires a server URL".to_owned(),
        )
    })?;
    let username = account.username.clone().ok_or_else(|| {
        ConnectorManagerError::InvalidInput(
            "configured CalDAV account requires a username".to_owned(),
        )
    })?;
    let password = account.password.clone().ok_or_else(|| {
        ConnectorManagerError::InvalidInput(
            "configured CalDAV account requires a password".to_owned(),
        )
    })?;
    super::validate_password_replacement(password.expose_secret())?;
    super::validate_account_fields(&server_url, &username, account.timeout_seconds, false)?;
    Ok(DesiredAccount {
        source_key,
        name: name.to_owned(),
        server_url,
        username,
        password,
        timeout_seconds: account.timeout_seconds,
        enabled,
    })
}

fn same_server_url(left: &str, right: &str) -> bool {
    match (Url::parse(left), Url::parse(right)) {
        (Ok(left), Ok(right)) => left == right,
        _ => left == right,
    }
}

enum AccountReconciliation {
    Created,
    Adopted,
    Updated,
    Unchanged,
    Deferred,
    Ambiguous,
}
