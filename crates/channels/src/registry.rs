use std::{
    collections::HashMap,
    sync::{Arc, RwLock as StdRwLock},
};

use {
    async_trait::async_trait,
    tokio::sync::RwLock,
    tracing::{instrument, warn},
};

use {
    super::plugin::{
        ChannelDescriptor, ChannelOutbound, ChannelPlugin, ChannelStreamOutbound, ChannelType,
        InteractiveMessage, StreamReceiver, ThreadMessage,
    },
    crate::{Error, Result, config_view::ChannelConfigView, plugin::ChannelHealthSnapshot},
};

use moltis_common::types::ReplyPayload;

#[cfg(feature = "metrics")]
use moltis_metrics::{channels as ch_metrics, gauge};

/// Production channel registry with O(1) account→plugin routing.
///
/// The registry owns all channel plugins and maintains an account index
/// for fast outbound routing. Lives in `crates/channels/` — no gateway
/// dependency.
///
/// Plugins are stored behind `tokio::sync::RwLock` because `start_account`
/// and `stop_account` are async (they may perform I/O during connection
/// setup). The account index uses `std::sync::RwLock` since it is never
/// held across await points.
pub struct ChannelRegistry {
    /// channel_type -> plugin instance (behind async RwLock for start/stop)
    plugins: HashMap<String, Arc<RwLock<dyn ChannelPlugin>>>,
    /// account_id -> channel_type for O(1) outbound routing
    account_index: StdRwLock<HashMap<String, String>>,
}

impl Default for ChannelRegistry {
    fn default() -> Self {
        Self::new()
    }
}

impl ChannelRegistry {
    pub fn new() -> Self {
        Self {
            plugins: HashMap::new(),
            account_index: StdRwLock::new(HashMap::new()),
        }
    }

    /// Register a plugin by its `id()`.
    #[instrument(skip(self, plugin))]
    pub async fn register(&mut self, plugin: Arc<RwLock<dyn ChannelPlugin>>) {
        let id = {
            let p = plugin.read().await;
            p.id().to_string()
        };
        self.plugins.insert(id, plugin);
        #[cfg(feature = "metrics")]
        gauge!(ch_metrics::ACTIVE).set(self.plugins.len() as f64);
    }

    /// Get a plugin by channel type identifier.
    pub fn get(&self, channel_type: &str) -> Option<&Arc<RwLock<dyn ChannelPlugin>>> {
        self.plugins.get(channel_type)
    }

    /// List all registered channel type identifiers.
    pub fn list(&self) -> Vec<&str> {
        self.plugins.keys().map(|s| s.as_str()).collect()
    }

    /// Start an account on the appropriate plugin and update the index.
    #[instrument(skip(self, config), fields(channel_type, account_id))]
    pub async fn start_account(
        &self,
        channel_type: &str,
        account_id: &str,
        config: serde_json::Value,
    ) -> Result<()> {
        let plugin = self
            .plugins
            .get(channel_type)
            .ok_or_else(|| Error::invalid_input(format!("unknown channel type: {channel_type}")))?;

        // Check for duplicate account_id across all channel types.
        {
            let index = self.account_index.read().unwrap_or_else(|e| e.into_inner());
            if let Some(existing_ct) = index.get(account_id)
                && existing_ct != channel_type
            {
                return Err(Error::invalid_input(format!(
                    "account_id '{account_id}' already registered under channel type '{existing_ct}'"
                )));
            }
        }

        {
            let mut p = plugin.write().await;
            p.start_account(account_id, config).await?;
        }

        {
            let mut index = self
                .account_index
                .write()
                .unwrap_or_else(|e| e.into_inner());
            index.insert(account_id.to_string(), channel_type.to_string());
        }
        Ok(())
    }

    /// Stop an account and remove it from the index.
    #[instrument(skip(self), fields(channel_type, account_id))]
    pub async fn stop_account(&self, channel_type: &str, account_id: &str) -> Result<()> {
        let plugin = self
            .plugins
            .get(channel_type)
            .ok_or_else(|| Error::invalid_input(format!("unknown channel type: {channel_type}")))?;

        {
            let mut p = plugin.write().await;
            p.stop_account(account_id).await?;
        }

        {
            let mut index = self
                .account_index
                .write()
                .unwrap_or_else(|e| e.into_inner());
            index.remove(account_id);
        }
        Ok(())
    }

