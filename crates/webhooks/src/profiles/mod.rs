//! Source profile implementations for different webhook providers.

pub mod generic;
pub mod github;
pub mod gitlab;
pub mod stripe;

use axum::http::HeaderMap;

use crate::types::{AuthMode, EventCatalogEntry, NormalizedPayload, ProfileSummary};

/// A source profile defines how to authenticate, parse, filter, normalize,
/// and respond to events from a specific webhook provider.
pub trait SourceProfile: Send + Sync {
    /// Profile identifier used in config and database.
    fn id(&self) -> &str;

    /// Human-readable name for the UI.
    fn display_name(&self) -> &str;

    /// Default auth mode for this profile.
    fn default_auth_mode(&self) -> AuthMode;

    /// Known event types with descriptions for UI filter checkboxes.
    fn event_catalog(&self) -> Vec<EventCatalogEntry>;

    /// Extract event type from request headers and/or body.
    fn parse_event_type(&self, headers: &HeaderMap, body: &[u8]) -> Option<String>;

    /// Extract idempotency key from request.
    fn parse_delivery_key(&self, headers: &HeaderMap, body: &[u8]) -> Option<String>;

    /// Extract entity key for per_entity session grouping.
    fn entity_key(&self, event_type: &str, body: &serde_json::Value) -> Option<String>;

    /// Produce a structured summary of the payload for the agent prompt.
    fn normalize_payload(&self, event_type: &str, body: &serde_json::Value) -> NormalizedPayload;

    /// Whether this profile provides response tools.
    fn has_response_tools(&self) -> bool {
        false
    }

    /// UI setup guidance markdown.
    fn setup_guide(&self) -> &str;

    /// Build a summary for the profiles list RPC.
    fn summary(&self) -> ProfileSummary {
        ProfileSummary {
            id: self.id().into(),
            display_name: self.display_name().into(),
            default_auth_mode: self.default_auth_mode(),
            event_catalog: self.event_catalog(),
            has_response_tools: self.has_response_tools(),
            setup_guide: self.setup_guide().into(),
        }
    }
}

/// Registry of all available source profiles.
pub struct ProfileRegistry {
    profiles: Vec<Box<dyn SourceProfile>>,
}

impl ProfileRegistry {
    /// Create registry with all built-in profiles.
    pub fn new() -> Self {
        Self {
            profiles: vec![
                Box::new(generic::GenericProfile),
                Box::new(github::GitHubProfile),
                Box::new(gitlab::GitLabProfile),
                Box::new(stripe::StripeProfile),
            ],
        }
    }

    /// Look up a profile by ID.
    pub fn get(&self, id: &str) -> Option<&dyn SourceProfile> {
        self.profiles
            .iter()
            .find(|p| p.id() == id)
            .map(|p| p.as_ref())
    }

    /// List all available profiles.
    pub fn list(&self) -> Vec<ProfileSummary> {
        self.profiles.iter().map(|p| p.summary()).collect()
    }
}

impl Default for ProfileRegistry {
    fn default() -> Self {
        Self::new()
    }
}
