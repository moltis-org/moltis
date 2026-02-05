//! Chat template formatting for various model families.
//!
//! Different LLM families use different prompt formats. This module provides
//! template formatting for Llama3, ChatML (Qwen/Kimi), Mistral, and DeepSeek.

use serde_json::Value;

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

/// Format messages using the specified chat template.
///
/// Messages should be JSON objects with "role" and "content" fields.
#[must_use]
pub fn format_messages(messages: &[Value], hint: ChatTemplateHint) -> String {
    match hint {
        ChatTemplateHint::Auto | ChatTemplateHint::ChatML => format_chatml(messages),
        ChatTemplateHint::Llama3 => format_llama3(messages),
        ChatTemplateHint::Mistral => format_mistral(messages),
        ChatTemplateHint::DeepSeek => format_deepseek(messages),
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
fn format_chatml(messages: &[Value]) -> String {
    let mut output = String::new();

    for msg in messages {
        let role = msg["role"].as_str().unwrap_or("user");
        let content = msg["content"].as_str().unwrap_or("");

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
fn format_llama3(messages: &[Value]) -> String {
    let mut output = String::from("<|begin_of_text|>");

    for msg in messages {
        let role = msg["role"].as_str().unwrap_or("user");
        let content = msg["content"].as_str().unwrap_or("");

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
fn format_mistral(messages: &[Value]) -> String {
    let mut output = String::from("<s>");
    let mut in_inst = false;
    let mut system_content = String::new();

    for msg in messages {
        let role = msg["role"].as_str().unwrap_or("user");
        let content = msg["content"].as_str().unwrap_or("");

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
fn format_deepseek(messages: &[Value]) -> String {
    let mut output = String::from("<|begin▁of▁sentence|>");

    for msg in messages {
        let role = msg["role"].as_str().unwrap_or("user");
        let content = msg["content"].as_str().unwrap_or("");

        match role {
            "system" => {
                output.push_str("system\n");
                output.push_str(content);
                output.push('\n');
            },
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
    use {super::*, serde_json::json};

    fn simple_messages() -> Vec<Value> {
        vec![
            json!({"role": "system", "content": "You are a helpful assistant."}),
            json!({"role": "user", "content": "Hello!"}),
        ]
    }

    fn multi_turn_messages() -> Vec<Value> {
        vec![
            json!({"role": "system", "content": "You are a helpful assistant."}),
            json!({"role": "user", "content": "What is 2+2?"}),
            json!({"role": "assistant", "content": "4"}),
            json!({"role": "user", "content": "And 3+3?"}),
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
        let empty: Vec<Value> = vec![];
        let result = format_chatml(&empty);
        assert!(result.ends_with("<|im_start|>assistant\n"));

        let result = format_llama3(&empty);
        assert!(result.ends_with("<|start_header_id|>assistant<|end_header_id|>\n\n"));
    }
}