    /// Register an account in the index without starting it.
    ///
    /// Used by flows that create accounts outside of `start_account`
    /// (e.g. OIDC two-phase login).
    pub fn index_account(&self, account_id: &str, channel_type: &str) {
        let mut index = self
            .account_index
            .write()
            .unwrap_or_else(|e| e.into_inner());
        index.insert(account_id.to_string(), channel_type.to_string());
    }

    /// Resolve account_id → channel_type via the index.
    pub fn resolve_channel_type(&self, account_id: &str) -> Option<String> {
        let index = self.account_index.read().unwrap_or_else(|e| e.into_inner());
        index.get(account_id).cloned()
    }

    /// Resolve an outbound sender for the given account.
    pub async fn resolve_outbound(&self, account_id: &str) -> Option<Arc<dyn ChannelOutbound>> {
        let channel_type = self.resolve_channel_type(account_id)?;
        let plugin = self.plugins.get(&channel_type)?;
        let p = plugin.read().await;
        Some(p.shared_outbound())
    }

    /// Resolve a streaming outbound sender for the given account.
    pub async fn resolve_stream(&self, account_id: &str) -> Option<Arc<dyn ChannelStreamOutbound>> {
        let channel_type = self.resolve_channel_type(account_id)?;
        let plugin = self.plugins.get(&channel_type)?;
        let p = plugin.read().await;
        Some(p.shared_stream_outbound())
    }

    /// List all active accounts as `(channel_type, account_id)` pairs.
    pub fn all_accounts(&self) -> Vec<(String, String)> {
        let index = self.account_index.read().unwrap_or_else(|e| e.into_inner());
        index
            .iter()
            .map(|(aid, ct)| (ct.clone(), aid.clone()))
            .collect()
    }

    /// Probe health of all accounts across all plugins.
    #[instrument(skip(self))]
    pub async fn status_all(&self) -> Vec<ChannelHealthSnapshot> {
        let mut results = Vec::new();
        for (channel_type, plugin) in &self.plugins {
            let account_ids = {
                let p = plugin.read().await;
                p.account_ids()
            };

            for account_id in account_ids {
                let p = plugin.read().await;
                let Some(status) = p.status() else {
                    continue;
                };
                match status.probe(&account_id).await {
                    Ok(snap) => results.push(snap),
                    Err(e) => {
                        warn!(channel_type, account_id, "health probe failed: {e}");
                        results.push(ChannelHealthSnapshot {
                            connected: false,
                            account_id: account_id.clone(),
                            details: Some(format!("probe error: {e}")),
                            extra: None,
                        });
                    },
                }
            }
        }
        results
    }

    /// Get the typed config view for an account via the registry.
    pub async fn account_config(&self, account_id: &str) -> Option<Box<dyn ChannelConfigView>> {
        let channel_type = self.resolve_channel_type(account_id)?;
        let plugin = self.plugins.get(&channel_type)?;
        let p = plugin.read().await;
        p.account_config(account_id)
    }

    /// Get the raw JSON config for an account via the registry.
    pub async fn account_config_json(&self, account_id: &str) -> Option<serde_json::Value> {
        let channel_type = self.resolve_channel_type(account_id)?;
        let plugin = self.plugins.get(&channel_type)?;
        let p = plugin.read().await;
        p.account_config_json(account_id)
    }

    /// Fetch thread messages for context injection.
    ///
    /// Resolves the plugin for the given account and calls its thread context
    /// provider. Returns an empty vec if the plugin does not support threads.
    pub async fn fetch_thread_messages(
        &self,
        account_id: &str,
        channel_id: &str,
        thread_id: &str,
        limit: usize,
    ) -> Result<Vec<ThreadMessage>> {
        let channel_type = self
            .resolve_channel_type(account_id)
            .ok_or_else(|| Error::unknown_account(account_id))?;
        let plugin = self
            .plugins
            .get(&channel_type)
            .ok_or_else(|| Error::invalid_input(format!("unknown channel type: {channel_type}")))?;
        let p = plugin.read().await;
        match p.thread_context() {
            Some(ctx) => {
                ctx.fetch_thread_messages(account_id, channel_id, thread_id, limit)
                    .await
            },
            None => Ok(Vec::new()),
        }
    }

