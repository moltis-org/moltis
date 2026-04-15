//! DM access control for Nostr channels.
//!
//! Reuses `DmPolicy` from `moltis_channels::gating` and maps Nostr pubkeys
//! to the allowlist/OTP gating model used by other channels.

use {moltis_channels::gating::DmPolicy, nostr_sdk::prelude::PublicKey};

/// Check whether a sender pubkey is authorized under the given DM policy.
///
/// `allowed` should be pre-parsed pubkeys (cached at config-load time) to
/// avoid re-parsing on every inbound message.
pub fn check_dm_access(
    sender: &PublicKey,
    policy: &DmPolicy,
    allowed: &[PublicKey],
) -> Result<(), AccessDenied> {
    match policy {
        DmPolicy::Disabled => Err(AccessDenied::Disabled),
        DmPolicy::Open => Ok(()),
        DmPolicy::Allowlist => {
            if allowed.iter().any(|pk| pk == sender) {
                Ok(())
            } else {
                Err(AccessDenied::NotAllowlisted)
            }
        },
    }
}

/// Reason a DM was denied.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AccessDenied {
    /// DMs are globally disabled for this account.
    Disabled,
    /// Sender pubkey is not in the allowlist.
    NotAllowlisted,
}

impl std::fmt::Display for AccessDenied {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Disabled => write!(f, "DMs are disabled"),
            Self::NotAllowlisted => write!(f, "sender not in allowlist"),
        }
    }
}

#[cfg(test)]
mod tests {
    use {moltis_channels::gating::DmPolicy, nostr_sdk::prelude::Keys};

    use super::*;

    #[test]
    fn open_policy_allows_anyone() {
        let sender = Keys::generate().public_key();
        let result = check_dm_access(&sender, &DmPolicy::Open, &[]);
        assert!(result.is_ok());
    }

    #[test]
    fn disabled_policy_denies_everyone() {
        let sender = Keys::generate().public_key();
        let result = check_dm_access(&sender, &DmPolicy::Disabled, &[]);
        assert_eq!(result, Err(AccessDenied::Disabled));
    }

    #[test]
    fn allowlist_policy_allows_listed() {
        let keys = Keys::generate();
        let allowed = vec![keys.public_key()];
        let result = check_dm_access(&keys.public_key(), &DmPolicy::Allowlist, &allowed);
        assert!(result.is_ok());
    }

    #[test]
    fn allowlist_policy_denies_unlisted() {
        let sender = Keys::generate().public_key();
        let other = Keys::generate().public_key();
        let allowed = vec![other];
        let result = check_dm_access(&sender, &DmPolicy::Allowlist, &allowed);
        assert_eq!(result, Err(AccessDenied::NotAllowlisted));
    }

    #[test]
    fn empty_allowlist_denies_all() {
        let sender = Keys::generate().public_key();
        let result = check_dm_access(&sender, &DmPolicy::Allowlist, &[]);
        assert_eq!(result, Err(AccessDenied::NotAllowlisted));
    }
}
