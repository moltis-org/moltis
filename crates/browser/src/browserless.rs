use {
    base64::{Engine as _, engine::general_purpose::STANDARD},
    serde::Serialize,
    url::Url,
};

use crate::{error::Error, types::BrowserlessApiVersion};

type Result<T> = std::result::Result<T, Error>;

#[derive(Serialize)]
struct LaunchOptions<'a> {
    args: &'a [String],
}

/// Compute the Browserless session timeout from Moltis pool lifecycle settings.
pub(crate) fn session_timeout_ms(
    idle_timeout_secs: u64,
    navigation_timeout_ms: u64,
    max_instance_lifetime_secs: u64,
) -> u64 {
    let ceiling_ms = max_instance_lifetime_secs.saturating_mul(1000);
    idle_timeout_secs
        .max(max_instance_lifetime_secs)
        .saturating_mul(1000)
        .max(navigation_timeout_ms)
        .min(ceiling_ms)
}

/// Build the environment passed to a Browserless container.
///
/// V1 receives both timeout names because older images only understand
/// `CONNECTION_TIMEOUT`, while keeping `TIMEOUT` preserves compatibility with
/// v1-compatible images that already accepted the newer alias. V2 launch
/// options are sent on the websocket URL rather than through removed
/// `DEFAULT_LAUNCH_ARGS` and `PREBOOT_CHROME` variables.
pub(crate) fn container_env(
    api_version: BrowserlessApiVersion,
    timeout_ms: u64,
    launch_args: &[String],
) -> Result<Vec<String>> {
    match api_version {
        BrowserlessApiVersion::V1 => {
            let launch_args = serde_json::to_string(launch_args).map_err(|error| {
                Error::LaunchFailed(format!(
                    "failed to serialize Browserless v1 launch arguments: {error}"
                ))
            })?;
            Ok(vec![
                format!("DEFAULT_LAUNCH_ARGS={launch_args}"),
                format!("TIMEOUT={timeout_ms}"),
                format!("CONNECTION_TIMEOUT={timeout_ms}"),
                "MAX_CONCURRENT_SESSIONS=1".to_string(),
                "PREBOOT_CHROME=true".to_string(),
            ])
        },
        BrowserlessApiVersion::V2 => Ok(vec![
            format!("TIMEOUT={timeout_ms}"),
            "CONCURRENT=1".to_string(),
        ]),
    }
}

/// Build the Browserless websocket URL for the configured API version.
pub(crate) fn websocket_url(
    host: &str,
    port: u16,
    api_version: BrowserlessApiVersion,
    launch_args: &[String],
) -> Result<String> {
    let base_url = format!("ws://{host}:{port}");
    if api_version == BrowserlessApiVersion::V1 {
        return Ok(base_url);
    }

    let mut url = Url::parse(&base_url).map_err(|error| {
        Error::LaunchFailed(format!(
            "failed to build Browserless websocket URL from {base_url}: {error}"
        ))
    })?;
    let launch = serde_json::to_vec(&LaunchOptions { args: launch_args }).map_err(|error| {
        Error::LaunchFailed(format!(
            "failed to serialize Browserless v2 launch options: {error}"
        ))
    })?;
    let encoded_launch = STANDARD.encode(launch);
    url.query_pairs_mut().append_pair("launch", &encoded_launch);
    Ok(url.to_string())
}

#[cfg(test)]
#[allow(clippy::expect_used, clippy::unwrap_used)]
mod tests {
    use super::*;

    #[test]
    fn session_timeout_uses_moltis_lifecycle_floor() {
        assert_eq!(session_timeout_ms(300, 30_000, 1800), 1_800_000);
    }

    #[test]
    fn session_timeout_caps_at_max_lifetime() {
        assert_eq!(session_timeout_ms(3_600, 30_000, 1800), 1_800_000);
        assert_eq!(session_timeout_ms(60, 3_900_000, 1800), 1_800_000);
        assert_eq!(session_timeout_ms(60, 600_000, 1800), 1_800_000);
    }

    #[test]
    fn v1_environment_keeps_legacy_settings_and_both_timeout_names() {
        let args = vec![
            "--window-size=1920,1080".to_string(),
            "--user-data-dir=/data/browser-profile".to_string(),
        ];
        let env = container_env(BrowserlessApiVersion::V1, 1_800_000, &args).unwrap();

        assert_eq!(env, vec![
            r#"DEFAULT_LAUNCH_ARGS=["--window-size=1920,1080","--user-data-dir=/data/browser-profile"]"#,
            "TIMEOUT=1800000",
            "CONNECTION_TIMEOUT=1800000",
            "MAX_CONCURRENT_SESSIONS=1",
            "PREBOOT_CHROME=true",
        ]);
    }

    #[test]
    fn v2_environment_omits_removed_v1_settings() {
        let env = container_env(BrowserlessApiVersion::V2, 1_800_000, &[
            "--window-size=1920,1080".to_string(),
        ])
        .unwrap();

        assert_eq!(env, vec!["TIMEOUT=1800000", "CONCURRENT=1"]);
        assert!(env.iter().all(|entry| !entry.starts_with("DEFAULT_")));
        assert!(env.iter().all(|entry| !entry.starts_with("PREBOOT_")));
    }

    #[test]
    fn v1_websocket_url_remains_unchanged() {
        let url = websocket_url("browser-host.local", 45029, BrowserlessApiVersion::V1, &[
            "--window-size=1920,1080".to_string(),
        ])
        .unwrap();

        assert_eq!(url, "ws://browser-host.local:45029");
    }

    #[test]
    fn v2_websocket_url_contains_base64_launch_options() {
        let args = vec![
            "--window-size=1920,1080".to_string(),
            "--user-data-dir=/data/profile with spaces".to_string(),
        ];
        let url = websocket_url(
            "browser-host.local",
            45029,
            BrowserlessApiVersion::V2,
            &args,
        )
        .unwrap();
        let parsed = Url::parse(&url).unwrap();
        let encoded = parsed
            .query_pairs()
            .find_map(|(key, value)| (key == "launch").then(|| value.into_owned()))
            .expect("launch query parameter");
        let decoded = STANDARD.decode(encoded).unwrap();
        let launch: serde_json::Value = serde_json::from_slice(&decoded).unwrap();

        assert_eq!(launch["args"], serde_json::json!(args));
    }
}