    /// Update account config via the registry.
    pub async fn update_account_config(
        &self,
        account_id: &str,
        config: serde_json::Value,
    ) -> Result<()> {
        let channel_type = self
            .resolve_channel_type(account_id)
            .ok_or_else(|| Error::unknown_account(account_id))?;
        let plugin = self
            .plugins
            .get(&channel_type)
            .ok_or_else(|| Error::invalid_input(format!("unknown channel type: {channel_type}")))?;
        let p = plugin.read().await;
        p.update_account_config(account_id, config)
    }

    /// Retry deferred account setup for a running account.
    pub async fn retry_account_setup(&self, account_id: &str) -> Result<()> {
        let channel_type = self
            .resolve_channel_type(account_id)
            .ok_or_else(|| Error::unknown_account(account_id))?;
        let plugin = self
            .plugins
            .get(&channel_type)
            .ok_or_else(|| Error::invalid_input(format!("unknown channel type: {channel_type}")))?;
        let mut p = plugin.write().await;
        p.retry_account_setup(account_id).await
    }

    /// Returns descriptors for all registered channel types.
    ///
    /// Parses each registered plugin ID as a [`ChannelType`] and returns its
    /// static descriptor. Unknown plugin IDs are silently skipped.
    pub fn descriptors(&self) -> Vec<ChannelDescriptor> {
        self.plugins
            .keys()
            .filter_map(|id| id.parse::<ChannelType>().ok())
            .map(|ct| ct.descriptor())
            .collect()
    }
}

// ── RegistryOutboundRouter ──────────────────────────────────────────────────

/// Outbound router that delegates to the correct plugin via the registry index.
///
/// Replaces `MultiChannelOutbound` from the gateway. Lives in
/// `crates/channels/` — no gateway dependency.
pub struct RegistryOutboundRouter {
    registry: Arc<ChannelRegistry>,
}

impl RegistryOutboundRouter {
    pub fn new(registry: Arc<ChannelRegistry>) -> Self {
        Self { registry }
    }
}

#[async_trait]
impl ChannelOutbound for RegistryOutboundRouter {
    async fn send_text(
        &self,
        account_id: &str,
        to: &str,
        text: &str,
        reply_to: Option<&str>,
    ) -> Result<()> {
        let outbound = self
            .registry
            .resolve_outbound(account_id)
            .await
            .ok_or_else(|| Error::unknown_account(account_id))?;
        outbound.send_text(account_id, to, text, reply_to).await
    }

    async fn send_media(
        &self,
        account_id: &str,
        to: &str,
        payload: &ReplyPayload,
        reply_to: Option<&str>,
    ) -> Result<()> {
        let outbound = self
            .registry
            .resolve_outbound(account_id)
            .await
            .ok_or_else(|| Error::unknown_account(account_id))?;
        outbound.send_media(account_id, to, payload, reply_to).await
    }

    async fn send_typing(&self, account_id: &str, to: &str) -> Result<()> {
        let outbound = self
            .registry
            .resolve_outbound(account_id)
            .await
            .ok_or_else(|| Error::unknown_account(account_id))?;
        outbound.send_typing(account_id, to).await
    }

    async fn send_text_with_suffix(
        &self,
        account_id: &str,
        to: &str,
        text: &str,
        suffix_html: &str,
        reply_to: Option<&str>,
    ) -> Result<()> {
        let outbound = self
            .registry
            .resolve_outbound(account_id)
            .await
            .ok_or_else(|| Error::unknown_account(account_id))?;
        outbound
            .send_text_with_suffix(account_id, to, text, suffix_html, reply_to)
            .await
    }

