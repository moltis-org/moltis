use serde_json::{Map, Value, json};

pub(super) const ENDPOINT: &str = "https://api.search.brave.com/res/v1/web/search";

// Keep these values aligned with Brave's Web Search API enums. They are intentionally
// provider-local: Perplexity, Firecrawl, and DuckDuckGo do not share this contract.
const COUNTRIES: &[&str] = &[
    "AR", "AU", "AT", "BE", "BR", "CA", "CL", "DK", "FI", "FR", "DE", "GR", "HK", "IN", "ID", "IT",
    "JP", "KR", "MY", "MX", "NL", "NZ", "NO", "CN", "PL", "PT", "PH", "RU", "SA", "ZA", "ES", "SE",
    "CH", "TW", "TR", "GB", "US", "ALL",
];

const SEARCH_LANGUAGES: &[&str] = &[
    "ar", "eu", "bn", "bg", "ca", "zh-hans", "zh-hant", "hr", "cs", "da", "nl", "en", "en-gb",
    "et", "fi", "fr", "gl", "de", "el", "gu", "he", "hi", "hu", "is", "it", "ja", "jp", "kn", "ko",
    "lv", "lt", "ms", "ml", "mr", "nb", "pl", "pt-br", "pt-pt", "pa", "ro", "ru", "sr", "sk", "sl",
    "es", "sv", "ta", "te", "th", "tr", "uk", "vi",
];

const UI_LANGUAGES: &[&str] = &[
    "es-AR", "en-AU", "de-AT", "nl-BE", "fr-BE", "pt-BR", "en-CA", "fr-CA", "es-CL", "da-DK",
    "fi-FI", "fr-FR", "de-DE", "el-GR", "zh-HK", "en-IN", "en-ID", "it-IT", "ja-JP", "ko-KR",
    "en-MY", "es-MX", "nl-NL", "en-NZ", "no-NO", "zh-CN", "pl-PL", "en-PH", "ru-RU", "en-ZA",
    "es-ES", "sv-SE", "fr-CH", "de-CH", "zh-TW", "tr-TR", "en-GB", "en-US", "es-US",
];

#[derive(Debug, Default, Eq, PartialEq)]
pub(super) struct Params {
    country: Option<&'static str>,
    search_lang: Option<&'static str>,
    ui_lang: Option<&'static str>,
    freshness: Option<String>,
}

impl Params {
    pub(super) fn from_json(params: &Value) -> Self {
        let requested_country = string_param(params, "country");
        let country = requested_country.map(|value| canonical(value, COUNTRIES).unwrap_or("ALL"));

        let requested_ui_lang = string_param(params, "ui_lang");
        let ui_lang = requested_ui_lang.and_then(|value| canonical(value, UI_LANGUAGES));

        let search_lang = string_param(params, "search_lang")
            .and_then(|value| normalize_search_language(value, country, ui_lang));
        let freshness = string_param(params, "freshness").and_then(normalize_freshness);

        Self {
            country,
            search_lang,
            ui_lang,
            freshness,
        }
    }

    pub(super) fn has_optional_filters(&self) -> bool {
        self.country.is_some()
            || self.search_lang.is_some()
            || self.ui_lang.is_some()
            || self.freshness.is_some()
    }

    pub(super) fn request_url(&self, endpoint: &str, query: &str, count: u8) -> String {
        let mut url = format!(
            "{endpoint}?q={}&count={count}",
            super::urlencoding::encode(query)
        );
        append_param(&mut url, "country", self.country);
        append_param(&mut url, "search_lang", self.search_lang);
        append_param(&mut url, "ui_lang", self.ui_lang);
        append_param(&mut url, "freshness", self.freshness.as_deref());
        url
    }
}

pub(super) fn parameter_properties() -> Map<String, Value> {
    Map::from_iter([
        (
            "country".to_string(),
            json!({
                "type": "string",
                "description": "Brave Search country market. Use ALL when the target country is not listed.",
                "enum": COUNTRIES,
            }),
        ),
        (
            "search_lang".to_string(),
            json!({
                "type": "string",
                "description": "Brave Search result language.",
                "enum": SEARCH_LANGUAGES,
            }),
        ),
        (
            "ui_lang".to_string(),
            json!({
                "type": "string",
                "description": "Brave Search response UI language.",
                "enum": UI_LANGUAGES,
            }),
        ),
        (
            "freshness".to_string(),
            json!({
                "type": "string",
                "description": "Brave freshness filter: pd (past day), pw (past week), pm (past month), py (past year), or YYYY-MM-DDtoYYYY-MM-DD."
            }),
        ),
    ])
}

fn string_param<'a>(params: &'a Value, name: &str) -> Option<&'a str> {
    params
        .get(name)
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
}

fn canonical(value: &str, allowed: &'static [&'static str]) -> Option<&'static str> {
    let normalized = value.replace('_', "-");
    allowed
        .iter()
        .copied()
        .find(|candidate| candidate.eq_ignore_ascii_case(&normalized))
}

