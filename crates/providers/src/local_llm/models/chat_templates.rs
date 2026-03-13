//! Chat template formatting for various model families.
//!
//! Different LLM families use different prompt formats. This module provides
//! template formatting for Llama3, ChatML (Qwen/Kimi), Mistral, and DeepSeek.

use moltis_agents::model::{ChatMessage, UserContent};

/// Hint for which chat template to use.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum ChatTemplateHint {
    /// Try to use the model's embedded template, fall back to ChatML.
    #[default]
    Auto,
    /// Llama 3 format: `<|begin_of_text|><|start_header_id|>system<|end_header_id|>...`
    Llama3,
    /// ChatML format: `<|im_start|>system\n...<|im_end|>` (Qwen, Kimi, Yi)
    ChatML,
    /// Mistral format: `[INST] ... [/INST]`
    Mistral,
    /// DeepSeek format (similar to ChatML with minor differences)
    DeepSeek,
}

impl ChatTemplateHint {
    /// Parse from string (for config).
    #[must_use]
    pub fn parse(s: &str) -> Self {
        match s.to_lowercase().as_str() {
            "llama3" | "llama" => Self::Llama3,
            "chatml" | "qwen" | "kimi" | "yi" => Self::ChatML,
            "mistral" => Self::Mistral,
            "deepseek" => Self::DeepSeek,
            _ => Self::Auto,
        }
    }
}

/// Extract role and content strings from a `ChatMessage`.
fn role_content(msg: &ChatMessage) -> (&str, &str) {
    match msg {
        ChatMessage::System { content } => ("system", content.as_str()),
        ChatMessage::User { content } => match content {
            UserContent::Text(text) => ("user", text.as_str()),
            UserContent::Multimodal(_) => ("user", ""),
        },
        ChatMessage::Assistant { content, .. } => ("assistant", content.as_deref().unwrap_or("")),
        ChatMessage::Tool { content, .. } => ("tool", content.as_str()),
    }
}

/// Merge consecutive system messages into a single system message.
/// This prevents Jinja template errors in GGUF models that expect
/// at most one system message at the beginning of the conversation.
#[must_use]
fn merge_consecutive_system_messages(messages: &[ChatMessage]) -> Vec<ChatMessage> {
    let mut result = Vec::with_capacity(messages.len());
    let mut pending_system_content: Option<String> = None;

    for msg in messages {
        match msg {
            ChatMessage::System { content } => {
                // Accumulate system message content
                if let Some(ref mut existing) = pending_system_content {
                    existing.push('\n');
                    existing.push_str(content);
                } else {
                    pending_system_content = Some(content.clone());
                }
            },
            other => {
                // Flush any pending system message before adding non-system message
                if let Some(content) = pending_system_content.take() {
                    result.push(ChatMessage::System { content });
                }
                result.push(other.clone());
            },
        }
    }

    // Flush any remaining system message at the end
    if let Some(content) = pending_system_content {
        result.push(ChatMessage::System { content });
    }

    result
}

/// Format messages using the specified chat template.
#[must_use]
pub fn format_messages(messages: &[ChatMessage], hint: ChatTemplateHint) -> String {
    // Merge consecutive system messages to prevent Jinja template errors
    let merged_messages = merge_consecutive_system_messages(messages);

    match hint {
        ChatTemplateHint::Auto | ChatTemplateHint::ChatML => format_chatml(&merged_messages),
        ChatTemplateHint::Llama3 => format_llama3(&merged_messages),
        ChatTemplateHint::Mistral => format_mistral(&merged_messages),
        ChatTemplateHint::DeepSeek => format_deepseek(&merged_messages),
    }
}