    async fn send_html(
        &self,
        account_id: &str,
        to: &str,
        html: &str,
        reply_to: Option<&str>,
    ) -> Result<()> {
        let outbound = self
            .registry
            .resolve_outbound(account_id)
            .await
            .ok_or_else(|| Error::unknown_account(account_id))?;
        outbound.send_html(account_id, to, html, reply_to).await
    }

    async fn send_text_silent(
        &self,
        account_id: &str,
        to: &str,
        text: &str,
        reply_to: Option<&str>,
    ) -> Result<()> {
        let outbound = self
            .registry
            .resolve_outbound(account_id)
            .await
            .ok_or_else(|| Error::unknown_account(account_id))?;
        outbound
            .send_text_silent(account_id, to, text, reply_to)
            .await
    }

    async fn send_location(
        &self,
        account_id: &str,
        to: &str,
        latitude: f64,
        longitude: f64,
        title: Option<&str>,
        reply_to: Option<&str>,
    ) -> Result<()> {
        let outbound = self
            .registry
            .resolve_outbound(account_id)
            .await
            .ok_or_else(|| Error::unknown_account(account_id))?;
        outbound
            .send_location(account_id, to, latitude, longitude, title, reply_to)
            .await
    }

    async fn send_interactive(
        &self,
        account_id: &str,
        to: &str,
        message: &InteractiveMessage,
        reply_to: Option<&str>,
    ) -> Result<()> {
        let outbound = self
            .registry
            .resolve_outbound(account_id)
            .await
            .ok_or_else(|| Error::unknown_account(account_id))?;
        outbound
            .send_interactive(account_id, to, message, reply_to)
            .await
    }

    async fn add_reaction(
        &self,
        account_id: &str,
        channel_id: &str,
        message_id: &str,
        emoji: &str,
    ) -> Result<()> {
        let outbound = self
            .registry
            .resolve_outbound(account_id)
            .await
            .ok_or_else(|| Error::unknown_account(account_id))?;
        outbound
            .add_reaction(account_id, channel_id, message_id, emoji)
            .await
    }

    async fn remove_reaction(
        &self,
        account_id: &str,
        channel_id: &str,
        message_id: &str,
        emoji: &str,
    ) -> Result<()> {
        let outbound = self
            .registry
            .resolve_outbound(account_id)
            .await
            .ok_or_else(|| Error::unknown_account(account_id))?;
        outbound
            .remove_reaction(account_id, channel_id, message_id, emoji)
            .await
    }
}

#[async_trait]
impl ChannelStreamOutbound for RegistryOutboundRouter {
    async fn send_stream(
        &self,
        account_id: &str,
        to: &str,
        reply_to: Option<&str>,
        stream: StreamReceiver,
    ) -> Result<()> {
        let stream_out = self
            .registry
            .resolve_stream(account_id)
            .await
            .ok_or_else(|| Error::unknown_account(account_id))?;
        stream_out
            .send_stream(account_id, to, reply_to, stream)
            .await
    }

