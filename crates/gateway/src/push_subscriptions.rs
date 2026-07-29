use anyhow::Result;

use super::{
    MAX_SUBSCRIPTIONS, PushService, PushSubscription, PushSubscriptionValidationError,
    RevokedEndpoint, validate_endpoint, validate_endpoint_destination,
};

impl PushService {
    /// Add a subscription without reviving a remotely revoked endpoint.
    pub async fn add_subscription(
        &self,
        sub: PushSubscription,
        replaces: Option<&str>,
    ) -> Result<()> {
        self.add_subscription_with_revival(sub, replaces, false)
            .await
    }

    /// Add a subscription, optionally reviving the endpoint being registered.
    ///
    /// `replaces` carries the endpoint a browser rotated away from. Revival
    /// never clears that endpoint's tombstone: only the newly registered
    /// endpoint represents an explicit enable action.
    pub async fn add_subscription_with_revival(
        &self,
        sub: PushSubscription,
        replaces: Option<&str>,
        revive: bool,
    ) -> Result<()> {
        let endpoint = Self::validate_subscription_shape(&sub)?;
        if let Some(old) = replaces {
            // A replacement is an opaque deletion key and need not resolve.
            validate_endpoint(old)?;
        }
        {
            let store = self.store.read().await;
            if !revive && store.is_revoked(&sub.endpoint, replaces) {
                return Err(PushSubscriptionValidationError::Revoked.into());
            }
        }
        validate_endpoint_destination(&endpoint).await?;

        let mut store = self.store.write().await;
        if !revive && store.is_revoked(&sub.endpoint, replaces) {
            return Err(PushSubscriptionValidationError::Revoked.into());
        }

        let mut candidate = store.clone();
        let retained = candidate
            .subscriptions
            .iter()
            .filter(|existing| {
                existing.endpoint != sub.endpoint
                    && replaces.is_none_or(|old| existing.endpoint != old)
            })
            .count();
        if retained >= MAX_SUBSCRIPTIONS {
            return Err(PushSubscriptionValidationError::LimitReached.into());
        }
        if revive {
            candidate
                .revoked_endpoints
                .retain(|revoked| revoked.endpoint != sub.endpoint);
        }
        candidate
            .subscriptions
            .retain(|existing| existing.endpoint != sub.endpoint);
        if let Some(old) = replaces {
            candidate
                .subscriptions
                .retain(|existing| existing.endpoint != old);
        }
        candidate.subscriptions.push(sub);

        self.commit_store(&mut store, candidate).await?;
        if let Some(old) = replaces {
            self.presence.write().await.remove(old);
        }
        tracing::info!("Added push subscription");
        Ok(())
    }

    /// Remove a subscription and durably revoke its endpoint.
    pub async fn remove_subscription(&self, endpoint: &str) -> Result<()> {
        validate_endpoint(endpoint)?;
        let mut store = self.store.write().await;
        let mut candidate = store.clone();
        let removed = candidate.subscriptions.len();
        candidate
            .subscriptions
            .retain(|subscription| subscription.endpoint != endpoint);
        let removed = candidate.subscriptions.len() < removed;
        if !candidate
            .revoked_endpoints
            .iter()
            .any(|revoked| revoked.endpoint == endpoint)
        {
            candidate.revoked_endpoints.push(RevokedEndpoint {
                endpoint: endpoint.to_string(),
                revoked_at: chrono::Utc::now(),
            });
        }

        self.commit_store(&mut store, candidate).await?;
        self.presence.write().await.remove(endpoint);
        if removed {
            tracing::info!("Removed push subscription");
        }
        Ok(())
    }
}