/// Format using ChatML template (Qwen, Kimi, Yi).
///
/// ```text
/// <|im_start|>system
/// {system_message}<|im_end|>
/// <|im_start|>user
/// {user_message}<|im_end|>
/// <|im_start|>assistant
/// ```
///
/// System messages are consolidated at the beginning to avoid Jinja template
/// errors with models that require system messages to come first.
fn format_chatml(messages: &[ChatMessage]) -> String {
    let mut output = String::new();

    // First, output all system messages to satisfy templates that require
    // system messages at the beginning (e.g., Qwen via llama.cpp).
    for msg in messages {
        if let ChatMessage::System { content } = msg {
            output.push_str("<|im_start|>system\n");
            output.push_str(content);
            output.push_str("<|im_end|>\n");
        }
    }

    // Then output all non-system messages in order.
    for msg in messages {
        let (role, content) = role_content(msg);
        if role == "system" {
            continue; // Already handled above.
        }

        output.push_str("<|im_start|>");
        output.push_str(role);
        output.push('\n');
        output.push_str(content);
        output.push_str("<|im_end|>\n");
    }

    // Add the assistant prefix for generation
    output.push_str("<|im_start|>assistant\n");
    output
}

/// Format using Llama 3 template.
///
/// ```text
/// <|begin_of_text|><|start_header_id|>system<|end_header_id|>
///
/// {system_message}<|eot_id|><|start_header_id|>user<|end_header_id|>
///
/// {user_message}<|eot_id|><|start_header_id|>assistant<|end_header_id|>
/// ```
///
/// System messages are consolidated at the beginning to avoid Jinja template
/// errors with models that require system messages to come first.
fn format_llama3(messages: &[ChatMessage]) -> String {
    let mut output = String::from("<|begin_of_text|>");

    // First, output all system messages to satisfy templates that require
    // system messages at the beginning.
    for msg in messages {
        if let ChatMessage::System { content } = msg {
            output.push_str("<|start_header_id|>system<|end_header_id|>\n\n");
            output.push_str(content);
            output.push_str("<|eot_id|>");
        }
    }

    // Then output all non-system messages in order.
    for msg in messages {
        let (role, content) = role_content(msg);
        if role == "system" {
            continue; // Already handled above.
        }

        output.push_str("<|start_header_id|>");
        output.push_str(role);
        output.push_str("<|end_header_id|>\n\n");
        output.push_str(content);
        output.push_str("<|eot_id|>");
    }

    // Add the assistant prefix for generation
    output.push_str("<|start_header_id|>assistant<|end_header_id|>\n\n");
    output
}

/// Format using Mistral template.
///
/// ```text
/// <s>[INST] {system_message}
///
/// {user_message} [/INST]
/// ```
fn format_mistral(messages: &[ChatMessage]) -> String {
    let mut output = String::from("<s>");
    let mut in_inst = false;
    let mut system_content = String::new();

    for msg in messages {
        let (role, content) = role_content(msg);

        match role {
            "system" => {
                // System message is prepended to the first user message
                system_content = content.to_string();
            },
            "user" => {
                if in_inst {
                    output.push_str("</s>");
                }
                output.push_str("[INST] ");
                if !system_content.is_empty() {
                    output.push_str(&system_content);
                    output.push_str("\n\n");
                    system_content.clear();
                }
                output.push_str(content);
                output.push_str(" [/INST]");
                in_inst = true;
            },
            "assistant" => {
                output.push_str(content);
                in_inst = false;
            },
            _ => {},
        }
    }

    output
}

/// Format using DeepSeek template (similar to ChatML).
///
/// ```text
/// <|begin▁of▁sentence|>system
/// {system_message}
/// <|User|>{user_message}
/// <|Assistant|>
/// ```
///
/// System messages are consolidated at the beginning to avoid Jinja template
/// errors with models that require system messages to come first.
fn format_deepseek(messages: &[ChatMessage]) -> String {
    let mut output = String::from("<|begin▁of▁sentence|>");

    // First, output all system messages to satisfy templates that require
    // system messages at the beginning.
    for msg in messages {
        if let ChatMessage::System { content } = msg {
            output.push_str("system\n");
            output.push_str(content);
            output.push('\n');
        }
    }

    // Then output all non-system messages in order.
    for msg in messages {
        let (role, content) = role_content(msg);
        if role == "system" {
            continue; // Already handled above.
        }

        match role {
            "user" => {
                output.push_str("<|User|>");
                output.push_str(content);
                output.push('\n');
            },
            "assistant" => {
                output.push_str("<|Assistant|>");
                output.push_str(content);
                output.push('\n');
            },
            _ => {},
        }
    }

    // Add the assistant prefix for generation
    output.push_str("<|Assistant|>");
    output
}