    async fn is_stream_enabled(&self, account_id: &str) -> bool {
        let Some(stream_out) = self.registry.resolve_stream(account_id).await else {
            return false;
        };
        stream_out.is_stream_enabled(account_id).await
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use {
        super::*,
        crate::{
            gating::{DmPolicy, GroupPolicy},
            plugin::{ChannelStatus, StreamEvent},
        },
        tokio::sync::mpsc,
    };

    /// Minimal config view for testing contract tests.
    #[derive(Debug)]
    struct TestConfigView;

    impl ChannelConfigView for TestConfigView {
        fn allowlist(&self) -> &[String] {
            &[]
        }

        fn group_allowlist(&self) -> &[String] {
            &[]
        }

        fn dm_policy(&self) -> DmPolicy {
            DmPolicy::default()
        }

        fn group_policy(&self) -> GroupPolicy {
            GroupPolicy::default()
        }

        fn model(&self) -> Option<&str> {
            None
        }

        fn model_provider(&self) -> Option<&str> {
            None
        }
    }

    /// Minimal plugin for testing.
    struct TestPlugin {
        id: String,
        accounts: std::sync::Mutex<HashMap<String, serde_json::Value>>,
        outbound: NullOutbound,
    }

    impl TestPlugin {
        fn new(id: &str) -> Self {
            Self {
                id: id.to_string(),
                accounts: std::sync::Mutex::new(HashMap::new()),
                outbound: NullOutbound,
            }
        }
    }

    #[async_trait]
    impl ChannelPlugin for TestPlugin {
        fn id(&self) -> &str {
            &self.id
        }

        fn name(&self) -> &str {
            &self.id
        }

        async fn start_account(
            &mut self,
            account_id: &str,
            config: serde_json::Value,
        ) -> Result<()> {
            self.accounts
                .lock()
                .unwrap()
                .insert(account_id.to_string(), config);
            Ok(())
        }

        async fn stop_account(&mut self, account_id: &str) -> Result<()> {
            self.accounts.lock().unwrap().remove(account_id);
            Ok(())
        }

        fn outbound(&self) -> Option<&dyn ChannelOutbound> {
            Some(&self.outbound)
        }

        fn status(&self) -> Option<&dyn ChannelStatus> {
            Some(self)
        }

        fn has_account(&self, account_id: &str) -> bool {
            self.accounts.lock().unwrap().contains_key(account_id)
        }

        fn account_ids(&self) -> Vec<String> {
            self.accounts.lock().unwrap().keys().cloned().collect()
        }

        fn account_config(&self, account_id: &str) -> Option<Box<dyn ChannelConfigView>> {
            if self.has_account(account_id) {
                Some(Box::new(TestConfigView))
            } else {
                None
            }
        }

        fn update_account_config(
            &self,
            _account_id: &str,
            _config: serde_json::Value,
        ) -> Result<()> {
            Ok(())
        }

        fn shared_outbound(&self) -> Arc<dyn ChannelOutbound> {
            Arc::new(NullOutbound)
        }

        fn shared_stream_outbound(&self) -> Arc<dyn ChannelStreamOutbound> {
            Arc::new(NullStreamOutbound)
        }
    }

    #[async_trait]
    impl ChannelStatus for TestPlugin {
        async fn probe(&self, account_id: &str) -> Result<ChannelHealthSnapshot> {
            Ok(ChannelHealthSnapshot {
                connected: self.has_account(account_id),
                account_id: account_id.to_string(),
                details: None,
                extra: None,
            })
        }
    }

    struct NullOutbound;

    #[async_trait]
    impl ChannelOutbound for NullOutbound {
        async fn send_text(&self, _: &str, _: &str, _: &str, _: Option<&str>) -> Result<()> {
            Ok(())
        }

        async fn send_media(
            &self,
            _: &str,
            _: &str,
            _: &ReplyPayload,
            _: Option<&str>,
        ) -> Result<()> {
            Ok(())
        }
    }

    struct NullStreamOutbound;

    #[async_trait]
    impl ChannelStreamOutbound for NullStreamOutbound {
        async fn send_stream(
            &self,
            _: &str,
            _: &str,
            _: Option<&str>,
            mut stream: StreamReceiver,
        ) -> Result<()> {
            while let Some(event) = stream.recv().await {
                if matches!(event, StreamEvent::Done | StreamEvent::Error(_)) {
                    break;
                }
            }
            Ok(())
        }
    }

    #[tokio::test]
    async fn register_and_list() {
        let mut registry = ChannelRegistry::new();
        let plugin = Arc::new(RwLock::new(TestPlugin::new("telegram")));
        registry.register(plugin).await;

        let types = registry.list();
        assert_eq!(types, vec!["telegram"]);
    }

    #[tokio::test]
    async fn start_and_resolve_account() {
        let mut registry = ChannelRegistry::new();
        registry
            .register(Arc::new(RwLock::new(TestPlugin::new("telegram"))))
            .await;

        registry
            .start_account("telegram", "bot1", serde_json::json!({}))
            .await
            .unwrap();

        assert_eq!(
            registry.resolve_channel_type("bot1"),
            Some("telegram".into())
        );
    }

    #[tokio::test]
    async fn stop_removes_from_index() {
        let mut registry = ChannelRegistry::new();
        registry
            .register(Arc::new(RwLock::new(TestPlugin::new("telegram"))))
            .await;

        registry
            .start_account("telegram", "bot1", serde_json::json!({}))
            .await
            .unwrap();
        assert!(registry.resolve_channel_type("bot1").is_some());

        registry.stop_account("telegram", "bot1").await.unwrap();
        assert!(registry.resolve_channel_type("bot1").is_none());
    }

    #[tokio::test]
    async fn duplicate_account_id_different_channel_rejected() {
        let mut registry = ChannelRegistry::new();
        registry
            .register(Arc::new(RwLock::new(TestPlugin::new("telegram"))))
            .await;
        registry
            .register(Arc::new(RwLock::new(TestPlugin::new("discord"))))
            .await;

        registry
            .start_account("telegram", "shared-id", serde_json::json!({}))
            .await
            .unwrap();

        let result = registry
            .start_account("discord", "shared-id", serde_json::json!({}))
            .await;
        assert!(result.is_err());
    }

    #[tokio::test]
    async fn same_account_id_same_channel_allowed() {
        let mut registry = ChannelRegistry::new();
        registry
            .register(Arc::new(RwLock::new(TestPlugin::new("telegram"))))
            .await;

        registry
            .start_account("telegram", "bot1", serde_json::json!({}))
            .await
            .unwrap();

        // Re-registering same account_id on same channel type should succeed.
        let result = registry
            .start_account("telegram", "bot1", serde_json::json!({}))
            .await;
        assert!(result.is_ok());
    }

    #[tokio::test]
    async fn unknown_channel_type_errors() {
        let registry = ChannelRegistry::new();
        let result = registry
            .start_account("slack", "bot1", serde_json::json!({}))
            .await;
        assert!(result.is_err());
    }

    #[tokio::test]
    async fn all_accounts_lists_pairs() {
        let mut registry = ChannelRegistry::new();
        registry
            .register(Arc::new(RwLock::new(TestPlugin::new("telegram"))))
            .await;
        registry
            .register(Arc::new(RwLock::new(TestPlugin::new("discord"))))
            .await;

        registry
            .start_account("telegram", "tg1", serde_json::json!({}))
            .await
            .unwrap();
        registry
            .start_account("discord", "dc1", serde_json::json!({}))
            .await
            .unwrap();

        let mut accounts = registry.all_accounts();
        accounts.sort();
        assert_eq!(accounts, vec![
            ("discord".into(), "dc1".into()),
            ("telegram".into(), "tg1".into()),
        ]);
    }

    #[tokio::test]
    async fn outbound_router_delegates() {
        let mut registry = ChannelRegistry::new();
        registry
            .register(Arc::new(RwLock::new(TestPlugin::new("telegram"))))
            .await;

        registry
            .start_account("telegram", "bot1", serde_json::json!({}))
            .await
            .unwrap();

        let registry = Arc::new(registry);
        let router = RegistryOutboundRouter::new(Arc::clone(&registry));

        // Should resolve and delegate (NullOutbound returns Ok)
        let result = router.send_text("bot1", "42", "hello", None).await;
        assert!(result.is_ok());
    }

    #[tokio::test]
    async fn outbound_router_unknown_account_errors() {
        let registry = Arc::new(ChannelRegistry::new());
        let router = RegistryOutboundRouter::new(registry);

        let result = router.send_text("nonexistent", "42", "hello", None).await;
        assert!(result.is_err());
    }

    #[tokio::test]
    async fn stream_router_delegates() {
        let mut registry = ChannelRegistry::new();
        registry
            .register(Arc::new(RwLock::new(TestPlugin::new("telegram"))))
            .await;

        registry
            .start_account("telegram", "bot1", serde_json::json!({}))
            .await
            .unwrap();

        let registry = Arc::new(registry);
        let router = RegistryOutboundRouter::new(Arc::clone(&registry));

        let (tx, rx) = mpsc::channel(8);
        tx.send(StreamEvent::Done).await.unwrap();
        drop(tx);

        let result = router.send_stream("bot1", "42", None, rx).await;
        assert!(result.is_ok());
    }

    #[tokio::test]
    async fn stream_enabled_unknown_account() {
        let registry = Arc::new(ChannelRegistry::new());
        let router = RegistryOutboundRouter::new(registry);
        assert!(!router.is_stream_enabled("nonexistent").await);
    }

    #[tokio::test]
    async fn resolve_outbound_returns_none_for_unknown() {
        let registry = ChannelRegistry::new();
        assert!(registry.resolve_outbound("unknown").await.is_none());
    }

    #[tokio::test]
    async fn resolve_stream_returns_none_for_unknown() {
        let registry = ChannelRegistry::new();
        assert!(registry.resolve_stream("unknown").await.is_none());
    }

    #[tokio::test]
    async fn descriptors_returns_registered_types() {
        let mut registry = ChannelRegistry::new();
        registry
            .register(Arc::new(RwLock::new(TestPlugin::new("telegram"))))
            .await;
        registry
            .register(Arc::new(RwLock::new(TestPlugin::new("discord"))))
            .await;

        let mut descs: Vec<String> = registry
            .descriptors()
            .iter()
            .map(|d| d.channel_type.to_string())
            .collect();
        descs.sort();
        assert_eq!(descs, vec!["discord", "telegram"]);
    }

    #[test]
    fn descriptors_empty_registry() {
        let registry = ChannelRegistry::new();
        assert!(registry.descriptors().is_empty());
    }

    // ── Contract tests ──────────────────────────────────────────────────────

    #[tokio::test]
    async fn contract_lifecycle_start_stop() {
        let mut plugin = TestPlugin::new("test");
        crate::contract::lifecycle_start_stop(&mut plugin)
            .await
            .unwrap();
    }

    #[tokio::test]
    async fn contract_double_start_same_account() {
        let mut plugin = TestPlugin::new("test");
        crate::contract::double_start_same_account(&mut plugin)
            .await
            .unwrap();
    }

    #[tokio::test]
    async fn contract_stop_unknown_account() {
        let mut plugin = TestPlugin::new("test");
        crate::contract::stop_unknown_account(&mut plugin)
            .await
            .unwrap();
    }

    #[tokio::test]
    async fn contract_config_view_after_start() {
        let mut plugin = TestPlugin::new("test");
        crate::contract::config_view_after_start(&mut plugin)
            .await
            .unwrap();
    }

    #[tokio::test]
    async fn contract_outbound_available_after_start() {
        let mut plugin = TestPlugin::new("test");
        crate::contract::outbound_available_after_start(&mut plugin)
            .await
            .unwrap();
    }

    #[tokio::test]
    async fn contract_shared_outbound_send_succeeds_after_start() {
        let mut plugin = TestPlugin::new("test");
        crate::contract::shared_outbound_send_succeeds_after_start(&mut plugin)
            .await
            .unwrap();
    }

    #[tokio::test]
    async fn contract_stream_completes_on_done_signal() {
        let mut plugin = TestPlugin::new("test");
        crate::contract::stream_completes_on_done_signal(&mut plugin)
            .await
            .unwrap();
    }

    #[tokio::test]
    async fn contract_stream_completes_on_error_signal() {
        let mut plugin = TestPlugin::new("test");
        crate::contract::stream_completes_on_error_signal(&mut plugin)
            .await
            .unwrap();
    }

    #[tokio::test]
    async fn contract_outbound_error_classification() {
        let mut plugin = TestPlugin::new("test");
        crate::contract::outbound_error_classification(&mut plugin)
            .await
            .unwrap();
    }

    #[tokio::test]
    async fn contract_probe_unknown_account_returns_disconnected() {
        let plugin = TestPlugin::new("test");
        crate::contract::probe_unknown_account_returns_disconnected(&plugin)
            .await
            .unwrap();
    }

    #[tokio::test]
    async fn contract_probe_started_account_returns_connected() {
        let mut plugin = TestPlugin::new("test");
        crate::contract::probe_started_account_returns_connected(&mut plugin)
            .await
            .unwrap();
    }
}