fn normalize_search_language(
    value: &str,
    country: Option<&str>,
    ui_lang: Option<&str>,
) -> Option<&'static str> {
    if let Some(language) = canonical(value, SEARCH_LANGUAGES) {
        return Some(language);
    }

    let normalized = value.replace('_', "-").to_ascii_lowercase();
    let base = normalized.split('-').next()?;
    if let Some(language) = canonical(base, SEARCH_LANGUAGES) {
        return Some(language);
    }

    let preferred_region = ui_lang
        .and_then(|locale| locale.split_once('-').map(|(_, region)| region))
        .or_else(|| country.filter(|country| *country != "ALL"));
    if base == "zh" {
        return match preferred_region {
            Some("CN") => Some("zh-hans"),
            Some("HK" | "TW") => Some("zh-hant"),
            _ => None,
        };
    }

    let mut candidates = SEARCH_LANGUAGES
        .iter()
        .copied()
        .filter(|candidate| candidate.starts_with(&format!("{base}-")));
    let first = candidates.next()?;
    if candidates.next().is_none() {
        return Some(first);
    }

    preferred_region.and_then(|region| {
        SEARCH_LANGUAGES.iter().copied().find(|candidate| {
            candidate
                .rsplit_once('-')
                .is_some_and(|(_, suffix)| suffix.eq_ignore_ascii_case(region))
                && candidate.starts_with(&format!("{base}-"))
        })
    })
}

fn normalize_freshness(value: &str) -> Option<String> {
    let normalized = match value.to_ascii_lowercase().as_str() {
        "pd" | "day" => "pd",
        "pw" | "week" => "pw",
        "pm" | "month" => "pm",
        "py" | "year" => "py",
        _ if looks_like_date_range(value) => value,
        _ => return None,
    };
    Some(normalized.to_string())
}

fn looks_like_date_range(value: &str) -> bool {
    let Some((start, end)) = value.split_once("to") else {
        return false;
    };
    [start, end].into_iter().all(|date| {
        date.len() == 10
            && date.chars().enumerate().all(|(index, ch)| {
                matches!(index, 4 | 7) && ch == '-'
                    || !matches!(index, 4 | 7) && ch.is_ascii_digit()
            })
    })
}

fn append_param(url: &mut String, name: &str, value: Option<&str>) {
    if let Some(value) = value {
        url.push('&');
        url.push_str(name);
        url.push('=');
        url.push_str(&super::urlencoding::encode(value));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn normalizes_unsupported_market_without_region_specific_assumptions() {
        let params = Params::from_json(&json!({
            "country": "PY",
            "search_lang": "es",
            "ui_lang": "es-PY",
        }));
        assert_eq!(params.country, Some("ALL"));
        assert_eq!(params.search_lang, Some("es"));
        assert_eq!(params.ui_lang, None);
    }

    #[test]
    fn infers_split_search_language_from_supported_region() {
        let params = Params::from_json(&json!({
            "country": "br",
            "search_lang": "pt",
            "ui_lang": "pt_BR",
        }));
        assert_eq!(params.country, Some("BR"));
        assert_eq!(params.search_lang, Some("pt-br"));
        assert_eq!(params.ui_lang, Some("pt-BR"));
    }

    #[test]
    fn infers_chinese_script_from_supported_region() {
        let simplified = Params::from_json(&json!({
            "country": "CN",
            "search_lang": "zh",
            "ui_lang": "zh-CN",
        }));
        assert_eq!(simplified.search_lang, Some("zh-hans"));

        let traditional = Params::from_json(&json!({
            "country": "TW",
            "search_lang": "zh",
            "ui_lang": "zh-TW",
        }));
        assert_eq!(traditional.search_lang, Some("zh-hant"));
    }

    #[test]
    fn drops_invalid_optional_values_and_normalizes_freshness_aliases() {
        let params = Params::from_json(&json!({
            "search_lang": "not-a-language",
            "ui_lang": "not-a-locale",
            "freshness": "month",
        }));
        assert_eq!(params.search_lang, None);
        assert_eq!(params.ui_lang, None);
        assert_eq!(params.freshness.as_deref(), Some("pm"));
    }

    #[test]
    fn request_url_only_contains_sanitized_brave_parameters() {
        let params = Params::from_json(&json!({
            "country": "py",
            "search_lang": "es-AR",
            "ui_lang": "es-PY",
            "freshness": "week",
        }));
        assert_eq!(
            params.request_url(ENDPOINT, "ração senior", 5),
            concat!(
                "https://api.search.brave.com/res/v1/web/search?",
                "q=ra%C3%A7%C3%A3o%20senior&count=5&country=ALL&search_lang=es&freshness=pw"
            )
        );
    }

    #[test]
    fn accepts_documented_custom_date_range() {
        let params = Params::from_json(&json!({
            "freshness": "2026-08-01to2026-08-25",
        }));
        assert_eq!(params.freshness.as_deref(), Some("2026-08-01to2026-08-25"));
    }

    #[test]
    fn schema_lists_only_supported_localization_values() {
        let properties = parameter_properties();
        assert!(properties["country"]["enum"].as_array().is_some());
        assert!(properties["search_lang"]["enum"].as_array().is_some());
        assert!(properties["ui_lang"]["enum"].as_array().is_some());
    }
}