#[cfg(test)]
mod tests {
    use super::*;

    fn simple_messages() -> Vec<ChatMessage> {
        vec![
            ChatMessage::system("You are a helpful assistant."),
            ChatMessage::user("Hello!"),
        ]
    }

    fn multi_turn_messages() -> Vec<ChatMessage> {
        vec![
            ChatMessage::system("You are a helpful assistant."),
            ChatMessage::user("What is 2+2?"),
            ChatMessage::assistant("4"),
            ChatMessage::user("And 3+3?"),
        ]
    }

    #[test]
    fn test_chatml_format() {
        let result = format_chatml(&simple_messages());
        assert!(result.contains("<|im_start|>system"));
        assert!(result.contains("You are a helpful assistant."));
        assert!(result.contains("<|im_end|>"));
        assert!(result.contains("<|im_start|>user"));
        assert!(result.contains("Hello!"));
        assert!(result.ends_with("<|im_start|>assistant\n"));
    }

    #[test]
    fn test_chatml_multi_turn() {
        let result = format_chatml(&multi_turn_messages());
        assert!(result.contains("<|im_start|>assistant\n4<|im_end|>"));
        assert!(result.contains("And 3+3?"));
    }

    #[test]
    fn test_llama3_format() {
        let result = format_llama3(&simple_messages());
        assert!(result.starts_with("<|begin_of_text|>"));
        assert!(result.contains("<|start_header_id|>system<|end_header_id|>"));
        assert!(result.contains("You are a helpful assistant."));
        assert!(result.contains("<|eot_id|>"));
        assert!(result.contains("<|start_header_id|>user<|end_header_id|>"));
        assert!(result.ends_with("<|start_header_id|>assistant<|end_header_id|>\n\n"));
    }

    #[test]
    fn test_mistral_format() {
        let result = format_mistral(&simple_messages());
        assert!(result.starts_with("<s>"));
        assert!(result.contains("[INST]"));
        assert!(result.contains("You are a helpful assistant."));
        assert!(result.contains("Hello!"));
        assert!(result.contains("[/INST]"));
    }

    #[test]
    fn test_mistral_multi_turn() {
        let result = format_mistral(&multi_turn_messages());
        // Should have both user turns
        assert!(result.contains("What is 2+2?"));
        assert!(result.contains("And 3+3?"));
        // Should have assistant response
        assert!(result.contains("4"));
    }

    #[test]
    fn test_deepseek_format() {
        let result = format_deepseek(&simple_messages());
        assert!(result.starts_with("<|begin▁of▁sentence|>"));
        assert!(result.contains("system\nYou are a helpful assistant."));
        assert!(result.contains("<|User|>Hello!"));
        assert!(result.ends_with("<|Assistant|>"));
    }

    #[test]
    fn test_format_messages_dispatch() {
        let messages = simple_messages();

        let chatml = format_messages(&messages, ChatTemplateHint::ChatML);
        assert!(chatml.contains("<|im_start|>"));

        let llama = format_messages(&messages, ChatTemplateHint::Llama3);
        assert!(llama.contains("<|begin_of_text|>"));

        let mistral = format_messages(&messages, ChatTemplateHint::Mistral);
        assert!(mistral.contains("[INST]"));

        let deepseek = format_messages(&messages, ChatTemplateHint::DeepSeek);
        assert!(deepseek.contains("<|User|>"));

        // Auto should default to ChatML
        let auto = format_messages(&messages, ChatTemplateHint::Auto);
        assert!(auto.contains("<|im_start|>"));
    }

    #[test]
    fn test_chat_template_hint_parse() {
        assert_eq!(ChatTemplateHint::parse("llama3"), ChatTemplateHint::Llama3);
        assert_eq!(ChatTemplateHint::parse("LLAMA"), ChatTemplateHint::Llama3);
        assert_eq!(ChatTemplateHint::parse("chatml"), ChatTemplateHint::ChatML);
        assert_eq!(ChatTemplateHint::parse("qwen"), ChatTemplateHint::ChatML);
        assert_eq!(
            ChatTemplateHint::parse("mistral"),
            ChatTemplateHint::Mistral
        );
        assert_eq!(
            ChatTemplateHint::parse("deepseek"),
            ChatTemplateHint::DeepSeek
        );
        assert_eq!(ChatTemplateHint::parse("unknown"), ChatTemplateHint::Auto);
    }

