pub(super) fn sanitize_args(args: &[String]) -> Vec<String> {
    let mut sanitized = args.to_vec();
    let mut redact_next = false;
    for arg in &mut sanitized {
        if redact_next {
            *arg = "[REDACTED]".to_string();
            redact_next = false;
            continue;
        }
        if let Some((flag, _)) = arg.split_once('=')
            && likely_secret_flag(flag)
        {
            *arg = format!("{flag}=[REDACTED]");
            continue;
        }
        redact_next = likely_secret_flag(arg);
    }
    sanitized
}

fn likely_secret_flag(value: &str) -> bool {
    let upper = value
        .trim_start_matches('-')
        .replace('-', "_")
        .to_ascii_uppercase();
    upper.contains("TOKEN")
        || upper.contains("SECRET")
        || upper.contains("PASSWORD")
        || upper.contains("API_KEY")
        || upper.ends_with("_KEY")
}

#[cfg(test)]
pub(super) fn expected_candidates(preview: &serde_json::Value) -> serde_json::Value {
    serde_json::Value::Array(
        preview["candidates"]
            .as_array()
            .into_iter()
            .flatten()
            .map(|candidate| {
                serde_json::json!({
                    "identity": candidate["identity"],
                    "digest": candidate["digest"],
                })
            })
            .collect(),
    )
}
