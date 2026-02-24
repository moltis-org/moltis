//! Share page rendering: template structs and helper functions for generating
//! the static HTML share page and the OG social-image SVG.

use {
    askama::Template,
    base64::Engine as _,
    chrono::{Local, TimeZone, Utc},
};

use moltis_gateway::share_store::{ShareSnapshot, ShareVisibility, SharedMessageRole};

// ---------------------------------------------------------------------------
// Public entry points
// ---------------------------------------------------------------------------

/// Render a complete share HTML page (the same output the Askama template
/// produces) without requiring a per-request nonce.
///
/// `share_image_url` is the absolute or origin-relative URL to the OG image
/// SVG for this share (e.g. `/share/{id}/og-image.svg`).
pub fn render_share_html(
    snapshot: &ShareSnapshot,
    identity: &moltis_config::ResolvedIdentity,
    share_id: &str,
    visibility: ShareVisibility,
    view_count: u64,
    share_image_url: &str,
) -> crate::Result<String> {
    let meta = build_session_share_meta(identity, snapshot);
    let messages = map_share_message_views(snapshot, identity);
    let assistant_name = identity_name(identity).to_owned();
    let assistant_emoji = identity
        .emoji
        .as_deref()
        .map(str::trim)
        .filter(|v| !v.is_empty())
        .unwrap_or("\u{1F916}")
        .to_string();
    let visibility_label = if visibility == ShareVisibility::Public {
        "public"
    } else {
        "private"
    };

    let template = ShareHtmlTemplate {
        page_title: &meta.title,
        share_title: &meta.title,
        share_description: &meta.description,
        share_site_name: &meta.site_name,
        share_image_url,
        share_image_alt: &meta.image_alt,
        assistant_name: &assistant_name,
        assistant_emoji: &assistant_emoji,
        view_count,
        share_visibility: visibility_label,
        messages: &messages,
    };

    template.render().map_err(|e| {
        crate::Error::message(format!(
            "failed to render share template for {share_id}: {e}"
        ))
    })
}

/// Render the OG social-image SVG for a share.
pub fn render_share_og_svg(
    snapshot: &ShareSnapshot,
    identity: &moltis_config::ResolvedIdentity,
) -> String {
    build_share_social_image_svg(snapshot, identity)
}

// ---------------------------------------------------------------------------
// Askama template structs
// ---------------------------------------------------------------------------

#[derive(Template)]
#[template(path = "share.html", escape = "html")]
pub(crate) struct ShareHtmlTemplate<'a> {
    pub page_title: &'a str,
    pub share_title: &'a str,
    pub share_description: &'a str,
    pub share_site_name: &'a str,
    pub share_image_url: &'a str,
    pub share_image_alt: &'a str,
    pub assistant_name: &'a str,
    pub assistant_emoji: &'a str,
    pub view_count: u64,
    pub share_visibility: &'a str,
    pub messages: &'a [ShareMessageView],
}

pub(crate) struct ShareMessageView {
    pub role_class: &'static str,
    pub role_label: String,
    pub content: String,
    pub reasoning: Option<String>,
    pub audio_data_url: Option<String>,
    pub image_preview_data_url: Option<String>,
    pub image_link_data_url: Option<String>,
    pub image_preview_width: u32,
    pub image_preview_height: u32,
    pub image_has_dimensions: bool,
    pub tool_state_class: Option<&'static str>,
    pub tool_state_label: Option<&'static str>,
    pub tool_state_badge_class: Option<&'static str>,
    pub is_exec_card: bool,
    pub exec_card_class: Option<&'static str>,
    pub exec_command: Option<String>,
    pub map_link_google: Option<String>,
    pub map_link_apple: Option<String>,
    pub map_link_openstreetmap: Option<String>,
    pub created_at_ms: Option<u64>,
    pub created_at_label: Option<String>,
    pub footer: Option<String>,
}

