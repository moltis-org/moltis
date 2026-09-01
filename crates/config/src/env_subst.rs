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
    if !input.contains("${") {
        return input.to_string();
    }

    let mut result = String::with_capacity(input.len());
    let mut cursor = 0;

    while let Some(relative_start) = input[cursor..].find("${") {
        let start = cursor + relative_start;
        result.push_str(&input[cursor..start]);

        let name_start = start + 2;
        let Some(relative_end) = input[name_start..].find('}') else {
            // Malformed — emit literal rest of input, matching the previous
            // char-by-char behavior that consumed the unterminated placeholder.
            result.push_str("${");
            result.push_str(&input[name_start..]);
            return result;
        };

        let end = name_start + relative_end;
        let var_name = &input[name_start..end];
        if var_name.is_empty() {
            // Malformed — emit literal without the consumed closing brace.
            result.push_str("${");
        } else if let Some(value) = lookup(var_name) {
            result.push_str(&value);
        } else {
            // Leave unresolved placeholder as-is.
            result.push_str("${");
            result.push_str(var_name);
            result.push('}');
        }

        cursor = end + 1;
    }

    result.push_str(&input[cursor..]);

    result
}

/// Replace braced and bare environment placeholders using only explicit overrides.
pub fn substitute_env_placeholders(input: &str, overrides: &HashMap<String, String>) -> String {
    let mut out = String::with_capacity(input.len());
    let bytes = input.as_bytes();
    let mut cursor = 0usize;

    while let Some(relative_dollar) = input[cursor..].find('$') {
        let dollar_idx = cursor + relative_dollar;
        out.push_str(&input[cursor..dollar_idx]);

        let after_dollar = dollar_idx + 1;
        if after_dollar >= input.len() {
            out.push('$');
            cursor = after_dollar;
            break;
        }

        if bytes[after_dollar] == b'{' {
            let name_start = after_dollar + 1;
            let Some(relative_close) = input[name_start..].find('}') else {
                out.push_str(&input[dollar_idx..]);
                cursor = input.len();
                break;
            };
            let close_idx = name_start + relative_close;
            if close_idx > name_start {
                let name = &input[name_start..close_idx];
                if let Some(value) = lookup_override(name, overrides) {
                    out.push_str(value);
                } else {
                    out.push_str(&input[dollar_idx..=close_idx]);
                }
                cursor = close_idx + 1;
                continue;
            }

            out.push_str(&input[dollar_idx..]);
            break;
        }

        let next = bytes[after_dollar];
        if !is_env_ident_start(next as char) {
            out.push('$');
            cursor = after_dollar;
            continue;
        }

        let name_start = after_dollar;
        let mut name_end = name_start + 1;
        while name_end < input.len() && is_env_ident_continue(bytes[name_end] as char) {
            name_end += 1;
        }

        let name = &input[name_start..name_end];
        if let Some(value) = lookup_override(name, overrides) {
            out.push_str(value);
        } else {
            out.push_str(&input[dollar_idx..name_end]);
        }
        cursor = name_end;
    }

    out.push_str(&input[cursor..]);
    out
}

#[must_use]
pub fn contains_env_placeholder(input: &str) -> bool {
    let bytes = input.as_bytes();
    input.match_indices('$').any(|(dollar_idx, _)| {
        let after_dollar = dollar_idx + 1;
        if after_dollar >= input.len() {
            return false;
        }

        if bytes[after_dollar] == b'{' {
            let name_start = after_dollar + 1;
            return input[name_start..]
                .find('}')
                .is_some_and(|relative_close| relative_close > 0);
        }

        is_env_ident_start(bytes[after_dollar] as char)
    })
}

#[must_use]
pub fn is_entire_env_placeholder(value: &str) -> bool {
    let trimmed = value.trim();
    if trimmed.is_empty() {
        return false;
    }

    if let Some(name) = trimmed
        .strip_prefix("${")
        .and_then(|rest| rest.strip_suffix('}'))
    {
        return !name.is_empty() && name.chars().all(is_env_ident_continue);
    }

    trimmed
        .strip_prefix('$')
        .is_some_and(|name| !name.is_empty() && name.chars().all(is_env_ident_continue))
}

fn lookup_override<'a>(name: &str, overrides: &'a HashMap<String, String>) -> Option<&'a str> {
    overrides
        .get(name)
        .map(String::as_str)
        .filter(|value| !value.trim().is_empty())
}

fn is_env_ident_start(ch: char) -> bool {
    ch == '_' || ch.is_ascii_alphabetic()
}

fn is_env_ident_continue(ch: char) -> bool {
    ch == '_' || ch.is_ascii_alphanumeric()
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

    #[test]
    fn explicit_placeholders_support_braced_and_bare_keys() {
        let overrides = HashMap::from([
            ("ONE".to_string(), "first".to_string()),
            ("TWO".to_string(), "second".to_string()),
        ]);

        assert_eq!(
            substitute_env_placeholders("x=${ONE}&y=$TWO", &overrides),
            "x=first&y=second"
        );
    }

    #[test]
    fn detects_unresolved_placeholders() {
        assert!(contains_env_placeholder("$MCP_URL"));
        assert!(contains_env_placeholder("https://${MCP_HOST}/mcp"));
        assert!(!contains_env_placeholder("https://example.com/mcp"));
        assert!(is_entire_env_placeholder("${MCP_URL}"));
        assert!(!is_entire_env_placeholder("https://${MCP_HOST}/mcp"));
    }
}
