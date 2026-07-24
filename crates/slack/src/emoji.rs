//! Emoji name normalization for Slack reactions.
//!
//! Slack's `reactions.add` / `reactions.remove` expect emoji **shortcodes**
//! (e.g. `white_check_mark`), not raw Unicode glyphs (`✅`) and not
//! colon-wrapped forms (`:eyes:`). Slack silently drops names it does not
//! recognize, so any name that reaches the Web API — from a config override, a
//! phase-reaction map, or a future model-specified value — must be normalized
//! first. This mirrors the map openclaw's `actions.ts` maintains for the same
//! reason.

use std::borrow::Cow;

/// Normalize an emoji name into a Slack reaction shortcode.
///
/// - Strips surrounding colons (`:eyes:` → `eyes`).
/// - Drops a trailing skin-tone modifier (`thumbsup::skin-tone-3` → `thumbsup`).
/// - Maps common Unicode glyphs to their shortcode (`✅` → `white_check_mark`).
/// - Otherwise returns the input unchanged (already a shortcode).
///
/// Returns a [`Cow`] so the common already-normalized case does not allocate.
#[must_use]
pub fn normalize_reaction_name(name: &str) -> Cow<'_, str> {
    let trimmed = name.trim();

    // Map a raw glyph first (before colon/skin-tone handling, which only apply
    // to shortcode forms).
    if let Some(shortcode) = glyph_to_shortcode(trimmed) {
        return Cow::Borrowed(shortcode);
    }

    // Strip surrounding colons and any `::skin-tone-N` suffix.
    let without_colons = trimmed.trim_matches(':');
    let base = without_colons
        .split_once("::")
        .map_or(without_colons, |(head, _tail)| head);

    if base == name {
        Cow::Borrowed(name)
    } else {
        Cow::Owned(base.to_string())
    }
}

/// Map a raw Unicode emoji glyph to its Slack shortcode, if known.
///
/// Covers the glyphs Moltis emits for acknowledgment and phase reactions plus a
/// handful of frequently-used ones. Unknown glyphs fall through to the caller.
fn glyph_to_shortcode(glyph: &str) -> Option<&'static str> {
    let shortcode = match glyph {
        "✅" | "✔️" | "✔" => "white_check_mark",
        "❌" | "✖️" => "x",
        "👀" => "eyes",
        "⏳" => "hourglass_flowing_sand",
        "⌛" => "hourglass",
        "⚠️" | "⚠" => "warning",
        "🧠" => "brain",
        "🛠️" | "🛠" => "hammer_and_wrench",
        "🌐" => "globe_with_meridians",
        "💻" => "computer",
        "🏗️" | "🏗" => "building_construction",
        "🛫" => "airplane_departure",
        "🗜️" | "🗜" => "clamp",
        "👍" => "thumbsup",
        "👎" => "thumbsdown",
        "🎉" => "tada",
        "🔥" => "fire",
        "👋" => "wave",
        "🚀" => "rocket",
        _ => return None,
    };
    Some(shortcode)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn passes_through_plain_shortcode() {
        assert_eq!(
            normalize_reaction_name("white_check_mark"),
            "white_check_mark"
        );
    }

    #[test]
    fn strips_surrounding_colons() {
        assert_eq!(normalize_reaction_name(":eyes:"), "eyes");
    }

    #[test]
    fn maps_common_glyphs() {
        assert_eq!(normalize_reaction_name("✅"), "white_check_mark");
        assert_eq!(normalize_reaction_name("❌"), "x");
        assert_eq!(normalize_reaction_name("👀"), "eyes");
        assert_eq!(normalize_reaction_name("🧠"), "brain");
    }

    #[test]
    fn strips_skin_tone_modifier() {
        assert_eq!(normalize_reaction_name("thumbsup::skin-tone-3"), "thumbsup");
        assert_eq!(normalize_reaction_name(":wave::skin-tone-2:"), "wave");
    }

    #[test]
    fn trims_whitespace() {
        assert_eq!(normalize_reaction_name("  eyes  "), "eyes");
    }

    #[test]
    fn unknown_glyph_passes_through() {
        // Not in the map — returned unchanged for Slack to accept or reject.
        assert_eq!(
            normalize_reaction_name("some_custom_emoji"),
            "some_custom_emoji"
        );
    }
}
