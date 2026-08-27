//! Apple Container identifier generation.

use sha2::{Digest, Sha256};

pub(crate) const APPLE_CONTAINER_ID_MAX_LEN: usize = 64;
const DIGEST_LEN: usize = 24;
const APPLE_CONTAINER_PREFIX_MAX_LEN: usize = 27;

pub(crate) fn apple_container_prefix(prefix: &str) -> String {
    let normalized = normalize_component(prefix, "moltis-sandbox");
    compact_component(
        &normalized,
        prefix,
        APPLE_CONTAINER_PREFIX_MAX_LEN,
        normalized != prefix,
    )
}

pub fn has_apple_container_prefix(name: &str, prefix: &str) -> bool {
    let prefix = apple_container_prefix(prefix);
    name.strip_prefix(&prefix)
        .is_some_and(|remainder| remainder.starts_with('-'))
}

pub fn apple_container_name(prefix: &str, key: &str, generation: u32) -> String {
    let prefix = apple_container_prefix(prefix);
    let normalized_key = normalize_component(key, "sandbox");
    let suffix = if generation == 0 {
        String::new()
    } else {
        format!("-g{generation}")
    };
    let key_max_len = APPLE_CONTAINER_ID_MAX_LEN.saturating_sub(prefix.len() + suffix.len() + 1);
    let key = compact_component(&normalized_key, key, key_max_len, normalized_key != key);
    format!("{prefix}-{key}{suffix}")
}

fn normalize_component(value: &str, fallback: &str) -> String {
    let normalized: String = value
        .chars()
        .map(|ch| {
            if ch.is_ascii_alphanumeric() || matches!(ch, '-' | '_' | '.') {
                ch
            } else {
                '-'
            }
        })
        .collect();
    if normalized.is_empty() {
        fallback.to_string()
    } else {
        normalized
    }
}

fn compact_component(value: &str, hash_source: &str, max_len: usize, force: bool) -> String {
    if !force && value.len() <= max_len {
        return value.to_string();
    }

    let digest = format!("{:x}", Sha256::digest(hash_source.as_bytes()));
    if max_len <= DIGEST_LEN {
        return digest.get(..max_len).unwrap_or(digest.as_str()).to_string();
    }
    let readable_len = max_len - DIGEST_LEN - 1;
    let readable = value
        .get(..readable_len)
        .unwrap_or(value)
        .trim_end_matches(['-', '_', '.']);
    let short_digest = digest.get(..DIGEST_LEN).unwrap_or(digest.as_str());
    format!("{readable}-{short_digest}")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn preserves_names_that_fit() {
        assert_eq!(
            apple_container_name("moltis-test-sandbox", "session-abc", 0),
            "moltis-test-sandbox-session-abc"
        );
    }

    #[test]
    fn bounds_default_session_name() {
        let name = apple_container_name(
            "moltis-moltis-sandbox",
            "session-4fc5159e-0cb6-49f2-8eba-553ba3ec897b",
            0,
        );
        assert!(name.len() <= APPLE_CONTAINER_ID_MAX_LEN);
        assert!(name.starts_with("moltis-moltis-sandbox-session-"));
    }

    #[test]
    fn reserves_space_for_generation_suffix() {
        let name = apple_container_name(
            "moltis-moltis-sandbox",
            "session-4fc5159e-0cb6-49f2-8eba-553ba3ec897b",
            u32::MAX,
        );
        assert!(name.len() <= APPLE_CONTAINER_ID_MAX_LEN);
        assert!(name.ends_with("-g4294967295"));
    }

    #[test]
    fn long_inputs_are_stable_and_distinct() {
        let first = apple_container_name(&"prefix".repeat(20), &"key-a".repeat(20), 0);
        let repeated = apple_container_name(&"prefix".repeat(20), &"key-a".repeat(20), 0);
        let second = apple_container_name(&"prefix".repeat(20), &"key-b".repeat(20), 0);
        assert_eq!(first, repeated);
        assert_ne!(first, second);
        assert!(first.len() <= APPLE_CONTAINER_ID_MAX_LEN);
        assert!(has_apple_container_prefix(&first, &"prefix".repeat(20)));
    }

    #[test]
    fn normalized_inputs_remain_distinct() {
        let first = apple_container_name("moltis/sandbox", "session:abc", 0);
        let second = apple_container_name("moltis?sandbox", "session?abc", 0);
        assert_ne!(first, second);
        assert!(first.len() <= APPLE_CONTAINER_ID_MAX_LEN);
        assert!(has_apple_container_prefix(&first, "moltis/sandbox"));
    }
}
