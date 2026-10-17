//! The displayed conversation of one session, built from `chat.history` and
//! kept current by `chat` events. Pure state, no UI.

use {
    moltis_protocol::{
        chat_event::{ChatErrorInfo, ChatEventKind, ChatFinal},
        message::{ContentBlock, MessageContent, PersistedMessage},
    },
    serde_json::Value,
};

/// Longest tool output kept for display.
const MAX_TOOL_OUTPUT: usize = 4_000;

#[derive(Debug, Clone, PartialEq)]
pub struct Usage {
    pub input_tokens: u32,
    pub output_tokens: u32,
    pub duration_ms: u64,
}

impl Usage {
    /// Output tokens per second, when measurable.
    #[must_use]
    pub fn tokens_per_second(&self) -> Option<f64> {
        (self.duration_ms > 0 && self.output_tokens > 0)
            .then(|| f64::from(self.output_tokens) * 1000.0 / self.duration_ms as f64)
    }
}

#[derive(Debug, Clone, PartialEq, Default)]
pub struct AssistantMessage {
    pub text: String,
    pub reasoning: Option<String>,
    pub model: Option<String>,
    pub usage: Option<Usage>,
    /// Still receiving deltas.
    pub streaming: bool,
    /// Stopped by the user before it finished.
    pub aborted: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ToolStatus {
    Running,
    Succeeded,
    Failed,
}

#[derive(Debug, Clone, PartialEq)]
pub struct ToolCall {
    pub id: String,
    pub name: String,
    /// One-line summary of the arguments (e.g. the shell command).
    pub summary: String,
    pub status: ToolStatus,
    pub output: Option<String>,
}

#[derive(Debug, Clone, PartialEq)]
pub enum ItemKind {
    User { text: String },
    Assistant(AssistantMessage),
    Tool(ToolCall),
    Notice { title: String, message: String },
    Error { message: String },
}

/// One row of the transcript. `id` is stable across updates.
#[derive(Debug, Clone, PartialEq)]
pub struct Item {
    pub id: u64,
    pub kind: ItemKind,
}

/// What a run is doing right now.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum RunState {
    #[default]
    Idle,
    /// Accepted, waiting for the model.
    Thinking,
    Streaming,
    RunningTool,
}

/// How the item list changed, so a list view can update incrementally.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Change {
    None,
    Appended { at: usize, count: usize },
    Updated { index: usize },
    Removed { index: usize },
    Reset,
}

#[derive(Debug, Default)]
pub struct Transcript {
    items: Vec<Item>,
    next_id: u64,
    state: RunState,
    /// Index of the assistant message receiving deltas.
    draft: Option<usize>,
}

impl Transcript {
    #[must_use]
    pub fn from_history(messages: &[PersistedMessage]) -> Self {
        let mut transcript = Self::default();
        for message in messages {
            if let Some(kind) = item_from_history(message) {
                transcript.push(kind);
            }
        }
        transcript
    }

    #[must_use]
    pub fn items(&self) -> &[Item] {
        &self.items
    }

    #[must_use]
    pub fn state(&self) -> RunState {
        self.state
    }

    fn push(&mut self, kind: ItemKind) -> Change {
        self.next_id += 1;
        self.items.push(Item {
            id: self.next_id,
            kind,
        });
        Change::Appended {
            at: self.items.len() - 1,
            count: 1,
        }
    }

    fn draft_mut(&mut self) -> Option<&mut AssistantMessage> {
        let index = self.draft?;
        match &mut self.items.get_mut(index)?.kind {
            ItemKind::Assistant(message) => Some(message),
            _ => None,
        }
    }

    /// The draft's index, creating an empty streaming message if needed.
    fn ensure_draft(&mut self) -> (usize, bool) {
        if let Some(index) = self.draft
            && matches!(
                self.items.get(index).map(|item| &item.kind),
                Some(ItemKind::Assistant(_))
            )
        {
            return (index, false);
        }
        self.push(ItemKind::Assistant(AssistantMessage {
            streaming: true,
            ..AssistantMessage::default()
        }));
        let index = self.items.len() - 1;
        self.draft = Some(index);
        (index, true)
    }

    fn changed(index: usize, created: bool) -> Change {
        if created {
            Change::Appended {
                at: index,
                count: 1,
            }
        } else {
            Change::Updated { index }
        }
    }

    /// Two changes in one event: report the second alone only if the first
    /// changed nothing, otherwise ask the view to rebuild.
    fn combine(first: Change, second: Change) -> Change {
        if first == Change::None {
            second
        } else {
            Change::Reset
        }
    }

    /// End the current draft. An empty one is removed.
    fn close_draft(&mut self) -> Change {
        let Some(index) = self.draft.take() else {
            return Change::None;
        };
        let empty = match self.items.get_mut(index).map(|item| &mut item.kind) {
            Some(ItemKind::Assistant(message)) => {
                message.streaming = false;
                message.text.is_empty() && message.reasoning.is_none()
            },
            _ => return Change::None,
        };
        if empty {
            self.items.remove(index);
            Change::Removed { index }
        } else {
            Change::Updated { index }
        }
    }

