//! Adaptive Card builders for Teams messages.
//!
//! Provides constructors for common card types: welcome cards, structured
//! responses, and generic card attachments.

/// Build an Adaptive Card attachment envelope for Bot Framework activities.
pub fn card_attachment(card: serde_json::Value) -> serde_json::Value {
    serde_json::json!({
        "contentType": "application/vnd.microsoft.card.adaptive",
        "content": card,
    })
}

/// Build a Bot Framework activity that sends an Adaptive Card.
pub fn card_activity(card: serde_json::Value, fallback_text: Option<&str>) -> serde_json::Value {
    let mut activity = serde_json::json!({
        "type": "message",
        "attachments": [card_attachment(card)],
    });
    if let Some(text) = fallback_text {
        activity["text"] = serde_json::Value::String(text.to_string());
    }
    activity
}

/// Build a welcome card shown when a user first messages the bot in a DM.
pub fn build_welcome_card(bot_name: &str, prompt_starters: &[String]) -> serde_json::Value {
    let mut body: Vec<serde_json::Value> = vec![serde_json::json!({
        "type": "TextBlock",
        "size": "Medium",
        "weight": "Bolder",
        "text": format!("Welcome to {bot_name}!"),
        "wrap": true,
    })];

    if !prompt_starters.is_empty() {
        body.push(serde_json::json!({
            "type": "TextBlock",
            "text": "Try one of these to get started:",
            "wrap": true,
            "spacing": "Medium",
        }));

        for starter in prompt_starters {
            body.push(serde_json::json!({
                "type": "ActionSet",
                "actions": [{
                    "type": "Action.Submit",
                    "title": starter,
                    "data": { "msteams": { "type": "imBack", "value": starter } },
                }],
            }));
        }
    } else {
        body.push(serde_json::json!({
            "type": "TextBlock",
            "text": "Send me a message to get started.",
            "wrap": true,
            "spacing": "Medium",
        }));
    }

    serde_json::json!({
        "type": "AdaptiveCard",
        "$schema": "http://adaptivecards.io/schemas/adaptive-card.json",
        "version": "1.4",
        "body": body,
    })
}

/// Build a simple text for group welcome messages (not a card, just text).
pub fn build_group_welcome_text(bot_name: &str) -> String {
    format!("Hi! I'm {bot_name}. Mention me with @{bot_name} and I'll respond to your message.")
}

/// Build a poll card with multiple choice options.
pub fn build_poll_card(
    question: &str,
    options: &[String],
    max_selections: u32,
) -> serde_json::Value {
    let mut choices: Vec<serde_json::Value> = Vec::with_capacity(options.len());
    for (i, opt) in options.iter().enumerate() {
        choices.push(serde_json::json!({
            "title": opt,
            "value": i.to_string(),
        }));
    }

    let is_multi = max_selections > 1;

    serde_json::json!({
        "type": "AdaptiveCard",
        "$schema": "http://adaptivecards.io/schemas/adaptive-card.json",
        "version": "1.4",
        "body": [
            {
                "type": "TextBlock",
                "text": question,
                "weight": "Bolder",
                "size": "Medium",
                "wrap": true,
            },
            {
                "type": "Input.ChoiceSet",
                "id": "poll_choice",
                "style": "expanded",
                "isMultiSelect": is_multi,
                "choices": choices,
            }
        ],
        "actions": [{
            "type": "Action.Submit",
            "title": "Vote",
            "data": { "action": "poll_vote" },
        }],
    })
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;

    #[test]
    fn card_attachment_has_correct_content_type() {
        let card = serde_json::json!({"type": "AdaptiveCard"});
        let att = card_attachment(card);
        assert_eq!(
            att["contentType"],
            "application/vnd.microsoft.card.adaptive"
        );
        assert_eq!(att["content"]["type"], "AdaptiveCard");
    }

    #[test]
    fn card_activity_includes_attachment() {
        let card = serde_json::json!({"type": "AdaptiveCard"});
        let activity = card_activity(card, Some("fallback"));
        assert_eq!(activity["type"], "message");
        assert_eq!(activity["text"], "fallback");
        assert!(activity["attachments"].is_array());
    }

    #[test]
    fn welcome_card_with_starters() {
        let card = build_welcome_card("TestBot", &["Hello".into(), "Help".into()]);
        assert_eq!(card["type"], "AdaptiveCard");
        let body = card["body"].as_array().unwrap();
        // Title + instruction + 2 action sets
        assert_eq!(body.len(), 4);
    }

    #[test]
    fn welcome_card_no_starters() {
        let card = build_welcome_card("TestBot", &[]);
        let body = card["body"].as_array().unwrap();
        // Title + default text
        assert_eq!(body.len(), 2);
    }

    #[test]
    fn poll_card_has_choices() {
        let card = build_poll_card("Favorite?", &["A".into(), "B".into(), "C".into()], 1);
        let body = card["body"].as_array().unwrap();
        let choice_set = &body[1];
        assert_eq!(choice_set["type"], "Input.ChoiceSet");
        assert_eq!(choice_set["choices"].as_array().unwrap().len(), 3);
        assert_eq!(choice_set["isMultiSelect"], false);
    }

    #[test]
    fn poll_card_multi_select() {
        let card = build_poll_card("Pick two", &["X".into(), "Y".into()], 2);
        let body = card["body"].as_array().unwrap();
        assert_eq!(body[1]["isMultiSelect"], true);
    }

    #[test]
    fn group_welcome_text_includes_name() {
        let text = build_group_welcome_text("MyBot");
        assert!(text.contains("MyBot"));
    }
}