pub(crate) struct ShareMeta {
    pub title: String,
    pub description: String,
    pub site_name: String,
    pub image_alt: String,
}

// ---------------------------------------------------------------------------
// Helper functions
// ---------------------------------------------------------------------------

pub(crate) fn identity_name(identity: &moltis_config::ResolvedIdentity) -> &str {
    let name = identity.name.trim();
    if name.is_empty() {
        "moltis"
    } else {
        name
    }
}

pub(crate) fn truncate_for_meta(text: &str, max: usize) -> String {
    if text.len() <= max {
        text.to_string()
    } else {
        format!("{}…", &text[..text.floor_char_boundary(max)])
    }
}

fn first_share_message_preview(snapshot: &ShareSnapshot) -> String {
    let mut out = String::new();
    for msg in &snapshot.messages {
        if msg.content.trim().is_empty() {
            continue;
        }
        if !out.is_empty() {
            out.push_str(" — ");
        }
        out.push_str(msg.content.trim());
        if out.len() >= 180 {
            break;
        }
    }

    if out.is_empty() {
        "Shared conversation snapshot from Moltis".to_string()
    } else {
        truncate_for_meta(&out, 220)
    }
}

pub(crate) fn build_session_share_meta(
    identity: &moltis_config::ResolvedIdentity,
    snapshot: &ShareSnapshot,
) -> ShareMeta {
    let agent_name = identity_name(identity);
    let session_name = snapshot
        .session_label
        .as_deref()
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .unwrap_or("Session");

    let title = format!("{session_name} · shared via {agent_name}");
    let description = first_share_message_preview(snapshot);
    let image_alt = format!("{session_name} shared from {agent_name}");

    ShareMeta {
        title,
        description,
        site_name: agent_name.to_owned(),
        image_alt,
    }
}

pub(crate) fn human_share_time(ts_ms: u64) -> String {
    let millis = ts_ms.min(i64::MAX as u64) as i64;
    Utc.timestamp_millis_opt(millis)
        .single()
        .map(|utc| {
            utc.with_timezone(&Local)
                .format("%Y-%m-%d %H:%M")
                .to_string()
        })
        .unwrap_or_else(|| "1970-01-01 00:00".to_string())
}

fn share_user_label(identity: &moltis_config::ResolvedIdentity) -> String {
    identity
        .user_name
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .unwrap_or("User")
        .to_string()
}

fn share_assistant_label(identity: &moltis_config::ResolvedIdentity) -> String {
    let name = identity_name(identity);
    match identity
        .emoji
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
    {
        Some(emoji) => format!("{emoji} {name}"),
        None => name.to_string(),
    }
}

fn image_dimensions_from_data_url(data_url: &str) -> Option<(u32, u32)> {
    let (meta, body) = data_url.split_once(',')?;
    if !meta.starts_with("data:image/") || !meta.contains(";base64") {
        return None;
    }
    let bytes = base64::engine::general_purpose::STANDARD
        .decode(body.trim())
        .ok()?;
    let metadata = moltis_media::image_ops::get_image_metadata(&bytes).ok()?;
    Some((metadata.width, metadata.height))
}