    #[test]
    fn test_empty_messages() {
        let empty: Vec<ChatMessage> = vec![];
        let result = format_chatml(&empty);
        assert!(result.ends_with("<|im_start|>assistant\n"));

        let result = format_llama3(&empty);
        assert!(result.ends_with("<|start_header_id|>assistant<|end_header_id|>\n\n"));
    }

    #[test]
    fn test_merge_consecutive_system_messages() {
        let messages = vec![
            ChatMessage::system("You are a helpful assistant."),
            ChatMessage::system("Be concise."),
            ChatMessage::user("Hello!"),
        ];

        let merged = merge_consecutive_system_messages(&messages);
        assert_eq!(merged.len(), 2);
        assert!(matches!(merged[0], ChatMessage::System { .. }));
        assert!(matches!(merged[1], ChatMessage::User { .. }));

        // Check that system messages were merged
        if let ChatMessage::System { content } = &merged[0] {
            assert!(content.contains("You are a helpful assistant."));
            assert!(content.contains("Be concise."));
        } else {
            panic!("Expected system message");
        }
    }

    #[test]
    fn test_merge_multiple_system_messages_with_user_between() {
        let messages = vec![
            ChatMessage::system("First system message."),
            ChatMessage::user("User message."),
            ChatMessage::system("Second system message."),
            ChatMessage::system("Third system message."),
            ChatMessage::assistant("Assistant response."),
        ];

        let merged = merge_consecutive_system_messages(&messages);
        assert_eq!(merged.len(), 4);
        assert!(matches!(merged[0], ChatMessage::System { .. }));
        assert!(matches!(merged[1], ChatMessage::User { .. }));
        assert!(matches!(merged[2], ChatMessage::System { .. }));
        assert!(matches!(merged[3], ChatMessage::Assistant { .. }));

        // Check first system message is unchanged
        if let ChatMessage::System { content } = &merged[0] {
            assert_eq!(content, "First system message.");
        }

        // Check second and third system messages were merged
        if let ChatMessage::System { content } = &merged[2] {
            assert!(content.contains("Second system message."));
            assert!(content.contains("Third system message."));
        }
    }

    #[test]
    fn test_no_merge_for_single_system_message() {
        let messages = vec![
            ChatMessage::system("Single system message."),
            ChatMessage::user("Hello!"),
        ];

        let merged = merge_consecutive_system_messages(&messages);
        assert_eq!(merged.len(), 2);

        if let ChatMessage::System { content } = &merged[0] {
            assert_eq!(content, "Single system message.");
        } else {
            panic!("Expected system message");
        }
    }

    #[test]
    fn test_merge_system_messages_at_end() {
        let messages = vec![
            ChatMessage::user("Hello!"),
            ChatMessage::system("First system message."),
            ChatMessage::system("Second system message."),
        ];

        let merged = merge_consecutive_system_messages(&messages);
        assert_eq!(merged.len(), 2);
        assert!(matches!(merged[0], ChatMessage::User { .. }));
        assert!(matches!(merged[1], ChatMessage::System { .. }));

        if let ChatMessage::System { content } = &merged[1] {
            assert!(content.contains("First system message."));
            assert!(content.contains("Second system message."));
        }
    }

    #[test]
    fn test_format_messages_with_merged_system_messages() {
        // Test that format_messages properly merges system messages before formatting
        let messages = vec![
            ChatMessage::system("You are helpful."),
            ChatMessage::system("Be concise."),
            ChatMessage::user("Hello!"),
        ];

        let result = format_messages(&messages, ChatTemplateHint::ChatML);
        // Should only have one system block, not two
        let system_count = result.matches("<|im_start|>system").count();
        assert_eq!(
            system_count, 1,
            "Expected only one system message block after merging"
        );
        assert!(result.contains("You are helpful."));
        assert!(result.contains("Be concise."));
    }
}