    /// Apply one `chat` event of this session.
    pub fn apply(&mut self, event: &ChatEventKind) -> Change {
        match event {
            ChatEventKind::UserMessage { text } => {
                self.state = RunState::Thinking;
                self.push(ItemKind::User { text: text.clone() })
            },
            ChatEventKind::Thinking => {
                self.state = RunState::Thinking;
                Change::None
            },
            ChatEventKind::ThinkingDone => Change::None,
            ChatEventKind::ThinkingText { text } => {
                let (index, created) = self.ensure_draft();
                if let Some(draft) = self.draft_mut() {
                    draft.reasoning = Some(text.clone());
                }
                Self::changed(index, created)
            },
            ChatEventKind::Delta { text } => {
                self.state = RunState::Streaming;
                let (index, created) = self.ensure_draft();
                if let Some(draft) = self.draft_mut() {
                    draft.text.push_str(text);
                }
                Self::changed(index, created)
            },
            ChatEventKind::ToolCallStart {
                tool_call_id,
                tool_name,
                arguments,
                ..
            } => {
                self.state = RunState::RunningTool;
                let closed = self.close_draft();
                let pushed = self.push_tool(tool_call_id, tool_name, arguments.as_ref());
                Self::combine(closed, pushed)
            },
            ChatEventKind::ToolCallEnd {
                tool_call_id,
                success,
                result,
                error,
                ..
            } => {
                self.state = RunState::Thinking;
                let Some(index) = self.items.iter().rposition(
                    |item| matches!(&item.kind, ItemKind::Tool(tool) if tool.id == *tool_call_id),
                ) else {
                    return Change::None;
                };
                if let ItemKind::Tool(tool) = &mut self.items[index].kind {
                    tool.status = if *success {
                        ToolStatus::Succeeded
                    } else {
                        ToolStatus::Failed
                    };
                    tool.output = if *success {
                        result.as_ref().map(tool_output)
                    } else {
                        error.as_ref().map(error_text)
                    };
                }
                Change::Updated { index }
            },
            ChatEventKind::Final(done) => {
                self.state = RunState::Idle;
                self.finish(done)
            },
            ChatEventKind::Error { error } => {
                self.state = RunState::Idle;
                let closed = self.close_draft();
                let pushed = self.push(ItemKind::Error {
                    message: error.display_text().to_string(),
                });
                Self::combine(closed, pushed)
            },
            ChatEventKind::Retrying {
                error,
                retry_after_ms,
            } => self.push(ItemKind::Notice {
                title: "Retrying".into(),
                message: retry_message(error, *retry_after_ms),
            }),
            ChatEventKind::Aborted {
                partial_message, ..
            } => {
                self.state = RunState::Idle;
                if self.draft.is_none()
                    && let Some(content) = partial_message
                        .as_ref()
                        .and_then(|p| p.content.clone())
                        .filter(|c| !c.is_empty())
                {
                    self.push(ItemKind::Assistant(AssistantMessage {
                        text: content,
                        aborted: true,
                        ..AssistantMessage::default()
                    }));
                    return Change::Reset;
                }
                if let Some(draft) = self.draft_mut() {
                    draft.aborted = true;
                }
                match self.close_draft() {
                    Change::Removed { .. } => Change::Reset,
                    change => change,
                }
            },
            ChatEventKind::Notice { title, message } => self.push(ItemKind::Notice {
                title: title.clone(),
                message: message.clone(),
            }),
            ChatEventKind::Queued => self.push(ItemKind::Notice {
                title: "Queued".into(),
                message: "The message will be sent when the current reply finishes.".into(),
            }),
            ChatEventKind::QueueCleared | ChatEventKind::Other => Change::None,
            ChatEventKind::SessionCleared => {
                *self = Self {
                    next_id: self.next_id,
                    ..Self::default()
                };
                Change::Reset
            },
        }
    }

    fn push_tool(&mut self, id: &str, name: &str, arguments: Option<&Value>) -> Change {
        self.push(ItemKind::Tool(ToolCall {
            id: id.into(),
            name: name.into(),
            summary: arguments.map(argument_summary).unwrap_or_default(),
            status: ToolStatus::Running,
            output: None,
        }))
    }

