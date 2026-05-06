//! NIP-01 profile metadata publishing.

use nostr_sdk::prelude::{Client, Metadata, Url};

use crate::{config::NostrProfile, error::Error};

/// Publish NIP-01 profile metadata (kind:0) for the bot identity.
pub async fn publish_profile(client: &Client, profile: &NostrProfile) -> Result<(), Error> {
    let mut metadata = Metadata::new();

    if let Some(name) = &profile.name {
        metadata = metadata.name(name);
    }
    if let Some(display_name) = &profile.display_name {
        metadata = metadata.display_name(display_name);
    }
    if let Some(about) = &profile.about {
        metadata = metadata.about(about);
    }
    if let Some(picture) = &profile.picture {
        let url = Url::parse(picture)
            .map_err(|e| Error::Config(format!("invalid profile picture URL: {e}")))?;
        metadata = metadata.picture(url);
    }
    if let Some(nip05) = &profile.nip05 {
        metadata = metadata.nip05(nip05);
    }

    client.set_metadata(&metadata).await.map_err(Error::Sdk)?;

    tracing::info!("published NIP-01 profile metadata");
    Ok(())
}