pub(crate) fn map_share_message_views(
    snapshot: &ShareSnapshot,
    identity: &moltis_config::ResolvedIdentity,
) -> Vec<ShareMessageView> {
    let user_label = share_user_label(identity);
    let assistant_label = share_assistant_label(identity);

    snapshot
        .messages
        .iter()
        .filter_map(|msg| {
            let (role_class, role_label) = match msg.role {
                SharedMessageRole::User => ("user", user_label.clone()),
                SharedMessageRole::Assistant => ("assistant", assistant_label.clone()),
                SharedMessageRole::ToolResult => ("tool", "Tool".to_string()),
                SharedMessageRole::System | SharedMessageRole::Notice => return None,
            };
            let footer = match msg.role {
                SharedMessageRole::Assistant => match (&msg.provider, &msg.model) {
                    (Some(provider), Some(model)) => Some(format!("{provider} / {model}")),
                    (None, Some(model)) => Some(model.clone()),
                    (Some(provider), None) => Some(provider.clone()),
                    (None, None) => None,
                },
                SharedMessageRole::User
                | SharedMessageRole::ToolResult
                | SharedMessageRole::System
                | SharedMessageRole::Notice => None,
            };
            let (tool_state_class, tool_state_label, tool_state_badge_class) = match msg.role {
                SharedMessageRole::ToolResult => match msg.tool_success {
                    Some(true) => (Some("msg-tool-success"), Some("Success"), Some("ok")),
                    Some(false) => (Some("msg-tool-fail"), Some("Failed"), Some("fail")),
                    None => (None, None, None),
                },
                SharedMessageRole::User
                | SharedMessageRole::Assistant
                | SharedMessageRole::System
                | SharedMessageRole::Notice => (None, None, None),
            };
            let (is_exec_card, exec_card_class, exec_command) = match msg.role {
                SharedMessageRole::ToolResult => {
                    if msg.tool_name.as_deref() == Some("exec") {
                        let card_class = match msg.tool_success {
                            Some(true) => Some("exec-ok"),
                            Some(false) => Some("exec-err"),
                            None => None,
                        };
                        (true, card_class, msg.tool_command.clone())
                    } else {
                        (false, None, None)
                    }
                },
                SharedMessageRole::User
                | SharedMessageRole::Assistant
                | SharedMessageRole::System
                | SharedMessageRole::Notice => (false, None, None),
            };
            let (
                image_preview_data_url,
                image_link_data_url,
                image_preview_width,
                image_preview_height,
                image_has_dimensions,
            ) = if let Some(image) = msg.image.as_ref() {
                let preview = &image.preview;
                let link = image
                    .full
                    .as_ref()
                    .map_or_else(|| preview.data_url.clone(), |full| full.data_url.clone());
                (
                    Some(preview.data_url.clone()),
                    Some(link),
                    preview.width,
                    preview.height,
                    true,
                )
            } else if let Some(legacy_data_url) = msg.image_data_url.clone() {
                if let Some((width, height)) = image_dimensions_from_data_url(&legacy_data_url) {
                    (
                        Some(legacy_data_url.clone()),
                        Some(legacy_data_url),
                        width,
                        height,
                        true,
                    )
                } else {
                    (
                        Some(legacy_data_url.clone()),
                        Some(legacy_data_url),
                        0,
                        0,
                        false,
                    )
                }
            } else {
                (None, None, 0, 0, false)
            };
            Some(ShareMessageView {
                role_class,
                role_label,
                content: msg.content.clone(),
                reasoning: msg
                    .reasoning
                    .as_deref()
                    .map(str::trim)
                    .filter(|value| !value.is_empty())
                    .map(ToOwned::to_owned),
                audio_data_url: msg.audio_data_url.clone(),
                image_preview_data_url,
                image_link_data_url,
                image_preview_width,
                image_preview_height,
                image_has_dimensions,
                tool_state_class,
                tool_state_label,
                tool_state_badge_class,
                is_exec_card,
                exec_card_class,
                exec_command,
                map_link_google: msg
                    .map_links
                    .as_ref()
                    .and_then(|links| links.google_maps.clone()),
                map_link_apple: msg
                    .map_links
                    .as_ref()
                    .and_then(|links| links.apple_maps.clone()),
                map_link_openstreetmap: msg
                    .map_links
                    .as_ref()
                    .and_then(|links| links.openstreetmap.clone()),
                created_at_ms: msg.created_at,
                created_at_label: msg.created_at.map(human_share_time),
                footer,
            })
        })
        .collect()
}

// ---------------------------------------------------------------------------
// Social-image SVG rendering
// ---------------------------------------------------------------------------

