use std::{borrow::Cow, collections::HashSet};

use {
    moltis_agents::runner::RunnerEvent,
    moltis_channels::plugin::{ChannelTaskStatus, ChannelTaskUpdate},
};

pub(super) fn channel_task_update(
    event: &RunnerEvent,
    known_tools: &HashSet<String>,
) -> Option<ChannelTaskUpdate> {
    match event {
        RunnerEvent::ToolCallStart { id, name, .. } => Some(ChannelTaskUpdate {
            id: id.clone(),
            title: safe_title(name, known_tools),
            status: ChannelTaskStatus::InProgress,
        }),
        RunnerEvent::ToolCallEnd {
            id, name, success, ..
        } => Some(ChannelTaskUpdate {
            id: id.clone(),
            title: safe_title(name, known_tools),
            status: if *success {
                ChannelTaskStatus::Complete
            } else {
                ChannelTaskStatus::Error
            },
        }),
        RunnerEvent::ToolCallRejected { id, name, .. } => Some(ChannelTaskUpdate {
            id: id.clone(),
            title: safe_title(name, known_tools),
            status: ChannelTaskStatus::Error,
        }),
        _ => None,
    }
}

fn safe_title(name: &str, known_tools: &HashSet<String>) -> String {
    let canonical = canonical_tool_name(name);
    let canonical = canonical
        .strip_suffix("_wasm")
        .filter(|alias| known_tools.contains(*alias))
        .unwrap_or(canonical.as_ref());
    if known_tools.contains(canonical)
        && !canonical.is_empty()
        && canonical
            .chars()
            .all(|character| character.is_ascii_alphanumeric() || matches!(character, '_' | '-'))
    {
        canonical.to_string()
    } else {
        "tool".to_string()
    }
}

fn canonical_tool_name(name: &str) -> Cow<'_, str> {
    let trimmed = name.trim();
    let unquoted = trimmed
        .strip_prefix('"')
        .and_then(|value| value.strip_suffix('"'))
        .unwrap_or(trimmed);
    let without_prefix = unquoted.strip_prefix("functions_").unwrap_or(unquoted);
    let canonical = without_prefix
        .rfind('_')
        .and_then(|position| {
            let suffix = &without_prefix[position + 1..];
            (!suffix.is_empty() && suffix.bytes().all(|byte| byte.is_ascii_digit()) && position > 0)
                .then_some(&without_prefix[..position])
        })
        .unwrap_or(without_prefix);
    if canonical == name {
        Cow::Borrowed(name)
    } else {
        Cow::Owned(canonical.to_string())
    }
}