    fn finish(&mut self, done: &ChatFinal) -> Change {
        let usage = Some(Usage {
            input_tokens: done.input_tokens,
            output_tokens: done.output_tokens,
            duration_ms: done.duration_ms,
        });
        let model = (!done.model.is_empty()).then(|| done.model.clone());
        if self.draft.is_some() {
            let index = self.draft.take();
            if let Some(index) = index
                && let Some(ItemKind::Assistant(message)) =
                    self.items.get_mut(index).map(|item| &mut item.kind)
            {
                if !done.text.is_empty() {
                    message.text.clone_from(&done.text);
                }
                if done.reasoning.is_some() {
                    message.reasoning.clone_from(&done.reasoning);
                }
                message.model = model;
                message.usage = usage;
                message.streaming = false;
                return Change::Updated { index };
            }
        }
        if done.text.is_empty() {
            return Change::None;
        }
        self.push(ItemKind::Assistant(AssistantMessage {
            text: done.text.clone(),
            reasoning: done.reasoning.clone(),
            model,
            usage,
            streaming: false,
            aborted: false,
        }))
    }
}

fn item_from_history(message: &PersistedMessage) -> Option<ItemKind> {
    match message {
        PersistedMessage::User { content, .. } => Some(ItemKind::User {
            text: user_text(content),
        }),
        PersistedMessage::Assistant {
            content,
            reasoning,
            model,
            output_tokens,
            input_tokens,
            duration_ms,
            ..
        } => {
            if content.is_empty() && reasoning.is_none() {
                return None;
            }
            let usage = output_tokens.map(|output| Usage {
                input_tokens: input_tokens.unwrap_or_default(),
                output_tokens: output,
                duration_ms: duration_ms.unwrap_or_default(),
            });
            Some(ItemKind::Assistant(AssistantMessage {
                text: content.clone(),
                reasoning: reasoning.clone(),
                model: model.clone(),
                usage,
                streaming: false,
                aborted: false,
            }))
        },
        PersistedMessage::ToolResult {
            tool_call_id,
            tool_name,
            arguments,
            success,
            result,
            error,
            ..
        } => Some(ItemKind::Tool(ToolCall {
            id: tool_call_id.clone(),
            name: tool_name.clone(),
            summary: arguments.as_ref().map(argument_summary).unwrap_or_default(),
            status: if *success {
                ToolStatus::Succeeded
            } else {
                ToolStatus::Failed
            },
            output: if *success {
                result.as_ref().map(tool_output)
            } else {
                error.clone()
            },
        })),
        PersistedMessage::Notice { content, .. } => Some(ItemKind::Notice {
            title: String::new(),
            message: content.clone(),
        }),
        PersistedMessage::System { .. } | PersistedMessage::Tool { .. } => None,
    }
}

fn user_text(content: &MessageContent) -> String {
    match content {
        MessageContent::Text(text) => text.clone(),
        MessageContent::Multimodal(blocks) => blocks
            .iter()
            .map(|block| match block {
                ContentBlock::Text { text } => text.as_str(),
                ContentBlock::ImageUrl { .. } => "[image]",
            })
            .collect::<Vec<_>>()
            .join("\n"),
    }
}

fn truncate(text: &str, max: usize) -> String {
    if text.len() <= max {
        return text.to_string();
    }
    let mut end = max;
    while !text.is_char_boundary(end) {
        end -= 1;
    }
    format!("{}…", &text[..end])
}

/// The most telling argument (command, path, query, url), else compact JSON.
fn argument_summary(arguments: &Value) -> String {
    let preferred = [
        "command",
        "cmd",
        "path",
        "file_path",
        "query",
        "url",
        "pattern",
    ];
    let line = preferred
        .iter()
        .find_map(|key| arguments.get(key).and_then(Value::as_str))
        .map(String::from)
        .unwrap_or_else(|| match arguments {
            Value::Object(map) if map.is_empty() => String::new(),
            other => other.to_string(),
        });
    truncate(line.lines().next().unwrap_or_default(), 200)
}

fn tool_output(result: &Value) -> String {
    let text = match (
        result.get("stdout").and_then(Value::as_str),
        result.get("stderr").and_then(Value::as_str),
    ) {
        (None, None) => match result {
            Value::String(text) => text.clone(),
            other => serde_json::to_string_pretty(other).unwrap_or_default(),
        },
        (stdout, stderr) => [stdout, stderr]
            .into_iter()
            .flatten()
            .filter(|s| !s.trim().is_empty())
            .collect::<Vec<_>>()
            .join("\n"),
    };
    truncate(text.trim_end(), MAX_TOOL_OUTPUT)
}

fn error_text(error: &Value) -> String {
    let text = match error {
        Value::String(text) => text.clone(),
        other => other
            .get("message")
            .or_else(|| other.get("detail"))
            .and_then(Value::as_str)
            .map_or_else(|| other.to_string(), String::from),
    };
    truncate(&text, MAX_TOOL_OUTPUT)
}

fn retry_message(error: &ChatErrorInfo, retry_after_ms: Option<u64>) -> String {
    match retry_after_ms {
        Some(ms) => format!(
            "{} — retrying in {}s",
            error.display_text(),
            ms.div_ceil(1000)
        ),
        None => error.display_text().to_string(),
    }
}

#[cfg(test)]
#[path = "transcript_tests.rs"]
mod tests;