static SHARE_SOCIAL_BRAND_ICON_DATA_URL: std::sync::LazyLock<String> =
    std::sync::LazyLock::new(|| {
        let encoded = base64::engine::general_purpose::STANDARD
            .encode(include_bytes!("assets/icons/favicon-compact-512.png"));
        format!("data:image/png;base64,{encoded}")
    });

fn escape_svg_text(text: &str) -> String {
    text.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
        .replace('\'', "&#39;")
}

fn normalize_share_social_text(text: &str) -> String {
    text.split_whitespace().collect::<Vec<_>>().join(" ")
}

fn wrap_share_social_line(text: &str, max_chars: usize) -> Vec<String> {
    if max_chars == 0 {
        return vec![];
    }

    let mut lines = Vec::new();
    let mut current = String::new();
    let mut current_len = 0usize;

    for raw_word in text.split_whitespace() {
        let mut word = raw_word.to_string();
        let mut word_len = word.chars().count();
        if word_len > max_chars {
            word = truncate_for_meta(&word, max_chars.saturating_sub(1));
            word_len = word.chars().count();
        }

        if current.is_empty() {
            current.push_str(&word);
            current_len = word_len;
            continue;
        }

        if current_len + 1 + word_len <= max_chars {
            current.push(' ');
            current.push_str(&word);
            current_len += 1 + word_len;
            continue;
        }

        lines.push(current);
        current = word;
        current_len = word_len;
    }

    if !current.is_empty() {
        lines.push(current);
    }

    lines
}

fn build_share_social_text_lines(
    snapshot: &ShareSnapshot,
    identity: &moltis_config::ResolvedIdentity,
    max_chars: usize,
    max_lines: usize,
) -> Vec<String> {
    let user_label = share_user_label(identity);
    let assistant_label = share_assistant_label(identity);
    let mut lines = Vec::new();
    let mut truncated = false;

    for msg in &snapshot.messages {
        let role = match msg.role {
            SharedMessageRole::User => user_label.as_str(),
            SharedMessageRole::Assistant => assistant_label.as_str(),
            SharedMessageRole::ToolResult => "Tool",
            SharedMessageRole::System | SharedMessageRole::Notice => continue,
        };
        let content = normalize_share_social_text(&msg.content);
        if content.is_empty() {
            continue;
        }
        let snippet = format!("{role}: {content}");
        let wrapped = wrap_share_social_line(&snippet, max_chars);
        for line in wrapped {
            if lines.len() >= max_lines {
                truncated = true;
                break;
            }
            lines.push(line);
        }
        if truncated {
            break;
        }
    }

    if lines.is_empty() {
        lines.push("Shared conversation snapshot".to_string());
    } else if truncated && let Some(last) = lines.last_mut() {
        *last = truncate_for_meta(last, max_chars.saturating_sub(1));
    }

    lines
}

