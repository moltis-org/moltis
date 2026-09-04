use std::collections::HashMap;

use crate::types::{ExternalAgentEvent, TokenUsage};

use super::wire::{AgyEvent, AgyState, AgyStepType, AgyStepUpdate, AgySubagent, AgyUsage};

/// Stateful conversion from AGY's versioned wire frames to Moltis' stable
/// external-agent event contract.
#[derive(Debug, Default)]
pub(super) struct Translator {
    session_id: Option<String>,
    buffered_text: String,
    tool_names: HashMap<String, String>,
    failed_steps: u32,
    last_usage: Option<TokenUsage>,
}

impl Translator {
    pub(super) fn with_session_id(session_id: Option<String>) -> Self {
        Self {
            session_id,
            ..Self::default()
        }
    }

    pub(super) fn translate(&mut self, event: AgyEvent) -> Vec<ExternalAgentEvent> {
        match event {
            AgyEvent::Init(init) => self
                .bind_session(init.conversation_id)
                .into_iter()
                .collect(),
            AgyEvent::StepUpdate(step) => self.translate_step(step),
            AgyEvent::Result(result) => {
                let mut events = Vec::new();
                if let Some(bound) = self.bind_session(result.conversation_id) {
                    events.push(bound);
                }

                let usage = result
                    .usage
                    .as_ref()
                    .map(token_usage)
                    .or_else(|| self.last_usage.take());
                let status = result.status.to_ascii_uppercase();
                if status != "SUCCESS" {
                    self.failed_steps = 0;
                    self.buffered_text.clear();
                    let raw = result.error.unwrap_or(result.response);
                    let message = if status == "INTERRUPTED" && raw.trim().is_empty() {
                        "AGY turn was interrupted".to_string()
                    } else {
                        explain_error(&raw)
                    };
                    events.push(ExternalAgentEvent::Error(message));
                    return events;
                }

                if self.failed_steps > 0 {
                    let count = std::mem::take(&mut self.failed_steps);
                    events.push(ExternalAgentEvent::Notice(format!(
                        "{count} AGY step{} failed and did not take effect; verify the result before relying on the reply.",
                        if count == 1 { "" } else { "s" }
                    )));
                }

                let text = if result.response.is_empty() {
                    std::mem::take(&mut self.buffered_text)
                } else {
                    self.buffered_text.clear();
                    result.response
                };
                if !text.is_empty() {
                    events.push(ExternalAgentEvent::TextDelta(text));
                }
                events.push(ExternalAgentEvent::Done { usage });
                events
            },
        }
    }

    pub(super) fn take_partial_text(&mut self) -> Option<String> {
        let text = std::mem::take(&mut self.buffered_text);
        (!text.is_empty()).then_some(text)
    }

    fn bind_session(&mut self, candidate: String) -> Option<ExternalAgentEvent> {
        if candidate.trim().is_empty() || self.session_id.as_deref() == Some(candidate.as_str()) {
            return None;
        }
        self.session_id = Some(candidate.clone());
        Some(ExternalAgentEvent::SessionBound {
            external_session_id: candidate,
        })
    }

    fn translate_step(&mut self, step: AgyStepUpdate) -> Vec<ExternalAgentEvent> {
        if let Some(usage) = step.usage.as_ref() {
            self.last_usage = Some(token_usage(usage));
        }
        match step.step_type {
            AgyStepType::AgentResponse => {
                if let Some(delta) = step.text_delta {
                    self.buffered_text.push_str(&delta);
                }
                Vec::new()
            },
            AgyStepType::Tool => self.translate_tool(step),
            AgyStepType::Subagent => self.translate_subagent(step),
            AgyStepType::ErrorMessage => {
                if step.state == AgyState::Done {
                    self.failed_steps = self.failed_steps.saturating_add(1);
                }
                Vec::new()
            },
            AgyStepType::UserInput
            | AgyStepType::Checkpoint
            | AgyStepType::SystemMessage
            | AgyStepType::Unknown => Vec::new(),
        }
    }

