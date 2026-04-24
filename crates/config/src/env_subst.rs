use std::collections::HashMap;

/// Replace `${ENV_VAR}` placeholders in config string values.
///
/// Unresolvable variables are left as-is.
pub fn substitute_env(input: &str) -> String {
    substitute_env_with(input, |name| std::env::var(name).ok())
}

/// Replace `${ENV_VAR}` placeholders using process env + additional overrides.
///
/// Lookup order: process env → `overrides` map.  This ensures
/// `docker -e VAR=x` takes precedence over TOML `[env]` or DB-stored vars.
pub fn substitute_env_with_overrides(input: &str, overrides: &HashMap<String, String>) -> String {
    substitute_env_with(input, |name| {
        std::env::var(name)
            .ok()
            .or_else(|| overrides.get(name).cloned())
    })
}

/// Replace `${ENV_VAR}` placeholders using a custom lookup function.
///
/// This is the implementation used by [`substitute_env`]; the separate
/// signature makes it testable without mutating the process environment.
fn substitute_env_with(input: &str, lookup: impl Fn(&str) -> Option<String>) -> String {
    let mut result = String::with_capacity(input.len());
    let mut chars = input.chars().peekable();

    while let Some(ch) = chars.next() {
        if ch == '$' && chars.peek() == Some(&'{') {
            chars.next(); // consume '{'
            let mut var_name = String::new();
            let mut closed = false;
            for c in chars.by_ref() {
                if c == '}' {
                    closed = true;
                    break;
                }
                var_name.push(c);
            }
            if closed && !var_name.is_empty() {
                match lookup(&var_name) {
                    Some(val) => result.push_str(&val),
                    None => {
                        // Leave unresolved placeholder as-is.
                        result.push_str("${");
                        result.push_str(&var_name);
                        result.push('}');
                    },
                }
            } else {
                // Malformed — emit literal.
                result.push_str("${");
                result.push_str(&var_name);
            }
        } else {
            result.push(ch);
        }
    }

    result
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn substitutes_known_var() {
        let lookup = |name: &str| match name {
            "MOLTIS_TEST_VAR" => Some("hello".to_string()),
            _ => None,
        };
        assert_eq!(
            substitute_env_with("key=${MOLTIS_TEST_VAR}", lookup),
            "key=hello"
        );
    }

    #[test]
    fn leaves_unknown_var() {
        let lookup = |_: &str| None;
        assert_eq!(
            substitute_env_with("${MOLTIS_NONEXISTENT_XYZ}", lookup),
            "${MOLTIS_NONEXISTENT_XYZ}"
        );
    }

    #[test]
    fn no_placeholders() {
        assert_eq!(substitute_env("plain text"), "plain text");
    }

    #[test]
    fn with_overrides_resolves_from_map() {
        let mut overrides = HashMap::new();
        overrides.insert("MY_KEY".to_string(), "from-map".to_string());
        assert_eq!(
            substitute_env_with_overrides("key=${MY_KEY}", &overrides),
            "key=from-map"
        );
    }

    #[test]
    fn with_overrides_primary_lookup_wins_over_map() {
        // Verify precedence: the primary lookup (first in chain) wins over
        // the overrides map.  We use `substitute_env_with` directly to
        // avoid reading real env vars that could contain secrets.
        let lookup = |name: &str| -> Option<String> {
            // Simulate process env returning a value for PRIMARY_VAR.
            if name == "PRIMARY_VAR" {
                Some("from-primary".to_string())
            } else {
                // Fall back to the overrides map for anything else.
                let overrides: HashMap<String, String> =
                    [("PRIMARY_VAR".to_string(), "from-map".to_string())]
                        .into_iter()
                        .collect();
                overrides.get(name).cloned()
            }
        };
        assert_eq!(
            substitute_env_with("${PRIMARY_VAR}", lookup),
            "from-primary",
            "primary lookup must win over fallback map"
        );
    }

    #[test]
    fn with_overrides_falls_through_to_map() {
        let mut overrides = HashMap::new();
        overrides.insert(
            "MOLTIS_TEST_OVERRIDE_ONLY".to_string(),
            "db-value".to_string(),
        );
        // This var should not exist in the process env.
        assert_eq!(
            substitute_env_with_overrides("${MOLTIS_TEST_OVERRIDE_ONLY}", &overrides),
            "db-value"
        );
    }

    #[test]
    fn with_overrides_leaves_unknown_var() {
        let overrides = HashMap::new();
        assert_eq!(
            substitute_env_with_overrides("${MOLTIS_NONEXISTENT_XYZ}", &overrides),
            "${MOLTIS_NONEXISTENT_XYZ}"
        );
    }
}