fn build_share_social_image_svg(
    snapshot: &ShareSnapshot,
    identity: &moltis_config::ResolvedIdentity,
) -> String {
    const MAX_CHARS_PER_LINE: usize = 64;
    const MAX_LINES: usize = 6;
    const WIDTH: usize = 1200;
    const HEIGHT: usize = 630;

    let agent_name = identity_name(identity);
    let session_name = snapshot
        .session_label
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .unwrap_or("Shared session");
    let title = truncate_for_meta(session_name, 90);
    let subtitle = format!(
        "{agent_name} • {} messages • {}",
        snapshot.cutoff_message_count,
        human_share_time(snapshot.created_at)
    );
    let lines = build_share_social_text_lines(snapshot, identity, MAX_CHARS_PER_LINE, MAX_LINES);

    let mut conversation_lines = String::new();
    for (idx, line) in lines.iter().enumerate() {
        let y = 260 + idx * 48;
        conversation_lines.push_str(&format!(
            "<text x=\"78\" y=\"{y}\" fill=\"#e5e7eb\" font-size=\"29\" font-family=\"Inter, system-ui, sans-serif\">{}</text>",
            escape_svg_text(line)
        ));
    }

    format!(
        "<svg xmlns=\"http://www.w3.org/2000/svg\" width=\"{WIDTH}\" height=\"{HEIGHT}\" viewBox=\"0 0 {WIDTH} {HEIGHT}\">\
<defs>\
  <linearGradient id=\"bg\" x1=\"0\" y1=\"0\" x2=\"1\" y2=\"1\">\
    <stop offset=\"0%\" stop-color=\"#0f172a\"/>\
    <stop offset=\"100%\" stop-color=\"#020617\"/>\
  </linearGradient>\
  <linearGradient id=\"accent\" x1=\"0\" y1=\"0\" x2=\"1\" y2=\"0\">\
    <stop offset=\"0%\" stop-color=\"#22c55e\"/>\
    <stop offset=\"100%\" stop-color=\"#16a34a\"/>\
  </linearGradient>\
  <radialGradient id=\"glow\" cx=\"0\" cy=\"0\" r=\"1\" gradientTransform=\"translate(1120 88) rotate(90) scale(240 300)\">\
    <stop offset=\"0%\" stop-color=\"#22c55e\" stop-opacity=\"0.2\"/>\
    <stop offset=\"100%\" stop-color=\"#22c55e\" stop-opacity=\"0\"/>\
  </radialGradient>\
  <clipPath id=\"brand-clip\">\
    <circle cx=\"1080\" cy=\"118\" r=\"50\"/>\
  </clipPath>\
</defs>\
<rect width=\"{WIDTH}\" height=\"{HEIGHT}\" fill=\"url(#bg)\"/>\
<rect width=\"{WIDTH}\" height=\"{HEIGHT}\" fill=\"url(#glow)\"/>\
<rect x=\"44\" y=\"40\" width=\"1112\" height=\"550\" rx=\"26\" fill=\"#0b1220\" fill-opacity=\"0.84\" stroke=\"#334155\"/>\
<rect x=\"74\" y=\"76\" width=\"8\" height=\"112\" rx=\"4\" fill=\"url(#accent)\"/>\
<circle cx=\"1080\" cy=\"118\" r=\"58\" fill=\"#0f172a\" stroke=\"#334155\" stroke-width=\"2\"/>\
<image x=\"1030\" y=\"68\" width=\"100\" height=\"100\" href=\"{}\" clip-path=\"url(#brand-clip)\"/>\
<text x=\"98\" y=\"120\" fill=\"#f8fafc\" font-size=\"46\" font-family=\"Inter, system-ui, sans-serif\" font-weight=\"700\">{}</text>\
<text x=\"98\" y=\"164\" fill=\"#93c5fd\" font-size=\"25\" font-family=\"Inter, system-ui, sans-serif\">{}</text>\
<line x1=\"74\" y1=\"210\" x2=\"1126\" y2=\"210\" stroke=\"#334155\" stroke-width=\"1\"/>\
{}\
<text x=\"1122\" y=\"584\" text-anchor=\"end\" fill=\"#9ca3af\" font-size=\"22\" font-family=\"Inter, system-ui, sans-serif\">By Moltis</text>\
</svg>",
        SHARE_SOCIAL_BRAND_ICON_DATA_URL.as_str(),
        escape_svg_text(&title),
        escape_svg_text(&subtitle),
        conversation_lines
    )
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use {
        super::*,
        moltis_gateway::share_store::{SharedMessage, SharedMessageRole},
    };

    fn default_identity() -> moltis_config::ResolvedIdentity {
        moltis_config::ResolvedIdentity {
            name: "Moltis".to_owned(),
            user_name: Some("Tester".to_owned()),
            emoji: Some("\u{1F916}".to_owned()),
            ..Default::default()
        }
    }

    fn shared_msg(role: SharedMessageRole, content: &str) -> SharedMessage {
        SharedMessage {
            role,
            content: content.to_string(),
            reasoning: None,
            audio_data_url: None,
            image: None,
            image_data_url: None,
            map_links: None,
            tool_success: None,
            tool_name: None,
            tool_command: None,
            created_at: None,
            model: None,
            provider: None,
        }
    }

    fn minimal_snapshot() -> ShareSnapshot {
        let mut assistant_msg = shared_msg(SharedMessageRole::Assistant, "Hi there!");
        assistant_msg.provider = Some("openai".to_string());
        assistant_msg.model = Some("gpt-4".to_string());

        ShareSnapshot {
            session_key: "sess-1".to_string(),
            session_label: Some("My chat".to_string()),
            cutoff_message_count: 2,
            created_at: 1_700_000_000_000,
            messages: vec![shared_msg(SharedMessageRole::User, "Hello!"), assistant_msg],
        }
    }

    #[test]
    fn render_share_html_produces_valid_output() -> crate::Result<()> {
        let snapshot = minimal_snapshot();
        let identity = default_identity();
        let html = render_share_html(
            &snapshot,
            &identity,
            "abc-123",
            ShareVisibility::Public,
            42,
            "/share/abc-123/og-image.svg",
        )?;

        assert!(html.contains("<!DOCTYPE html>"));
        assert!(html.contains("class=\"share-toolbar\""));
        assert!(html.contains("class=\"theme-toggle\""));
        assert!(html.contains("42 views"));
        assert!(html.contains("public"));
        assert!(html.contains("Hello!"));
        assert!(html.contains("Hi there!"));
        assert!(html.contains("openai / gpt-4"));
        // No nonce in the output
        assert!(!html.contains("nonce-"));
        // External script references present
        assert!(html.contains("src=\"/assets/js/share-theme-init.js\""));
        assert!(html.contains("src=\"/assets/js/share-app.mjs\""));
        Ok(())
    }

    #[test]
    fn render_share_og_svg_produces_svg() {
        let snapshot = minimal_snapshot();
        let identity = default_identity();
        let svg = render_share_og_svg(&snapshot, &identity);

        assert!(svg.starts_with("<svg"));
        assert!(svg.contains("My chat"));
        assert!(svg.contains("By Moltis"));
    }

    #[test]
    fn share_template_renders_theme_toggle_and_audio() {
        let messages = vec![ShareMessageView {
            role_class: "assistant",
            role_label: "\u{1F916} Moltis".to_string(),
            content: "Audio response".to_string(),
            reasoning: Some("Step 1\nStep 2".to_string()),
            audio_data_url: Some("data:audio/ogg;base64,T2dnUw==".to_string()),
            image_preview_data_url: Some("data:image/png;base64,ZmFrZQ==".to_string()),
            image_link_data_url: Some("data:image/png;base64,ZmFrZQ==".to_string()),
            image_preview_width: 600,
            image_preview_height: 400,
            image_has_dimensions: true,
            tool_state_class: None,
            tool_state_label: None,
            tool_state_badge_class: None,
            is_exec_card: false,
            exec_card_class: None,
            exec_command: None,
            map_link_google: Some(
                "https://www.google.com/maps/search/?api=1&query=Tartine+Bakery".to_string(),
            ),
            map_link_apple: Some("https://maps.apple.com/?q=Tartine+Bakery".to_string()),
            map_link_openstreetmap: Some(
                "https://www.openstreetmap.org/search?query=Tartine+Bakery".to_string(),
            ),
            created_at_ms: Some(1_770_966_725_000),
            created_at_label: Some("2026-02-13 05:32:05 UTC".to_string()),
            footer: Some("provider / model".to_string()),
        }];
        let template = ShareHtmlTemplate {
            page_title: "title",
            share_title: "title",
            share_description: "desc",
            share_site_name: "site",
            share_image_url: "https://www.moltis.org/og-social.jpg?v=4",
            share_image_alt: "alt",
            assistant_name: "Moltis",
            assistant_emoji: "\u{1F916}",
            view_count: 7,
            share_visibility: "public",
            messages: &messages,
        };
        let html = template.render().unwrap_or_default();
        assert!(html.contains("class=\"share-toolbar\""));
        assert!(html.contains("class=\"theme-toggle\""));
        assert!(html.contains("data-theme-val=\"light\""));
        assert!(html.contains("data-theme-val=\"dark\""));
        assert!(html.contains("class=\"share-page-footer\""));
        assert!(html.contains("margin-bottom: 14px;"));
        assert!(html.contains("Get your AI assistant at"));
        assert!(html.contains("src=\"/assets/icons/icon-96.png\""));
        assert!(!html.contains("data-epoch-ms=\"1770966600000\""));
        assert!(html.contains("data-epoch-ms=\"1770966725000\""));
        assert!(html.contains("data-audio-src=\"data:audio/ogg;base64,T2dnUw==\""));
        assert!(html.contains("width=\"600\""));
        assert!(html.contains("height=\"400\""));
        assert!(html.contains("data-image-viewer-open=\"true\""));
        assert!(html.contains("data-image-viewer=\"true\""));
        assert!(html.contains("class=\"msg-map-link-icon\""));
        assert!(html.contains("src=\"/assets/icons/map-google-maps.svg\""));
        assert!(html.contains("src=\"/assets/icons/map-apple-maps.svg\""));
        assert!(html.contains("src=\"/assets/icons/map-openstreetmap.svg\""));
        assert!(html.contains("class=\"msg-reasoning\""));
        assert!(html.contains("Reasoning"));
    }

    #[test]
    fn map_share_message_views_skips_system_and_notice() {
        let identity = moltis_config::ResolvedIdentity {
            name: "Moltis".to_owned(),
            user_name: Some("Fabien".to_owned()),
            emoji: Some("\u{1F916}".to_owned()),
            ..Default::default()
        };
        let snapshot = ShareSnapshot {
            session_key: "s".to_string(),
            session_label: None,
            cutoff_message_count: 3,
            created_at: 0,
            messages: vec![
                shared_msg(SharedMessageRole::System, "system prompt"),
                shared_msg(SharedMessageRole::Notice, "notice"),
                shared_msg(SharedMessageRole::User, "hi"),
            ],
        };
        let views = map_share_message_views(&snapshot, &identity);
        assert_eq!(views.len(), 1);
        assert_eq!(views[0].role_class, "user");
    }

    #[test]
    fn share_labels_use_identity_user_and_emoji() {
        let identity = moltis_config::ResolvedIdentity {
            name: "Moltis".to_owned(),
            emoji: Some("\u{1F916}".to_owned()),
            user_name: Some("Fabien".to_owned()),
            ..Default::default()
        };
        assert_eq!(share_user_label(&identity), "Fabien");
        assert_eq!(share_assistant_label(&identity), "\u{1F916} Moltis");
    }

    #[test]
    fn share_labels_fallback_when_identity_fields_missing() {
        let identity = moltis_config::ResolvedIdentity {
            name: "   ".to_owned(),
            user_name: Some("   ".to_owned()),
            emoji: Some("   ".to_owned()),
            ..Default::default()
        };
        assert_eq!(share_user_label(&identity), "User");
        assert_eq!(share_assistant_label(&identity), "moltis");
    }

    #[test]
    fn share_social_image_svg_uses_session_content_and_escapes() {
        let identity = moltis_config::ResolvedIdentity {
            name: "Moltis".to_owned(),
            user_name: Some("Fabien".to_owned()),
            emoji: Some("\u{1F916}".to_owned()),
            ..Default::default()
        };
        let snapshot = ShareSnapshot {
            session_key: "main".to_string(),
            session_label: Some("Release checklist".to_string()),
            cutoff_message_count: 2,
            created_at: 1_770_966_600_000,
            messages: vec![
                shared_msg(
                    SharedMessageRole::User,
                    "Need to validate <script>alert(1)</script> path",
                ),
                shared_msg(SharedMessageRole::Assistant, "Run tests, then deploy."),
            ],
        };

        let svg = render_share_og_svg(&snapshot, &identity);
        assert!(svg.contains("Release checklist"));
        assert!(svg.contains("&lt;script&gt;alert(1)&lt;/script&gt;"));
        assert!(!svg.contains("Need to validate <script>alert(1)</script> path"));
        assert!(svg.contains("Fabien: Need to validate"));
        assert!(svg.contains("data:image/png;base64,"));
        assert!(svg.contains("By Moltis"));
    }

    #[test]
    fn map_share_message_views_includes_tool_result_media_and_links() {
        use moltis_gateway::share_store::{SharedImageAsset, SharedImageSet, SharedMapLinks};

        let identity = default_identity();
        let snapshot = ShareSnapshot {
            session_key: "main".to_string(),
            session_label: Some("main".to_string()),
            cutoff_message_count: 1,
            created_at: 1_770_966_600_000,
            messages: vec![SharedMessage {
                role: SharedMessageRole::ToolResult,
                content: "Tartine Bakery".to_string(),
                reasoning: None,
                audio_data_url: None,
                image: Some(SharedImageSet {
                    preview: SharedImageAsset {
                        data_url: "data:image/png;base64,ZmFrZQ==".to_string(),
                        width: 600,
                        height: 400,
                    },
                    full: None,
                }),
                image_data_url: None,
                map_links: Some(SharedMapLinks {
                    apple_maps: Some("https://maps.apple.com/?q=Tartine+Bakery".to_string()),
                    google_maps: Some(
                        "https://www.google.com/maps/search/?api=1&query=Tartine+Bakery"
                            .to_string(),
                    ),
                    openstreetmap: None,
                }),
                tool_success: Some(true),
                tool_name: Some("show_map".to_string()),
                tool_command: None,
                created_at: Some(1_770_966_604_000),
                model: None,
                provider: None,
            }],
        };

        let views = map_share_message_views(&snapshot, &identity);
        assert_eq!(views.len(), 1);
        assert_eq!(views[0].role_class, "tool");
        assert_eq!(views[0].role_label, "Tool");
        assert!(
            views[0]
                .image_preview_data_url
                .as_deref()
                .unwrap_or_default()
                .starts_with("data:image/png;base64,")
        );
        assert_eq!(views[0].image_preview_width, 600);
        assert_eq!(views[0].image_preview_height, 400);
        assert!(views[0].image_has_dimensions);
        assert_eq!(views[0].tool_state_class, Some("msg-tool-success"));
        assert_eq!(views[0].tool_state_label, Some("Success"));
        assert_eq!(views[0].tool_state_badge_class, Some("ok"));
        assert!(!views[0].is_exec_card);
        assert!(views[0].exec_card_class.is_none());
        assert!(views[0].exec_command.is_none());
        assert!(views[0].map_link_google.is_some());
        assert!(views[0].map_link_apple.is_some());
        assert!(views[0].map_link_openstreetmap.is_none());
    }

    #[test]
    fn map_share_message_views_marks_exec_tool_cards() {
        let identity = default_identity();
        let mut msg = shared_msg(SharedMessageRole::ToolResult, "{\n  \"ok\": true\n}");
        msg.tool_success = Some(false);
        msg.tool_name = Some("exec".to_string());
        msg.tool_command = Some("curl -s https://example.com".to_string());

        let snapshot = ShareSnapshot {
            session_key: "main".to_string(),
            session_label: Some("main".to_string()),
            cutoff_message_count: 1,
            created_at: 1_770_966_600_000,
            messages: vec![msg],
        };

        let views = map_share_message_views(&snapshot, &identity);
        assert_eq!(views.len(), 1);
        assert!(views[0].is_exec_card);
        assert_eq!(views[0].exec_card_class, Some("exec-err"));
        assert_eq!(
            views[0].exec_command.as_deref(),
            Some("curl -s https://example.com")
        );
    }
}