    fn translate_tool(&mut self, step: AgyStepUpdate) -> Vec<ExternalAgentEvent> {
        let id = self.step_id(&step);
        let raw_name = step
            .tool_info
            .as_ref()
            .map(|info| info.name.trim())
            .filter(|name| !name.is_empty())
            .map(str::to_string)
            .or(step.tool_name.clone())
            .unwrap_or_else(|| "tool".to_string());
        let name = display_tool_name(
            &raw_name,
            step.tool_info
                .as_ref()
                .and_then(|info| info.parameters.as_ref()),
        );

        match step.state {
            AgyState::Active => {
                self.tool_names.insert(id.clone(), name.clone());
                let arguments = step
                    .tool_info
                    .and_then(|info| info.parameters)
                    .map(|value| value.to_string())
                    .unwrap_or_else(|| "{}".to_string());
                vec![ExternalAgentEvent::ToolCallStart {
                    id,
                    name,
                    arguments,
                }]
            },
            AgyState::Done | AgyState::Error => {
                let name = self.tool_names.remove(&id).unwrap_or(name);
                let success = step.state == AgyState::Done;
                let result = step.tool_info.and_then(|info| {
                    if success {
                        info.output
                    } else {
                        info.error
                            .map(|error| error.message)
                            .or(info.output)
                            .or_else(|| Some("AGY tool failed".to_string()))
                    }
                });
                vec![ExternalAgentEvent::ToolCallEnd {
                    id,
                    name,
                    success,
                    result,
                }]
            },
            AgyState::Unknown => Vec::new(),
        }
    }

    fn translate_subagent(&mut self, step: AgyStepUpdate) -> Vec<ExternalAgentEvent> {
        let id = self.step_id(&step);
        let subagents = step
            .subagent_info
            .map(|info| info.subagents)
            .unwrap_or_default();
        let name = subagent_name(&subagents);
        match step.state {
            AgyState::Active => {
                self.tool_names.insert(id.clone(), name.clone());
                vec![ExternalAgentEvent::ToolCallStart {
                    id,
                    name,
                    arguments: subagent_arguments(&subagents).to_string(),
                }]
            },
            AgyState::Done | AgyState::Error => vec![ExternalAgentEvent::ToolCallEnd {
                name: self.tool_names.remove(&id).unwrap_or(name),
                id,
                success: step.state == AgyState::Done,
                result: None,
            }],
            AgyState::Unknown => Vec::new(),
        }
    }

    fn step_id(&self, step: &AgyStepUpdate) -> String {
        let namespace = (!step.conversation_id.is_empty())
            .then_some(step.conversation_id.as_str())
            .or(self.session_id.as_deref())
            .unwrap_or("unbound");
        format!("agy-{namespace}-step-{}", step.step_index)
    }
}

fn display_tool_name(raw_name: &str, parameters: Option<&serde_json::Value>) -> String {
    if raw_name != "call_mcp_tool" {
        return raw_name.to_string();
    }
    let Some(parameters) = parameters else {
        return raw_name.to_string();
    };
    match (
        parameters
            .get("ServerName")
            .and_then(serde_json::Value::as_str),
        parameters
            .get("ToolName")
            .and_then(serde_json::Value::as_str),
    ) {
        (Some(server), Some(tool)) => format!("{server}/{tool}"),
        (None, Some(tool)) => tool.to_string(),
        _ => raw_name.to_string(),
    }
}

fn subagent_name(subagents: &[AgySubagent]) -> String {
    match subagents {
        [] => "subagent".to_string(),
        [one] => {
            let label = [one.role.trim(), one.type_name.trim()]
                .into_iter()
                .find(|value| !value.is_empty())
                .unwrap_or("subagent");
            format!("subagent: {label}")
        },
        many => format!("{} subagents", many.len()),
    }
}

fn subagent_arguments(subagents: &[AgySubagent]) -> serde_json::Value {
    serde_json::Value::Array(
        subagents
            .iter()
            .map(|subagent| {
                serde_json::json!({
                    "type": subagent.type_name,
                    "role": subagent.role,
                    "prompt": subagent.initial_prompt,
                })
            })
            .collect(),
    )
}

fn token_usage(usage: &AgyUsage) -> TokenUsage {
    TokenUsage {
        input_tokens: u32::try_from(usage.input_tokens).unwrap_or(u32::MAX),
        output_tokens: u32::try_from(usage.output_tokens).unwrap_or(u32::MAX),
    }
}

fn explain_error(raw: &str) -> String {
    let lower = raw.to_ascii_lowercase();
    if lower.contains("authentication")
        || lower.contains("not logged in")
        || lower.contains("sign in")
    {
        return format!(
            "Antigravity is not signed in. Run `agy` in a terminal to complete Google OAuth, then try again. (agy: {raw})"
        );
    }
    if raw.trim().is_empty() {
        "AGY turn failed without an error message".to_string()
    } else {
        raw.to_string()
    }
}

#[cfg(test)]
mod tests {
    use {super::*, crate::runtimes::antigravity::wire::parse_line};

    fn translate(lines: &[&str]) -> Vec<ExternalAgentEvent> {
        let mut translator = Translator::default();
        lines
            .iter()
            .filter_map(|line| parse_line(line))
            .flat_map(|event| translator.translate(event))
            .collect()
    }

    #[test]
    fn tool_events_are_paired_and_mcp_target_is_visible() {
        let events = translate(&[
            r#"{"event":"init","conversation_id":"conv-1","init":{}}"#,
            r#"{"event":"step_update","step_update":{"step_index":3,"state":"ACTIVE","step_type":"tool","tool_name":"call_mcp_tool","tool_info":{"name":"call_mcp_tool","parameters":{"ServerName":"tra","ToolName":"tra_capability","Arguments":{}}}}}"#,
            r#"{"event":"step_update","step_update":{"step_index":3,"state":"DONE","step_type":"tool","tool_name":"call_mcp_tool","tool_info":{"name":"call_mcp_tool","output":"ok"}}}"#,
        ]);

        assert!(matches!(
            &events[1],
            ExternalAgentEvent::ToolCallStart { id, name, .. }
                if id == "agy-conv-1-step-3" && name == "tra/tra_capability"
        ));
        assert!(matches!(
            &events[2],
            ExternalAgentEvent::ToolCallEnd { id, success: true, result: Some(result), .. }
                if id == "agy-conv-1-step-3" && result == "ok"
        ));
    }

    #[test]
    fn intermediate_text_is_buffered_and_terminal_text_is_authoritative() {
        let events = translate(&[
            r#"{"event":"step_update","step_update":{"step_index":2,"state":"ACTIVE","step_type":"agent_response","text_delta":"broken partial"}}"#,
            r#"{"event":"result","result":{"conversation_id":"conv-1","status":"SUCCESS","response":"正確答案","usage":{"input_tokens":9,"output_tokens":4}}}"#,
        ]);

        assert!(!events.iter().any(
            |event| matches!(event, ExternalAgentEvent::TextDelta(text) if text == "broken partial")
        ));
        assert!(events.iter().any(
            |event| matches!(event, ExternalAgentEvent::TextDelta(text) if text == "正確答案")
        ));
        assert!(
            events
                .iter()
                .any(|event| matches!(event, ExternalAgentEvent::Done {
                    usage: Some(TokenUsage {
                        input_tokens: 9,
                        output_tokens: 4
                    })
                }))
        );
    }

    #[test]
    fn subagent_dispatch_is_exposed_without_private_handles() {
        let events = translate(&[
            r#"{"event":"step_update","step_update":{"conversation_id":"conv-1","step_index":7,"state":"ACTIVE","step_type":"subagent","subagent_info":{"subagents":[{"type_name":"research","role":"Source Scout","initial_prompt":"Find primary sources","conversation_id":"private-id","log_uri":"file:///private/log"}]}}}"#,
        ]);
        let ExternalAgentEvent::ToolCallStart {
            name, arguments, ..
        } = &events[0]
        else {
            panic!("expected subagent start");
        };
        assert!(name.contains("Source Scout"));
        assert!(arguments.contains("Find primary sources"));
        assert!(!arguments.contains("private-id"));
        assert!(!arguments.contains("private/log"));
    }

    #[test]
    fn auth_failure_is_actionable() {
        let events = translate(&[
            r#"{"event":"result","result":{"conversation_id":"","status":"ERROR","error":"authentication failed or timed out"}}"#,
        ]);
        assert!(matches!(
            &events[0],
            ExternalAgentEvent::Error(message)
                if message.contains("Google OAuth") && message.contains("agy")
        ));
    }
}
